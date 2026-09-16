//! 首启初始化向导（`/api/local-auth/setup`），**免鉴权**——它要解决的正是
//! 「一个管理员都还没有」的鸡生蛋问题。
//!
//! 因此这一组的每一条防线都必须自己成立：
//! 1. **个人版整组 404**：个人版免登录，没有管理员这个概念。
//! 2. **库里已经有「能登录的管理员」时整组 409**。这是最关键的一条：
//!    少了它，任何人随时都能再建一个管理员，等于一条永久后门。
//! 3. **一次性令牌**：启动时生成、打印在控制台（只有能看服务器日志的人拿得到）、
//!    30 分钟过期、用一次就作废；比对走常量时间，令牌绝不进日志。
//! 4. 新用户的角色**强制**是 admin，请求体里没有 role 字段可写。
//!
//! 令牌只活在进程内存里（`LocalAuthRuntime`），重启即失效并重新打印——这是
//! 刻意的，见 `services::local_auth::runtime::SetupToken`。

use std::{net::SocketAddr, time::Instant};

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{Extensions, HeaderMap, header},
    response::Json as ResponseJson,
};
use db::models::local_user::{
    LocalUserRole, LocalUsers, NewLocalUser, normalize_username, validate_username,
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use services::services::{
    local_auth::{
        password::hash_password, rate_limit::client_ip, runtime::LocalAuthRuntime,
        token::generate_setup_token,
    },
    server_settings::ServerMode,
};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;

use super::{
    LocalAuthUser,
    password_routes::{LoginOutcome, map_local_user_error, start_session},
};
use crate::{DeploymentImpl, error::ApiError};

/// 初始化向导在限速器里用的「用户名」。理由同 `invite_routes::INVITE_RATE_KEY`：
/// 拿请求里的用户名当键，就等于让攻击者能烧掉任意用户的登录桶。
const SETUP_RATE_KEY: &str = "\u{2}setup";

/// 「已经初始化过了」的固定文案。
const ALREADY_INITIALIZED: &str = "已初始化";

#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct SetupTokenQuery {
    #[serde(default)]
    pub token: Option<String>,
}

/// `GET /api/local-auth/setup` 的响应。
///
/// 200 就意味着 `valid == true`：令牌缺失、错误、过期一律 401，
/// **不返回 `valid:false`**——那等于告诉调用方「这条路还开着，只是你猜错了」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct SetupStatusResponse {
    pub valid: bool,
}

/// 建第一个管理员。**刻意没有 `role` / `status` 字段**：
/// 角色强制是 admin，请求体里多传的键被 serde 忽略。
#[derive(Debug, Clone, Deserialize, TS)]
pub struct SetupAdminRequest {
    pub token: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// 这台服务器现在需不需要走初始化向导。
///
/// 判据与 `/api/local-auth/bootstrap` 的 `needs_setup` **完全一致**：
/// `count_login_capable_admins() == 0`（active + admin + 至少一种凭据）。
/// 迁移写入的本机用户 `local` 是 admin 但没有任何凭据，所以它不算数——
/// 不需要按 id 特判 `DEFAULT_USER_ID`，「有没有凭据」本身就是对的判据。
async fn 需要初始化(pool: &SqlitePool, runtime: &LocalAuthRuntime) -> Result<(), ApiError> {
    if runtime.mode() != ServerMode::Team {
        return Err(ApiError::NotFound);
    }
    let admins = LocalUsers::count_login_capable_admins(pool)
        .await
        .map_err(ApiError::from)?;
    if admins != 0 {
        return Err(ApiError::Conflict(ALREADY_INITIALIZED.to_string()));
    }
    Ok(())
}

// ------------------------------------------------------------- GET /setup

pub(crate) async fn handle_setup_status(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    token: Option<&str>,
    now: Instant,
) -> Result<SetupStatusResponse, ApiError> {
    需要初始化(pool, runtime).await?;

    // 空串也算缺失：否则 `?token=` 会去和「没发过令牌」的空值比。
    let token = token.map(str::trim).filter(|value| !value.is_empty());
    let Some(token) = token else {
        return Err(ApiError::Unauthorized);
    };
    // **校验不消费**：前端可以反复刷新向导页。
    if !runtime.setup_token_valid(token, now) {
        return Err(ApiError::Unauthorized);
    }
    Ok(SetupStatusResponse { valid: true })
}

pub(crate) async fn setup_status(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<SetupTokenQuery>,
) -> Result<ResponseJson<ApiResponse<SetupStatusResponse>>, ApiError> {
    let payload = handle_setup_status(
        &deployment.db().pool,
        deployment.local_auth(),
        query.token.as_deref(),
        Instant::now(),
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(payload)))
}

