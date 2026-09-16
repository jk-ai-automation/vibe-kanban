//! 管理员专用接口，挂在 `/api/admin` 下。
//!
//! **鉴权是结构性的：** [`router`] 把这一组全部路由包进
//! [`require_admin_middleware`]，任何新增到本模块的路由自动被覆盖，
//! 不依赖作者记得在 handler 里写守卫。每个纯逻辑函数额外再调一次
//! [`require_admin`]，即使 handler 被挂到别的路由组也还挡得住——
//! 两道防线各有一条测试钉着。
//!
//! 本组必须嵌在 `require_local_session` **里面**（见 `routes/mod.rs`）：
//! `CurrentUser` 是那道中间件放进 extensions 的，未登录请求在更外层就已 401。

pub mod users;

use std::sync::{Mutex, OnceLock};

use axum::{Router, extract::Request, middleware::Next, response::Response};

use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::local_session::{CurrentUser, current_user},
};

/// 管理员端点表：完整路径（含 `/api` 前缀）→ 方法。供契约测试对账。
static ADMIN_REGISTRY: OnceLock<Mutex<Vec<(String, &'static str)>>> = OnceLock::new();

fn admin_registry() -> &'static Mutex<Vec<(String, &'static str)>> {
    ADMIN_REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn register_admin(path: &str, method: &'static str) {
    let mut registry = admin_registry().lock().expect("端点表被污染");
    let entry = (format!("/api{path}"), method);
    if !registry.contains(&entry) {
        registry.push(entry);
    }
}

/// 读取管理员端点表。调用前需先构造一次 [`router`]。
pub fn admin_endpoints() -> Vec<(String, &'static str)> {
    let mut list = admin_registry().lock().expect("端点表被污染").clone();
    list.sort();
    list
}

/// 管理员守卫。member 一律 403，未登录在更外层就已经 401。
///
/// 返回 403 而不是 404：路径本身不是秘密，而 404 会让前端把「权限不够」
/// 误报成「接口不存在」。
// ApiError 体积较大，但全仓库的 handler 都用它；与 origin.rs 的处理一致。
#[allow(clippy::result_large_err)]
pub(crate) fn require_admin(actor: &CurrentUser) -> Result<(), ApiError> {
    if actor.is_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden("需要管理员权限".to_string()))
    }
}

/// 路由层的管理员守卫。挂在整个 `/api/admin` 组上。
pub(crate) async fn require_admin_middleware(
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // 取不到 CurrentUser 说明本组没被 require_local_session 包住——
    // 这时 **必须 401**，绝不能 fail-open 放行。
    let actor = current_user(request.extensions())?;
    require_admin(actor)?;
    Ok(next.run(request).await)
}

/// `LocalUserError` → `ApiError` 的统一映射。与 `local_auth::password_routes`
/// 里的那一份保持一致（唯一约束 → 409，找不到 → 404，校验失败 → 400）。
pub(crate) fn map_local_user_error(error: db::models::local_user::LocalUserError) -> ApiError {
    use db::models::local_user::LocalUserError;
    match error {
        LocalUserError::InvalidUsername => ApiError::BadRequest(error.to_string()),
        LocalUserError::Validation(message) => ApiError::BadRequest(message),
        LocalUserError::Conflict(message) => ApiError::Conflict(message),
        LocalUserError::NotFound => ApiError::NotFound,
        LocalUserError::Database(err) => ApiError::from(err),
    }
}

/// 全部管理员路由，已套上 [`require_admin_middleware`]。
///
/// `.layer()` 只作用于**在它之前**挂上去的路由，所以新增路由必须写在
/// `.layer(...)` 这一行之前。下面的契约测试会验证端点表与实际注册一致。
pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .merge(users::router())
        .layer(axum::middleware::from_fn(require_admin_middleware))
}

#[cfg(test)]
mod tests {
    use db::models::local_user::LocalUserRole;
    use uuid::Uuid;

    use super::*;

    fn 身份(role: LocalUserRole) -> CurrentUser {
        CurrentUser {
            id: Uuid::new_v4(),
            username: "someone".to_string(),
            role,
        }
    }

    #[test]
    fn member_过不了管理员守卫() {
        let err = require_admin(&身份(LocalUserRole::Member)).expect_err("member 必须被拒");
        assert!(matches!(err, ApiError::Forbidden(_)), "实际：{err:?}");
    }

    #[test]
    fn admin_过得了管理员守卫() {
        require_admin(&身份(LocalUserRole::Admin)).expect("admin 应通过");
    }

    /// 端点表就是「哪些路径需要管理员」的唯一答案；新增任何一条都必须显式改这条测试。
    #[test]
    fn 管理员端点清单() {
        let _ = router();
        assert_eq!(
            admin_endpoints(),
            vec![
                ("/api/admin/users".to_string(), "GET"),
                ("/api/admin/users".to_string(), "POST"),
                ("/api/admin/users/{id}".to_string(), "PATCH"),
                ("/api/admin/users/{id}/password".to_string(), "POST"),
            ]
        );
    }

    /// `/api/admin` 前缀下的路径**全部**在本模块里，因此全部被
    /// `require_admin_middleware` 覆盖。反过来说，本模块里也不能混进
    /// 非 `/admin` 前缀的路径（那会绕过前端的权限约定）。
    #[test]
    fn 管理员端点全部在_admin_前缀下() {
        let _ = router();
        let endpoints = admin_endpoints();
        assert!(!endpoints.is_empty(), "端点表不该为空");
        for (path, _) in endpoints {
            assert!(path.starts_with("/api/admin/"), "{path} 不在 /api/admin 下");
        }
    }

    /// 全仓库只有本模块注册 `/admin` 路径：别处新挂一条就会绕过管理员守卫。
    #[test]
    fn 只有本模块注册_admin_路径() {
        let 根 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
        let mut 越界 = Vec::new();
        遍历(&根, &mut |路径, 内容| {
            if 路径.components().any(|c| c.as_os_str() == "admin") {
                return;
            }
            for 行 in 内容.lines() {
                let 行 = 行.trim();
                if 行.starts_with("//") {
                    continue;
                }
                if 行.contains(".route(\"/admin") || 行.contains(".nest(\"/admin") {
                    越界.push(format!("{}: {}", 路径.display(), 行));
                }
            }
        });
        assert!(
            越界.is_empty(),
            "这些地方在 routes/admin 之外注册了 /admin 路径，会绕过管理员守卫：{越界:#?}"
        );
    }

    fn 遍历(目录: &std::path::Path, 回调: &mut impl FnMut(&std::path::Path, &str)) {
        let Ok(项) = std::fs::read_dir(目录) else {
            return;
        };
        for 条目 in 项.flatten() {
            let 路径 = 条目.path();
            if 路径.is_dir() {
                遍历(&路径, 回调);
            } else if 路径.extension().is_some_and(|e| e == "rs")
                && let Ok(内容) = std::fs::read_to_string(&路径)
            {
                回调(&路径, &内容);
            }
        }
    }
}
