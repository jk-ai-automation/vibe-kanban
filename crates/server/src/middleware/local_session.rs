//! 会话中间件与请求上下文里的当前用户。
//!
//! 「哪些路由免鉴权」是**结构性**的：没有被 [`require_local_session`] 包住的
//! 路由才免鉴权（见 `routes/mod.rs` 的装配）。这里**绝不**按
//! `request.uri().path()` 比对白名单——在 `.nest("/api", ...)` 内部
//! `uri().path()` 是去掉 `/api` 前缀的（证据：`error_logging.rs:9-12` 与
//! `relay_request_signature.rs` 都必须改用 `OriginalUri` 才能拿到完整路径），
//! 按字符串放行太容易写宽。

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{self, HeaderMap, Method, header, request::Parts},
    middleware::Next,
    response::Response,
};
use db::models::{
    local_auth::LocalSessions,
    local_project::DEFAULT_USER_ID,
    local_user::{LocalUserRole, LocalUserStatus, LocalUsers},
};
use relay_client::RELAY_HEADER;
use services::services::{
    local_auth::{
        csrf::csrf_check,
        runtime::LocalAuthRuntime,
        token::{
            CSRF_COOKIE, CSRF_HEADER, MACHINE_TOKEN_HEADER, SESSION_COOKIE, hash_session_token,
            parse_cookie,
        },
    },
    server_settings::ServerMode,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

/// 拒绝原因。会进日志，必须是编译期常量，不能拼进任何用户可控的内容。
pub const RELAY_DISABLED_IN_TEAM_MODE: &str = "relay_disabled_in_team_mode";

/// 一次请求该怎么过认证这道门。纯函数判定，方便单独穷举测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionGate {
    /// 免登录，注入本机所有者身份。
    PersonalBypass,
    /// 必须有有效会话。
    RequireSession,
    /// 直接拒绝（401），附固定的拒绝原因。
    Reject(&'static str),
}

/// 判定表。**顺序即优先级**：relay 的拒绝规则排在机器令牌之前，
/// 否则攻击者可以拿一个泄露的本机令牌配上 relay 头绕过整条 relay 禁令。
pub fn session_gate(mode: ServerMode, is_relay: bool, machine_token_matches: bool) -> SessionGate {
    match mode {
        // 个人版行为与历史版本逐字一致：不看任何头，一律放行。
        ServerMode::Personal => SessionGate::PersonalBypass,
        ServerMode::Team if is_relay => SessionGate::Reject(RELAY_DISABLED_IN_TEAM_MODE),
        ServerMode::Team if machine_token_matches => SessionGate::PersonalBypass,
        ServerMode::Team => SessionGate::RequireSession,
    }
}

/// 请求上下文里的当前用户。由 [`require_local_session`] 注入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    pub id: Uuid,
    pub username: String,
    pub role: LocalUserRole,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == LocalUserRole::Admin
    }
}

/// 从 request extensions 里取当前用户。
///
/// **缺失时必须报 401，绝不回退到 `DEFAULT_USER_ID`**：一旦某条路由漏挂了
/// 中间件，回退就等于「以本机所有者身份静默写入」，在团队模式下是越权。
// ApiError 体积较大，但全仓库的 handler 都用它；与 origin.rs:39 的处理一致。
#[allow(clippy::result_large_err)]
pub fn current_user(extensions: &http::Extensions) -> Result<&CurrentUser, ApiError> {
    extensions
        .get::<CurrentUser>()
        .ok_or(ApiError::Unauthorized)
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        current_user(&parts.extensions).cloned()
    }
}

/// 从 `Cookie` 头里取会话令牌。空值一律当作「没有会话」——
/// 否则 `hash_session_token("")` 会去库里查一个固定哈希。
pub fn session_token_from_header(cookie_header: Option<&str>) -> Option<String> {
    parse_cookie(cookie_header, SESSION_COOKIE).filter(|token| !token.is_empty())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
}

/// 会话中间件。挂在受保护路由组上。
pub async fn require_local_session(
    State(deployment): State<DeploymentImpl>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    use deployment::Deployment;

    let runtime = deployment.local_auth().clone();
    let pool = deployment.db().pool.clone();

    let current = authenticate(&runtime, &pool, request.method(), request.headers()).await?;

    request.extensions_mut().insert(current);
    Ok(next.run(request).await)
}

