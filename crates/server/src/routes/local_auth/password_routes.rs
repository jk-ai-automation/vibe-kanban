//! 账号密码登录 / 登出 / 当前用户 / 改密。
//!
//! 每个 handler 都拆成「薄 axum 外壳 + 拿 `&SqlitePool` 与 `&LocalAuthRuntime`
//! 的纯逻辑函数」，逻辑函数可以直接用 `db::test_support::TestDb` 测，
//! 不必把整个 `Deployment` 拉起来。
//!
//! 本文件**不写任何 `sqlx::query!` 宏**：`crates/server/.sqlx` 没有脚本维护
//! （`scripts/prepare-db.js` 只覆盖 `crates/db/.sqlx`），所有编译期校验的 SQL
//! 一律留在 `crates/db`。

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::Json as ResponseJson,
};
use chrono::Utc;
use db::models::{
    local_auth::{LocalSessions, NewLocalSession},
    local_user::{LocalUser, LocalUserStatus, LocalUsers, normalize_username},
};
use deployment::Deployment;
use services::services::{
    local_auth::{
        password::{dummy_password_hash, hash_password, verify_password},
        runtime::LocalAuthRuntime,
        token::{
            build_csrf_clear_cookie, build_csrf_cookie, build_session_clear_cookie,
            build_session_cookie, generate_csrf_token, generate_session_token, hash_session_token,
        },
    },
    server_settings::ServerMode,
};
use sqlx::SqlitePool;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{ChangePasswordRequest, LocalAuthBootstrap, LocalAuthUser, LocalLoginRequest};
use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::local_session::{CurrentUser, session_token_from_header},
};

impl From<&LocalUser> for LocalAuthUser {
    fn from(user: &LocalUser) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            email: user.email.clone(),
            role: user.role.as_str().to_string(),
            avatar_color: user.avatar_color.clone(),
        }
    }
}

/// 登录成功后要下发的东西。两条 `Set-Cookie` 必须用 `append` 挂上去，
/// 用 `insert` 会让第二条把第一条覆盖掉。
#[derive(Debug, Clone)]
pub(crate) struct LoginOutcome {
    pub user: LocalAuthUser,
    pub session_cookie: String,
    pub csrf_cookie: String,
}

impl LoginOutcome {
    // ApiError 体积较大，但全仓库的 handler 都用它；与 origin.rs 的处理一致。
    #[allow(clippy::result_large_err)]
    fn headers(&self) -> Result<HeaderMap, ApiError> {
        let mut headers = HeaderMap::new();
        for cookie in [&self.session_cookie, &self.csrf_cookie] {
            let value = HeaderValue::from_str(cookie)
                .map_err(|_| ApiError::BadRequest("Cookie 构造失败".to_string()))?;
            headers.append(header::SET_COOKIE, value);
        }
        Ok(headers)
    }
}

fn cookie_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------- bootstrap

pub(crate) async fn handle_bootstrap(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    cookie_header: Option<&str>,
) -> Result<LocalAuthBootstrap, ApiError> {
    let team = runtime.mode() == ServerMode::Team;

    let authenticated = match session_token_from_header(cookie_header) {
        Some(token) => {
            LocalSessions::find_valid_by_token_hash(pool, &hash_session_token(&token), Utc::now())
                .await
                .map_err(ApiError::from)?
                .is_some()
        }
        None => false,
    };

    // 个人模式免登录，等价于「永远已登录」。
    let authenticated = authenticated || !team;

    let needs_setup = if team {
        LocalUsers::count_login_capable_admins(pool)
            .await
            .map_err(ApiError::from)?
            == 0
    } else {
        false
    };

    Ok(LocalAuthBootstrap {
        mode: runtime.mode().as_str().to_string(),
        require_login: team,
        authenticated,
        needs_setup,
        providers: runtime.available_providers(),
        allow_oauth_signup: runtime.allow_oauth_signup(),
    })
}

