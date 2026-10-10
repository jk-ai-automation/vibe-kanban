//! 引擎与执行层之间的接缝。引擎只依赖 [`StageLauncher`]：生产环境用
//! [`ContainerStageLauncher`] 接到容器服务；测试用假实现或 [`NoopStageLauncher`]。

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use db::models::{
    execution_process::{ExecutionProcess, ExecutionProcessRunReason, ExecutionProcessStatus},
    session::{CreateSession, Session},
    workspace::Workspace,
    workspace_repo::WorkspaceRepo,
};
use executors::{
    actions::{
        ExecutorAction, ExecutorActionType,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    },
    profile::ExecutorConfig,
};
use tokio::sync::RwLock;
use utils::log_msg::LogMsg;
use uuid::Uuid;

use super::engine::PipelineError;
use crate::services::{config::Config, container::ContainerService};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedStep {
    pub session_id: Uuid,
    /// 会话里第一个进程（有 setup 脚本时是 setup 进程）。
    pub execution_process_id: Uuid,
}

#[async_trait]
pub trait StageLauncher: Send + Sync {
    /// 确保工作区目录与各仓库 worktree 存在，返回工作区根目录（container_ref）。
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError>;

    /// 在工作区里开新会话跑编码智能体；`run_setup` 为 true 时先跑 setup 脚本。
    /// `plugin_dirs` 是会话级插件目录（技能包），只有 Claude Code 会用。
    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 在已有会话里跑检查脚本（run_reason = PipelineStep）。
    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 停掉工作区里正在跑的进程（取消运行时用）。
    async fn stop_workspace(&self, workspace_id: Uuid);

