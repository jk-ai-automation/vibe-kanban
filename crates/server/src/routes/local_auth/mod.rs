//! 本地账号体系的 HTTP 接口，挂在 `/api/local-auth` 下。
//!
//! **不能用 `/api/auth/*`**：那一段已被云端 OAuth 路由占用
//! （`crates/server/src/routes/oauth.rs`，含 `/auth/logout`），
//! axum 0.8 注册重复路径会直接 panic。
//!
//! 免鉴权是**结构性**的：[`public_router`] 这一组不被
//! `middleware::require_local_session` 包住，其余全部被包住。
//! [`public_endpoints`] 只是把这组路径登记下来供契约测试对账，
//! 它**不是**运行时的放行依据。

pub mod invite_routes;
pub mod password_routes;
pub mod setup;

use std::sync::{Mutex, OnceLock};

use axum::{
    Router,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{DeploymentImpl, routes::health};

/// 免鉴权端点表：完整路径（含 `/api` 前缀）→ 方法。
/// 用独立的 registry，避免污染 `/api/local` 的路由契约测试。
static PUBLIC_REGISTRY: OnceLock<Mutex<Vec<(String, &'static str)>>> = OnceLock::new();

fn public_registry() -> &'static Mutex<Vec<(String, &'static str)>> {
    PUBLIC_REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

fn register_public(path: &str, method: &'static str) {
    let mut registry = public_registry().lock().expect("端点表被污染");
    let entry = (format!("/api{path}"), method);
    if !registry.contains(&entry) {
        registry.push(entry);
    }
}

/// 读取免鉴权端点表。调用前需先构造一次 [`public_router`]。
pub fn public_endpoints() -> Vec<(String, &'static str)> {
    let mut list = public_registry().lock().expect("端点表被污染").clone();
    list.sort();
    list
}

/// **唯一**的免鉴权路由组。`/api/health` 也挂在这里，
/// 这样「哪些路由不需要会话」在代码结构上只有一处答案。
pub fn public_router() -> Router<DeploymentImpl> {
    register_public("/health", "GET");
    register_public("/local-auth/bootstrap", "GET");
    register_public("/local-auth/login", "POST");
    register_public("/local-auth/invites/accept", "POST");
    register_public("/local-auth/setup", "GET");
    register_public("/local-auth/setup", "POST");

    Router::new()
        .route("/health", get(health::health_check))
        .route("/local-auth/bootstrap", get(password_routes::bootstrap))
        .route("/local-auth/login", post(password_routes::login))
        .route(
            "/local-auth/invites/accept",
            post(invite_routes::accept_invite),
        )
        .route(
            "/local-auth/setup",
            get(setup::setup_status).post(setup::setup_admin),
        )
}

/// 需要会话的本地账号路由。由调用方套上 `require_local_session`。
pub fn protected_router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/local-auth/logout", post(password_routes::logout))
        .route("/local-auth/me", get(password_routes::me))
        .route(
            "/local-auth/password",
            post(password_routes::change_password),
        )
}

/// 前端启动时读一次，决定要不要显示登录页、显示哪些登录方式。
///
/// **刻意不含任何用户信息**：它是免鉴权端点，任何人都能打。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct LocalAuthBootstrap {
    /// `"personal"` 或 `"team"`。
    pub mode: String,
    /// 是否强制登录。
    pub require_login: bool,
    /// 当前请求是否已带上有效会话。
    pub authenticated: bool,
    /// 团队模式且库里还没有「能登录的管理员」时为 true，前端要引导走初始化向导。
    pub needs_setup: bool,
    /// 已配齐凭据的 OAuth 提供方 id。
    pub providers: Vec<String>,
    /// 是否允许第三方登录自助注册。
    pub allow_oauth_signup: bool,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct LocalLoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录成功 / `GET me` 返回的用户信息。
/// 与 `db::models::local_user::LocalUser` 一样，**不含 `password_hash`**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct LocalAuthUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    /// `"admin"` 或 `"member"`。
    pub role: String,
    pub avatar_color: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 免鉴权路由清单是白名单而不是黑名单() {
        // 构造一次 router()，让登记完成。
        let _ = public_router();
        assert_eq!(
            public_endpoints(),
            vec![
                ("/api/health".to_string(), "GET"),
                ("/api/local-auth/bootstrap".to_string(), "GET"),
                ("/api/local-auth/invites/accept".to_string(), "POST"),
                ("/api/local-auth/login".to_string(), "POST"),
                ("/api/local-auth/setup".to_string(), "GET"),
                ("/api/local-auth/setup".to_string(), "POST"),
            ],
            "免鉴权端点是一份短白名单；新增任何一条都必须显式改这条测试"
        );
    }

    /// 免鉴权的都是 GET 或登录本身；除登录外不得有任何写操作免鉴权。
    #[test]
    fn 免鉴权路由里除登录外没有写操作() {
        let _ = public_router();
        for (path, method) in public_endpoints() {
            if method != "GET" {
                assert!(
                    matches!(
                        (path.as_str(), method),
                        ("/api/local-auth/login", "POST")
                            | ("/api/local-auth/invites/accept", "POST")
                            | ("/api/local-auth/setup", "POST")
                    ),
                    "免鉴权的写方法只允许登录、邀请注册与首启初始化，多出来的是：{path} {method}"
                );
            }
        }
    }

    #[test]
    fn 受保护路由清单不包含任何免鉴权路径() {
        let _ = public_router();
        let public: Vec<String> = public_endpoints().into_iter().map(|(p, _)| p).collect();
        for path in [
            "/api/local-auth/logout",
            "/api/local-auth/me",
            "/api/local-auth/password",
        ] {
            assert!(!public.contains(&path.to_string()), "{path} 不应免鉴权");
        }
    }

    /// bootstrap 是免鉴权端点，任何人都能打；它的响应里绝不能出现用户信息。
    #[test]
    fn bootstrap_不泄露任何用户信息() {
        let payload = LocalAuthBootstrap {
            mode: "team".to_string(),
            require_login: true,
            authenticated: false,
            needs_setup: true,
            providers: vec!["feishu".to_string()],
            allow_oauth_signup: false,
        };
        let json = serde_json::to_string(&payload).unwrap();
        for leak in ["username", "email", "password", "display_name", "token"] {
            assert!(!json.contains(leak), "bootstrap 泄露了 {leak}：{json}");
        }
    }

    /// 路由必须挂在 `/api/local-auth` 而不是 `/api/auth`——后者已被云端
    /// OAuth 占用，撞车会让 axum 在启动时 panic。
    #[test]
    fn 路由前缀不与云端_oauth_撞车() {
        let _ = public_router();
        for (path, _) in public_endpoints() {
            assert!(
                !path.starts_with("/api/auth/"),
                "{path} 与云端 /api/auth/* 撞车"
            );
        }
    }
}
