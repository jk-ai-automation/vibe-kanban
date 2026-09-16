mod handler;
mod tools;

use std::path::Path;

use anyhow::Context;
use db::models::{requests::ContainerQuery, workspace::WorkspaceContext};
use rmcp::{handler::server::tool::ToolRouter, schemars};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) use crate::ApiResponseEnvelope;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct McpRepoContext {
    #[schemars(description = "The unique identifier of the repository")]
    pub repo_id: Uuid,
    #[schemars(description = "The name of the repository")]
    pub repo_name: String,
    #[schemars(description = "The target branch for this repository in this workspace")]
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct McpContext {
    #[schemars(description = "The organization ID (if workspace is linked to remote)")]
    pub organization_id: Option<Uuid>,
    #[schemars(description = "The remote project ID (if workspace is linked to remote)")]
    pub project_id: Option<Uuid>,
    #[schemars(description = "The remote issue ID (if workspace is linked to a remote issue)")]
    pub issue_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "The orchestrator session ID when running in orchestrator mode")]
    pub orchestrator_session_id: Option<Uuid>,
    pub workspace_id: Uuid,
    pub workspace_branch: String,
    #[schemars(
        description = "Repository info and target branches for each repo in this workspace"
    )]
    pub workspace_repos: Vec<McpRepoContext>,
}

#[derive(Debug, Clone)]
pub enum McpMode {
    Global,
    Orchestrator,
}

#[derive(Debug, Clone)]
pub struct McpServer {
    client: reqwest::Client,
    base_url: String,
    tool_router: ToolRouter<McpServer>,
    context: Option<McpContext>,
    mode: McpMode,
}

/// 由本机令牌拼出的默认请求头。
///
/// 读不到令牌就返回空表——**不加空头**：服务端对空令牌一律不匹配
/// （`LocalAuthRuntime::machine_token_matches`），加一个空头只会被明确拒掉，
/// 而个人模式下服务端本来就不看这个头。
fn machine_token_headers(token: Option<String>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    // 只接受可打印 ASCII。我们生成的令牌是 base64url，本来就满足；
    // `HeaderValue::from_str` 其实会放行 0x80..0xFF（obs-text），
    // 但那样的头在服务端 `to_str()` 会失败而被当成「没带令牌」，
    // 与其发一个注定无效的头，不如在这里就拒掉。
    if let Some(token) = token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_graphic()))
        && let Ok(value) = reqwest::header::HeaderValue::from_str(&token)
    {
        headers.insert(utils::assets::MACHINE_TOKEN_HEADER, value);
    }
    headers
}