// ------------------------------------------------------------ POST /setup

pub(crate) async fn handle_setup_admin(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    payload: &SetupAdminRequest,
    user_agent: Option<String>,
    ip: Option<String>,
    now: Instant,
) -> Result<LoginOutcome, ApiError> {
    需要初始化(pool, runtime).await?;

    let ip_key = ip.as_deref().unwrap_or("");
    // 限速在读库与 Argon2 之前。
    if let Err(等待) = runtime.login_limiter().check(SETUP_RATE_KEY, ip_key, now) {
        return Err(ApiError::TooManyRequests(format!(
            "尝试过于频繁，请 {} 秒后再试",
            等待.as_secs().max(1)
        )));
    }

    let token = payload.token.trim();
    if token.is_empty() || !runtime.setup_token_valid(token, now) {
        runtime
            .login_limiter()
            .record_failure(SETUP_RATE_KEY, ip_key, now);
        return Err(ApiError::Unauthorized);
    }

    // 令牌是一次性的，所以**先把所有可预见的失败挡在消费之前**，
    // 否则一次拼错的用户名就会烧掉令牌，管理员只能重启服务器再来一遍。
    let username = normalize_username(&payload.username);
    validate_username(&username).map_err(map_local_user_error)?;
    if LocalUsers::find_by_username(pool, &username)
        .await
        .map_err(ApiError::from)?
        .is_some()
    {
        return Err(ApiError::Conflict("用户名已被占用".to_string()));
    }
    let password_hash =
        hash_password(&payload.password).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    // 消费令牌：一次性的判据就在这一步，两个并发请求只有一个能拿到 true。
    if !runtime.consume_setup_token(token, now) {
        return Err(ApiError::Unauthorized);
    }

    let user = LocalUsers::create(
        pool,
        NewLocalUser {
            username: payload.username.clone(),
            display_name: payload.display_name.clone(),
            email: payload.email.clone(),
            password_hash: Some(password_hash),
            // **强制 admin**，与请求体无关。
            role: LocalUserRole::Admin,
        },
    )
    .await
    .map_err(map_local_user_error)?;

    // 双保险：建成之后令牌一定是空的（consume 已清，这里再清一次也无害）。
    runtime.clear_setup_token();
    runtime.login_limiter().reset(SETUP_RATE_KEY, ip_key);

    start_session(pool, runtime, &user, user_agent, ip).await
}

pub(crate) async fn setup_admin(
    State(deployment): State<DeploymentImpl>,
    extensions: Extensions,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<SetupAdminRequest>,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<LocalAuthUser>>), ApiError> {
    let runtime = deployment.local_auth();
    let ip = client_ip(
        runtime.trust_proxy(),
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip()),
    );
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let outcome = handle_setup_admin(
        &deployment.db().pool,
        runtime,
        &payload,
        user_agent,
        Some(ip),
        Instant::now(),
    )
    .await?;
    let response_headers = outcome.headers()?;
    Ok((
        response_headers,
        ResponseJson(ApiResponse::success(outcome.user)),
    ))
}

// --------------------------------------------------------------- 启动打印

/// 团队模式且还没有可登录的管理员时，生成一次性令牌并把初始化链接打印到控制台。
///
/// 返回 `true` 表示这次确实打印了链接。**只在日志里出现一次**，不写文件、不落库。
pub async fn announce_setup_link_if_needed(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    base_url: &str,
) -> bool {
    if runtime.mode() != ServerMode::Team {
        return false;
    }
    match LocalUsers::count_login_capable_admins(pool).await {
        Ok(0) => {}
        Ok(_) => return false,
        Err(err) => {
            tracing::warn!(?err, "查询可登录管理员数量失败，跳过初始化链接打印");
            return false;
        }
    }

    let token = runtime.issue_setup_token(generate_setup_token(), Instant::now());
    // 这是明文令牌唯一一次出现在日志里：拿得到服务器控制台的人本来就有更高的权限。
    tracing::info!(
        "首次启动：请在 30 分钟内打开 {} 创建管理员",
        setup_link(base_url, &token)
    );
    true
}

