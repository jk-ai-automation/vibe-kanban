//! 成员管理接口（`/api/admin/users`），仅管理员可用。
//!
//! 每个 handler 都拆成「薄 axum 外壳 + 拿 `&SqlitePool` 与 `&CurrentUser`
//! 的纯逻辑函数」，纯函数可以直接用 `db::test_support::TestDb` 测，
//! 不必把整个 `Deployment` 拉起来。
//!
//! **鉴权有两道：** 路由上的 [`super::require_admin_middleware`]（结构性的，
//! 新增路由自动覆盖）与每个纯函数第一行的 [`super::require_admin`]
//! （即使有人把 handler 挂错了路由组也还挡得住）。两道都有测试。
//!
//! 本文件**不写任何 `sqlx::query!` 宏**，所有编译期校验的 SQL 一律留在
//! `crates/db`（`crates/server/.sqlx` 没有脚本维护）。

use axum::{
    Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use db::models::{
    local_auth::LocalSessions,
    local_user::{LocalUser, LocalUserRole, LocalUserStatus, LocalUsers, NewLocalUser},
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use services::services::local_auth::password::hash_password;
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{map_local_user_error, register_admin, require_admin};
use crate::{DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser};

/// 成员列表里的一行。与 `LocalAuthUser` 的区别是多了状态与时间，
/// 供管理页显示；同样**不含 `password_hash`**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct AdminUserInfo {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    /// `"admin"` 或 `"member"`。
    pub role: String,
    /// `"active"` 或 `"disabled"`。
    pub status: String,
    pub avatar_color: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl From<&LocalUser> for AdminUserInfo {
    fn from(user: &LocalUser) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            email: user.email.clone(),
            role: user.role.as_str().to_string(),
            status: user.status.as_str().to_string(),
            avatar_color: user.avatar_color.clone(),
            created_at: user.created_at,
            last_login_at: user.last_login_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct ListLocalUsersResponse {
    pub users: Vec<AdminUserInfo>,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct CreateLocalUserRequest {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password: String,
    /// `"admin"` 或 `"member"`。其它值一律 400，不静默降级成 member。
    pub role: String,
}

/// 改成员。四个字段都是可选的，省略的字段保持原值。
///
/// `email` 是双层 `Option`：字段缺失 → `None`（不动），
/// 传 `null` → `Some(None)`（清空），传字符串 → `Some(Some(_))`（改成它）。
#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct UpdateLocalUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    #[ts(optional, as = "Option<Option<String>>")]
    pub email: Option<Option<String>>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct AdminResetPasswordRequest {
    pub new_password: String,
}

// ApiError 体积较大，但全仓库的 handler 都用它；与 origin.rs 的处理一致。
#[allow(clippy::result_large_err)]
fn parse_role(raw: &str) -> Result<LocalUserRole, ApiError> {
    match raw.trim() {
        "admin" => Ok(LocalUserRole::Admin),
        "member" => Ok(LocalUserRole::Member),
        other => Err(ApiError::BadRequest(format!("未知角色：{other}"))),
    }
}

#[allow(clippy::result_large_err)]
fn parse_status(raw: &str) -> Result<LocalUserStatus, ApiError> {
    match raw.trim() {
        "active" => Ok(LocalUserStatus::Active),
        "disabled" => Ok(LocalUserStatus::Disabled),
        other => Err(ApiError::BadRequest(format!("未知状态：{other}"))),
    }
}

/// 「改完之后还剩至少一个能真正登录进来的管理员吗」。
///
/// 判据用的是 `count_login_capable_admins_except`（active + admin + 有凭据）
/// 而不是单纯的 `role='admin'`：迁移写入的本机用户是个没有任何凭据的 admin，
/// 只数角色会把它算进去，于是「把唯一一个真管理员降权」会被放行，
/// 系统从此没人能登录，也没人能改回来。
async fn 确保仍有可登录管理员(
    pool: &SqlitePool,
    被改的用户: Uuid,
) -> Result<(), ApiError> {
    let 其余 = LocalUsers::count_login_capable_admins_except(pool, 被改的用户)
        .await
        .map_err(ApiError::from)?;
    if 其余 == 0 {
        return Err(ApiError::Conflict("至少保留一个可用管理员".to_string()));
    }
    Ok(())
}

// ------------------------------------------------------------------- 列表

pub(crate) async fn handle_list_users(
    pool: &SqlitePool,
    actor: &CurrentUser,
) -> Result<ListLocalUsersResponse, ApiError> {
    require_admin(actor)?;
    let users = LocalUsers::find_all(pool).await.map_err(ApiError::from)?;
    Ok(ListLocalUsersResponse {
        users: users.iter().map(AdminUserInfo::from).collect(),
    })
}

pub(crate) async fn list_users(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
) -> Result<ResponseJson<ApiResponse<ListLocalUsersResponse>>, ApiError> {
    let payload = handle_list_users(&deployment.db().pool, &actor).await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------------- 建号

pub(crate) async fn handle_create_user(
    pool: &SqlitePool,
    actor: &CurrentUser,
    payload: &CreateLocalUserRequest,
) -> Result<AdminUserInfo, ApiError> {
    require_admin(actor)?;
    let role = parse_role(&payload.role)?;
    // 密码强度与长度上限走与登录同一套校验（Argon2id + 字节长度）。
    let password_hash =
        hash_password(&payload.password).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let user = LocalUsers::create(
        pool,
        NewLocalUser {
            username: payload.username.clone(),
            display_name: payload.display_name.clone(),
            email: payload.email.clone(),
            password_hash: Some(password_hash),
            role,
        },
    )
    .await
    .map_err(map_local_user_error)?;
    Ok(AdminUserInfo::from(&user))
}

pub(crate) async fn create_user(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    axum::Json(payload): axum::Json<CreateLocalUserRequest>,
) -> Result<ResponseJson<ApiResponse<AdminUserInfo>>, ApiError> {
    let user = handle_create_user(&deployment.db().pool, &actor, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(user)))
}

// ------------------------------------------------------------------- 改人

/// 改成员资料 / 角色 / 状态。
///
/// 三条自锁死防线，缺一不可：
/// 1. **不能改自己的角色**——否则唯一的管理员可以把自己降成 member。
/// 2. **不能停用自己**——同上，而且当场把自己踢下线。
/// 3. **不能让「可登录的管理员」归零**——管理员 A 把管理员 B 降权/停用时，
///    如果 B 是最后一个能登录的管理员，系统同样永久失去管理能力。
pub(crate) async fn handle_update_user(
    pool: &SqlitePool,
    actor: &CurrentUser,
    target_id: Uuid,
    payload: &UpdateLocalUserRequest,
) -> Result<AdminUserInfo, ApiError> {
    require_admin(actor)?;

    let target = LocalUsers::find_by_id(pool, target_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    let 新角色 = payload.role.as_deref().map(parse_role).transpose()?;
    let 新状态 = payload.status.as_deref().map(parse_status).transpose()?;
    let 是自己 = target.id == actor.id;

    if let Some(role) = 新角色
        && role != target.role
    {
        if 是自己 {
            return Err(ApiError::Forbidden("不能修改自己的角色".to_string()));
        }
        if target.role == LocalUserRole::Admin {
            确保仍有可登录管理员(pool, target.id).await?;
        }
    }

    if let Some(status) = 新状态
        && status != target.status
    {
        if 是自己 {
            return Err(ApiError::Forbidden("不能停用自己".to_string()));
        }
        if status == LocalUserStatus::Disabled {
            确保仍有可登录管理员(pool, target.id).await?;
        }
    }

    if payload.display_name.is_some() || payload.email.is_some() {
        let display_name = payload
            .display_name
            .clone()
            .unwrap_or_else(|| target.display_name.clone());
        let email = match &payload.email {
            Some(value) => value.clone(),
            None => target.email.clone(),
        };
        LocalUsers::update_profile(pool, target.id, &display_name, email.as_deref())
            .await
            .map_err(map_local_user_error)?;
    }

    if let Some(role) = 新角色
        && role != target.role
    {
        LocalUsers::set_role(pool, target.id, role)
            .await
            .map_err(map_local_user_error)?;
    }

    if let Some(status) = 新状态
        && status != target.status
    {
        LocalUsers::set_status(pool, target.id, status)
            .await
            .map_err(map_local_user_error)?;
        if status == LocalUserStatus::Disabled {
            // 停用必须**当场**撤销全部会话：光把 status 改掉，攻击者手里那张
            // Cookie 在 find_valid_by_token_hash 里确实会被 JOIN 条件挡住，
            // 但撤销是显式的、可审计的，不依赖那条 JOIN 永远不被改写。
            LocalSessions::revoke_all_for_user(pool, target.id, Utc::now())
                .await
                .map_err(ApiError::from)?;
        }
    }

    let updated = LocalUsers::find_by_id(pool, target.id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    Ok(AdminUserInfo::from(&updated))
}

pub(crate) async fn update_user(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(target_id): Path<Uuid>,
    axum::Json(payload): axum::Json<UpdateLocalUserRequest>,
) -> Result<ResponseJson<ApiResponse<AdminUserInfo>>, ApiError> {
    let user = handle_update_user(&deployment.db().pool, &actor, target_id, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(user)))
}

// -------------------------------------------------------------- 重置密码

/// 管理员重置他人密码。
///
/// 返回 `()`：**新密码不回显、旧密码读不回来**（库里只有 Argon2id 哈希，
/// 本函数也从不读它）。成功后撤销**目标用户**的全部会话，操作者自己的不动。
pub(crate) async fn handle_reset_password(
    pool: &SqlitePool,
    actor: &CurrentUser,
    target_id: Uuid,
    payload: &AdminResetPasswordRequest,
) -> Result<(), ApiError> {
    require_admin(actor)?;

    let target = LocalUsers::find_by_id(pool, target_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // 强度校验在写库之前：弱密码不得留下任何痕迹。
    let new_hash = hash_password(&payload.new_password)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;

    LocalUsers::set_password_hash(pool, target.id, Some(&new_hash))
        .await
        .map_err(map_local_user_error)?;
    LocalSessions::revoke_all_for_user(pool, target.id, Utc::now())
        .await
        .map_err(ApiError::from)?;
    Ok(())
}

pub(crate) async fn reset_password(
    State(deployment): State<DeploymentImpl>,
    actor: CurrentUser,
    Path(target_id): Path<Uuid>,
    axum::Json(payload): axum::Json<AdminResetPasswordRequest>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    handle_reset_password(&deployment.db().pool, &actor, target_id, &payload).await?;
    Ok(ResponseJson(ApiResponse::success("OK".to_string())))
}

// ------------------------------------------------------------------- 路由

pub(crate) fn router() -> Router<DeploymentImpl> {
    register_admin("/admin/users", "GET");
    register_admin("/admin/users", "POST");
    register_admin("/admin/users/{id}", "PATCH");
    register_admin("/admin/users/{id}/password", "POST");

    Router::new()
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/{id}", patch(update_user))
        .route("/admin/users/{id}/password", post(reset_password))
}

#[cfg(test)]
mod tests {
    use db::{models::local_project::DEFAULT_USER_ID, test_support::TestDb};
    use services::services::local_auth::token::hash_session_token;

    use super::*;

    async fn 建用户(
        test_db: &TestDb,
        username: &str,
        role: LocalUserRole,
        password: Option<&str>,
    ) -> LocalUser {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: None,
                password_hash: password.map(|p| hash_password(p).unwrap()),
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

    /// 给用户造一条会话，返回明文令牌。
    async fn 建会话(test_db: &TestDb, user_id: Uuid) -> String {
        let token = format!("token-{}", Uuid::new_v4());
        LocalSessions::create(
            test_db.pool(),
            db::models::local_auth::NewLocalSession {
                user_id,
                token_hash: hash_session_token(&token),
                expires_at: Utc::now() + chrono::Duration::days(30),
                user_agent: None,
                ip: None,
            },
        )
        .await
        .expect("建会话失败");
        token
    }

    async fn 会话还有效(test_db: &TestDb, token: &str) -> bool {
        LocalSessions::find_valid_by_token_hash(
            test_db.pool(),
            &hash_session_token(token),
            Utc::now(),
        )
        .await
        .unwrap()
        .is_some()
    }

    fn 建号请求(username: &str, password: &str, role: &str) -> CreateLocalUserRequest {
        CreateLocalUserRequest {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password: password.to_string(),
            role: role.to_string(),
        }
    }

    // ------------------------------------------------------- 鉴权：403 / 401

    #[tokio::test]
    async fn member_调用列表返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_list_users(test_db.pool(), &身份(&bob))
            .await
            .expect_err("member 不得看成员列表");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn member_调用建号返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_create_user(
            test_db.pool(),
            &身份(&bob),
            &建号请求("mallory", "hunter2hunter2", "admin"),
        )
        .await
        .expect_err("member 不得建号");
        assert!(matches!(err, ApiError::Forbidden(_)));
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "mallory")
                .await
                .unwrap()
                .is_none(),
            "403 之后库里不得留下用户行"
        );
    }

    /// member 想给自己提权，只有 admin 路由这一条路，而它是 403。
    #[tokio::test]
    async fn member_不能把自己改成_admin() {
        let test_db = TestDb::new().await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_update_user(
            test_db.pool(),
            &身份(&bob),
            bob.id,
            &UpdateLocalUserRequest {
                role: Some("admin".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("member 不得自提权");
        assert!(matches!(err, ApiError::Forbidden(_)));
        assert_eq!(
            LocalUsers::find_by_id(test_db.pool(), bob.id)
                .await
                .unwrap()
                .unwrap()
                .role,
            LocalUserRole::Member,
            "角色必须原封不动"
        );
    }

    #[tokio::test]
    async fn member_调用重置密码返回_403() {
        let test_db = TestDb::new().await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let 原哈希 = LocalUsers::find_password_hash(test_db.pool(), amy.id)
            .await
            .unwrap();
        let err = handle_reset_password(
            test_db.pool(),
            &身份(&bob),
            amy.id,
            &AdminResetPasswordRequest {
                new_password: "brandnewpass".to_string(),
            },
        )
        .await
        .expect_err("member 不得重置别人的密码");
        assert!(matches!(err, ApiError::Forbidden(_)));
        assert_eq!(
            LocalUsers::find_password_hash(test_db.pool(), amy.id)
                .await
                .unwrap(),
            原哈希,
            "403 之后密码不得被改动"
        );
    }

    #[tokio::test]
    async fn admin_调用列表成功() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let list = handle_list_users(test_db.pool(), &身份(&amy))
            .await
            .unwrap();
        assert!(list.users.iter().any(|u| u.username == "amy"));
    }

    /// 个人版的固定本机用户就是 admin，因此 admin 路由在个人版照常可用，
    /// 不需要按模式做任何特殊分支。
    #[tokio::test]
    async fn personal_模式的本机用户可以用_admin_路由() {
        let test_db = TestDb::new().await;
        let local = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert!(
            handle_list_users(test_db.pool(), &身份(&local))
                .await
                .is_ok()
        );
    }

    // ----------------------------------------------------- 不能自锁死

    #[tokio::test]
    async fn admin_不能把自己降级成_member() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        建用户(
            &test_db,
            "zoe",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_update_user(
            test_db.pool(),
            &身份(&amy),
            amy.id,
            &UpdateLocalUserRequest {
                role: Some("member".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不得自降权");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn admin_不能停用自己() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        建用户(
            &test_db,
            "zoe",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_update_user(
            test_db.pool(),
            &身份(&amy),
            amy.id,
            &UpdateLocalUserRequest {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不得停用自己");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn 不能停用最后一个可登录管理员() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        // bob 是 member，但假设他被提权后想停掉 amy——这里直接用 amy 自己的
        // 身份去停 amy 会被「不能停用自己」挡住，所以造第二个管理员来发起。
        LocalUsers::set_role(test_db.pool(), bob.id, LocalUserRole::Admin)
            .await
            .unwrap();
        // bob 现在是 admin，但**没有**被算成「可登录管理员」的前提是他有密码，
        // 所以这里把 bob 的密码清掉，让 amy 成为唯一一个可登录管理员。
        LocalUsers::set_password_hash(test_db.pool(), bob.id, None)
            .await
            .unwrap();
        let bob = LocalUsers::find_by_id(test_db.pool(), bob.id)
            .await
            .unwrap()
            .unwrap();

        let err = handle_update_user(
            test_db.pool(),
            &身份(&bob),
            amy.id,
            &UpdateLocalUserRequest {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不得停用最后一个可登录管理员");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
        assert_eq!(
            LocalUsers::find_by_id(test_db.pool(), amy.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            LocalUserStatus::Active
        );
    }

    #[tokio::test]
    async fn 不能把最后一个可登录管理员降级() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(&test_db, "bob", LocalUserRole::Admin, None).await;
        let err = handle_update_user(
            test_db.pool(),
            &身份(&bob),
            amy.id,
            &UpdateLocalUserRequest {
                role: Some("member".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不得降级最后一个可登录管理员");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
    }

    /// 只有「没凭据的本机 admin」陪着不算数：那种 admin 登不进来。
    #[tokio::test]
    async fn 本机固定用户不算可用管理员() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let local = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(local.role, LocalUserRole::Admin, "前提：本机用户是 admin");
        let err = handle_update_user(
            test_db.pool(),
            &身份(&local),
            amy.id,
            &UpdateLocalUserRequest {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("本机用户没有密码，不能顶替最后一个可登录管理员");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn 还有别的可登录管理员时可以停用一个管理员() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let zoe = 建用户(
            &test_db,
            "zoe",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let updated = handle_update_user(
            test_db.pool(),
            &身份(&zoe),
            amy.id,
            &UpdateLocalUserRequest {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("还有别的管理员时应允许");
        assert_eq!(updated.status, "disabled");
    }

    /// 自我保护只挡角色与状态，改自己的显示名照常。
    #[tokio::test]
    async fn admin_可以改自己的显示名() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let updated = handle_update_user(
            test_db.pool(),
            &身份(&amy),
            amy.id,
            &UpdateLocalUserRequest {
                display_name: Some("艾米".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.display_name, "艾米");
    }

    /// 只发 `role: "admin"`（与现值相同）不算「改角色」，不该被自我保护挡住。
    #[tokio::test]
    async fn 把自己的角色改成同一个值不报错() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        handle_update_user(
            test_db.pool(),
            &身份(&amy),
            amy.id,
            &UpdateLocalUserRequest {
                role: Some("admin".to_string()),
                status: Some("active".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("原地不动的改动应该通过");
    }

    // --------------------------------------------------------- 建号校验

    #[tokio::test]
    async fn 建号时用户名冲突返回_409() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        建用户(&test_db, "bob", LocalUserRole::Member, None).await;
        let err = handle_create_user(
            test_db.pool(),
            &身份(&amy),
            &建号请求("BOB", "hunter2hunter2", "member"),
        )
        .await
        .expect_err("重名应 409");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn 建号时弱密码返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        for 弱密码 in ["1234567", "", "       "] {
            let err = handle_create_user(
                test_db.pool(),
                &身份(&amy),
                &建号请求("carol", 弱密码, "member"),
            )
            .await
            .expect_err("弱密码应 400");
            assert!(matches!(err, ApiError::BadRequest(_)), "实际：{err:?}");
        }
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "carol")
                .await
                .unwrap()
                .is_none(),
            "弱密码不得留下用户行"
        );
    }

    #[tokio::test]
    async fn 建号时非法角色返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        for 角色 in ["owner", "ADMIN", "", "superadmin"] {
            let err = handle_create_user(
                test_db.pool(),
                &身份(&amy),
                &建号请求("carol", "hunter2hunter2", 角色),
            )
            .await
            .expect_err("非法角色应 400");
            assert!(matches!(err, ApiError::BadRequest(_)), "角色 {角色:?}");
        }
    }

    #[tokio::test]
    async fn 建号时非法用户名返回_400() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_create_user(
            test_db.pool(),
            &身份(&amy),
            &建号请求("ad min", "hunter2hunter2", "member"),
        )
        .await
        .expect_err("非法用户名应 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "实际：{err:?}");
    }

    #[tokio::test]
    async fn 改不存在的用户返回_404() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_update_user(
            test_db.pool(),
            &身份(&amy),
            Uuid::new_v4(),
            &UpdateLocalUserRequest {
                display_name: Some("x".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不存在应 404");
        assert!(matches!(err, ApiError::NotFound));
    }

    #[tokio::test]
    async fn 重置不存在的用户的密码返回_404() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let err = handle_reset_password(
            test_db.pool(),
            &身份(&amy),
            Uuid::new_v4(),
            &AdminResetPasswordRequest {
                new_password: "brandnewpass".to_string(),
            },
        )
        .await
        .expect_err("不存在应 404");
        assert!(matches!(err, ApiError::NotFound));
    }

    // --------------------------------------------------------- 会话撤销

    #[tokio::test]
    async fn 重置他人密码会撤销该用户全部会话() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let bob令牌一 = 建会话(&test_db, bob.id).await;
        let bob令牌二 = 建会话(&test_db, bob.id).await;

        handle_reset_password(
            test_db.pool(),
            &身份(&amy),
            bob.id,
            &AdminResetPasswordRequest {
                new_password: "brandnewpass".to_string(),
            },
        )
        .await
        .unwrap();

        assert!(
            !会话还有效(&test_db, &bob令牌一).await,
            "第一台设备应被踢下线"
        );
        assert!(
            !会话还有效(&test_db, &bob令牌二).await,
            "第二台设备也应被踢下线"
        );
    }

    #[tokio::test]
    async fn 重置他人密码不会撤销操作者自己的会话() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let amy令牌 = 建会话(&test_db, amy.id).await;
        建会话(&test_db, bob.id).await;

        handle_reset_password(
            test_db.pool(),
            &身份(&amy),
            bob.id,
            &AdminResetPasswordRequest {
                new_password: "brandnewpass".to_string(),
            },
        )
        .await
        .unwrap();

        assert!(
            会话还有效(&test_db, &amy令牌).await,
            "操作者不该把自己踢下线"
        );
    }

    #[tokio::test]
    async fn 停用用户后其会话立即失效() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let bob令牌 = 建会话(&test_db, bob.id).await;
        assert!(会话还有效(&test_db, &bob令牌).await, "前提：会话原本有效");

        handle_update_user(
            test_db.pool(),
            &身份(&amy),
            bob.id,
            &UpdateLocalUserRequest {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(
            !会话还有效(&test_db, &bob令牌).await,
            "停用后手里那张 Cookie 必须当场失效"
        );
        let 未撤销: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM local_sessions WHERE user_id = ?1 AND revoked_at IS NULL",
        )
        .bind(bob.id)
        .fetch_one(test_db.pool())
        .await
        .unwrap();
        assert_eq!(
            未撤销, 0,
            "停用必须显式撤销会话，不能只靠查询里的 JOIN 条件"
        );
    }

    #[tokio::test]
    async fn 重置密码时弱密码被拒且旧密码不变() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        let bob令牌 = 建会话(&test_db, bob.id).await;
        let 原哈希 = LocalUsers::find_password_hash(test_db.pool(), bob.id)
            .await
            .unwrap();

        let err = handle_reset_password(
            test_db.pool(),
            &身份(&amy),
            bob.id,
            &AdminResetPasswordRequest {
                new_password: "short".to_string(),
            },
        )
        .await
        .expect_err("弱密码应 400");
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert_eq!(
            LocalUsers::find_password_hash(test_db.pool(), bob.id)
                .await
                .unwrap(),
            原哈希,
            "失败的重置不得改动密码"
        );
        assert!(
            会话还有效(&test_db, &bob令牌).await,
            "失败的重置不得踢人下线"
        );
    }

    // ----------------------------------------------------------- 不泄密

    #[tokio::test]
    async fn 列表与建号响应里不含密码哈希() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let created = handle_create_user(
            test_db.pool(),
            &身份(&amy),
            &建号请求("carol", "hunter2hunter2", "member"),
        )
        .await
        .unwrap();
        let list = handle_list_users(test_db.pool(), &身份(&amy))
            .await
            .unwrap();

        for json in [
            serde_json::to_string(&created).unwrap(),
            serde_json::to_string(&list).unwrap(),
        ] {
            for 泄漏 in ["password", "argon2", "token", "hash"] {
                assert!(!json.contains(泄漏), "响应泄露了 {泄漏}：{json}");
            }
        }
    }

    /// 重置密码的返回类型是 `()`：连「新密码是什么」都回不去，
    /// 更别说读回旧密码。
    #[tokio::test]
    async fn 重置密码接口不回显任何密码() {
        let test_db = TestDb::new().await;
        let amy = 建用户(
            &test_db,
            "amy",
            LocalUserRole::Admin,
            Some("hunter2hunter2"),
        )
        .await;
        let bob = 建用户(
            &test_db,
            "bob",
            LocalUserRole::Member,
            Some("hunter2hunter2"),
        )
        .await;
        handle_reset_password(
            test_db.pool(),
            &身份(&amy),
            bob.id,
            &AdminResetPasswordRequest {
                new_password: "brandnewpass".to_string(),
            },
        )
        .await
        .unwrap();

        // 返回值是 `()`——序列化出来是 `null`，装不下任何密码。
        let 返回值: () = ();
        assert_eq!(serde_json::to_string(&返回值).unwrap(), "null");
        // 而且库里存的是 Argon2id 哈希，不是明文。
        let 哈希 = LocalUsers::find_password_hash(test_db.pool(), bob.id)
            .await
            .unwrap()
            .unwrap();
        assert!(哈希.starts_with("$argon2id$"));
        assert!(!哈希.contains("brandnewpass"), "库里不得留下明文密码");
    }

    // ------------------------------------------------------- 结构性守卫

    /// 每个 `handle_*` 纯函数的第一件事必须是 `require_admin(actor)?`。
    /// 中间件是第一道防线，这条是第二道——防止有人把 handler 挂到别的路由组里。
    #[test]
    fn 每个_handler_都调用了_require_admin() {
        let source = include_str!("users.rs");
        let 段落: Vec<&str> = source
            .split("pub(crate) async fn handle_")
            .skip(1)
            .collect();
        assert!(段落.len() >= 4, "至少有 4 个 handler，实际 {}", 段落.len());
        for 段 in 段落 {
            let 名字 = 段.split('(').next().unwrap_or("");
            let 函数体 = 段.split("\npub").next().unwrap_or(段);
            assert!(
                函数体.contains("require_admin(actor)?"),
                "handle_{名字} 没有在函数体里调用 require_admin(actor)?"
            );
        }
    }

    /// 本模块**不提供删除用户的接口**：删号会连带把他建过的工作区、
    /// 写过的评论的作者置空，而「最后一个管理员被删掉」是不可逆的。
    /// 需要收回权限时用「停用」（可逆、会话立即失效）。
    #[test]
    fn 没有删除用户的路由() {
        let _ = super::super::router();
        for (path, method) in super::super::admin_endpoints() {
            assert!(
                !(path.starts_with("/api/admin/users") && method == "DELETE"),
                "成员管理刻意不提供删除用户的接口（{path}），请用停用"
            );
        }
    }
}