/// 带本机令牌默认头的 HTTP 客户端。
///
/// 团队模式（`VK_MODE=team`）下 `/api/*` 全线要求会话，而本进程既没有
/// Cookie 也没有 Origin；这个头是它唯一的等价凭据。
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .default_headers(machine_token_headers(utils::assets::read_machine_token()))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl McpServer {
    pub fn new_global(base_url: &str) -> Self {
        Self {
            client: build_client(),
            base_url: base_url.to_string(),
            tool_router: Self::global_mode_router(),
            context: None,
            mode: McpMode::Global,
        }
    }

    pub fn new_orchestrator(base_url: &str) -> Self {
        Self {
            client: build_client(),
            base_url: base_url.to_string(),
            tool_router: Self::orchestrator_mode_router(),
            context: None,
            mode: McpMode::Orchestrator,
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub async fn init(mut self) -> anyhow::Result<Self> {
        let context = self.fetch_context_at_startup().await?;

        if context.is_none() {
            self.tool_router.map.remove("get_context");
            tracing::debug!("VK context not available, get_context tool will not be registered");
        } else {
            tracing::info!("VK context loaded, get_context tool available");
        }

        self.context = context;
        Ok(self)
    }

    pub fn mode(&self) -> &McpMode {
        &self.mode
    }

    async fn fetch_context_at_startup(&self) -> anyhow::Result<Option<McpContext>> {
        let current_dir = std::env::current_dir().context("Failed to resolve current directory")?;
        let canonical_path = current_dir.canonicalize().unwrap_or(current_dir);
        let normalized_path = utils::path::normalize_macos_private_alias(&canonical_path);

        match self.try_fetch_attempt_context(&normalized_path).await {
            Ok(Some(ctx)) => Ok(Some(
                self.build_mcp_context_from_workspace_context(&ctx).await,
            )),
            Ok(None) | Err(_) if matches!(self.mode(), McpMode::Global) => Ok(None),
            Ok(None) => anyhow::bail!(
                "Failed to load orchestrator MCP context from /api/containers/attempt-context"
            ),
            Err(error) => Err(error.context("Failed to load orchestrator MCP context")),
        }
    }

    async fn try_fetch_attempt_context(
        &self,
        path: &Path,
    ) -> anyhow::Result<Option<WorkspaceContext>> {
        let url = self.url("/api/containers/attempt-context");
        let query = ContainerQuery {
            container_ref: path.to_string_lossy().to_string(),
        };

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.client.get(&url).query(&query).send(),
        )
        .await
        .context("Timed out fetching /api/containers/attempt-context")?
        .context("Failed to fetch /api/containers/attempt-context")?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let api_response: ApiResponseEnvelope<WorkspaceContext> = response
            .json()
            .await
            .context("Failed to parse /api/containers/attempt-context response")?;

        if !api_response.success {
            return Ok(None);
        }

        Ok(api_response.data)
    }

    async fn build_mcp_context_from_workspace_context(&self, ctx: &WorkspaceContext) -> McpContext {
        let workspace_repos: Vec<McpRepoContext> = ctx
            .workspace_repos
            .iter()
            .map(|rwb| McpRepoContext {
                repo_id: rwb.repo.id,
                repo_name: rwb.repo.name.clone(),
                target_branch: rwb.target_branch.clone(),
            })
            .collect();

        let workspace_id = ctx.workspace.id;
        let workspace_branch = ctx.workspace.branch.clone();
        let orchestrator_session_id = if matches!(self.mode(), McpMode::Orchestrator) {
            ctx.orchestrator_session_id
        } else {
            None
        };

        let (project_id, issue_id, organization_id) = self
            .fetch_remote_workspace_context(workspace_id)
            .await
            .unwrap_or((None, None, None));

        McpContext {
            organization_id,
            project_id,
            issue_id,
            orchestrator_session_id,
            workspace_id,
            workspace_branch,
            workspace_repos,
        }
    }

    async fn fetch_remote_workspace_context(
        &self,
        local_workspace_id: Uuid,
    ) -> Option<(Option<Uuid>, Option<Uuid>, Option<Uuid>)> {
        let url = self.url(&format!(
            "/api/remote/workspaces/by-local-id/{}",
            local_workspace_id
        ));

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(2000),
            self.client.get(&url).send(),
        )
        .await
        .ok()?
        .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let api_response: ApiResponseEnvelope<api_types::Workspace> = response.json().await.ok()?;

        if !api_response.success {
            return None;
        }

        let remote_ws = api_response.data?;
        let project_id = remote_ws.project_id;

        // Fetch the project to get organization_id
        let org_id = self.fetch_remote_organization_id(project_id).await;

        Some((Some(project_id), remote_ws.issue_id, org_id))
    }

    async fn fetch_remote_organization_id(&self, project_id: Uuid) -> Option<Uuid> {
        let url = self.url(&format!("/api/remote/projects/{}", project_id));

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(2000),
            self.client.get(&url).send(),
        )
        .await
        .ok()?
        .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let api_response: ApiResponseEnvelope<api_types::Project> = response.json().await.ok()?;
        let project = api_response.data?;
        Some(project.organization_id)
    }
}

#[cfg(test)]
mod machine_token_tests {
    use super::*;

    /// 团队模式下这是 MCP 唯一的凭据。头名写错、令牌被改动，
    /// 表现都是「所有工具静默 401」，所以逐字钉住。
    #[test]
    fn 有令牌时按小写头名原样带上() {
        let headers = machine_token_headers(Some("abc-123_XYZ".to_string()));
        assert_eq!(
            headers
                .get(utils::assets::MACHINE_TOKEN_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("abc-123_XYZ")
        );
        assert_eq!(utils::assets::MACHINE_TOKEN_HEADER, "x-vk-machine-token");
    }

    /// 文件末尾的换行不能带进请求头：`HeaderValue::from_str` 会直接失败，
    /// 表现为「头静默消失」。裁掉后再塞。
    #[test]
    fn 首尾空白会被裁掉后再带上() {
        let headers = machine_token_headers(Some("  tok  \n".to_string()));
        assert_eq!(
            headers
                .get(utils::assets::MACHINE_TOKEN_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("tok")
        );
    }

    /// 没令牌、空令牌、只含空白：一律**不加头**，不是加一个空头。
    #[test]
    fn 没有令牌时不加头() {
        for token in [None, Some(String::new()), Some("   ".to_string())] {
            let headers = machine_token_headers(token.clone());
            assert!(headers.is_empty(), "{token:?} 不该产生请求头");
        }
    }

    /// 令牌里混进非法字符时宁可不加头，也不能 panic 把 MCP 整个打死。
    #[test]
    fn 非法字符不会让进程崩溃() {
        for token in ["a\nb", "a\rb", "a\0b", "令牌"] {
            let headers = machine_token_headers(Some(token.to_string()));
            assert!(headers.is_empty(), "{token:?} 不该被塞进请求头");
        }
    }
}
