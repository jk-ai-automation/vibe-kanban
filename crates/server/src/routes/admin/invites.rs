//! 邀请码管理（`/api/admin/invites`），仅管理员可用。
//!
//! 自助注册的那一头在 `routes/local_auth/invite_routes.rs`（免鉴权）。
//!
//! 安全约定：
//! - 库里只存邀请码的 SHA-256，**明文只在创建那一次的响应里出现**，
//!   之后列表接口再也拿不到（[`LocalInvite`] 上根本没有这个字段）。
//! - 邀请码 32 字节 `OsRng`，熵与会话令牌同级——它是免鉴权注册的唯一凭据。

use axum::{
    Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{delete, get},
};
use chrono::{DateTime, Duration, Utc};
use db::models::{
    local_auth::{DEFAULT_INVITE_TTL_DAYS, LocalInvite, LocalInvites, NewLocalInvite},
    local_user::LocalUserRole,
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use services::services::local_auth::token::{generate_invite_code, hash_invite_code};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{register_admin, require_admin};
use crate::{DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser};

/// 邀请码有效期的上限：再长就等于一张永久后门。
pub const MAX_INVITE_TTL_DAYS: i64 = 30;

/// 列表里的一条邀请。**不含 `code_hash`，更不含明文邀请码。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct LocalInviteInfo {
    pub id: Uuid,
    /// `"admin"` 或 `"member"`。
    pub role: String,
    pub created_by: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub used_by: Option<Uuid>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<&LocalInvite> for LocalInviteInfo {
    fn from(invite: &LocalInvite) -> Self {
        Self {
            id: invite.id,
            role: invite.role.as_str().to_string(),
            created_by: invite.created_by,
            expires_at: invite.expires_at,
            used_by: invite.used_by,
            used_at: invite.used_at,
            created_at: invite.created_at,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct CreateInviteRequest {
    /// `"admin"` 或 `"member"`，缺省 `"member"`。
    #[serde(default)]
    pub role: Option<String>,
    /// 有效期天数，缺省 [`DEFAULT_INVITE_TTL_DAYS`]，上限 [`MAX_INVITE_TTL_DAYS`]。
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

/// 建邀请的响应。**`code` 是明文邀请码，这是它唯一一次出现的地方**——
/// 管理员没抄下来就只能作废重建。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct CreateInviteResponse {
    pub invite: LocalInviteInfo,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct ListInvitesResponse {
    pub invites: Vec<LocalInviteInfo>,
}

// ApiError 体积较大，但全仓库的 handler 都用它；与 origin.rs 的处理一致。
#[allow(clippy::result_large_err)]
fn parse_role(raw: Option<&str>) -> Result<LocalUserRole, ApiError> {
    match raw.map(str::trim).unwrap_or("member") {
        "admin" => Ok(LocalUserRole::Admin),
        "member" => Ok(LocalUserRole::Member),
        other => Err(ApiError::BadRequest(format!("未知角色：{other}"))),
    }
}

#[allow(clippy::result_large_err)]
fn parse_ttl_days(raw: Option<i64>) -> Result<i64, ApiError> {
    let days = raw.unwrap_or(DEFAULT_INVITE_TTL_DAYS);
    if !(1..=MAX_INVITE_TTL_DAYS).contains(&days) {
        return Err(ApiError::BadRequest(format!(
            "有效期必须在 1～{MAX_INVITE_TTL_DAYS} 天之间"
        )));
    }
    Ok(days)
}

// ------------------------------------------------------------------- 列表

pub(crate) async fn handle_list_invites(
    pool: &SqlitePool,
    actor: &CurrentUser,
) -> Result<ListInvitesResponse, ApiError> {
    require_admin(actor)?;
    let invites = LocalInvites::find_all(pool).await.map_err(ApiError::from)?;
    Ok(ListInvitesResponse {
        invites: invites.iter().map(LocalInviteInfo::from).collect(),
    })
}

pub(crate) async fn list_invites(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
) -> Result<ResponseJson<ApiResponse<ListInvitesResponse>>, ApiError> {
    let payload = handle_list_invites(&deployment.db().pool, &actor).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 建码

pub(crate) async fn handle_create_invite(
    pool: &SqlitePool,
    actor: &CurrentUser,
    payload: &CreateInviteRequest,
) -> Result<CreateInviteResponse, ApiError> {
    require_admin(actor)?;
    let role = parse_role(payload.role.as_deref())?;
    let days = parse_ttl_days(payload.expires_in_days)?;

    // 明文只在这个作用域里出现：不进库、不进日志、只随本次响应返回一次。
    let code = generate_invite_code();
    let invite = LocalInvites::create(
        pool,
        NewLocalInvite {
            code_hash: hash_invite_code(&code),
            role,
            created_by: Some(actor.id),
            expires_at: Utc::now() + Duration::days(days),
        },
    )
    .await
    .map_err(ApiError::from)?;

    Ok(CreateInviteResponse {
        invite: LocalInviteInfo::from(&invite),
        code,
    })
}

pub(crate) async fn create_invite(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    axum::Json(payload): axum::Json<CreateInviteRequest>,
) -> Result<ResponseJson<ApiResponse<CreateInviteResponse>>, ApiError> {
    let payload = handle_create_invite(&deployment.db().pool, &actor, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 作废

pub(crate) async fn handle_delete_invite(
    pool: &SqlitePool,
    actor: &CurrentUser,
    invite_id: Uuid,
) -> Result<(), ApiError> {
    require_admin(actor)?;
    let removed = LocalInvites::delete(pool, invite_id)
        .await
        .map_err(ApiError::from)?;
    if removed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) async fn delete_invite(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(invite_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    handle_delete_invite(&deployment.db().pool, &actor, invite_id).await?;
    Ok(ResponseJson(ApiResponse::success("OK".to_string())))
}

// ------------------------------------------------------------------- 路由

pub(crate) fn router() -> Router<DeploymentImpl> {
    register_admin("/admin/invites", "GET");
    register_admin("/admin/invites", "POST");
    register_admin("/admin/invites/{id}", "DELETE");

    Router::new()
        .route("/admin/invites", get(list_invites).post(create_invite))
        .route("/admin/invites/{id}", delete(delete_invite))
}

#[cfg(test)]
mod tests {
    use db::{
        models::local_user::{LocalUser, LocalUsers, NewLocalUser},
        test_support::TestDb,
    };

    use super::*;

    async fn 建用户(test_db: &TestDb, username: &str, role: LocalUserRole) -> LocalUser {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: None,
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

    #[tokio::test]
    async fn member_创建邀请返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let err =
            handle_create_invite(test_db.pool(), &身份(&bob), &CreateInviteRequest::default())
                .await
                .expect_err("member 不得建邀请");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
        assert!(
            LocalInvites::find_all(test_db.pool())
                .await
                .unwrap()
                .is_empty(),
            "403 之后库里不得留下邀请"
        );
    }

    #[tokio::test]
    async fn member_读邀请列表返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let err = handle_list_invites(test_db.pool(), &身份(&bob))
            .await
            .expect_err("member 不得看邀请列表");
        assert!(matches!(err, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn member_作废邀请返回_403() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Member).await;
        let created =
            handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
                .await
                .unwrap();
        let err = handle_delete_invite(test_db.pool(), &身份(&bob), created.invite.id)
            .await
            .expect_err("member 不得作废邀请");
        assert!(matches!(err, ApiError::Forbidden(_)));
        assert_eq!(
            LocalInvites::find_all(test_db.pool()).await.unwrap().len(),
            1,
            "403 之后邀请必须还在"
        );
    }

    /// **明文邀请码只在创建响应里出现一次。** 列表接口拿不到它，
    /// 序列化出来的 `LocalInviteInfo` 里连哈希都没有。
    #[tokio::test]
    async fn 明文邀请码只在创建响应里出现一次() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let created =
            handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
                .await
                .unwrap();
        let code = created.code.clone();
        assert_eq!(code.len(), 43, "邀请码熵不足：{code}");

        let list = handle_list_invites(test_db.pool(), &身份(&amy))
            .await
            .unwrap();
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains(&code), "列表泄露了明文邀请码：{json}");
        assert!(!json.contains("code"), "列表不得含任何 code 字段：{json}");

        let 单条 = serde_json::to_string(&created.invite).unwrap();
        assert!(!单条.contains(&code), "邀请对象本身不得含明文：{单条}");
        assert!(
            !单条.contains(&hash_invite_code(&code)),
            "邀请对象本身不得含哈希：{单条}"
        );
    }

    /// 库里存的必须是哈希，不是明文——直接查底表确认。
    #[tokio::test]
    async fn 库里只存哈希不存明文() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let created =
            handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
                .await
                .unwrap();
        let 存的: String = sqlx::query_scalar("SELECT code_hash FROM local_invites LIMIT 1")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(存的, hash_invite_code(&created.code));
        assert_ne!(存的, created.code, "库里不得存明文");
        assert_eq!(存的.len(), 64, "应是 SHA-256 十六进制");
    }

    #[tokio::test]
    async fn 两次建码不会重复() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let a = handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
            .await
            .unwrap();
        let b = handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
            .await
            .unwrap();
        assert_ne!(a.code, b.code);
        assert_ne!(a.invite.id, b.invite.id);
    }

    #[tokio::test]
    async fn 默认角色是_member_默认有效期七天() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let created =
            handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
                .await
                .unwrap();
        assert_eq!(created.invite.role, "member", "默认不得是 admin");
        let 天数 = (created.invite.expires_at - Utc::now()).num_hours();
        assert!(
            (7 * 24 - 1..=7 * 24).contains(&天数),
            "默认有效期应是 7 天，实际 {天数} 小时"
        );
    }

    #[tokio::test]
    async fn 非法角色与非法有效期返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        for 角色 in ["owner", "ADMIN", ""] {
            let err = handle_create_invite(
                test_db.pool(),
                &身份(&amy),
                &CreateInviteRequest {
                    role: Some(角色.to_string()),
                    expires_in_days: None,
                },
            )
            .await
            .expect_err("非法角色应 400");
            assert!(matches!(err, ApiError::BadRequest(_)), "角色 {角色:?}");
        }
        for 天数 in [0_i64, -1, MAX_INVITE_TTL_DAYS + 1, i64::MAX] {
            let err = handle_create_invite(
                test_db.pool(),
                &身份(&amy),
                &CreateInviteRequest {
                    role: None,
                    expires_in_days: Some(天数),
                },
            )
            .await
            .expect_err("非法有效期应 400");
            assert!(matches!(err, ApiError::BadRequest(_)), "天数 {天数}");
        }
        assert!(
            LocalInvites::find_all(test_db.pool())
                .await
                .unwrap()
                .is_empty(),
            "400 之后库里不得留下邀请"
        );
    }

    #[tokio::test]
    async fn 作废不存在的邀请返回_404() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let err = handle_delete_invite(test_db.pool(), &身份(&amy), Uuid::new_v4())
            .await
            .expect_err("不存在应 404");
        assert!(matches!(err, ApiError::NotFound));
    }

    #[tokio::test]
    async fn 作废后邀请码不能再用() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", LocalUserRole::Admin).await;
        let created =
            handle_create_invite(test_db.pool(), &身份(&amy), &CreateInviteRequest::default())
                .await
                .unwrap();
        handle_delete_invite(test_db.pool(), &身份(&amy), created.invite.id)
            .await
            .unwrap();
        assert!(
            LocalInvites::find_usable_by_code_hash(
                test_db.pool(),
                &hash_invite_code(&created.code),
                Utc::now()
            )
            .await
            .unwrap()
            .is_none()
        );
    }

    /// 与 users.rs 同样的结构性守卫：每个纯函数第一行都要有 require_admin。
    #[test]
    fn 每个_handler_都调用了_require_admin() {
        let source = include_str!("invites.rs");
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
