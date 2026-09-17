use api_types::{
    AuthMethodsResponse, HandoffInitRequest, HandoffRedeemRequest, LocalLoginRequest,
    ProfileResponse, StatusResponse,
};
use axum::{
    Router,
    extract::{Json, Query, State},
    http::{Response, StatusCode, header},
    response::Json as ResponseJson,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use deployment::Deployment;
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use services::services::{
    config::save_config_to_file,
    local_auth::token::{
        HANDOFF_NONCE_COOKIE, build_handoff_nonce_clear_cookie, build_handoff_nonce_cookie,
        parse_cookie,
    },
    oauth_credentials::Credentials,
    oauth_handoff::{
        HANDOFF_TTL_MINUTES, HandoffRejection, NonceBinding, cloud_handoff_allowed,
        generate_handoff_nonce,
    },
    remote_sync,
};
use sha2::{Digest, Sha256};
use ts_rs::TS;
use utils::{assets::config_path, jwt::extract_expiration, response::ApiResponse};
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError, runtime::relay_registration};

/// Base64-encoded 32x32 app icon (from `crates/tauri-app/icons/32x32.png`).
const APP_ICON_BASE64: &str = "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";

/// Shared CSS styles for standalone OAuth HTML pages (success & error).
/// Colors and typography match the app's design system (light mode defaults
/// from `packages/web-core/src/app/styles/new/index.css`).
const AUTH_PAGE_STYLES: &str = r#"<style>
  @import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&display=swap');
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: 'IBM Plex Sans', -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #f2f2f2;
    color: #333;
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .container {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 24px;
    padding: 24px;
  }
  .logo { width: 40px; height: 40px; }
  .content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .title {
    font-size: 13px;
    font-weight: 500;
    color: #0d0d0d;
  }
  .subtitle {
    font-size: 12px;
    color: #636363;
  }
</style>"#;

/// Response from GET /api/auth/token - returns the current access token
#[derive(Debug, Serialize, TS)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response from GET /api/auth/user - returns the current user ID
#[derive(Debug, Serialize, TS)]
pub struct CurrentUserResponse {
    pub user_id: String,
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/auth/methods", get(auth_methods))
        .route("/auth/handoff/init", post(handoff_init))
        .route("/auth/handoff/complete", get(handoff_complete))
        .route("/auth/local/login", post(local_login))
        .route("/auth/logout", post(logout))
        .route("/auth/status", get(status))
        .route("/auth/token", get(get_token))
        .route("/auth/user", get(get_current_user))
}

