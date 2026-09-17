//! 删除申请的**申请人侧**接口（`/api/workspace-delete-requests`）。
//!
//! 审批侧在 `routes/admin/workspace_delete_requests.rs`，走 `/api/admin` 前缀，
//! 由 `require_admin_middleware` 整组包住。这里刻意分成两个模块：
//! 「谁能调」靠**路由挂在哪一组**来决定，而不是靠 handler 里记得写 if。
//!
//! 安全约定：
//! - 列表**永远只返回调用者自己提交的**申请，过滤条件写在 SQL 的 WHERE 里。
//! - 撤回别人的申请返回 **404 而不是 403**：403 会确认「这个 id 存在」，
//!   让 member 能拿它枚举别人的申请。申请 id 在这里是要保密的，
//!   与 `/api/admin` 的「路径本身不是秘密」不同。
//! - 管理员调建申请返回 400：他直接删就行，让 admin 也能排队会凭空多出
//!   一条「自己批自己」的路径。

use axum::{
    Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{delete, get},
};
use chrono::{DateTime, Utc};
use db::models::{
    workspace::Workspace,
    workspace_delete_request::{
        WorkspaceDeleteRequest, WorkspaceDeleteRequestError, WorkspaceDeleteRequests,
    },
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser};

/// 一条删除申请的对外视图。
///
/// **字段是白名单**：没有 `container_ref`、没有 worktree 路径、没有申请人的
/// 邮箱，也没有驳回理由（那是给申请人看的下一版内容，本期不暴露列表）。
/// 有一条测试把序列化结果拿去搜这些词。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct WorkspaceDeleteRequestInfo {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_name: Option<String>,
    pub workspace_branch: String,
    pub requested_by_user_id: Option<Uuid>,
    pub requested_by_username: Option<String>,
    pub reason: Option<String>,
    /// 目前只可能是 `"pending"`：两个列表接口都只查待处理。
    pub status: String,
    pub created_at: DateTime<Utc>,
}

