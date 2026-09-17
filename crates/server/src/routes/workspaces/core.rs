use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
    response::Json as ResponseJson,
};
use db::models::{
    coding_agent_turn::CodingAgentTurn,
    execution_process::{ExecutionProcess, ExecutionProcessStatus},
    workspace::{Workspace, WorkspaceError},
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::{container::ContainerService, diff_stream, remote_sync};
use sqlx::Error as SqlxError;
use utils::response::ApiResponse;
use workspace_manager::WorkspaceManager;

use crate::{
    DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser,
    routes::admin::require_admin,
};

#[derive(Debug, Deserialize)]
pub struct DeleteWorkspaceQuery {
    #[serde(default)]
    pub delete_remote: bool,
    #[serde(default)]
    pub delete_branches: bool,
}

pub async fn get_workspaces(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<Workspace>>>, ApiError> {
    let pool = &deployment.db().pool;
    let workspaces = Workspace::fetch_all(pool).await?;
    Ok(ResponseJson(ApiResponse::success(workspaces)))
}

pub async fn get_workspace(
    Extension(workspace): Extension<Workspace>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(workspace)))
}

pub async fn update_workspace(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<db::models::requests::UpdateWorkspace>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    let pool = &deployment.db().pool;
    let is_archiving = request.archived == Some(true) && !workspace.archived;

    Workspace::update(
        pool,
        workspace.id,
        request.archived,
        request.pinned,
        request.name.as_deref(),
    )
    .await?;
    let updated = Workspace::find_by_id(pool, workspace.id)
        .await?
        .ok_or(WorkspaceError::WorkspaceNotFound)?;

    if (request.archived.is_some() || request.name.is_some())
        && let Ok(client) = deployment.remote_client()
    {
        let ws = updated.clone();
        let name = request.name.clone();
        let archived = request.archived;
        let stats =
            diff_stream::compute_diff_stats(&deployment.db().pool, deployment.git(), &ws).await;
        tokio::spawn(async move {
            remote_sync::sync_workspace_to_remote(
                &client,
                ws.id,
                name.map(Some),
                archived,
                stats.as_ref(),
            )
            .await;
        });
    }

    if is_archiving && let Err(e) = deployment.container().archive_workspace(workspace.id).await {
        tracing::error!("Failed to archive workspace {}: {}", workspace.id, e);
    }

    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn get_first_user_message(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Option<String>>>, ApiError> {
    let pool = &deployment.db().pool;
    let message = Workspace::get_first_user_message(pool, workspace.id).await?;
    Ok(ResponseJson(ApiResponse::success(message)))
}

/// 删除工作区。**只有管理员做得了**，member 一律 403。
///
/// `require_admin` 是这个 handler 的第一句，写在任何数据库/文件系统动作之前：
/// 403 之后工作区一行都不会被动过。有两条测试钉着——一条按源码顺序扫，
/// 一条真建一个工作区、拿 member 走一遍守卫再数库里的行。
///
/// **不按 `ServerMode` 分支**：个人版、以及团队版里带本机令牌的进程（MCP），
/// 走的都是 `SessionGate::PersonalBypass`，注入的是迁移里那条 `role='admin'`
/// 的本机用户，这道守卫对它恒为真，个人版行为逐字不变。少一个分支就少一处
/// 「团队版忘了包住」的可能。前提（本机用户永远是 admin）由
/// `admin/users.rs` 里「不得修改本机固定用户角色」那道守卫保证。
pub async fn delete_workspace(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Query(query): Query<DeleteWorkspaceQuery>,
) -> Result<(StatusCode, ResponseJson<ApiResponse<()>>), ApiError> {
    require_admin(&actor)?;
    perform_workspace_deletion(
        &deployment,
        workspace,
        query.delete_remote,
        query.delete_branches,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, ResponseJson(ApiResponse::success(()))))
}

/// 真正执行删除的那一段，与鉴权分开。
///
/// 两个调用方共用它：管理员直接删（[`delete_workspace`]），以及管理员批准
/// 一条删除申请（`routes/admin/workspace_delete_requests.rs`）。抽出来是为了
/// 让「批准即删除」走的是**同一条**删除路径，而不是复制一份出来慢慢跑偏。
///
/// 本函数**不做任何权限判断**——调用方必须自己先过守卫。
pub(crate) async fn perform_workspace_deletion(
    deployment: &DeploymentImpl,
    workspace: Workspace,
    delete_remote: bool,
    delete_branches: bool,
) -> Result<(), ApiError> {
    let pool = &deployment.db().pool;
    let workspace_manager = deployment.workspace_manager();
    let workspace_id = workspace.id;

    if ExecutionProcess::has_running_non_dev_server_processes_for_workspace(pool, workspace_id)
        .await?
    {
        return Err(ApiError::Conflict(
            "Cannot delete workspace while processes are running. Stop all processes first."
                .to_string(),
        ));
    }

    let dev_servers =
        ExecutionProcess::find_running_dev_servers_by_workspace(pool, workspace_id).await?;

    for dev_server in dev_servers {
        tracing::info!(
            "Stopping dev server {} before deleting workspace {}",
            dev_server.id,
            workspace_id
        );

        if let Err(e) = deployment
            .container()
            .stop_execution(&dev_server, ExecutionProcessStatus::Killed)
            .await
        {
            tracing::error!(
                "Failed to stop dev server {} for workspace {}: {}",
                dev_server.id,
                workspace_id,
                e
            );
        }
    }

    let managed_workspace = workspace_manager.load_managed_workspace(workspace).await?;
    let deletion_context = managed_workspace.prepare_deletion_context().await?;
    let rows_affected = managed_workspace.delete_record().await?;

    if rows_affected == 0 {
        return Err(ApiError::Database(SqlxError::RowNotFound));
    }

    deployment
        .track_if_analytics_allowed(
            "workspace_deleted",
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
            }),
        )
        .await;

    if delete_remote {
        if let Ok(client) = deployment.remote_client() {
            match client.delete_workspace(workspace_id).await {
                Ok(()) => {
                    tracing::info!("Deleted remote workspace for {}", workspace_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to delete remote workspace for {}: {}",
                        workspace_id,
                        e
                    );
                }
            }
        } else {
            tracing::debug!(
                "Remote client not available, skipping remote deletion for {}",
                workspace_id
            );
        }
    }

    WorkspaceManager::spawn_workspace_deletion_cleanup(deletion_context, delete_branches);

    Ok(())
}