pub(crate) async fn bootstrap(
    State(deployment): State<DeploymentImpl>,
    headers: HeaderMap,
) -> Result<ResponseJson<ApiResponse<LocalAuthBootstrap>>, ApiError> {
    let payload = handle_bootstrap(
        &deployment.db().pool,
        deployment.local_auth(),
        cookie_header(&headers),
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// -------------------------------------------------------------------- login

/// 登录。
///
/// 失败一律返回 [`ApiError::Unauthorized`]，**不区分**「用户不存在」「密码错误」
/// 「账号被停用」「只有第三方登录没有密码」四种情况：响应体、状态码、耗时都相同。
/// 耗时相同靠的是——用户不存在时也拿 [`dummy_password_hash`] 跑一遍 Argon2。
///
/// 与计划的偏差：计划要求把文案固定成「用户名或密码错误」，但
/// `ApiError` 没有「带自定义消息的 401」变体，而计划同时要求不新增变体。
/// 这里用 `ApiError::Unauthorized`（固定文案，同样不可区分），
/// 面向用户的中文提示由前端（任务 G）按状态码给出。
pub(crate) async fn handle_login(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    payload: &LocalLoginRequest,
    user_agent: Option<String>,
    ip: Option<String>,
) -> Result<LoginOutcome, ApiError> {
    let username = normalize_username(&payload.username);
    let user = LocalUsers::find_by_username(pool, &username)
        .await
        .map_err(ApiError::from)?;

    // 只有「存在且启用」的用户才拿真哈希；其余一律拿假哈希，
    // 保证四条失败路径的耗时落在同一量级。
    let stored_hash = match &user {
        Some(user) if user.status == LocalUserStatus::Active => {
            LocalUsers::find_password_hash(pool, user.id)
                .await
                .map_err(ApiError::from)?
        }
        _ => None,
    };
    let hash_to_check = stored_hash.unwrap_or_else(|| dummy_password_hash().to_string());
    let password_ok = verify_password(&payload.password, &hash_to_check);

    let Some(user) = user else {
        return Err(ApiError::Unauthorized);
    };
    if !password_ok || user.status != LocalUserStatus::Active {
        return Err(ApiError::Unauthorized);
    }

    let outcome = start_session(pool, runtime, &user, user_agent, ip).await?;
    if let Err(err) = LocalUsers::touch_last_login(pool, user.id, Utc::now()).await {
        // 记不上最近登录时间不该让登录失败。
        tracing::warn!(?err, "记录 last_login_at 失败");
    }
    Ok(outcome)
}

/// 建会话并生成两条 Cookie。明文令牌只在这个函数里出现，不进日志。
async fn start_session(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    user: &LocalUser,
    user_agent: Option<String>,
    ip: Option<String>,
) -> Result<LoginOutcome, ApiError> {
    let ttl_days = runtime.session_ttl_days();
    let secure = runtime.secure_cookies();
    let token = generate_session_token();
    let csrf = generate_csrf_token();

    LocalSessions::create(
        pool,
        NewLocalSession {
            user_id: user.id,
            token_hash: hash_session_token(&token),
            expires_at: Utc::now() + chrono::Duration::days(i64::from(ttl_days)),
            user_agent,
            ip,
        },
    )
    .await
    .map_err(ApiError::from)?;

    Ok(LoginOutcome {
        user: LocalAuthUser::from(user),
        session_cookie: build_session_cookie(&token, ttl_days, secure),
        csrf_cookie: build_csrf_cookie(&csrf, ttl_days, secure),
    })
}

pub(crate) async fn login(
    State(deployment): State<DeploymentImpl>,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<LocalLoginRequest>,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<LocalAuthUser>>), ApiError> {
    let outcome = handle_login(
        &deployment.db().pool,
        deployment.local_auth(),
        &payload,
        header_text(&headers, header::USER_AGENT),
        None, // 客户端 IP 需要 ConnectInfo，由任务 C3 接上
    )
    .await?;
    let response_headers = outcome.headers()?;
    Ok((
        response_headers,
        ResponseJson(ApiResponse::success(outcome.user)),
    ))
}

// ------------------------------------------------------------------- logout

/// 登出。只撤销**当前**这一条会话，别的设备不受影响。
/// 没有会话时也返回 Ok（幂等），并照样下发清除 Cookie。
pub(crate) async fn handle_logout(
    pool: &SqlitePool,
    cookie_header: Option<&str>,
) -> Result<(), ApiError> {
    let Some(token) = session_token_from_header(cookie_header) else {
        return Ok(());
    };
    let now = Utc::now();
    if let Some(session) =
        LocalSessions::find_valid_by_token_hash(pool, &hash_session_token(&token), now)
            .await
            .map_err(ApiError::from)?
    {
        LocalSessions::revoke(pool, session.id, now)
            .await
            .map_err(ApiError::from)?;
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn clear_cookie_headers(secure: bool) -> Result<HeaderMap, ApiError> {
    let mut headers = HeaderMap::new();
    for cookie in [
        build_session_clear_cookie(secure),
        build_csrf_clear_cookie(secure),
    ] {
        let value = HeaderValue::from_str(&cookie)
            .map_err(|_| ApiError::BadRequest("Cookie 构造失败".to_string()))?;
        headers.append(header::SET_COOKIE, value);
    }
    Ok(headers)
}

pub(crate) async fn logout(
    State(deployment): State<DeploymentImpl>,
    headers: HeaderMap,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<String>>), ApiError> {
    handle_logout(&deployment.db().pool, cookie_header(&headers)).await?;
    let response_headers = clear_cookie_headers(deployment.local_auth().secure_cookies())?;
    Ok((
        response_headers,
        ResponseJson(ApiResponse::success("OK".to_string())),
    ))
}

// ----------------------------------------------------------------------- me

pub(crate) async fn handle_me(pool: &SqlitePool, user_id: Uuid) -> Result<LocalAuthUser, ApiError> {
    let user = LocalUsers::find_by_id(pool, user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(LocalAuthUser::from(&user))
}

pub(crate) async fn me(
    State(deployment): State<DeploymentImpl>,
    current: CurrentUser,
) -> Result<ResponseJson<ApiResponse<LocalAuthUser>>, ApiError> {
    let user = handle_me(&deployment.db().pool, current.id).await?;
    Ok(ResponseJson(ApiResponse::success(user)))
}

// ----------------------------------------------------------- change password

/// 改密。
///
/// 成功后**撤销该用户的全部会话**（含当前这条），再建一条新会话并下发新 Cookie：
/// 别的设备被踢下线，当前浏览器不受影响。
pub(crate) async fn handle_change_password(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    user_id: Uuid,
    payload: &ChangePasswordRequest,
    user_agent: Option<String>,
    ip: Option<String>,
) -> Result<LoginOutcome, ApiError> {
    let user = LocalUsers::find_by_id(pool, user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::Unauthorized)?;
    if user.status != LocalUserStatus::Active {
        return Err(ApiError::Unauthorized);
    }

    // 没有旧密码（只绑了第三方登录）时不能靠「旧密码为空」改密。
    let stored = LocalUsers::find_password_hash(pool, user_id)
        .await
        .map_err(ApiError::from)?
        .unwrap_or_else(|| dummy_password_hash().to_string());
    if !verify_password(&payload.current_password, &stored) {
        return Err(ApiError::Unauthorized);
    }

    let new_hash = hash_password(&payload.new_password)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    LocalUsers::set_password_hash(pool, user_id, Some(&new_hash))
        .await
        .map_err(map_local_user_error)?;
    LocalSessions::revoke_all_for_user(pool, user_id, Utc::now())
        .await
        .map_err(ApiError::from)?;

    start_session(pool, runtime, &user, user_agent, ip).await
}

pub(crate) async fn change_password(
    State(deployment): State<DeploymentImpl>,
    current: CurrentUser,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<ChangePasswordRequest>,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<LocalAuthUser>>), ApiError> {
    let outcome = handle_change_password(
        &deployment.db().pool,
        deployment.local_auth(),
        current.id,
        &payload,
        header_text(&headers, header::USER_AGENT),
        None,
    )
    .await?;
    let response_headers = outcome.headers()?;
    Ok((
        response_headers,
        ResponseJson(ApiResponse::success(outcome.user)),
    ))
}

fn map_local_user_error(error: db::models::local_user::LocalUserError) -> ApiError {
    use db::models::local_user::LocalUserError;
    match error {
        LocalUserError::InvalidUsername => ApiError::BadRequest(error.to_string()),
        LocalUserError::Validation(message) => ApiError::BadRequest(message),
        LocalUserError::Conflict(message) => ApiError::Conflict(message),
        LocalUserError::NotFound => ApiError::NotFound,
        LocalUserError::Database(err) => ApiError::from(err),
    }
}

#[cfg(test)]
mod tests {
    use db::{
        models::local_user::{LocalUserRole, NewLocalUser},
        test_support::TestDb,
    };
    use services::services::server_settings::ServerSettings;

    use super::*;

    fn runtime(mode: ServerMode) -> LocalAuthRuntime {
        LocalAuthRuntime::new(
            ServerSettings {
                mode,
                ..ServerSettings::default()
            },
            String::new(),
        )
    }

    async fn 建用户(test_db: &TestDb, username: &str, password: Option<&str>) -> LocalUser {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: None,
                password_hash: password.map(|p| hash_password(p).unwrap()),
                role: LocalUserRole::Member,
            },
        )
        .await
        .expect("建用户失败")
    }

    fn 登录请求(username: &str, password: &str) -> LocalLoginRequest {
        LocalLoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    /// 从 `Set-Cookie` 串里抠出会话令牌明文。只在测试里用。
    fn 取令牌(session_cookie: &str) -> String {
        session_cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("vk_session=")
            .to_string()
    }

    // ---------------------------------------------------------- bootstrap

    #[tokio::test]
    async fn bootstrap_个人模式不要求登录且没有提供方() {
        let test_db = TestDb::new().await;
        let payload = handle_bootstrap(test_db.pool(), &runtime(ServerMode::Personal), None)
            .await
            .unwrap();
        assert_eq!(payload.mode, "personal");
        assert!(!payload.require_login);
        assert!(payload.authenticated, "个人模式等价于永远已登录");
        assert!(!payload.needs_setup);
        assert!(payload.providers.is_empty());
        assert!(!payload.allow_oauth_signup);
    }

    #[tokio::test]
    async fn bootstrap_团队模式首启需要初始化() {
        let test_db = TestDb::new().await;
        let payload = handle_bootstrap(test_db.pool(), &runtime(ServerMode::Team), None)
            .await
            .unwrap();
        assert_eq!(payload.mode, "team");
        assert!(payload.require_login);
        assert!(!payload.authenticated);
        assert!(
            payload.needs_setup,
            "迁移写入的本机用户没有密码，不算可登录的管理员"
        );
    }

    #[tokio::test]
    async fn bootstrap_伪造的_cookie_不会被当成已登录() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        for forged in [
            "vk_session=deadbeef",
            "vk_session=",
            "vk_session_evil=x",
            "VK_SESSION=x",
            "",
        ] {
            let payload = handle_bootstrap(test_db.pool(), &rt, Some(forged))
                .await
                .unwrap();
            assert!(!payload.authenticated, "{forged:?} 不应被当成已登录");
        }
    }

    #[tokio::test]
    async fn bootstrap_带真会话时报告已登录() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let outcome = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();

        let cookie = format!("vk_session={}", 取令牌(&outcome.session_cookie));
        let payload = handle_bootstrap(test_db.pool(), &rt, Some(&cookie))
            .await
            .unwrap();
        assert!(payload.authenticated);
    }

    // -------------------------------------------------------------- login

    #[tokio::test]
    async fn 正确密码可以登录并拿到两条_cookie() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;

        let outcome = handle_login(
            test_db.pool(),
            &runtime(ServerMode::Team),
            &登录请求("amy", "hunter2hunter2"),
            Some("测试浏览器".to_string()),
            Some("127.0.0.1".to_string()),
        )
        .await
        .expect("登录应成功");

        assert_eq!(outcome.user.username, "amy");
        assert!(outcome.session_cookie.contains("HttpOnly"));
        assert!(outcome.session_cookie.contains("SameSite=Lax"));
        assert!(outcome.session_cookie.contains("Path=/"));
        assert!(outcome.session_cookie.contains("Max-Age=2592000"));
        assert!(!outcome.csrf_cookie.contains("HttpOnly"));

        // 两条 Set-Cookie 必须都在响应头里（append 而不是 insert）。
        let headers = outcome.headers().unwrap();
        assert_eq!(
            headers.get_all(header::SET_COOKIE).iter().count(),
            2,
            "第二条 Set-Cookie 被覆盖了"
        );
    }

    #[tokio::test]
    async fn 明文_http_下不带_secure_而_https_下带() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;

        let 明文 = handle_login(
            test_db.pool(),
            &runtime(ServerMode::Team),
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(!明文.session_cookie.contains("Secure"));

        let https = LocalAuthRuntime::new(
            ServerSettings {
                mode: ServerMode::Team,
                public_base_url: Some("https://kanban.example.com".to_string()),
                ..ServerSettings::default()
            },
            String::new(),
        );
        let 加密 = handle_login(
            test_db.pool(),
            &https,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(加密.session_cookie.ends_with("; Secure"));
        assert!(加密.csrf_cookie.ends_with("; Secure"));
    }

    /// 用户不存在、密码错、账号停用、只有第三方登录——四条路径必须返回
    /// 完全一样的错误，调用方无从区分。
    #[tokio::test]
    async fn 四种登录失败返回同一种_401() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let 停用 = 建用户(&test_db, "bob", Some("hunter2hunter2")).await;
        LocalUsers::set_status(test_db.pool(), 停用.id, LocalUserStatus::Disabled)
            .await
            .unwrap();
        建用户(&test_db, "carol", None).await; // 只有第三方登录，没有密码

        let 用例 = [
            ("nobody", "hunter2hunter2"), // 用户不存在
            ("amy", "wrong-password-xx"), // 密码错
            ("bob", "hunter2hunter2"),    // 账号停用
            ("carol", "hunter2hunter2"),  // 没有密码
            ("amy", ""),                  // 空密码
        ];
        for (username, password) in 用例 {
            let err = handle_login(
                test_db.pool(),
                &rt,
                &登录请求(username, password),
                None,
                None,
            )
            .await
            .expect_err("{username} 应登录失败");
            assert!(
                matches!(err, ApiError::Unauthorized),
                "{username}/{password} 返回了可区分的错误：{err:?}"
            );
        }

        // 失败不得留下任何会话行。
        let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    }

    #[tokio::test]
    async fn 超长密码不会把登录打挂() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let err = handle_login(
            test_db.pool(),
            &runtime(ServerMode::Team),
            &登录请求("amy", &"a".repeat(1024 * 1024)),
            None,
            None,
        )
        .await
        .expect_err("超长密码应失败");
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[tokio::test]
    async fn 用户名大小写与空白不影响登录() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        for username in ["amy", "AMY", "  Amy  "] {
            handle_login(
                test_db.pool(),
                &runtime(ServerMode::Team),
                &登录请求(username, "hunter2hunter2"),
                None,
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("{username} 应能登录：{e:?}"));
        }
    }

    /// 库里存的必须是哈希。这里直接把 `local_sessions` 整表读出来，
    /// 断言明文令牌不出现在任何一列里。
    #[tokio::test]
    async fn 库里存的是哈希不是明文令牌() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let outcome = handle_login(
            test_db.pool(),
            &runtime(ServerMode::Team),
            &登录请求("amy", "hunter2hunter2"),
            Some("UA".to_string()),
            Some("127.0.0.1".to_string()),
        )
        .await
        .unwrap();
        let token = 取令牌(&outcome.session_cookie);
        assert_eq!(token.len(), 43, "令牌应是 43 字符 base64url");

        let rows: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT token_hash, user_agent, ip FROM local_sessions")
                .fetch_all(test_db.pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), 1);
        let (token_hash, ua, ip) = &rows[0];
        assert_eq!(*token_hash, hash_session_token(&token));
        assert_ne!(*token_hash, token, "库里不能是明文");
        assert!(!token_hash.contains(&token));
        assert_eq!(ua.as_deref(), Some("UA"));
        assert_eq!(ip.as_deref(), Some("127.0.0.1"));

        // 密码明文与哈希也不能出现在会话表里。
        let dump: Vec<String> = sqlx::query_scalar(
            "SELECT COALESCE(token_hash,'') || COALESCE(user_agent,'') || COALESCE(ip,'') \
             FROM local_sessions",
        )
        .fetch_all(test_db.pool())
        .await
        .unwrap();
        for row in dump {
            assert!(!row.contains("hunter2hunter2"));
            assert!(!row.contains(&token));
        }
    }

    #[tokio::test]
    async fn 同一用户可以多设备并发登录() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;

        let 手机 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        let 电脑 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_ne!(取令牌(&手机.session_cookie), 取令牌(&电脑.session_cookie));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn 登录会记录最近登录时间() {
        let test_db = TestDb::new().await;
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        assert!(user.last_login_at.is_none());
        handle_login(
            test_db.pool(),
            &runtime(ServerMode::Team),
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            LocalUsers::find_by_id(test_db.pool(), user.id)
                .await
                .unwrap()
                .unwrap()
                .last_login_at
                .is_some()
        );
    }

    // ------------------------------------------------------------- logout

    #[tokio::test]
    async fn 登出只撤销当前会话() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let 手机 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        let 电脑 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();

        let 手机令牌 = 取令牌(&手机.session_cookie);
        handle_logout(test_db.pool(), Some(&format!("vk_session={手机令牌}")))
            .await
            .unwrap();

        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                &hash_session_token(&手机令牌),
                Utc::now()
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                &hash_session_token(&取令牌(&电脑.session_cookie)),
                Utc::now()
            )
            .await
            .unwrap()
            .is_some(),
            "另一台设备不应被登出"
        );
    }

    #[tokio::test]
    async fn 未登录登出是幂等的() {
        let test_db = TestDb::new().await;
        for cookie in [None, Some("vk_session="), Some("vk_session=deadbeef")] {
            handle_logout(test_db.pool(), cookie)
                .await
                .expect("登出必须幂等");
        }
    }

    #[tokio::test]
    async fn 清除_cookie_的属性与下发时一致() {
        let headers = clear_cookie_headers(false).unwrap();
        let cookies: Vec<String> = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert_eq!(cookies.len(), 2);
        assert!(cookies.iter().all(|c| c.contains("Max-Age=0")));
        assert!(cookies.iter().all(|c| c.contains("Path=/")));
        assert!(cookies.iter().all(|c| c.contains("SameSite=Lax")));
        assert!(cookies.iter().any(|c| c.starts_with("vk_session=; ")));
        assert!(cookies.iter().any(|c| c.starts_with("vk_csrf=; ")));

        let secure = clear_cookie_headers(true).unwrap();
        assert!(
            secure
                .get_all(header::SET_COOKIE)
                .iter()
                .all(|v| v.to_str().unwrap().ends_with("; Secure"))
        );
    }

    // ----------------------------------------------------------------- me

    #[tokio::test]
    async fn me_返回当前用户且不含密码字段() {
        let test_db = TestDb::new().await;
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let me = handle_me(test_db.pool(), user.id).await.unwrap();
        assert_eq!(me.username, "amy");
        assert_eq!(me.role, "member");

        let json = serde_json::to_string(&me).unwrap();
        assert!(!json.contains("password"));
        assert!(!json.contains("argon2"));
    }

    #[tokio::test]
    async fn me_查不到用户时报_401() {
        let test_db = TestDb::new().await;
        let err = handle_me(test_db.pool(), Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    // ------------------------------------------------------- change password

    #[tokio::test]
    async fn 改密后旧会话全部失效当前浏览器换新_cookie() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let 手机 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        let 电脑 = handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();

        let outcome = handle_change_password(
            test_db.pool(),
            &rt,
            user.id,
            &ChangePasswordRequest {
                current_password: "hunter2hunter2".to_string(),
                new_password: "brand-new-password".to_string(),
            },
            None,
            None,
        )
        .await
        .expect("改密应成功");

        // 两条旧会话都失效。
        for old in [&手机, &电脑] {
            assert!(
                LocalSessions::find_valid_by_token_hash(
                    test_db.pool(),
                    &hash_session_token(&取令牌(&old.session_cookie)),
                    Utc::now()
                )
                .await
                .unwrap()
                .is_none(),
                "改密必须撤销全部旧会话"
            );
        }
        // 新 Cookie 立即可用。
        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                &hash_session_token(&取令牌(&outcome.session_cookie)),
                Utc::now()
            )
            .await
            .unwrap()
            .is_some(),
            "改密后当前浏览器应保持登录"
        );

        // 新密码生效、旧密码失效。
        handle_login(
            test_db.pool(),
            &rt,
            &登录请求("amy", "brand-new-password"),
            None,
            None,
        )
        .await
        .expect("新密码应能登录");
        assert!(
            handle_login(
                test_db.pool(),
                &rt,
                &登录请求("amy", "hunter2hunter2"),
                None,
                None
            )
            .await
            .is_err(),
            "旧密码必须失效"
        );
    }

    #[tokio::test]
    async fn 改密时旧密码不对一律_401_且不改动任何东西() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        let 原哈希 = LocalUsers::find_password_hash(test_db.pool(), user.id)
            .await
            .unwrap();

        for wrong in ["", "wrong-password-x", "hunter2hunter2 ", "HUNTER2HUNTER2"] {
            let err = handle_change_password(
                test_db.pool(),
                &rt,
                user.id,
                &ChangePasswordRequest {
                    current_password: wrong.to_string(),
                    new_password: "brand-new-password".to_string(),
                },
                None,
                None,
            )
            .await
            .expect_err("旧密码不对必须失败");
            assert!(matches!(err, ApiError::Unauthorized), "{wrong:?}");
        }

        assert_eq!(
            LocalUsers::find_password_hash(test_db.pool(), user.id)
                .await
                .unwrap(),
            原哈希,
            "失败路径不得改动密码"
        );
    }

    /// 只绑了第三方登录（没有密码）的用户，不能靠「旧密码随便填」设置密码。
    #[tokio::test]
    async fn 没有密码的用户不能直接改密() {
        let test_db = TestDb::new().await;
        let user = 建用户(&test_db, "carol", None).await;
        for attempt in ["", "anything"] {
            let err = handle_change_password(
                test_db.pool(),
                &runtime(ServerMode::Team),
                user.id,
                &ChangePasswordRequest {
                    current_password: attempt.to_string(),
                    new_password: "brand-new-password".to_string(),
                },
                None,
                None,
            )
            .await
            .expect_err("没有旧密码时不得放行");
            assert!(matches!(err, ApiError::Unauthorized));
        }
    }

    #[tokio::test]
    async fn 新密码要满足强度下限与上限() {
        let test_db = TestDb::new().await;
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        for bad in ["", "short", &"a".repeat(1025)] {
            let err = handle_change_password(
                test_db.pool(),
                &runtime(ServerMode::Team),
                user.id,
                &ChangePasswordRequest {
                    current_password: "hunter2hunter2".to_string(),
                    new_password: bad.to_string(),
                },
                None,
                None,
            )
            .await
            .expect_err("不合规的新密码必须被拒");
            assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
        }
    }

    #[tokio::test]
    async fn 被停用用户不能改密() {
        let test_db = TestDb::new().await;
        let user = 建用户(&test_db, "amy", Some("hunter2hunter2")).await;
        LocalUsers::set_status(test_db.pool(), user.id, LocalUserStatus::Disabled)
            .await
            .unwrap();
        let err = handle_change_password(
            test_db.pool(),
            &runtime(ServerMode::Team),
            user.id,
            &ChangePasswordRequest {
                current_password: "hunter2hunter2".to_string(),
                new_password: "brand-new-password".to_string(),
            },
            None,
            None,
        )
        .await
        .expect_err("停用用户不得改密");
        assert!(matches!(err, ApiError::Unauthorized));
    }
}