/// 中间件的全部判定逻辑，抽出来以便不起 `DeploymentImpl` 就能测。
///
/// 只读请求头与方法，不碰请求体。
#[allow(clippy::result_large_err)]
pub(crate) async fn authenticate(
    runtime: &LocalAuthRuntime,
    pool: &SqlitePool,
    method: &Method,
    headers: &HeaderMap,
) -> Result<CurrentUser, ApiError> {
    let is_relay = header_str(headers, RELAY_HEADER).is_some_and(|value| value == "1");
    let machine_ok = header_str(headers, MACHINE_TOKEN_HEADER)
        .is_some_and(|token| runtime.machine_token_matches(token));

    let current = match session_gate(runtime.mode(), is_relay, machine_ok) {
        SessionGate::Reject(reason) => {
            tracing::warn!(reason, "拒绝请求");
            return Err(ApiError::Unauthorized);
        }
        SessionGate::PersonalBypass => {
            // 个人版（以及团队版里的本机进程）走固定的本机所有者。
            // 这一行由迁移保证存在；查不到说明库被改坏了，**不能 fail-open**。
            let user = LocalUsers::find_by_id(pool, DEFAULT_USER_ID)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| {
                    tracing::error!("本机用户行缺失，数据库可能已损坏");
                    ApiError::Deployment(deployment::DeploymentError::Other(anyhow::anyhow!(
                        "本机用户行缺失"
                    )))
                })?;
            if user.status != LocalUserStatus::Active {
                tracing::error!("本机用户已被停用，拒绝免登录放行");
                return Err(ApiError::Unauthorized);
            }
            CurrentUser {
                id: user.id,
                username: user.username,
                role: user.role,
            }
        }
        SessionGate::RequireSession => {
            let cookie_header = header_str(headers, header::COOKIE.as_str());
            // 令牌明文只在这个作用域里出现，绝不进日志、绝不进错误消息。
            let token = session_token_from_header(cookie_header).ok_or(ApiError::Unauthorized)?;
            let now = chrono::Utc::now();
            let session =
                LocalSessions::find_valid_by_token_hash(pool, &hash_session_token(&token), now)
                    .await
                    .map_err(ApiError::from)?
                    .ok_or(ApiError::Unauthorized)?;
            // find_valid_by_token_hash 已经在 SQL 里要求 status='active'，
            // 这里再查一次只是为了拿用户名与角色。
            let user = LocalUsers::find_by_id(pool, session.user_id)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::Unauthorized)?;

            // 刷新 last_seen_at（5 分钟节流）。失败只记日志，不影响本次请求。
            if let Err(err) = LocalSessions::touch_last_seen(pool, session.id, now).await {
                tracing::warn!(?err, "刷新会话 last_seen_at 失败");
            }

            // 双提交 CSRF（设计 §6.3 第 3 条）。**只在这一分支做**：
            // 这里是「靠 Cookie 认出来的浏览器请求」，也是唯一存在 CSRF 的场景。
            // PersonalBypass 分支不校验——个人版与团队版的本机进程（MCP）
            // 根本不带 Cookie，加了只会把它们打死，而没有 Cookie 就没有
            // 可被跨站借用的环境凭据，跨站写由 Origin 强校验（C1）挡。
            //
            // 失败返回 **403 而不是 401**：401 会让前端误判成会话过期而跳登录页，
            // 用户反复登录也修不好（真正缺的是请求头）。
            if let Err(reason) = csrf_check(
                method.as_str(),
                parse_cookie(cookie_header, CSRF_COOKIE).as_deref(),
                header_str(headers, CSRF_HEADER),
            ) {
                // reason 是编译期常量，令牌本身绝不进日志。
                tracing::warn!(reason = reason.as_str(), "CSRF 校验失败");
                return Err(ApiError::Forbidden("请求缺少或不匹配 CSRF 令牌".into()));
            }

            CurrentUser {
                id: user.id,
                username: user.username,
                role: user.role,
            }
        }
    };

    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 个人模式一律放行() {
        assert_eq!(
            session_gate(ServerMode::Personal, false, false),
            SessionGate::PersonalBypass
        );
    }

    /// 个人模式的行为必须与历史版本逐字一致：relay 请求照常放行。
    #[test]
    fn 个人模式即使带_relay_头也放行() {
        assert_eq!(
            session_gate(ServerMode::Personal, true, false),
            SessionGate::PersonalBypass
        );
        assert_eq!(
            session_gate(ServerMode::Personal, true, true),
            SessionGate::PersonalBypass
        );
    }

    #[test]
    fn 团队模式默认要求会话() {
        assert_eq!(
            session_gate(ServerMode::Team, false, false),
            SessionGate::RequireSession
        );
    }

    /// 设计 §8.7 的 fail-closed 决定：relay 签名中间件在会话中间件**之内**，
    /// 会话层看不到它的校验结果，因此团队模式下 relay 请求一律拒绝。
    #[test]
    fn 团队模式拒绝_relay_请求() {
        assert_eq!(
            session_gate(ServerMode::Team, true, false),
            SessionGate::Reject(RELAY_DISABLED_IN_TEAM_MODE)
        );
    }

    #[test]
    fn 团队模式的机器令牌走本机所有者身份() {
        assert_eq!(
            session_gate(ServerMode::Team, false, true),
            SessionGate::PersonalBypass
        );
    }

    /// 同时带两个头时必须以「拒绝」为准，否则攻击者可以拿一个泄露的本机令牌
    /// 配上 relay 头绕过整条 relay 禁令。
    #[test]
    fn relay_头优先于机器令牌() {
        assert_eq!(
            session_gate(ServerMode::Team, true, true),
            SessionGate::Reject(RELAY_DISABLED_IN_TEAM_MODE)
        );
    }

    /// 四种输入组合全覆盖，防止以后改实现时漏掉某一格。
    #[test]
    fn 判定表全覆盖() {
        let 全部: Vec<(ServerMode, bool, bool, SessionGate)> = vec![
            (
                ServerMode::Personal,
                false,
                false,
                SessionGate::PersonalBypass,
            ),
            (
                ServerMode::Personal,
                false,
                true,
                SessionGate::PersonalBypass,
            ),
            (
                ServerMode::Personal,
                true,
                false,
                SessionGate::PersonalBypass,
            ),
            (
                ServerMode::Personal,
                true,
                true,
                SessionGate::PersonalBypass,
            ),
            (ServerMode::Team, false, false, SessionGate::RequireSession),
            (ServerMode::Team, false, true, SessionGate::PersonalBypass),
            (
                ServerMode::Team,
                true,
                false,
                SessionGate::Reject(RELAY_DISABLED_IN_TEAM_MODE),
            ),
            (
                ServerMode::Team,
                true,
                true,
                SessionGate::Reject(RELAY_DISABLED_IN_TEAM_MODE),
            ),
        ];
        for (mode, is_relay, machine_ok, expected) in 全部 {
            assert_eq!(
                session_gate(mode, is_relay, machine_ok),
                expected,
                "mode={mode:?} relay={is_relay} machine={machine_ok}"
            );
        }
    }

    #[tokio::test]
    async fn 没有中间件注入时_extractor_报_401() {
        use axum::{extract::FromRequestParts, http::Request};

        let (mut parts, _) = Request::builder().body(()).unwrap().into_parts();
        let err = CurrentUser::from_request_parts(&mut parts, &())
            .await
            .expect_err("缺少中间件注入时必须报错");
        assert!(
            matches!(err, ApiError::Unauthorized),
            "必须是 401，不能回退成本机所有者身份"
        );
    }

    #[test]
    fn 从_extensions_取当前用户_缺失时报_401() {
        let mut extensions = http::Extensions::new();
        assert!(matches!(
            current_user(&extensions),
            Err(ApiError::Unauthorized)
        ));

        let user = CurrentUser {
            id: uuid::Uuid::from_u128(2),
            username: "local".to_string(),
            role: LocalUserRole::Admin,
        };
        extensions.insert(user.clone());
        assert_eq!(current_user(&extensions).unwrap().username, "local");
    }

    /// 从 `Cookie` 头到「会话令牌」的完整路径：空值必须当作「没有会话」，
    /// 否则 `hash_session_token("")` 会去库里查一个固定哈希。
    #[test]
    fn 空的会话_cookie_当作没有会话() {
        assert_eq!(session_token_from_header(None), None);
        assert_eq!(session_token_from_header(Some("")), None);
        assert_eq!(session_token_from_header(Some("vk_session=")), None);
        assert_eq!(session_token_from_header(Some("vk_session=   ")), None);
        assert_eq!(session_token_from_header(Some("other=1")), None);
        assert_eq!(
            session_token_from_header(Some("vk_session=abc")).as_deref(),
            Some("abc")
        );
    }

    // ------------------------------------------------- C2：中间件层攻击样例

    mod 中间件层 {
        use chrono::{Duration, Utc};
        use db::{
            models::{
                local_auth::{LocalSessions, NewLocalSession},
                local_user::{LocalUser, LocalUserStatus, LocalUsers, NewLocalUser},
            },
            test_support::TestDb,
        };
        use services::services::{
            local_auth::token::{generate_csrf_token, generate_session_token},
            server_settings::ServerSettings,
        };

        use super::*;

        fn 运行时(mode: ServerMode, machine_token: &str) -> LocalAuthRuntime {
            LocalAuthRuntime::new(
                ServerSettings {
                    mode,
                    ..ServerSettings::default()
                },
                machine_token.to_string(),
            )
        }

        /// 攻击样例的请求头构造器。
        #[derive(Default)]
        struct 请求头 {
            cookies: Vec<String>,
            csrf_header: Option<String>,
            machine_token: Option<String>,
            relay: bool,
        }

        impl 请求头 {
            fn 新() -> Self {
                Self::default()
            }

            fn 会话(mut self, token: &str) -> Self {
                self.cookies.push(format!("{SESSION_COOKIE}={token}"));
                self
            }

            fn csrf_cookie(mut self, token: &str) -> Self {
                self.cookies.push(format!("{CSRF_COOKIE}={token}"));
                self
            }

            fn csrf_头(mut self, token: &str) -> Self {
                self.csrf_header = Some(token.to_string());
                self
            }

            fn 机器令牌(mut self, token: &str) -> Self {
                self.machine_token = Some(token.to_string());
                self
            }

            fn relay(mut self) -> Self {
                self.relay = true;
                self
            }

            fn 构造(&self) -> HeaderMap {
                let mut headers = HeaderMap::new();
                if !self.cookies.is_empty() {
                    headers.insert(
                        header::COOKIE,
                        self.cookies.join("; ").parse().expect("Cookie 头非法"),
                    );
                }
                if let Some(value) = &self.csrf_header {
                    headers.insert(CSRF_HEADER, value.parse().expect("CSRF 头非法"));
                }
                if let Some(value) = &self.machine_token {
                    headers.insert(MACHINE_TOKEN_HEADER, value.parse().expect("机器令牌头非法"));
                }
                if self.relay {
                    headers.insert(RELAY_HEADER, "1".parse().unwrap());
                }
                headers
            }
        }

        async fn 建用户(test_db: &TestDb, username: &str) -> LocalUser {
            LocalUsers::create(
                test_db.pool(),
                NewLocalUser {
                    username: username.to_string(),
                    display_name: username.to_string(),
                    email: None,
                    password_hash: None,
                    role: db::models::local_user::LocalUserRole::Member,
                },
            )
            .await
            .expect("建用户失败")
        }

        /// 建一条有效会话，返回明文会话令牌。
        async fn 建会话(test_db: &TestDb, user_id: uuid::Uuid, 有效天数: i64) -> String {
            let token = generate_session_token();
            LocalSessions::create(
                test_db.pool(),
                NewLocalSession {
                    user_id,
                    token_hash: hash_session_token(&token),
                    expires_at: Utc::now() + Duration::days(有效天数),
                    user_agent: None,
                    ip: None,
                },
            )
            .await
            .expect("建会话失败");
            token
        }

        fn 是_401(err: &ApiError) -> bool {
            matches!(err, ApiError::Unauthorized)
        }

        fn 是_403(err: &ApiError) -> bool {
            matches!(err, ApiError::Forbidden(_))
        }

        #[tokio::test]
        async fn 团队模式无_cookie_访问受保护路由返回_401() {
            let test_db = TestDb::new().await;
            let err = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::GET,
                &请求头::新().构造(),
            )
            .await
            .expect_err("无会话必须被拒");
            assert!(是_401(&err), "{err:?}");
        }

        /// 攻击样例：随手编一个 64 个 `a` 的令牌。
        #[tokio::test]
        async fn 团队模式伪造_cookie_返回_401() {
            let test_db = TestDb::new().await;
            for 伪造 in [
                "a".repeat(64),
                "a".repeat(43),
                "../../etc/passwd".to_string(),
            ] {
                let err = authenticate(
                    &运行时(ServerMode::Team, ""),
                    test_db.pool(),
                    &Method::GET,
                    &请求头::新().会话(&伪造).构造(),
                )
                .await
                .expect_err("伪造令牌必须被拒");
                assert!(是_401(&err), "{伪造} → {err:?}");
            }
        }

        #[tokio::test]
        async fn 团队模式过期会话返回_401() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "alice").await;
            let token = 建会话(&test_db, user.id, -1).await;
            let err = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::GET,
                &请求头::新().会话(&token).构造(),
            )
            .await
            .expect_err("过期会话必须被拒");
            assert!(是_401(&err), "{err:?}");
        }

        #[tokio::test]
        async fn 团队模式被停用用户返回_401() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "bob").await;
            let token = 建会话(&test_db, user.id, 30).await;
            LocalUsers::set_status(test_db.pool(), user.id, LocalUserStatus::Disabled)
                .await
                .expect("停用失败");
            let err = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::GET,
                &请求头::新().会话(&token).构造(),
            )
            .await
            .expect_err("停用用户必须被拒");
            assert!(是_401(&err), "{err:?}");
        }

        #[tokio::test]
        async fn 团队模式合法会话读方法不要求_csrf() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "carol").await;
            let token = 建会话(&test_db, user.id, 30).await;
            for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
                let current = authenticate(
                    &运行时(ServerMode::Team, ""),
                    test_db.pool(),
                    &method,
                    &请求头::新().会话(&token).构造(),
                )
                .await
                .unwrap_or_else(|err| panic!("{method} 应通过：{err:?}"));
                assert_eq!(current.username, "carol");
            }
        }

        /// **核心攻击样例**：跨站页面能让浏览器自动带上会话 Cookie，
        /// 但读不到 `vk_csrf` 的值，因此加不上 `X-VK-CSRF` 头。
        #[tokio::test]
        async fn 团队模式合法会话但写方法缺_csrf_返回_403() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "dave").await;
            let token = 建会话(&test_db, user.id, 30).await;
            let csrf = generate_csrf_token();

            for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                // 两条 Cookie 都在，就是没有请求头 —— 最典型的跨站写。
                let err = authenticate(
                    &运行时(ServerMode::Team, ""),
                    test_db.pool(),
                    &method,
                    &请求头::新().会话(&token).csrf_cookie(&csrf).构造(),
                )
                .await
                .unwrap_err();
                assert!(是_403(&err), "{method} 缺 CSRF 头应 403，实际 {err:?}");
            }
        }

        /// 403 而不是 401：401 会让前端误判成会话过期而跳登录页。
        #[tokio::test]
        async fn csrf_失败不是_401() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "erin").await;
            let token = 建会话(&test_db, user.id, 30).await;
            let err = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::POST,
                &请求头::新().会话(&token).构造(),
            )
            .await
            .unwrap_err();
            assert!(是_403(&err), "CSRF 失败必须是 403 而不是 401：{err:?}");
            assert!(!是_401(&err));
        }

        #[tokio::test]
        async fn 团队模式_csrf_头与_cookie_不匹配返回_403() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "frank").await;
            let token = 建会话(&test_db, user.id, 30).await;
            let csrf = generate_csrf_token();
            let 另一个 = generate_csrf_token();

            for (cookie值, 头值, 说明) in [
                (
                    Some(csrf.as_str()),
                    Some(另一个.as_str()),
                    "头与 Cookie 不同",
                ),
                (None, Some(csrf.as_str()), "只有头没有 Cookie"),
                (Some(csrf.as_str()), None, "只有 Cookie 没有头"),
                (Some(""), Some(""), "两者皆空"),
                (None, None, "两者皆无"),
            ] {
                let mut 头 = 请求头::新().会话(&token);
                if let Some(v) = cookie值 {
                    头 = 头.csrf_cookie(v);
                }
                if let Some(v) = 头值 {
                    头 = 头.csrf_头(v);
                }
                let err = authenticate(
                    &运行时(ServerMode::Team, ""),
                    test_db.pool(),
                    &Method::POST,
                    &头.构造(),
                )
                .await
                .unwrap_err();
                assert!(是_403(&err), "{说明} 应 403，实际 {err:?}");
            }
        }

        #[tokio::test]
        async fn 团队模式写方法_csrf_匹配时通过() {
            let test_db = TestDb::new().await;
            let user = 建用户(&test_db, "grace").await;
            let token = 建会话(&test_db, user.id, 30).await;
            let csrf = generate_csrf_token();
            let current = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::POST,
                &请求头::新()
                    .会话(&token)
                    .csrf_cookie(&csrf)
                    .csrf_头(&csrf)
                    .构造(),
            )
            .await
            .expect("双提交一致应通过");
            assert_eq!(current.username, "grace");
        }

        /// 个人版必须与历史版本逐字一致：不看任何头，直接给本机所有者身份。
        #[tokio::test]
        async fn 个人模式无_cookie_也能拿到本机用户() {
            let test_db = TestDb::new().await;
            let current = authenticate(
                &运行时(ServerMode::Personal, ""),
                test_db.pool(),
                &Method::GET,
                &请求头::新().构造(),
            )
            .await
            .expect("个人模式应放行");
            assert_eq!(current.id, DEFAULT_USER_ID);
        }

        /// 个人模式的写请求**不**校验 CSRF：那里根本不下发 Cookie，
        /// 没有可双提交的东西，加上只会把 MCP 与本机脚本全部打死。
        /// 个人模式的跨站写由 Origin 强校验（C1）挡住。
        #[tokio::test]
        async fn 个人模式写方法不校验_csrf() {
            let test_db = TestDb::new().await;
            for method in [Method::POST, Method::DELETE] {
                let current = authenticate(
                    &运行时(ServerMode::Personal, ""),
                    test_db.pool(),
                    &method,
                    &请求头::新().构造(),
                )
                .await
                .unwrap_or_else(|err| panic!("{method} 在个人模式应放行：{err:?}"));
                assert_eq!(current.id, DEFAULT_USER_ID);
            }
        }

        /// 团队模式下 MCP 带机器令牌做写操作，同样不该被 CSRF 打死
        /// （它不是浏览器，没有 Cookie，也设不了 Cookie）。
        #[tokio::test]
        async fn 团队模式机器令牌写方法不校验_csrf() {
            let test_db = TestDb::new().await;
            let current = authenticate(
                &运行时(ServerMode::Team, "machine-secret"),
                test_db.pool(),
                &Method::POST,
                &请求头::新().机器令牌("machine-secret").构造(),
            )
            .await
            .expect("机器令牌应放行");
            assert_eq!(current.id, DEFAULT_USER_ID);
        }

        #[tokio::test]
        async fn 团队模式错误的机器令牌仍要求会话() {
            let test_db = TestDb::new().await;
            for 送来的 in [
                "",
                " ",
                "machine-secre",
                "MACHINE-SECRET",
                "machine-secret2",
            ] {
                let err = authenticate(
                    &运行时(ServerMode::Team, "machine-secret"),
                    test_db.pool(),
                    &Method::POST,
                    &请求头::新().机器令牌(送来的).构造(),
                )
                .await
                .expect_err("错误的机器令牌必须被拒");
                assert!(是_401(&err), "{送来的:?} → {err:?}");
            }
        }

        /// 本机令牌尚未启用（空串）时，空的请求头不能被当成本机身份。
        #[tokio::test]
        async fn 机器令牌未启用时空头不匹配() {
            let test_db = TestDb::new().await;
            let err = authenticate(
                &运行时(ServerMode::Team, ""),
                test_db.pool(),
                &Method::POST,
                &请求头::新().机器令牌("").构造(),
            )
            .await
            .expect_err("空令牌必须被拒");
            assert!(是_401(&err), "{err:?}");
        }

        /// relay 头优先于机器令牌：拿一个泄露的本机令牌配上 relay 头
        /// 不能绕过团队模式的 relay 禁令。
        #[tokio::test]
        async fn 团队模式_relay_头优先于机器令牌() {
            let test_db = TestDb::new().await;
            let err = authenticate(
                &运行时(ServerMode::Team, "machine-secret"),
                test_db.pool(),
                &Method::POST,
                &请求头::新().relay().机器令牌("machine-secret").构造(),
            )
            .await
            .expect_err("团队模式的 relay 请求必须被拒");
            assert!(是_401(&err), "{err:?}");
        }
    }

    #[test]
    fn 拒绝原因是固定字符串不含任何用户输入() {
        // 拒绝原因会进日志，必须是编译期常量，不能拼进用户可控的内容。
        assert_eq!(RELAY_DISABLED_IN_TEAM_MODE, "relay_disabled_in_team_mode");
    }
}
