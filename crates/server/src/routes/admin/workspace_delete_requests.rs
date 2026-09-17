//! 删除申请的**审批侧**接口（`/api/admin/workspace-delete-requests`），仅管理员可用。
//!
//! 申请人侧（提交 / 撤回 / 我的申请）在 `routes/workspace_delete_requests.rs`。
//!
//! 核心约定：**批准即删除**。批准不是把状态改成「已批准」然后等着谁去执行——
//! 那会留下「已批准但工作区还在」的悬空状态。这里的顺序是：
//!
//! 1. [`handle_claim_approval`] 用一条**条件 UPDATE** 抢占申请
//!    （`WHERE id = ? AND status = 'pending'`）。这是唯一的胜者选举点：
//!    并发批准同一条申请时只有一个拿到 `rows_affected = 1`，
//!    其余得到 409，**一次真正的删除都不会多做**。全程没有「先查后写」。
//! 2. 抢占成功后走 `workspaces::core::perform_workspace_deletion`——
//!    与管理员直接删走的是同一条路径，不是复制出来的第二份。
//! 3. 工作区行一删，外键 `ON DELETE CASCADE` 当场带走这条申请，
//!    `approved` 因此从不落盘。
//! 4. 删除失败（有进程在跑 / IO 异常）则把抢占**条件回滚**成 `pending`，
//!    绝不留下悬空的已批准状态。

