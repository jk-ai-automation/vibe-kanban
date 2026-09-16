use std::net::SocketAddr;

use axum::{Router, extract::connect_info::IntoMakeServiceWithConnectInfo, routing::get};
use tower_http::{compression::CompressionLayer, validate_request::ValidateRequestHeaderLayer};

use crate::{DeploymentImpl, middleware};

pub mod approvals;
pub mod config;
pub mod containers;
pub mod filesystem;
// pub mod github;
pub mod attachments;
pub mod events;
pub mod execution_processes;
pub mod frontend;
pub mod health;
pub mod host_relay;
pub mod issues;
pub mod local_auth;
pub mod local_projects;
pub mod oauth;
pub mod organizations;
pub mod preview;
pub mod relay_auth;
pub mod releases;
pub mod remote;
pub mod repo;
pub mod scratch;
pub mod search;
pub mod sessions;
pub mod ssh_session;
pub mod tags;
pub mod terminal;
pub mod webrtc;
pub mod workspaces;

/// 带 `ConnectInfo<SocketAddr>` 的 make service。
///
/// 必须是 `with_connect_info` 版本：登录限速要按真实对端 IP 分桶，
/// 换回 `into_make_service()` 会让 handler 里的 `ConnectInfo` 永远取不到，
/// 所有请求挤进 `"unknown"` 一只桶，限速退化成全局锁死。
pub fn router(deployment: DeploymentImpl) -> IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
    // /health 从这一组移到免鉴权组（local_auth::public_router）。
    let relay_signed_routes = Router::new()
        .merge(config::router())
        .merge(containers::router(&deployment))
        .merge(workspaces::router(&deployment))
        .merge(execution_processes::router(&deployment))
        .merge(tags::router(&deployment))
        .merge(local_projects::router())
        .merge(issues::router())
        .merge(oauth::router())
        .merge(organizations::router())
        .merge(filesystem::router())
        .merge(repo::router())
        .merge(events::router(&deployment))
        .merge(approvals::router())
        .merge(scratch::router(&deployment))
        .merge(search::router(&deployment))
        .merge(preview::api_router())
        .merge(releases::router())
        .merge(sessions::router(&deployment))
        .merge(terminal::router())
        .route("/ssh-session", get(ssh_session::ssh_session_ws))
        .nest("/remote", remote::router())
        .merge(webrtc::router())
        .nest("/attachments", attachments::routes())
        .layer(axum::middleware::from_fn_with_state(
            deployment.clone(),
            middleware::sign_relay_response,
        ))
        .layer(axum::middleware::from_fn_with_state(
            deployment.clone(),
            middleware::require_relay_request_signature,
        ))
        .with_state(deployment.clone());

    // 受保护组：被 require_local_session 包住的一切。
    // 「哪些路由免鉴权」因此是结构性的 fail-closed——新增路由默认落在这一组里，
    // 而不是靠中间件内部比对路径白名单（nest("/api") 内 uri().path() 已去掉前缀，
    // 按字符串放行极易写宽）。
    let protected_routes = Router::new()
        .merge(relay_auth::router())
        .merge(host_relay::router(&deployment))
        .merge(relay_signed_routes)
        .merge(local_auth::protected_router())
        .layer(axum::middleware::from_fn_with_state(
            deployment.clone(),
            middleware::require_local_session,
        ));

    let api_routes = Router::new()
        .merge(local_auth::public_router())
        .merge(protected_routes)
        .layer(ValidateRequestHeaderLayer::custom(
            middleware::validate_origin,
        ))
        .layer(axum::middleware::from_fn(middleware::log_server_errors))
        .with_state(deployment);

    Router::new()
        .route("/", get(frontend::serve_frontend_root))
        .route("/{*path}", get(frontend::serve_frontend))
        .nest("/api", api_routes)
        .layer(CompressionLayer::new())
        .into_make_service_with_connect_info::<SocketAddr>()
}