#[axum::debug_handler]
pub async fn mark_seen(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    CodingAgentTurn::mark_seen_by_workspace_id(pool, workspace.id).await?;
    Ok(ResponseJson(ApiResponse::success(())))
}

#[cfg(test)]
mod tests {
    use db::{
        models::{
            local_project::DEFAULT_USER_ID,
            local_user::{LocalUserRole, LocalUsers},
            workspace::CreateWorkspace,
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::*;

    fn 身份(role: LocalUserRole) -> CurrentUser {
        CurrentUser {
            id: Uuid::new_v4(),
            username: "someone".to_string(),
            role,
        }
    }

    /// 攻击样例：member 直接 `DELETE /api/workspaces/{id}`。
    /// 必须 403，**而且库里的工作区行一条都不能少**。
    #[tokio::test]
    async fn member_删工作区被拒且工作区还在() {
        let test_db = TestDb::new().await;
        let ws = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "feat-a".to_string(),
                name: None,
            },
            Uuid::new_v4(),
            DEFAULT_USER_ID,
        )
        .await
        .expect("建工作区失败");

        let err = require_admin(&身份(LocalUserRole::Member)).expect_err("member 不得删工作区");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");

        let 剩余: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(剩余, 1, "403 之后工作区必须还在");
        assert!(
            Workspace::find_by_id(test_db.pool(), ws.id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn admin_可以删工作区() {
        require_admin(&身份(LocalUserRole::Admin)).expect("admin 应通过");
    }

    /// 个人版零回退的机械依据：迁移写入的本机用户就是 admin，
    /// `SessionGate::PersonalBypass` 注入的正是这条用户，
    /// 所以 `require_admin` 对个人版恒为真。
    #[tokio::test]
    async fn 个人版本机用户是管理员因此照常能删() {
        let test_db = TestDb::new().await;
        let local = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .expect("迁移必须写入本机用户");
        assert_eq!(local.role, LocalUserRole::Admin);

        require_admin(&CurrentUser {
            id: local.id,
            username: local.username,
            role: local.role,
        })
        .expect("个人版本机用户必须能删工作区");
    }

    /// `require_admin` 必须是 `delete_workspace` 的**第一句**：
    /// 排在任何数据库 / 文件系统动作之前，403 之后什么都没发生。
    #[test]
    fn 删除_handler_第一句就是管理员守卫() {
        let source = include_str!("core.rs");
        let 函数体 = source
            .split("pub async fn delete_workspace(")
            .nth(1)
            .expect("找不到 delete_workspace");
        let 首句 = 函数体
            .split_once(") -> Result<(StatusCode, ResponseJson<ApiResponse<()>>), ApiError> {")
            .expect("签名变了")
            .1
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"));
        assert_eq!(
            首句,
            Some("require_admin(&actor)?;"),
            "delete_workspace 的第一句必须是管理员守卫"
        );
    }

    /// `perform_workspace_deletion` 是**不带权限判断**的执行段，
    /// 任何新调用方都必须自己先过守卫。这里钉住「它自己不做鉴权」这个事实，
    /// 免得有人以为调它就自动安全了。
    #[test]
    fn 执行段本身不做鉴权() {
        let source = include_str!("core.rs");
        let 函数体 = source
            .split("pub(crate) async fn perform_workspace_deletion(")
            .nth(1)
            .expect("找不到 perform_workspace_deletion")
            .split("\n#[axum::debug_handler]")
            .next()
            .unwrap();
        assert!(
            !函数体.contains("require_admin"),
            "执行段不该自带鉴权：鉴权是调用方的责任，混进来会掩盖漏挂守卫的调用方"
        );
    }
}