    /// 测试阶段：先停掉工作区里已有的开发服务器（新一轮起服务前清场），再起**第一个**
    /// 配置了开发服务器脚本的仓库，脚本前注入 `env`。没有仓库配脚本时返回 `Ok(None)`。
    async fn start_dev_server(
        &self,
        workspace_id: Uuid,
        env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError>;

    /// 只停这一个开发服务器进程（`stop_workspace` 不停开发服务器）；已不在运行就什么也不做。
    async fn stop_dev_server(&self, execution_process_id: Uuid);

    /// 探测一次：能在超时内拿到任何 HTTP 响应（不看状态码）就算就绪。
    async fn probe_http(&self, url: &str) -> bool;

    /// 进程到目前为止的输出文本（运行中读内存，已结束读库）。
    async fn process_output(&self, execution_process_id: Uuid) -> String;

    /// 调用 atp 的命令（设置页可改）。
    async fn atp_command(&self) -> String;
}

pub const DEFAULT_ATP_COMMAND: &str = "atp";
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 把模板的 checks 拼成一个脚本：`set -e`，任一命令失败即以非零退出。
pub fn checks_script(checks: &[String]) -> String {
    let mut script = String::from("set -e\n");
    for check in checks {
        script.push_str(check.trim());
        script.push('\n');
    }
    script
}

/// 在开发服务器脚本前导出环境变量。值用单引号包住，内部单引号写成 `'\''`。
pub fn dev_server_script(script: &str, env: &[(String, String)]) -> String {
    let mut text = String::new();
    for (key, value) in env {
        text.push_str(&format!(
            "export {key}='{}'\n",
            value.replace('\'', r"'\''")
        ));
    }
    text.push_str(script);
    text
}

/// 设置里的 atp 命令去掉首尾空白；没填或全是空白时用 [`DEFAULT_ATP_COMMAND`]。
pub fn effective_atp_command(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .unwrap_or(DEFAULT_ATP_COMMAND)
        .to_string()
}

/// 只取标准输出与标准错误，按顺序拼起来。
fn output_text(messages: &[LogMsg]) -> String {
    messages
        .iter()
        .filter_map(|message| match message {
            LogMsg::Stdout(text) | LogMsg::Stderr(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

pub struct ContainerStageLauncher<C> {
    container: C,
    config: Arc<RwLock<Config>>,
}

impl<C> ContainerStageLauncher<C> {
    pub fn new(container: C, config: Arc<RwLock<Config>>) -> Self {
        Self { container, config }
    }
}

impl<C> ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync,
{
    async fn load_workspace(&self, workspace_id: Uuid) -> Result<Workspace, PipelineError> {
        Workspace::find_by_id(&self.container.db().pool, workspace_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("工作区 {workspace_id}")))
    }

    /// 停一个进程，失败只记日志。
    async fn stop_process(&self, process: &ExecutionProcess) {
        if let Err(e) = self
            .container
            .stop_execution(process, ExecutionProcessStatus::Killed)
            .await
        {
            tracing::warn!(execution_process_id = %process.id, "停止开发服务器失败: {e}");
        }
    }
}

fn launch_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Launch(error.to_string())
}

#[async_trait]
impl<C> StageLauncher for ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync + 'static,
{
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let root = self
            .container
            .ensure_container_exists(&workspace)
            .await
            .map_err(launch_error)?;
        Ok(PathBuf::from(root))
    }

    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let process = self
            .container
            .start_new_session(
                &workspace,
                executor_config.clone(),
                prompt,
                run_setup,
                plugin_dirs,
            )
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id: process.session_id,
            execution_process_id: process.id,
        })
    }

    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let session = Session::find_by_id(&self.container.db().pool, session_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("会话 {session_id}")))?;
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script,
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::PipelineCheck,
                working_dir: session.agent_working_dir.clone(),
            }),
            None,
        );
        let process = self
            .container
            .start_execution(
                &workspace,
                &session,
                &action,
                &ExecutionProcessRunReason::PipelineStep,
            )
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id,
            execution_process_id: process.id,
        })
    }

    async fn stop_workspace(&self, workspace_id: Uuid) {
        if let Ok(workspace) = self.load_workspace(workspace_id).await {
            self.container.try_stop(&workspace, false).await;
        }
    }

    async fn start_dev_server(
        &self,
        workspace_id: Uuid,
        env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError> {
        let pool = &self.container.db().pool;
        let workspace = self.load_workspace(workspace_id).await?;
        let repos = WorkspaceRepo::find_repos_for_workspace(pool, workspace.id).await?;
        let Some((repo, script)) = repos.iter().find_map(|repo| {
            repo.dev_server_script
                .as_deref()
                .filter(|script| !script.trim().is_empty())
                .map(|script| (repo, script))
        }) else {
            return Ok(None);
        };
        // 清场：停掉工作区里已有的开发服务器。
        match ExecutionProcess::find_running_dev_servers_by_workspace(pool, workspace_id).await {
            Ok(running) => {
                for process in running {
                    self.stop_process(&process).await;
                }
            }
            Err(e) => tracing::warn!(%workspace_id, "查询运行中的开发服务器失败: {e}"),
        }

        // 独立会话：开发服务器的退出事件不会被当成某个阶段的终点进程。
        let session = Session::create(
            pool,
            &CreateSession {
                executor: Some("dev-server".to_string()),
                name: None,
            },
            Uuid::new_v4(),
            workspace.id,
        )
        .await
        .map_err(launch_error)?;
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script: dev_server_script(script, env),
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::DevServer,
                working_dir: Some(repo.name.clone()),
            }),
            None,
        );
        let process = self
            .container
            .start_execution(
                &workspace,
                &session,
                &action,
                &ExecutionProcessRunReason::DevServer,
            )
            .await
            .map_err(launch_error)?;
        Ok(Some(process.id))
    }

    async fn stop_dev_server(&self, execution_process_id: Uuid) {
        match ExecutionProcess::find_by_id(&self.container.db().pool, execution_process_id).await {
            Ok(Some(process)) if process.status == ExecutionProcessStatus::Running => {
                self.stop_process(&process).await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(%execution_process_id, "读取开发服务器进程失败: {e}");
            }
        }
    }

    async fn probe_http(&self, url: &str) -> bool {
        // 不走系统代理：配了 HTTP_PROXY 时代理会替回环地址回 502，服务没起也被判成就绪。
        let Ok(client) = reqwest::Client::builder()
            .no_proxy()
            .timeout(PROBE_TIMEOUT)
            .danger_accept_invalid_certs(true)
            .build()
        else {
            return false;
        };
        client.get(url).send().await.is_ok()
    }

    async fn process_output(&self, execution_process_id: Uuid) -> String {
        if let Some(store) = self
            .container
            .get_msg_store_by_id(&execution_process_id)
            .await
        {
            return output_text(&store.get_history());
        }
        crate::services::execution_process::load_raw_log_messages(
            &self.container.db().pool,
            execution_process_id,
        )
        .await
        .map(|messages| output_text(&messages))
        .unwrap_or_default()
    }

    async fn atp_command(&self) -> String {
        effective_atp_command(self.config.read().await.pipeline_atp_command.as_deref())
    }
}