impl From<&WorkspaceDeleteRequest> for WorkspaceDeleteRequestInfo {
    fn from(request: &WorkspaceDeleteRequest) -> Self {
        Self {
            id: request.id,
            workspace_id: request.workspace_id,
            workspace_name: request.workspace_name.clone(),
            workspace_branch: request.workspace_branch.clone(),
            requested_by_user_id: request.requested_by_user_id,
            requested_by_username: request.requested_by_username.clone(),
            reason: request.reason.clone(),
            status: request.status.clone(),
            created_at: request.created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct ListWorkspaceDeleteRequestsResponse {
    pub requests: Vec<WorkspaceDeleteRequestInfo>,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct CreateWorkspaceDeleteRequestRequest {
    pub workspace_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `WorkspaceDeleteRequestError` → `ApiError`。
pub(crate) fn map_delete_request_error(error: WorkspaceDeleteRequestError) -> ApiError {
    match error {
        WorkspaceDeleteRequestError::Validation(message) => ApiError::BadRequest(message),
        WorkspaceDeleteRequestError::NotPending => ApiError::NotFound,
        WorkspaceDeleteRequestError::Database(err) => ApiError::from(err),
    }
}

// ------------------------------------------------------------------- 建申请

pub(crate) async fn handle_create_request(
    pool: &SqlitePool,
    actor: &CurrentUser,
    payload: &CreateWorkspaceDeleteRequestRequest,
) -> Result<WorkspaceDeleteRequestInfo, ApiError> {
    if actor.is_admin() {
        return Err(ApiError::BadRequest(
            "管理员可直接删除工作区，无需提交申请".to_string(),
        ));
    }

    // 工作区不存在就别往库里塞一条指向空气的申请。外键也会拦，
    // 但那会变成 500；这里明确成 404。
    if Workspace::find_by_id(pool, payload.workspace_id)
        .await?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }

    let request = WorkspaceDeleteRequests::create_or_get_pending(
        pool,
        payload.workspace_id,
        actor.id,
        payload.reason.as_deref(),
    )
    .await
    .map_err(map_delete_request_error)?;

    Ok(WorkspaceDeleteRequestInfo::from(&request))
}

pub(crate) async fn create_request(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    axum::Json(payload): axum::Json<CreateWorkspaceDeleteRequestRequest>,
) -> Result<ResponseJson<ApiResponse<WorkspaceDeleteRequestInfo>>, ApiError> {
    let info = handle_create_request(&deployment.db().pool, &actor, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(info)))
}

// ------------------------------------------------------------------- 我的申请

pub(crate) async fn handle_list_my_requests(
    pool: &SqlitePool,
    actor: &CurrentUser,
) -> Result<ListWorkspaceDeleteRequestsResponse, ApiError> {
    let requests = WorkspaceDeleteRequests::find_pending_by_requester(pool, actor.id).await?;
    Ok(ListWorkspaceDeleteRequestsResponse {
        requests: requests
            .iter()
            .map(WorkspaceDeleteRequestInfo::from)
            .collect(),
    })
}

pub(crate) async fn list_my_requests(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
) -> Result<ResponseJson<ApiResponse<ListWorkspaceDeleteRequestsResponse>>, ApiError> {
    let payload = handle_list_my_requests(&deployment.db().pool, &actor).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 撤回

pub(crate) async fn handle_withdraw_request(
    pool: &SqlitePool,
    actor: &CurrentUser,
    request_id: Uuid,
) -> Result<(), ApiError> {
    // 「是不是自己的」判据在 SQL 里：撤回别人的申请命中 0 行，
    // 别人的申请一个字节都没被碰过。
    let rows = WorkspaceDeleteRequests::withdraw(pool, request_id, actor.id).await?;
    if rows == 0 {
        // 不区分「不存在」与「不是你的」——区分开就成了枚举 oracle。
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) async fn withdraw_request(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(request_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    handle_withdraw_request(&deployment.db().pool, &actor, request_id).await?;
    Ok(ResponseJson(ApiResponse::success("OK".to_string())))
}

// ------------------------------------------------------------------- 路由

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route(
            "/workspace-delete-requests",
            get(list_my_requests).post(create_request),
        )
        .route("/workspace-delete-requests/{id}", delete(withdraw_request))
}

#[cfg(test)]
mod tests {
    use db::{
        models::{
            local_user::{LocalUser, LocalUserRole, LocalUsers, NewLocalUser},
            workspace::CreateWorkspace,
        },
        test_support::TestDb,
    };

    use super::*;

    async fn 建用户(test_db: &TestDb, username: &str, role: LocalUserRole) -> LocalUser {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: Some(format!("{username}@example.com")),
                password_hash: Some("$argon2id$x".to_string()),
                role,
            },
        )
        .await
        .expect("建用户失败")
    }

    fn 身份(user: &LocalUser) -> CurrentUser {
        CurrentUser {
            id: user.id,
            username: user.username.clone(),
            role: user.role,
        }
    }

    async fn 建工作区(test_db: &TestDb, creator: Uuid, branch: &str) -> Workspace {
        Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: branch.to_string(),
                name: Some(format!("ws-{branch}")),
            },
            Uuid::new_v4(),
            creator,
        )
        .await
        .expect("建工作区失败")
    }

    fn 请求(workspace_id: Uuid) -> CreateWorkspaceDeleteRequestRequest {
        CreateWorkspaceDeleteRequestRequest {
            workspace_id,
            reason: None,
        }
    }

    #[tokio::test]
    async fn member_能提交申请() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let info = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .expect("member 应能提交申请");
        assert_eq!(info.workspace_id, ws.id);
        assert_eq!(info.status, "pending");
        assert_eq!(info.requested_by_username.as_deref(), Some("bob"));
    }

    /// 管理员不该走申请流程——他直接删就行。
    #[tokio::test]
    async fn 管理员提交申请返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let ws = 建工作区(&test_db, amy.id, "feat-a").await;