use axum::{
    Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use db::models::{
    workspace::Workspace,
    workspace_delete_request::{WorkspaceDeleteRequest, WorkspaceDeleteRequests},
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{register_admin, require_admin};
use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::local_session::CurrentUser,
    routes::{
        workspace_delete_requests::{
            ListWorkspaceDeleteRequestsResponse, WorkspaceDeleteRequestInfo,
            map_delete_request_error,
        },
        workspaces::core::perform_workspace_deletion,
    },
};

#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct RejectWorkspaceDeleteRequestRequest {
    /// 驳回理由，可选，≤ 500 字符。
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct ApproveWorkspaceDeleteRequestRequest {
    /// 是否连同分支一起删，缺省 false——与 `DELETE /api/workspaces/{id}`
    /// 的 `delete_branches` 查询参数缺省值一致。
    #[serde(default)]
    pub delete_branches: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct WorkspaceDeleteRequestDecisionResponse {
    pub request_id: Uuid,
    pub workspace_id: Uuid,
    /// `"approved"`（工作区已删）或 `"rejected"`。
    pub outcome: String,
}

// ------------------------------------------------------------------- 待处理队列

pub(crate) async fn handle_list_pending_requests(
    pool: &SqlitePool,
    actor: &CurrentUser,
) -> Result<ListWorkspaceDeleteRequestsResponse, ApiError> {
    require_admin(actor)?;
    let requests = WorkspaceDeleteRequests::find_all_pending(pool).await?;
    Ok(ListWorkspaceDeleteRequestsResponse {
        requests: requests
            .iter()
            .map(WorkspaceDeleteRequestInfo::from)
            .collect(),
    })
}

pub(crate) async fn list_pending_requests(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
) -> Result<ResponseJson<ApiResponse<ListWorkspaceDeleteRequestsResponse>>, ApiError> {
    let payload = handle_list_pending_requests(&deployment.db().pool, &actor).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 驳回

pub(crate) async fn handle_reject_request(
    pool: &SqlitePool,
    actor: &CurrentUser,
    request_id: Uuid,
    payload: &RejectWorkspaceDeleteRequestRequest,
) -> Result<WorkspaceDeleteRequestDecisionResponse, ApiError> {
    require_admin(actor)?;
    // 先取 workspace_id 只是为了回响应；真正的「还是不是 pending」判据
    // 在下面那条条件 UPDATE 的 rows_affected 里。
    let request = WorkspaceDeleteRequests::find_by_id(pool, request_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let rows = WorkspaceDeleteRequests::reject(pool, request_id, actor.id, payload.note.as_deref())
        .await
        .map_err(map_delete_request_error)?;
    if rows == 0 {
        return Err(ApiError::Conflict("申请已被撤回或已处理".to_string()));
    }

    Ok(WorkspaceDeleteRequestDecisionResponse {
        request_id,
        workspace_id: request.workspace_id,
        outcome: "rejected".to_string(),
    })
}

pub(crate) async fn reject_request(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(request_id): Path<Uuid>,
    axum::Json(payload): axum::Json<RejectWorkspaceDeleteRequestRequest>,
) -> Result<ResponseJson<ApiResponse<WorkspaceDeleteRequestDecisionResponse>>, ApiError> {
    let payload =
        handle_reject_request(&deployment.db().pool, &actor, request_id, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 批准

/// 抢占一条待处理申请，返回它指向的工作区。
///
/// **这是批准流程唯一的胜者选举点。** 返回 `Ok` 表示本次调用赢得了这条申请，
/// 调用方**必须**接着去执行删除（失败则 [`WorkspaceDeleteRequests::release_claim`]）。
/// 返回 `Err` 时调用方**绝不能**删任何东西。
///
/// 三种失败都明确区分，且都不会误删：
/// - 申请不存在（含「工作区已被管理员直接删掉，级联带走了申请」）→ 404；
/// - 工作区不存在（理论上外键挡住了，这里兜底）→ 404；
/// - 申请已被撤回 / 已驳回 / 已被另一个管理员抢走 → 409。
pub(crate) async fn handle_claim_approval(
    pool: &SqlitePool,
    actor: &CurrentUser,
    request_id: Uuid,
) -> Result<Workspace, ApiError> {
    require_admin(actor)?;

    let request: WorkspaceDeleteRequest = WorkspaceDeleteRequests::find_by_id(pool, request_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // 工作区必须此刻仍然存在才谈得上删它。查不到就直接 404——
    // 绝不拿一个「同 id 的别的东西」去走删除路径。
    let workspace = Workspace::find_by_id(pool, request.workspace_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let rows = WorkspaceDeleteRequests::claim_for_approval(pool, request_id, actor.id).await?;
    if rows == 0 {
        return Err(ApiError::Conflict("申请已被撤回或已处理".to_string()));
    }

    Ok(workspace)
}

pub(crate) async fn approve_request(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(request_id): Path<Uuid>,
    payload: Option<axum::Json<ApproveWorkspaceDeleteRequestRequest>>,
) -> Result<ResponseJson<ApiResponse<WorkspaceDeleteRequestDecisionResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let delete_branches = payload
        .and_then(|axum::Json(p)| p.delete_branches)
        .unwrap_or(false);

    let workspace = handle_claim_approval(pool, &actor, request_id).await?;
    let workspace_id = workspace.id;

    // 抢占成功之后才动真格。失败必须把抢占放回去，否则会留下一条
    // 「已批准但工作区还在」的悬空申请，谁也再处理不了它。
    if let Err(err) = perform_workspace_deletion(
        &deployment,
        workspace,
        /* delete_remote */ false,
        delete_branches,
    )
    .await
    {
        if let Err(release_err) = WorkspaceDeleteRequests::release_claim(pool, request_id).await {
            tracing::error!(?release_err, %request_id, "回滚删除申请抢占失败");
        }
        return Err(err);
    }

    tracing::info!(%request_id, %workspace_id, admin = %actor.username, "批准删除申请并已删除工作区");

    Ok(ResponseJson(ApiResponse::success(
        WorkspaceDeleteRequestDecisionResponse {
            request_id,
            workspace_id,
            outcome: "approved".to_string(),
        },
    )))
}

// ------------------------------------------------------------------- 路由

pub(crate) fn router() -> Router<DeploymentImpl> {
    register_admin("/admin/workspace-delete-requests", "GET");
    register_admin("/admin/workspace-delete-requests/{id}/approve", "POST");
    register_admin("/admin/workspace-delete-requests/{id}/reject", "POST");

    Router::new()
        .route(
            "/admin/workspace-delete-requests",
            get(list_pending_requests),
        )
        .route(
            "/admin/workspace-delete-requests/{id}/approve",
            post(approve_request),
        )
        .route(
            "/admin/workspace-delete-requests/{id}/reject",
            post(reject_request),
        )
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
    use crate::routes::workspace_delete_requests::{
        CreateWorkspaceDeleteRequestRequest, handle_create_request, handle_withdraw_request,
    };

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

    async fn 建申请(test_db: &TestDb, requester: &LocalUser, workspace_id: Uuid) -> Uuid {
        handle_create_request(
            test_db.pool(),
            &身份(requester),
            &CreateWorkspaceDeleteRequestRequest {
                workspace_id,
                reason: None,
            },
        )
        .await
        .expect("建申请失败")
        .id
    }

    async fn 工作区数(test_db: &TestDb) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workspaces")
            .fetch_one(test_db.pool())
            .await
            .unwrap()
    }

    /// 攻击样例：member 直接调批准接口。
    #[tokio::test]
    async fn member_批准申请返回_403_且不删任何东西() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        let err = handle_claim_approval(test_db.pool(), &身份(&bob), req)
            .await
            .expect_err("member 不得批准");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
        assert_eq!(工作区数(&test_db).await, 1, "403 之后工作区必须还在");

        let still = WorkspaceDeleteRequests::find_by_id(test_db.pool(), req)
            .await
            .unwrap()
            .expect("申请必须还在");
        assert_eq!(still.status, "pending", "403 之后申请状态不得被动到");
    }

    /// 攻击样例：member 直接调驳回接口。
    #[tokio::test]
    async fn member_驳回申请返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        let err = handle_reject_request(
            test_db.pool(),
            &身份(&bob),
            req,
            &RejectWorkspaceDeleteRequestRequest::default(),
        )
        .await
        .expect_err("member 不得驳回");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
        assert_eq!(
            WorkspaceDeleteRequests::find_by_id(test_db.pool(), req)
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
    }

    /// 攻击样例：member 偷看审批队列（能看到别人还没处理的删除意图）。
    #[tokio::test]
    async fn member_读审批队列返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let err = handle_list_pending_requests(test_db.pool(), &身份(&bob))
            .await
            .expect_err("member 不得看审批队列");
        assert!(matches!(err, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn 管理员能看到全部待处理申请() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let eve = 建用户(&test_db, "eve", LocalUserRole::Member).await;
        let ws1 = 建工作区(&test_db, bob.id, "feat-a").await;
        let ws2 = 建工作区(&test_db, eve.id, "feat-b").await;
        建申请(&test_db, &bob, ws1.id).await;
        建申请(&test_db, &eve, ws2.id).await;

        let 队列 = handle_list_pending_requests(test_db.pool(), &身份(&amy))
            .await
            .unwrap();
        assert_eq!(队列.requests.len(), 2);
        let json = serde_json::to_string(&队列).unwrap();
        for 禁词 in ["container_ref", "worktree", "password", "example.com"] {
            assert!(!json.contains(禁词), "审批队列泄露了 {禁词}：{json}");
        }
    }

    /// 攻击样例：批准一条已被撤回的申请。必须明确失败，**且工作区不能被删**。
    #[tokio::test]
    async fn 批准已撤回的申请失败且不误删() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        handle_withdraw_request(test_db.pool(), &身份(&bob), req)
            .await
            .unwrap();

        let err = handle_claim_approval(test_db.pool(), &身份(&amy), req)
            .await
            .expect_err("已撤回的申请不得被批准");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
        assert_eq!(工作区数(&test_db).await, 1, "工作区不得被删");
    }

    /// 攻击样例：批准一条已被驳回的申请。
    #[tokio::test]
    async fn 批准已驳回的申请失败() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        handle_reject_request(
            test_db.pool(),
            &身份(&amy),
            req,
            &RejectWorkspaceDeleteRequestRequest::default(),
        )
        .await
        .unwrap();

        let err = handle_claim_approval(test_db.pool(), &身份(&amy), req)
            .await
            .expect_err("已驳回的申请不得被批准");
        assert!(matches!(err, ApiError::Conflict(_)));
        assert_eq!(工作区数(&test_db).await, 1);
    }

    /// 攻击样例：两个管理员并发批准同一条申请。
    /// **抢占只能成功一次**，也就是真正的删除只会发生一次。
    #[tokio::test]
    async fn 并发批准同一条申请只有一次抢占成功() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let zoe = 建用户(&test_db, "zoe", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        let 第一次 = handle_claim_approval(test_db.pool(), &身份(&amy), req).await;
        let 第二次 = handle_claim_approval(test_db.pool(), &身份(&zoe), req).await;

        assert!(第一次.is_ok(), "第一个管理员应抢到");
        let err = 第二次.expect_err("第二个管理员必须抢不到");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
    }

    /// 竞态：申请挂着的时候管理员绕过审批直接删掉了工作区。
    /// 批准必须干净地失败（404），不 panic，也不会去删别的工作区。
    #[tokio::test]
    async fn 申请期间工作区已被直接删掉时批准返回_404() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let 旁观 = 建工作区(&test_db, bob.id, "feat-b").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        Workspace::delete(test_db.pool(), ws.id).await.unwrap();

        let err = handle_claim_approval(test_db.pool(), &身份(&amy), req)
            .await
            .expect_err("工作区没了就批不了");
        assert!(matches!(err, ApiError::NotFound), "实际：{err:?}");

        assert!(
            Workspace::find_by_id(test_db.pool(), 旁观.id)
                .await
                .unwrap()
                .is_some(),
            "不得误删别的工作区"
        );
        assert_eq!(工作区数(&test_db).await, 1);
    }

    #[tokio::test]
    async fn 批准不存在的申请返回_404() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let err = handle_claim_approval(test_db.pool(), &身份(&amy), Uuid::new_v4())
            .await
            .expect_err("不存在应 404");
        assert!(matches!(err, ApiError::NotFound));
    }

    /// 抢占成功之后删除失败 → 回滚成 pending，不留悬空的已批准状态。
    #[tokio::test]
    async fn 抢占后回滚使申请重新可处理() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        handle_claim_approval(test_db.pool(), &身份(&amy), req)
            .await
            .unwrap();
        assert_eq!(
            WorkspaceDeleteRequests::release_claim(test_db.pool(), req)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            WorkspaceDeleteRequests::find_by_id(test_db.pool(), req)
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
        handle_claim_approval(test_db.pool(), &身份(&amy), req)
            .await
            .expect("回滚之后应能重新抢占");
    }

    #[tokio::test]
    async fn 驳回已撤回的申请返回_409() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;
        handle_withdraw_request(test_db.pool(), &身份(&bob), req)
            .await
            .unwrap();

        let err = handle_reject_request(
            test_db.pool(),
            &身份(&amy),
            req,
            &RejectWorkspaceDeleteRequestRequest::default(),
        )
        .await
        .expect_err("已撤回的申请不得被驳回");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn 驳回理由超长返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req = 建申请(&test_db, &bob, ws.id).await;

        let err = handle_reject_request(
            test_db.pool(),
            &身份(&amy),
            req,
            &RejectWorkspaceDeleteRequestRequest {
                note: Some("字".repeat(5000)),
            },
        )
        .await
        .expect_err("超长理由应 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "实际：{err:?}");
        assert_eq!(
            WorkspaceDeleteRequests::find_by_id(test_db.pool(), req)
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending",
            "400 之后申请状态不得被动到"
        );
    }

    /// 与 invites.rs / users.rs 同样的结构性守卫：每个纯函数第一行都要有
    /// `require_admin`。路由层的中间件是第一道防线，这是第二道。
    #[test]
    fn 每个_handler_都调用了_require_admin() {
        let source = include_str!("workspace_delete_requests.rs");
        let 段落: Vec<&str> = source
            .split("pub(crate) async fn handle_")
            .skip(1)
            .collect();
        assert!(段落.len() >= 3, "至少有 3 个 handler，实际 {}", 段落.len());
        for 段 in 段落 {
            let 名字 = 段.split('(').next().unwrap_or("");
            let 函数体 = 段.split("\npub").next().unwrap_or(段);
            assert!(
                函数体.contains("require_admin(actor)?"),
                "handle_{名字} 没有在函数体里调用 require_admin(actor)?"
            );
        }
    }
}