/// 不接执行层的启动器：所有启动都报错。给只测状态与错误映射的路由单测用。
pub struct NoopStageLauncher;

#[async_trait]
impl StageLauncher for NoopStageLauncher {
    async fn prepare_workspace(&self, _workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_agent_session(
        &self,
        _workspace_id: Uuid,
        _executor_config: &ExecutorConfig,
        _prompt: String,
        _run_setup: bool,
        _plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_checks(
        &self,
        _workspace_id: Uuid,
        _session_id: Uuid,
        _script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn stop_workspace(&self, _workspace_id: Uuid) {}

    async fn start_dev_server(
        &self,
        _workspace_id: Uuid,
        _env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn stop_dev_server(&self, _execution_process_id: Uuid) {}

    async fn probe_http(&self, _url: &str) -> bool {
        false
    }

    async fn process_output(&self, _execution_process_id: Uuid) -> String {
        String::new()
    }

    async fn atp_command(&self) -> String {
        DEFAULT_ATP_COMMAND.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 检查脚本逐行执行且任一失败即退出() {
        let script = checks_script(&["pnpm run lint".to_string(), "  cargo test  ".to_string()]);
        assert_eq!(script, "set -e\npnpm run lint\ncargo test\n");
    }

    #[tokio::test]
    async fn 空启动器所有启动都报错() {
        let launcher = NoopStageLauncher;
        let id = Uuid::new_v4();
        assert!(launcher.prepare_workspace(id).await.is_err());
        assert!(
            launcher
                .start_agent_session(
                    id,
                    &ExecutorConfig::new(executors::executors::BaseCodingAgent::ClaudeCode),
                    "p".to_string(),
                    true,
                    Vec::new()
                )
                .await
                .is_err()
        );
        assert!(
            launcher
                .start_checks(id, id, "true".to_string())
                .await
                .is_err()
        );
        launcher.stop_workspace(id).await;
        assert!(launcher.start_dev_server(id, &[]).await.is_err());
        assert!(!launcher.probe_http("http://127.0.0.1:1").await);
        assert_eq!(launcher.atp_command().await, "atp");
        launcher.stop_dev_server(id).await;
    }

    #[test]
    fn 开发服务器脚本前注入环境变量() {
        let script = dev_server_script(
            "pnpm run dev",
            &[
                ("PORT".to_string(), "4000".to_string()),
                (
                    "VK_BASE_URL".to_string(),
                    "http://127.0.0.1:4000".to_string(),
                ),
            ],
        );
        assert_eq!(
            script,
            "export PORT='4000'\nexport VK_BASE_URL='http://127.0.0.1:4000'\npnpm run dev"
        );
    }

    #[test]
    fn 环境变量值里的单引号被转义() {
        let script = dev_server_script("true", &[("A".to_string(), "it's".to_string())]);
        assert_eq!(script, "export A='it'\\''s'\ntrue");
    }

    #[test]
    fn atp_命令为空时回落到默认() {
        assert_eq!(effective_atp_command(None), "atp");
        assert_eq!(effective_atp_command(Some("   ")), "atp");
        assert_eq!(
            effective_atp_command(Some(" uv run --project /opt/atp atp ")),
            "uv run --project /opt/atp atp"
        );
    }
}