        let err = handle_create_request(test_db.pool(), &身份(&amy), &请求(ws.id))
            .await
            .expect_err("管理员不该排队");
        assert!(matches!(err, ApiError::BadRequest(_)), "实际：{err:?}");
        assert_eq!(
            WorkspaceDeleteRequests::find_all_pending(test_db.pool())
                .await
                .unwrap()
                .len(),
            0,
            "400 之后库里不得留下申请"
        );
    }

    #[tokio::test]
    async fn 给不存在的工作区提交申请返回_404() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let err = handle_create_request(test_db.pool(), &身份(&bob), &请求(Uuid::new_v4()))
            .await
            .expect_err("不存在的工作区应 404");
        assert!(matches!(err, ApiError::NotFound), "实际：{err:?}");
    }

    /// 攻击样例：反复点「申请删除」刷审批队列。
    #[tokio::test]
    async fn 重复提交不产生第二条待处理记录() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let first = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .unwrap();
        for _ in 0..4 {
            let again = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
                .await
                .unwrap();
            assert_eq!(again.id, first.id);
        }
        assert_eq!(
            WorkspaceDeleteRequests::find_all_pending(test_db.pool())
                .await
                .unwrap()
                .len(),
            1,
            "重复提交必须只留一条待处理"
        );
    }

    /// 攻击样例：申请人 A 撤回 B 的申请。
    #[tokio::test]
    async fn 撤回别人的申请返回_404_且原申请不受影响() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let eve = 建用户(&test_db, "eve", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .unwrap();

        let err = handle_withdraw_request(test_db.pool(), &身份(&eve), req.id)
            .await
            .expect_err("不得撤回别人的申请");
        assert!(
            matches!(err, ApiError::NotFound),
            "必须是 404（403 会泄露 id 存在）：{err:?}"
        );

        let still = WorkspaceDeleteRequests::find_by_id(test_db.pool(), req.id)
            .await
            .unwrap()
            .expect("B 的申请必须还在");
        assert_eq!(still.status, "pending");
    }

    /// 管理员也不能替别人撤回：撤回是申请人的动作，管理员要用驳回。
    #[tokio::test]
    async fn 管理员也撤不了别人的申请() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .unwrap();

        let err = handle_withdraw_request(test_db.pool(), &身份(&amy), req.id)
            .await
            .expect_err("管理员不得替别人撤回");
        assert!(matches!(err, ApiError::NotFound));
    }

    #[tokio::test]
    async fn 撤回自己的申请成功且不能重复撤回() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .unwrap();

        handle_withdraw_request(test_db.pool(), &身份(&bob), req.id)
            .await
            .expect("撤回自己的申请应成功");
        let err = handle_withdraw_request(test_db.pool(), &身份(&bob), req.id)
            .await
            .expect_err("重复撤回应 404");
        assert!(matches!(err, ApiError::NotFound));
    }

    /// 「我的申请」永远只有自己的，管理员调它也一样（那不是审批队列）。
    #[tokio::test]
    async fn 我的申请列表不含别人的() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let eve = 建用户(&test_db, "eve", LocalUserRole::Member).await;
        let ws1 = 建工作区(&test_db, bob.id, "feat-a").await;
        let ws2 = 建工作区(&test_db, eve.id, "feat-b").await;

        handle_create_request(test_db.pool(), &身份(&bob), &请求(ws1.id))
            .await
            .unwrap();
        handle_create_request(test_db.pool(), &身份(&eve), &请求(ws2.id))
            .await
            .unwrap();

        let 我的 = handle_list_my_requests(test_db.pool(), &身份(&bob))
            .await
            .unwrap();
        assert_eq!(我的.requests.len(), 1);
        assert_eq!(我的.requests[0].workspace_id, ws1.id);

        let 管理员的 = handle_list_my_requests(test_db.pool(), &身份(&amy))
            .await
            .unwrap();
        assert!(
            管理员的.requests.is_empty(),
            "这个接口是「我的申请」，不是审批队列"
        );
    }

    #[tokio::test]
    async fn 理由超长返回_400() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let err = handle_create_request(
            test_db.pool(),
            &身份(&bob),
            &CreateWorkspaceDeleteRequestRequest {
                workspace_id: ws.id,
                reason: Some("字".repeat(5000)),
            },
        )
        .await
        .expect_err("超长理由应 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "实际：{err:?}");
        assert_eq!(
            WorkspaceDeleteRequests::find_all_pending(test_db.pool())
                .await
                .unwrap()
                .len(),
            0,
            "400 之后库里不得留下申请"
        );
    }

    /// 响应里不得出现本地路径、账号敏感字段或审批内部字段。
    #[tokio::test]
    async fn 响应不泄漏不该给的字段() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        handle_create_request(test_db.pool(), &身份(&bob), &请求(ws.id))
            .await
            .unwrap();

        let list = handle_list_my_requests(test_db.pool(), &身份(&bob))
            .await
            .unwrap();
        let json = serde_json::to_string(&list).unwrap();
        for 禁词 in [
            "container_ref",
            "worktree",
            "password",
            "token",
            "email",
            "example.com",
            "decision_note",
            "decided_by",
            "task_id",
        ] {
            assert!(!json.contains(禁词), "响应泄露了 {禁词}：{json}");
        }
    }

    /// 本模块**不得**注册任何 `/admin` 前缀的路径：那会绕过
    /// `require_admin_middleware`。审批侧必须住在 `routes/admin` 里。
    #[test]
    fn 申请人侧不注册_admin_路径() {
        let source = include_str!("workspace_delete_requests.rs");
        assert!(
            !source.contains(".route(\"/admin"),
            "申请人侧模块不得注册 /admin 路径"
        );
    }
}