/// 初始化链接。抽成纯函数是为了能直接测格式——拼错了只有在生产首启那一次
/// 才会暴露，而那时管理员正卡在「链接打不开」上。
fn setup_link(base_url: &str, token: &str) -> String {
    format!("{}/login?setup={token}", base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use db::{
        models::{
            local_auth::LocalSessions,
            local_project::DEFAULT_USER_ID,
            local_user::{LocalUserStatus, NewLocalUser},
        },
        test_support::TestDb,
    };
    use services::services::{
        local_auth::{runtime::SETUP_TOKEN_TTL, token::hash_session_token},
        server_settings::ServerSettings,
    };

    use super::*;
    use crate::routes::local_auth::password_routes::handle_bootstrap;

    fn runtime(mode: ServerMode) -> LocalAuthRuntime {
        LocalAuthRuntime::new(
            ServerSettings {
                mode,
                ..ServerSettings::default()
            },
            String::new(),
        )
    }

    fn 建号请求(token: &str, username: &str, password: &str) -> SetupAdminRequest {
        SetupAdminRequest {
            token: token.to_string(),
            username: username.to_string(),
            display_name: username.to_string(),
            password: password.to_string(),
            email: None,
        }
    }

    /// 造一个「已经有可登录管理员」的库。
    async fn 造已初始化的库(test_db: &TestDb) {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: "boss".to_string(),
                display_name: "boss".to_string(),
                email: None,
                password_hash: Some(hash_password("hunter2hunter2").unwrap()),
                role: LocalUserRole::Admin,
            },
        )
        .await
        .unwrap();
    }

    // -------------------------------------------------------- 整组可达性

    /// **最关键的一条**：库里已有可登录管理员时，整组路由不可达。
    /// 少了它，任何人随时都能再建一个管理员。
    #[tokio::test]
    async fn 已有可登录管理员时整组返回_409() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);
        造已初始化的库(&test_db).await;

        let err = handle_setup_status(test_db.pool(), &rt, Some(&token), now)
            .await
            .expect_err("已初始化应 409");
        assert!(
            matches!(&err, ApiError::Conflict(m) if m == ALREADY_INITIALIZED),
            "实际：{err:?}"
        );

        let err = handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "mallory", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect_err("已初始化应 409");
        assert!(matches!(&err, ApiError::Conflict(m) if m == ALREADY_INITIALIZED));
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "mallory")
                .await
                .unwrap()
                .is_none(),
            "409 之后不得建出第二个管理员"
        );
    }

    /// 停用的管理员不算数：他登不进来，系统仍然需要初始化。
    #[tokio::test]
    async fn 管理员被停用后重新算作未初始化() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        造已初始化的库(&test_db).await;
        let boss = LocalUsers::find_by_username(test_db.pool(), "boss")
            .await
            .unwrap()
            .unwrap();
        LocalUsers::set_status(test_db.pool(), boss.id, LocalUserStatus::Disabled)
            .await
            .unwrap();

        let token = rt.issue_setup_token(generate_setup_token(), now);
        handle_setup_status(test_db.pool(), &rt, Some(&token), now)
            .await
            .expect("没有可登录的管理员就该放行");
    }

    /// 迁移写入的 `local` 是 admin 但没有任何凭据——**仍然算未初始化**。
    #[tokio::test]
    async fn 只有固定本机用户时仍然算未初始化() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let local = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(local.role, LocalUserRole::Admin, "前提：本机用户是 admin");
        assert_eq!(
            local.status,
            LocalUserStatus::Active,
            "前提：本机用户是启用的"
        );

        let token = rt.issue_setup_token(generate_setup_token(), now);
        handle_setup_status(test_db.pool(), &rt, Some(&token), now)
            .await
            .expect("只有没凭据的本机用户时必须仍可初始化");
    }

    #[tokio::test]
    async fn personal_模式下整组返回_404() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Personal);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        assert!(matches!(
            handle_setup_status(test_db.pool(), &rt, Some(&token), now)
                .await
                .expect_err("个人版应 404"),
            ApiError::NotFound
        ));
        assert!(matches!(
            handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(&token, "mallory", "hunter2hunter2"),
                None,
                None,
                now,
            )
            .await
            .expect_err("个人版应 404"),
            ApiError::NotFound
        ));
    }

    // ------------------------------------------------------------ 令牌

    #[tokio::test]
    async fn 令牌缺失为空或错误一律_401() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        rt.issue_setup_token("the-real-token".to_string(), now);

        for 错的 in [
            None,
            Some(""),
            Some("   "),
            Some("wrong-token"),
            Some("the-real-toke"),
            Some("THE-REAL-TOKEN"),
        ] {
            let err = handle_setup_status(test_db.pool(), &rt, 错的, now)
                .await
                .expect_err("缺失 / 空串 / 错误令牌都必须 401");
            assert!(matches!(err, ApiError::Unauthorized), "{错的:?}：{err:?}");
        }
        // 正确令牌照常放行，而且不会被上面几次失败消费掉。
        assert!(
            handle_setup_status(test_db.pool(), &rt, Some("the-real-token"), now)
                .await
                .unwrap()
                .valid
        );
    }

    #[tokio::test]
    async fn 错误令牌不能建管理员() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        rt.issue_setup_token("the-real-token".to_string(), now);

        for 错的 in ["", "   ", "wrong-token", "the-real-toke", "THE-REAL-TOKEN"] {
            let err = handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(错的, "mallory", "hunter2hunter2"),
                None,
                None,
                now,
            )
            .await
            .expect_err("错误令牌必须被拒");
            assert!(matches!(err, ApiError::Unauthorized), "{错的:?}：{err:?}");
        }
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "mallory")
                .await
                .unwrap()
                .is_none()
        );
        // 猜错不得作废正确的令牌。
        handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求("the-real-token", "amy", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect("正确令牌必须还能用");
    }

    /// 没发过令牌时**任何**输入都不能通过——包括空串（防 fail-open）。
    #[tokio::test]
    async fn 没发过令牌时一律_401() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        for 输入 in ["", "anything"] {
            let err = handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(输入, "mallory", "hunter2hunter2"),
                None,
                None,
                now,
            )
            .await
            .expect_err("没发过令牌时必须拒绝");
            assert!(matches!(err, ApiError::Unauthorized), "{输入:?}：{err:?}");
        }
    }

    #[tokio::test]
    async fn 令牌三十分钟后过期() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        handle_setup_status(
            test_db.pool(),
            &rt,
            Some(&token),
            now + SETUP_TOKEN_TTL - Duration::from_secs(1),
        )
        .await
        .expect("29 分 59 秒时仍有效");

        let err = handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "amy", "hunter2hunter2"),
            None,
            None,
            now + SETUP_TOKEN_TTL,
        )
        .await
        .expect_err("到点必须过期");
        assert!(matches!(err, ApiError::Unauthorized));
    }

    /// **一次性**：建成第一个管理员之后，同一张令牌再也用不了——
    /// 哪怕 30 分钟还没到。
    #[tokio::test]
    async fn 令牌用后立即失效() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "amy", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect("第一次应成功");

        // 第二次：先撞上「已初始化」这道 409（两道防线的外层）。
        let err = handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "mallory", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect_err("第二次必须失败");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");

        // 就算把库清空（绕过外层 409），令牌本身也已经作废了。
        sqlx::query("DELETE FROM local_users WHERE username = 'amy'")
            .execute(test_db.pool())
            .await
            .unwrap();
        let err = handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "mallory", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect_err("令牌是一次性的");
        assert!(matches!(err, ApiError::Unauthorized), "实际：{err:?}");
    }

    /// 并发两次 setup 只有一个成功：令牌一次性 + 已初始化检查双保险。
    ///
    /// 用多线程运行时，让两路真的并行进入 `handle_setup_admin`；
    /// 默认的单线程 runtime 会把两个 spawn 排成先后，测不到竞态。
    ///
    /// 注意：这条是端到端的合理性检查，**不是**一次性语义的权威测试——
    /// 竞态窗口很窄，跑一次未必撞上。真正把「令牌只能消费一次」钉死的是
    /// `services::local_auth::runtime` 里的 `令牌用后立即失效`（确定性的），
    /// 加上本文件里同名的那条（走 `clear_setup_token` 这第二道保险）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 并发两次_setup_只有一个成功() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        let 跑 = |用户名: &'static str| {
            let pool = test_db.pool().clone();
            let rt = rt.clone();
            let token = token.clone();
            async move {
                handle_setup_admin(
                    &pool,
                    &rt,
                    &建号请求(&token, 用户名, "hunter2hunter2"),
                    None,
                    None,
                    now,
                )
                .await
                .map(|o| o.user.username)
            }
        };
        let a = tokio::spawn(跑("racer-a"));
        let b = tokio::spawn(跑("racer-b"));
        let (ra, rb) = (a.await.unwrap(), b.await.unwrap());
        assert_eq!(
            [&ra, &rb].iter().filter(|r| r.is_ok()).count(),
            1,
            "只能有一个成功：{ra:?} / {rb:?}"
        );
        let 管理员数: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM local_users WHERE username LIKE 'racer-%'")
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(管理员数, 1);
    }

    // ------------------------------------------------------------ 建号

    #[tokio::test]
    async fn 建成的第一个用户角色强制是_admin() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        // 请求体里多塞 role/status，serde 直接忽略（DTO 上没有这两个字段）。
        let payload: SetupAdminRequest = serde_json::from_value(serde_json::json!({
            "token": token,
            "username": "amy",
            "display_name": "amy",
            "password": "hunter2hunter2",
            "role": "member",
            "status": "disabled",
        }))
        .expect("多余字段应被忽略");

        let outcome = handle_setup_admin(test_db.pool(), &rt, &payload, None, None, now)
            .await
            .unwrap();
        assert_eq!(outcome.user.role, "admin", "第一个用户必须是管理员");
        let amy = LocalUsers::find_by_username(test_db.pool(), "amy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(amy.status, LocalUserStatus::Active);
    }

    /// 初始化完成后，`bootstrap` 的 `needs_setup` 必须变成 false，
    /// 否则前端会一直把用户送回向导页。
    #[tokio::test]
    async fn 初始化完成后_needs_setup_变成_false() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        assert!(
            handle_bootstrap(test_db.pool(), &rt, None)
                .await
                .unwrap()
                .needs_setup
        );

        let token = rt.issue_setup_token(generate_setup_token(), now);
        handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "amy", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .unwrap();

        assert!(
            !handle_bootstrap(test_db.pool(), &rt, None)
                .await
                .unwrap()
                .needs_setup,
            "初始化完成后不该再要求走向导"
        );
    }

    #[tokio::test]
    async fn 建成后当场下发有效会话() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        let outcome = handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "amy", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .unwrap();

        let session_token = outcome
            .session_cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("vk_session=");
        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                &hash_session_token(session_token),
                chrono::Utc::now()
            )
            .await
            .unwrap()
            .is_some(),
            "建完管理员应当场登录"
        );
        assert!(outcome.csrf_cookie.starts_with("vk_csrf="));
    }

    /// 弱密码被拒，而且**不能烧掉令牌**——否则一次拼错就要重启服务器。
    #[tokio::test]
    async fn 弱密码被拒且令牌仍可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        for 弱密码 in ["1234567", "", "       "] {
            let err = handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(&token, "amy", 弱密码),
                None,
                None,
                now,
            )
            .await
            .expect_err("弱密码应 400");
            assert!(
                matches!(err, ApiError::BadRequest(_)),
                "{弱密码:?}：{err:?}"
            );
        }
        assert!(rt.setup_token_valid(&token, now), "失败的尝试不得烧掉令牌");
        handle_setup_admin(
            test_db.pool(),
            &rt,
            &建号请求(&token, "amy", "hunter2hunter2"),
            None,
            None,
            now,
        )
        .await
        .expect("改对密码后应能成功");
    }

    #[tokio::test]
    async fn 非法或占用的用户名被拒且令牌仍可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        let token = rt.issue_setup_token(generate_setup_token(), now);

        // `local` 是迁移写入的固定用户，必然占用。
        for (用户名, 期望冲突) in [("ad min", false), ("管理员", false), ("LOCAL", true)]
        {
            let err = handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(&token, 用户名, "hunter2hunter2"),
                None,
                None,
                now,
            )
            .await
            .expect_err("非法/占用的用户名应被拒");
            if 期望冲突 {
                assert!(matches!(err, ApiError::Conflict(_)), "{用户名:?}：{err:?}");
            } else {
                assert!(
                    matches!(err, ApiError::BadRequest(_)),
                    "{用户名:?}：{err:?}"
                );
            }
        }
        assert!(rt.setup_token_valid(&token, now), "失败的尝试不得烧掉令牌");
    }

    /// 反复猜令牌要被限速，且不得污染真实用户的登录桶（同 IP 场景）。
    #[tokio::test]
    async fn 反复猜令牌会被限速且不影响真实用户登录() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let now = Instant::now();
        const 同一个_IP: &str = "10.0.0.7";
        rt.issue_setup_token(generate_setup_token(), now);

        let mut 被限速 = false;
        for _ in 0..40 {
            let err = handle_setup_admin(
                test_db.pool(),
                &rt,
                &建号请求(&generate_setup_token(), "amy", "hunter2hunter2"),
                None,
                Some(同一个_IP.to_string()),
                now,
            )
            .await
            .expect_err("乱猜必失败");
            if matches!(err, ApiError::TooManyRequests(_)) {
                被限速 = true;
                break;
            }
        }
        assert!(被限速, "反复猜令牌必须被限速");
        assert!(
            rt.login_limiter().check("amy", 同一个_IP, now).is_ok(),
            "初始化向导的限速烧掉了真实用户 amy 的登录桶"
        );
    }

    // ------------------------------------------------------- 启动打印

    #[tokio::test]
    async fn 只有团队模式且未初始化时才打印链接() {
        let test_db = TestDb::new().await;

        let personal = runtime(ServerMode::Personal);
        assert!(
            !announce_setup_link_if_needed(test_db.pool(), &personal, "http://127.0.0.1:8080")
                .await
        );

        let team = runtime(ServerMode::Team);
        assert!(
            announce_setup_link_if_needed(test_db.pool(), &team, "http://127.0.0.1:8080").await,
            "团队模式首启必须打印链接"
        );

        造已初始化的库(&test_db).await;
        let team2 = runtime(ServerMode::Team);
        assert!(
            !announce_setup_link_if_needed(test_db.pool(), &team2, "http://127.0.0.1:8080").await,
            "已初始化就不该再发令牌"
        );
    }

    /// 链接格式：必须是 `<base>/login?setup=<token>`，
    /// 而且 base_url 末尾多一个斜杠也不能拼出 `//login`。
    #[test]
    fn 初始化链接格式() {
        assert_eq!(
            setup_link("http://127.0.0.1:8080", "abc"),
            "http://127.0.0.1:8080/login?setup=abc"
        );
        assert_eq!(
            setup_link("http://127.0.0.1:8080/", "abc"),
            "http://127.0.0.1:8080/login?setup=abc",
            "末尾斜杠不能拼出 //login"
        );
    }

    #[tokio::test]
    async fn 打印链接时发出的令牌确实可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        assert!(announce_setup_link_if_needed(test_db.pool(), &rt, "http://127.0.0.1:8080").await);
        // 令牌的明文只在日志里；这里只能验证「发过一张」。
        assert!(!rt.setup_token_valid("guess", Instant::now()), "乱猜不该过");
    }

    /// **令牌不进日志。** 本文件里唯一一处把令牌写进 `tracing` 的地方是
    /// 启动打印（那是给服务器管理员看的），除此之外任何 `tracing::` 调用
    /// 都不得带上令牌。
    #[test]
    fn 令牌只在启动打印里出现一次() {
        let source = include_str!("setup.rs");
        let 主体 = source.split("#[cfg(test)]").next().unwrap();
        // 唯一允许把令牌写进日志的地方：启动打印那一句（走 setup_link 拼装）。
        const 允许的那一句: &str = "setup_link(base_url, &token)";
        let 带令牌的日志: Vec<&str> = 主体
            .lines()
            .filter(|行| 行.contains("token") && 行.contains("tracing::"))
            .filter(|行| !行.contains(允许的那一句))
            .collect();
        assert!(
            带令牌的日志.is_empty(),
            "这些行把令牌写进了日志：{带令牌的日志:?}"
        );
        assert_eq!(
            主体.matches(允许的那一句).count(),
            1,
            "明文令牌只允许出现在启动打印那一句里"
        );
        // 错误路径上的日志也不得带令牌（它们拿到的是猜错的值，同样别回显）。
        for 行 in 主体.lines().filter(|行| 行.contains("tracing::")) {
            assert!(
                !行.contains("payload.token") && !行.contains("candidate"),
                "日志里出现了请求里的令牌：{行}"
            );
        }
    }
}
