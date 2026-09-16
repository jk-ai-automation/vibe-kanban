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
    http::{self, header, request::Parts},
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
    local_auth::token::{MACHINE_TOKEN_HEADER, SESSION_COOKIE, hash_session_token, parse_cookie},
    server_settings::ServerMode,
};
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

fn header_str<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
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
    let pool = &deployment.db().pool;

    let is_relay = header_str(&request, RELAY_HEADER).is_some_and(|value| value == "1");
    let machine_ok = header_str(&request, MACHINE_TOKEN_HEADER)
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
            let cookie_header = header_str(&request, header::COOKIE.as_str());
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

            // TODO(C2): 写方法（POST/PUT/PATCH/DELETE）在这里做 CSRF 双提交校验。
            CurrentUser {
                id: user.id,
                username: user.username,
                role: user.role,
            }
        }
    };

    request.extensions_mut().insert(current);
    Ok(next.run(request).await)
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

    #[test]
    fn 拒绝原因是固定字符串不含任何用户输入() {
        // 拒绝原因会进日志，必须是编译期常量，不能拼进用户可控的内容。
        assert_eq!(RELAY_DISABLED_IN_TEAM_MODE, "relay_disabled_in_team_mode");
    }
}