async fn auth_methods(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<AuthMethodsResponse>>, ApiError> {
    let client = deployment.remote_client()?;
    let methods = client.auth_methods().await?;
    Ok(ResponseJson(ApiResponse::success(methods)))
}

#[derive(Debug, Deserialize)]
struct HandoffInitPayload {
    provider: String,
    return_to: String,
}

#[derive(Debug, Serialize)]
struct HandoffInitResponseBody {
    handoff_id: Uuid,
    authorize_url: String,
}

/// 回调会不会落回**同一个浏览器**。只看 init 时发起方给的 `return_to`。
///
/// 网页版的 `return_to` 就是当前源下的回调地址，弹窗与主窗口共用 Cookie jar；
/// Tauri 桌面版则把授权页交给**系统浏览器**打开（`crates/tauri-app/src/main.rs`
/// 的 `on_new_window` 直接 Deny 并转给 `opener`），回调落在另一个 Cookie jar 里，
/// 前端因此会在 `return_to` 上带 `?source=desktop`（见 `OAuthDialog.tsx`）。
///
/// **只认 init 请求体里的 `return_to`**：那是一个同源 + JSON 的 POST，
/// 跨站页面构造不出来。`complete` 的 query 里也有一个同名的 `source`，
/// 但那个是攻击者可控的，**绝不能**用来决定要不要校验 nonce。
fn nonce_binding_for(return_to: &str, nonce: &str) -> NonceBinding {
    let is_desktop = url::Url::parse(return_to).is_ok_and(|url| {
        url.query_pairs()
            .any(|(k, v)| k == "source" && v == "desktop")
    });

    if is_desktop {
        NonceBinding::SystemBrowser
    } else {
        NonceBinding::Required(nonce.to_string())
    }
}

async fn handoff_init(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<HandoffInitPayload>,
) -> Result<
    (
        [(header::HeaderName, String); 1],
        ResponseJson<ApiResponse<HandoffInitResponseBody>>,
    ),
    ApiError,
> {
    // 第一层：团队模式整组不可达。放在最前面，连云端都不碰。
    if let Err(rejection) = cloud_handoff_allowed(deployment.local_auth().mode()) {
        tracing::warn!(reason = rejection.as_str(), "拒绝云端 OAuth handoff");
        return Err(ApiError::NotFound);
    }

    let client = deployment.remote_client()?;

    let app_verifier = generate_secret();
    let app_challenge = hash_sha256_hex(&app_verifier);

    let request = HandoffInitRequest {
        provider: payload.provider.clone(),
        return_to: payload.return_to.clone(),
        app_challenge,
    };

    let response = client.handoff_init(&request).await?;

    // 第二层：把这次 handoff 绑到发起登录的那个浏览器上。
    let nonce = generate_handoff_nonce();
    let binding = nonce_binding_for(&payload.return_to, &nonce);

    deployment
        .store_oauth_handoff(
            response.handoff_id,
            payload.provider,
            app_verifier,
            &binding,
        )
        .await;

    // 桌面版拿不到 Cookie，就别下发——免得在系统浏览器里留一条永远用不上的
    // Cookie，也免得让人误以为那条流程受 nonce 保护。
    let cookie = match binding {
        NonceBinding::Required(_) => build_handoff_nonce_cookie(
            &nonce,
            (HANDOFF_TTL_MINUTES * 60) as u64,
            deployment.local_auth().secure_cookies(),
        ),
        NonceBinding::SystemBrowser => {
            build_handoff_nonce_clear_cookie(deployment.local_auth().secure_cookies())
        }
    };

    Ok((
        [(header::SET_COOKIE, cookie)],
        ResponseJson(ApiResponse::success(HandoffInitResponseBody {
            handoff_id: response.handoff_id,
            authorize_url: response.authorize_url,
        })),
    ))
}

#[derive(Debug, Deserialize)]
struct HandoffCompleteQuery {
    handoff_id: Uuid,
    #[serde(default)]
    app_code: Option<String>,
    #[serde(default)]
    error: Option<String>,
    /// When set to "desktop", the callback page will not auto-close so the user
    /// can see the success message (e.g. when opened from the Tauri desktop app).
    #[serde(default)]
    source: Option<String>,
}

/// 被拒时的响应。
///
/// 无论哪种失败都**顺手清掉 nonce Cookie**：它是一次性的，留着只会成为
/// 下一次尝试的素材。响应体只用 [`HandoffRejection::user_message`] 的固定文案，
/// 绝不回显 `handoff_id`、`app_code`、nonce 或提供方返回的任何字符串。
fn handoff_rejected_response(
    rejection: HandoffRejection,
    secure_cookies: bool,
) -> Response<String> {
    let status = match rejection {
        // 团队模式下这条路由在功能上**不存在**。用 404 而不是 403：
        // 403 等于告诉攻击者「这里有个云端登录入口，去弄更高权限」，
        // 404 什么也没说。对合法管理员来说两者都要查文档，没有体验差别。
        HandoffRejection::DisabledInTeamMode => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    let mut response = simple_html_response(status, rejection.user_message().to_string());
    if let Ok(value) = build_handoff_nonce_clear_cookie(secure_cookies).parse() {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn handoff_complete(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<HandoffCompleteQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response<String>, ApiError> {
    let secure_cookies = deployment.local_auth().secure_cookies();

    // 第一层：团队模式整组不可达。必须排在**所有**其它分支之前。
    if let Err(rejection) = cloud_handoff_allowed(deployment.local_auth().mode()) {
        tracing::warn!(reason = rejection.as_str(), "拒绝云端 OAuth handoff 回调");
        return Ok(handoff_rejected_response(rejection, secure_cookies));
    }

    if query.error.is_some() {
        // **绝不回显 `error` 的内容**：它整个来自 URL，是攻击者可控的。
        // 原先直接 `format!` 进 HTML，等于在本应用自己的源上开了一个反射型
        // XSS——拿到 XSS 就能读走非 HttpOnly 的 `vk_csrf`，团队模式下
        // 直接等于接管账号。日志里也不记它，免得把脚本塞进日志查看器。
        tracing::warn!("OAuth 提供方回调返回了错误");
        return Ok(simple_html_response(
            StatusCode::BAD_REQUEST,
            "OAuth authorization failed. Please try again from the app.".to_string(),
        ));
    }

    let Some(app_code) = query.app_code.clone() else {
        return Ok(simple_html_response(
            StatusCode::BAD_REQUEST,
            "Missing authorization code in callback.".to_string(),
        ));
    };

    // 第二层：比对 init 时下发的一次性 nonce。
    //
    // `handoff_id` 与 `app_code` 都躺在 URL 里——浏览器历史、代理日志、
    // 录屏都会留下它们。nonce 只在 Cookie 里，跨站页面读不到、也带不来。
    let presented_nonce = parse_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(str::trim),
        HANDOFF_NONCE_COOKIE,
    );

    let pending = match deployment
        .take_oauth_handoff(&query.handoff_id, presented_nonce.as_deref())
        .await
    {
        Ok(pending) => pending,
        Err(rejection) => {
            // 只记固定 slug。`handoff_id` 原本是 `%query.handoff_id` 进日志的，
            // 但它是换凭据用的凭据之一，不该落盘。
            tracing::warn!(reason = rejection.as_str(), "拒绝 OAuth handoff 回调");
            return Ok(handoff_rejected_response(rejection, secure_cookies));
        }
    };

    let client = deployment.remote_client()?;

    let redeem_request = HandoffRedeemRequest {
        handoff_id: query.handoff_id,
        app_code,
        app_verifier: pending.app_verifier,
    };

    let redeem = client.handoff_redeem(&redeem_request).await?;

    finalize_login(
        &deployment,
        Credentials {
            access_token: Some(redeem.access_token.clone()),
            refresh_token: redeem.refresh_token.clone(),
            expires_at: None,
        },
    )
    .await?;

    let is_desktop = query.source.as_deref() == Some("desktop");
    let mut response = close_window_response(
        format!(
            "Signed in with {}. You can return to the app.",
            pending.provider
        ),
        is_desktop,
    );
    // nonce 用完即焚。
    if let Ok(value) = build_handoff_nonce_clear_cookie(secure_cookies).parse() {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    Ok(response)
}

async fn local_login(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<LocalLoginRequest>,
) -> Result<ResponseJson<ApiResponse<ProfileResponse>>, ApiError> {
    let client = deployment.remote_client()?;
    let response = client.local_login(&payload).await?;
    let profile = finalize_login(
        &deployment,
        Credentials {
            access_token: Some(response.access_token),
            refresh_token: response.refresh_token,
            expires_at: None,
        },
    )
    .await?;

    Ok(ResponseJson(ApiResponse::success(profile)))
}

async fn logout(State(deployment): State<DeploymentImpl>) -> Result<StatusCode, ApiError> {
    let auth_context = deployment.auth_context();

    if let Ok(client) = deployment.remote_client() {
        let _ = client.logout().await;
    }

    auth_context.clear_credentials().await.map_err(|e| {
        tracing::error!(?e, "failed to clear credentials");
        ApiError::Io(e)
    })?;

    auth_context.clear_profile().await;

    relay_registration::stop_relay(&deployment).await;

    Ok(StatusCode::NO_CONTENT)
}

async fn status(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<StatusResponse>>, ApiError> {
    use api_types::LoginStatus;

    let login_status = deployment.get_login_status().await;
    let degraded = deployment
        .auth_context()
        .remote_auth_degraded_slug()
        .await
        .map(|_| true);

    match login_status {
        LoginStatus::LoggedOut => Ok(ResponseJson(ApiResponse::success(StatusResponse {
            logged_in: false,
            profile: None,
            degraded,
        }))),
        LoginStatus::LoggedIn { profile } => {
            Ok(ResponseJson(ApiResponse::success(StatusResponse {
                logged_in: true,
                profile,
                degraded,
            })))
        }
    }
}

/// Returns the current access token (auto-refreshes if needed)
async fn get_token(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<TokenResponse>>, ApiError> {
    let remote_client = deployment.remote_client()?;

    // This will auto-refresh the token if expired
    let access_token = remote_client.access_token().await.map_err(ApiError::from)?;

    let creds = deployment.auth_context().get_credentials().await;
    let expires_at = creds.and_then(|c| c.expires_at);

    Ok(ResponseJson(ApiResponse::success(TokenResponse {
        access_token,
        expires_at,
    })))
}

async fn get_current_user(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<CurrentUserResponse>>, ApiError> {
    let remote_client = deployment.remote_client()?;

    // Get the access token from remote client
    let access_token = remote_client.access_token().await.map_err(ApiError::from)?;

    // Extract user ID from the JWT token's 'sub' claim
    let user_id = utils::jwt::extract_subject(&access_token)
        .map_err(|e| {
            tracing::error!("Failed to extract user ID from token: {}", e);
            ApiError::Unauthorized
        })?
        .to_string();

    Ok(ResponseJson(ApiResponse::success(CurrentUserResponse {
        user_id,
    })))
}

fn generate_secret() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

async fn finalize_login(
    deployment: &DeploymentImpl,
    mut credentials: Credentials,
) -> Result<ProfileResponse, ApiError> {
    let access_token = credentials
        .access_token
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Missing access token".to_string()))?;
    let expires_at = extract_expiration(access_token)
        .map_err(|err| ApiError::BadRequest(format!("Invalid access token: {err}")))?;
    credentials.expires_at = Some(expires_at);

    deployment
        .auth_context()
        .save_credentials(&credentials)
        .await
        .map_err(|e| {
            tracing::error!(?e, "failed to save credentials");
            ApiError::Io(e)
        })?;

    let config_guard = deployment.config().read().await;
    if !config_guard.analytics_enabled {
        let mut new_config = config_guard.clone();
        drop(config_guard);

        new_config.analytics_enabled = true;

        let config_path = config_path();
        if let Err(e) = save_config_to_file(&new_config, &config_path).await {
            tracing::warn!(
                ?e,
                "failed to save config after enabling analytics on login"
            );
        } else {
            let mut config = deployment.config().write().await;
            *config = new_config;
            drop(config);

            tracing::info!("analytics automatically enabled after successful login");

            if let Some(analytics) = deployment.analytics() {
                analytics.track_event(
                    deployment.user_id(),
                    "analytics_session_start",
                    Some(serde_json::json!({})),
                );
            }
        }
    } else {
        drop(config_guard);
    }

    let profile = match deployment.get_login_status().await {
        api_types::LoginStatus::LoggedIn {
            profile: Some(profile),
        } => profile,
        api_types::LoginStatus::LoggedIn { profile: None } | api_types::LoginStatus::LoggedOut => {
            return Err(ApiError::Unauthorized);
        }
    };

    if let Ok(client) = deployment.remote_client() {
        let pool = deployment.db().pool.clone();
        let git = deployment.git().clone();
        tokio::spawn(async move {
            remote_sync::sync_all_linked_workspaces(&client, &pool, &git).await;
        });
    }

    deployment.trigger_pr_sync();

    if let Some(analytics) = deployment.analytics() {
        analytics.track_event(
            deployment.user_id(),
            "$identify",
            Some(serde_json::json!({
                "email": profile.email,
            })),
        );
        analytics.track_event(
            &profile.user_id.to_string(),
            "$merge_dangerously",
            Some(serde_json::json!({
                "alias": deployment.user_id(),
            })),
        );
    }

    let relay_deployment = deployment.clone();
    tokio::spawn(async move {
        relay_registration::spawn_relay(&relay_deployment).await;
    });

    Ok(profile)
}

fn hash_sha256_hex(input: &str) -> String {
    let mut output = String::with_capacity(64);
    let digest = Sha256::digest(input.as_bytes());
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{:02x}", byte);
    }
    output
}

/// 转义要拼进 HTML 文本节点的内容。
///
/// 这两个页面是用 `format!` 拼出来的裸 HTML，没有模板引擎替我们把关。
/// 调用方现在传的都是固定文案，但「现在是固定的」不是一道防线——
/// 一旦哪天有人把 query 里的东西传进来，这里就是唯一拦得住的地方。
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

fn simple_html_response(status: StatusCode, message: String) -> Response<String> {
    let message = html_escape(&message);
    let body = format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>OAuth Error</title>
    {AUTH_PAGE_STYLES}
  </head>
  <body>
    <div class="container">
      <img class="logo" src="data:image/png;base64,{APP_ICON_BASE64}" alt="Vibe Kanban">
      <div class="content">
        <p class="title">{message}</p>
        <p class="subtitle">Please close this tab and try again.</p>
      </div>
    </div>
  </body>
</html>"#
    );
    Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .unwrap()
}

fn close_window_response(message: String, skip_auto_close: bool) -> Response<String> {
    let message = html_escape(&message);
    let script = if skip_auto_close {
        "" // Desktop app: leave the tab open so the user sees the message
    } else {
        "<script>\
           window.addEventListener('load', () => {\
             try { window.close(); } catch (err) {}\
             setTimeout(() => { window.close(); }, 150);\
           });\
         </script>"
    };
    let body = format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Authentication Complete</title>
    {script}
    {AUTH_PAGE_STYLES}
  </head>
  <body>
    <div class="container">
      <img class="logo" src="data:image/png;base64,{APP_ICON_BASE64}" alt="Vibe Kanban">
      <div class="content">
        <p class="title">{message}</p>
        <p class="subtitle">You can close this tab and return to the app.</p>
      </div>
    </div>
  </body>
</html>"#
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use services::services::server_settings::ServerMode;

    use super::*;

    const 网页版: &str = "http://kanban.lan:8080/api/auth/handoff/complete";
    const 桌面版: &str = "http://kanban.lan:8080/api/auth/handoff/complete?source=desktop";

    // --------------------------------------------- nonce 绑定只由 init 决定

    /// 网页版：弹窗与主窗口共用 Cookie jar，必须要求 nonce。
    #[test]
    fn 网页版回调要求_nonce() {
        assert_eq!(
            nonce_binding_for(网页版, "n0nce"),
            NonceBinding::Required("n0nce".into())
        );
    }

    /// 桌面版：回调落在系统浏览器里，拿不到 Cookie，强要 nonce 会打死登录。
    #[test]
    fn 桌面版回调豁免_nonce() {
        assert_eq!(
            nonce_binding_for(桌面版, "n0nce"),
            NonceBinding::SystemBrowser
        );
    }

    /// **攻击样例**：豁免判据只能来自 init 的 `return_to`。
    /// 攻击者在 `complete` 的 URL 上加 `?source=desktop` 是没用的——
    /// 那个参数只影响页面要不要自动关闭，碰不到 nonce 校验。
    /// 这条钉住 `nonce_binding_for` 只读 `return_to` 这一个入参。
    #[test]
    fn 豁免不能被回调_query_打开() {
        // 同一个 return_to（网页版）无论 nonce 是什么，都必须要求 nonce。
        for nonce in ["", "n0nce", "source=desktop"] {
            assert!(
                matches!(nonce_binding_for(网页版, nonce), NonceBinding::Required(_)),
                "{nonce:?}"
            );
        }
    }

    /// `source` 必须**恰好**是 desktop，别的值一律按网页版处理（fail-closed）。
    #[test]
    fn 只有恰好等于_desktop_才豁免() {
        for return_to in [
            "http://kanban.lan:8080/api/auth/handoff/complete?source=Desktop",
            "http://kanban.lan:8080/api/auth/handoff/complete?source=desktopx",
            "http://kanban.lan:8080/api/auth/handoff/complete?source=",
            "http://kanban.lan:8080/api/auth/handoff/complete?sourcex=desktop",
            "http://kanban.lan:8080/api/auth/handoff/complete#source=desktop",
            "http://kanban.lan:8080/api/auth/handoff/complete?x=source%3Ddesktop",
        ] {
            assert!(
                matches!(nonce_binding_for(return_to, "n"), NonceBinding::Required(_)),
                "{return_to} 不该被当成桌面版"
            );
        }
    }

    /// 解析不动的 `return_to` 按网页版处理，不能 fail-open 成豁免。
    #[test]
    fn 畸形_return_to_不豁免() {
        for return_to in [
            "",
            "not a url",
            "/api/auth/handoff/complete?source=desktop",
            "???",
        ] {
            assert!(
                matches!(nonce_binding_for(return_to, "n"), NonceBinding::Required(_)),
                "{return_to:?} 不该被当成桌面版"
            );
        }
    }

    // --------------------------------------------- 个人模式：端到端往返

    /// 模拟浏览器：从 `Set-Cookie` 里取出 `name=value`，拼成下次请求的 `Cookie` 头。
    ///
    /// 只取第一段，正是浏览器回送时的行为（属性不回送）。
    fn 浏览器回送(set_cookie: &str) -> String {
        format!(
            "vk_session=sess; {}; vk_csrf=csrf",
            set_cookie.split(';').next().unwrap()
        )
    }

    /// 从请求头里按 complete handler 的方式取 nonce。
    fn 取回_nonce(cookie_header: &str) -> Option<String> {
        parse_cookie(Some(cookie_header), HANDOFF_NONCE_COOKIE)
    }

    /// **验收硬要求**：个人模式下 init → complete 必须照常走通。
    ///
    /// 这条串起了真实的管道：生成 nonce → 决定绑定 → 下发 Cookie →
    /// 浏览器回送 → 解析 Cookie → 消费 handoff。除了云端那两次 HTTP
    /// （`handoff_init` / `handoff_redeem`），handler 里的每一步都在这里。
    #[tokio::test]
    async fn 个人模式网页版端到端往返成功() {
        use services::services::oauth_handoff::{HandoffStore, PendingHandoff};

        assert_eq!(cloud_handoff_allowed(ServerMode::Personal), Ok(()));

        // --- init 侧
        let store = HandoffStore::new();
        let handoff_id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        let binding = nonce_binding_for(网页版, &nonce);
        store
            .insert_now(handoff_id, "google".into(), "verifier-abc".into(), &binding)
            .await;
        let set_cookie =
            build_handoff_nonce_cookie(&nonce, (HANDOFF_TTL_MINUTES * 60) as u64, false);

        // --- 浏览器把 Cookie 带回回调
        let cookie_header = 浏览器回送(&set_cookie);

        // --- complete 侧
        let presented = 取回_nonce(&cookie_header);
        assert_eq!(presented.as_deref(), Some(nonce.as_str()));
        assert_eq!(
            store.take_now(&handoff_id, presented.as_deref()).await,
            Ok(PendingHandoff {
                provider: "google".into(),
                app_verifier: "verifier-abc".into(),
            }),
            "个人模式正常登录被挡住了"
        );
    }

    /// 桌面版同样要走通，且**不需要**任何 Cookie。
    #[tokio::test]
    async fn 个人模式桌面版端到端往返成功() {
        use services::services::oauth_handoff::HandoffStore;

        let store = HandoffStore::new();
        let handoff_id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        let binding = nonce_binding_for(桌面版, &nonce);
        assert_eq!(binding, NonceBinding::SystemBrowser);
        store
            .insert_now(handoff_id, "github".into(), "v".into(), &binding)
            .await;

        // 系统浏览器完全没有本站 Cookie。
        assert!(store.take_now(&handoff_id, None).await.is_ok());
    }

    /// **攻击样例（端到端）**：跨站伪造的回调。受害者浏览器会带上
    /// `SameSite=Lax` 的会话 Cookie，但**没有** `vk_handoff`——
    /// 它是 init 时才下发的，而攻击者没法让受害者的服务器发起 init。
    #[tokio::test]
    async fn 跨站伪造回调被拒() {
        use services::services::oauth_handoff::{HandoffRejection, HandoffStore};

        let store = HandoffStore::new();
        let handoff_id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        store
            .insert_now(
                handoff_id,
                "google".into(),
                "verifier-abc".into(),
                &nonce_binding_for(网页版, &nonce),
            )
            .await;

        // 攻击者的链接只能让浏览器带上会话/CSRF Cookie，带不来 nonce。
        let 伪造 = "vk_session=sess; vk_csrf=csrf";
        assert_eq!(取回_nonce(伪造), None);
        assert_eq!(
            store
                .take_now(&handoff_id, 取回_nonce(伪造).as_deref())
                .await,
            Err(HandoffRejection::MissingNonce)
        );
    }

    // --------------------------------------------- 拒绝响应：状态码与不泄漏

    /// 团队模式用 404：不确认这里存在一个云端登录入口。
    #[test]
    fn 团队模式回调返回_404() {
        let resp = handoff_rejected_response(HandoffRejection::DisabledInTeamMode, false);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// 其余四种失败一律 400，且**状态码也不区分**——
    /// 区分等于给攻击者一个「猜对了哪一半」的预言机。
    #[test]
    fn 四种校验失败状态码一致() {
        for r in [
            HandoffRejection::Unknown,
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            assert_eq!(
                handoff_rejected_response(r, false).status(),
                StatusCode::BAD_REQUEST,
                "{r:?}"
            );
        }
    }

    /// 四种失败的响应体必须逐字节相同，否则长度差本身就是预言机。
    #[test]
    fn 四种校验失败响应体逐字节一致() {
        let 基准 = handoff_rejected_response(HandoffRejection::Unknown, false).into_body();
        for r in [
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            assert_eq!(
                handoff_rejected_response(r, false).into_body(),
                基准,
                "{r:?} 的响应体与 Unknown 不同，成了预言机"
            );
        }
    }

    /// 拒绝时必须清掉 nonce Cookie：它是一次性的，留着只会成为下次尝试的素材。
    #[test]
    fn 拒绝时清掉_nonce_cookie() {
        for secure in [false, true] {
            let resp = handoff_rejected_response(HandoffRejection::NonceMismatch, secure);
            let cookie = resp
                .headers()
                .get(header::SET_COOKIE)
                .expect("必须下发清除 Cookie")
                .to_str()
                .unwrap();
            assert!(cookie.starts_with("vk_handoff=;"));
            assert!(cookie.contains("Max-Age=0"));
            assert_eq!(cookie.contains("; Secure"), secure);
        }
    }

    /// **拒绝响应体不能泄漏任何凭据素材。**
    #[test]
    fn 拒绝响应体不含凭据素材() {
        for r in [
            HandoffRejection::DisabledInTeamMode,
            HandoffRejection::Unknown,
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            let body = handoff_rejected_response(r, false)
                .into_body()
                .to_lowercase();
            for 敏感 in [
                "nonce",
                "handoff_id",
                "app_code",
                "app_verifier",
                "access_token",
                "refresh_token",
                "vk_handoff",
                "vk_session",
            ] {
                assert!(!body.contains(敏感), "{r:?} 的响应体泄漏了 {敏感}");
            }
        }
    }

    // --------------------------------------------- 反射型 XSS

    /// **攻击样例**：`?error=<script>…` 原先被直接 `format!` 进 HTML。
    /// 拿到本应用源上的 XSS 就能读走非 HttpOnly 的 `vk_csrf`，
    /// 团队模式下等于接管账号。
    #[test]
    fn html_转义挡住脚本注入() {
        let 恶意 = r#"<script>alert(document.cookie)</script>"#;
        let body = simple_html_response(StatusCode::BAD_REQUEST, 恶意.to_string()).into_body();
        assert!(!body.contains("<script>alert"));
        assert!(body.contains("&lt;script&gt;"));
    }

    /// 属性上下文的逃逸（`"` 与 `'`）也要挡住。
    #[test]
    fn html_转义覆盖引号与尖括号() {
        assert_eq!(html_escape(r#"<>&"'"#), "&lt;&gt;&amp;&quot;&#x27;");
        // 先转义 & 再转义其它，不能出现二次转义。
        assert_eq!(html_escape("a&lt;b"), "a&amp;lt;b");
    }

    /// 成功页也要转义：provider 来自 init 的请求体，不是编译期常量。
    #[test]
    fn 成功页同样转义() {
        let body = close_window_response(
            "Signed in with <img src=x onerror=alert(1)>.".to_string(),
            false,
        )
        .into_body();
        assert!(!body.contains("<img src=x"));
        assert!(body.contains("&lt;img"));
    }

    /// 正常文案不该被转义弄花。
    #[test]
    fn 正常文案不受影响() {
        assert_eq!(
            html_escape("Signed in with google."),
            "Signed in with google."
        );
        assert_eq!(html_escape(""), "");
    }
}
