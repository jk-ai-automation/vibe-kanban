//! 第三方登录（飞书 / Lark / Google）：授权跳转、回调、账号绑定。
//!
//! 路径契约（前端按这个拼，不要自己猜）：
//!
//! | 方法 | 路径 | 鉴权 | 作用 |
//! |---|---|---|---|
//! | GET | `/api/local-auth/oauth/{provider}/start` | 免鉴权 | 302 跳到提供方授权页 |
//! | GET | `/api/local-auth/oauth/{provider}/callback` | 免鉴权 | 提供方跳回来，成功后 302 到 `/` |
//! | POST | `/api/local-auth/oauth/{provider}/bind` | 需会话 + CSRF | 返回授权链接，供已登录用户绑定 |
//!
//! `{provider}` 只接受 `feishu` / `lark` / `google`，其余一律 404，
//! **原字符串绝不进入任何 URL、日志或 HTML**。
//!
//! 安全约定（每条都有对应测试）：
//! - `state` 服务端生成、一次性、10 分钟过期、与 provider 绑定，并靠一条
//!   HttpOnly 的 nonce Cookie 绑到发起方浏览器上。
//! - **不按 email 自动合并账号**：未绑定的身份撞上已有邮箱时一律拒绝，
//!   必须由该用户登录后主动绑定。这是防账号劫持的关键。
//! - `allow_oauth_signup` 默认关；关着时未绑定的身份不能自助建号。
//! - access token 只在内存里活过一次请求：不落库、不进日志、不进 URL。
//! - 回调的 `?error=` 等参数**绝不回显进 HTML**（云端 handoff 那条路径
//!   就因为回显开过一个反射型 XSS）。
//!
//! 本文件**不写任何 `sqlx::query!` 宏**，理由同 `password_routes.rs`。

use std::{
    net::SocketAddr,
    sync::OnceLock,
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, Query, State},
    http::{Extensions, HeaderMap, HeaderValue, StatusCode, header},
    response::{Json as ResponseJson, Response},
};
use chrono::Utc;
use db::models::{
    local_auth::{IdentityLinkError, LocalSessions, LocalUserIdentities},
    local_user::{LocalUser, LocalUserRole, LocalUserStatus, LocalUsers, NewLocalUser},
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use services::services::{
    local_auth::{
        oauth::{
            OAuthError, OAuthIdentity, PendingOAuthState, ResolvedProvider, STATE_TTL,
            TokenBodyFormat, build_authorize_url, extract_identity, parse_access_token,
            provider_config, redirect_uri, resolve_provider, token_request_params,
        },
        rate_limit::client_ip,
        runtime::LocalAuthRuntime,
        token::{
            OAUTH_NONCE_COOKIE, build_oauth_nonce_clear_cookie, build_oauth_nonce_cookie,
            generate_oauth_nonce, generate_oauth_state, generate_pkce_verifier, hash_session_token,
            parse_non_empty_cookie,
        },
    },
    server_settings::ServerMode,
};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::password_routes::{LoginOutcome, start_session};
use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::local_session::{CurrentUser, session_token_from_header},
};

/// 回调在限速器里用的「用户名」。
///
/// 与 D 批的邀请注册同一套做法：**不能用请求里的任何标识**，
/// 否则攻击者能拿限速当武器烧掉受害者的桶。真实用户名只允许
/// `[a-z0-9._-]`，这个前缀字符不可能与之相撞。
///
/// **只对失败计数。** 回调在 `state` 校验通过之后会向提供方发一次
/// 外网请求，这是一条 1:1 的放大路径；不限速就等于免费的外发请求代理。
/// `/start` 不限速：它只往一张有上限、会过期的表里塞一条，
/// 而给它限速反而让同一个 NAT 后面的同事互相把登录挤掉。
const OAUTH_CALLBACK_RATE_KEY: &str = "\u{2}oauth-callback";

/// 与提供方通信的超时。挂死的 IdP 不设超时就会把连接池占满。
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);

/// 提供方响应体的读取上限。
///
/// 对方（或中间人）返回一个无限流时，`bytes()` 会一直吃内存直到 OOM。
const MAX_PROVIDER_BODY: usize = 256 * 1024;

/// 登录成功后回到的地址。**同源常量**，不接受任何来自 URL 的跳转目标——
/// 可控的 `redirect` 参数就是一个开放重定向。
const LOGIN_REDIRECT: &str = "/";
/// 绑定成功后回到的地址。
const BIND_REDIRECT: &str = "/?oauth=bound";

// 面向用户的固定文案。**一个都不许拼进外部内容。**
const MSG_EXPIRED: &str = "登录已过期，请重试";
const MSG_PROVIDER_FAILED: &str = "第三方登录服务返回了异常响应，请稍后再试";
const MSG_AUTHORIZE_FAILED: &str = "第三方授权失败，请重试";
const MSG_EMAIL_TAKEN: &str = "该邮箱已被占用，请登录后在设置里绑定";
const MSG_SIGNUP_DISABLED: &str = "该账号尚未开通，请联系管理员";
const MSG_DISABLED_USER: &str = "账号已停用，请联系管理员";
const MSG_ALREADY_LINKED: &str = "该第三方账号已绑定到其他用户";
const MSG_UNKNOWN_PROVIDER: &str = "登录方式不可用";

/// 派生用户名时最多试多少个后缀。
const MAX_USERNAME_ATTEMPTS: u32 = 20;

// --------------------------------------------------------------- HTTP 客户端

/// 与提供方通信的共享客户端。
///
/// 进程内复用一个：每次请求新建一个 `Client` 会各自带一份连接池与 TLS 会话，
/// 在回调被刷的时候会把文件描述符吃光。
fn provider_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(PROVIDER_TIMEOUT)
            .connect_timeout(PROVIDER_TIMEOUT)
            // 授权码只能换一次；跟着重定向去一个陌生主机没有任何好处。
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

// ------------------------------------------------------------------- 请求/响应

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OAuthCallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    /// 提供方回报的错误。**只用于判断有没有，永不回显、永不进日志。**
    #[serde(default)]
    pub error: Option<String>,
}

/// `POST /bind` 的响应：前端拿到之后自己 `window.location.assign`。
///
/// 不直接回 302：`fetch` 会跟着重定向去提供方的域名，拿回来的是一个
/// 不能用的跨域响应；而且带 CSRF 头的请求本来就该由前端自己发起跳转。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct OAuthBindStart {
    pub authorize_url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StartOutcome {
    pub authorize_url: String,
    pub nonce_cookie: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CallbackOutcome {
    /// 登录流程会建会话；绑定流程不会（用户本来就已登录）。
    pub login: Option<LoginOutcome>,
    pub redirect: &'static str,
}

/// 回调的全部入参。打成一包是为了让 [`handle_callback`] 的参数个数
/// 不至于触发 `clippy::too_many_arguments`。
#[derive(Debug, Clone, Default)]
pub(crate) struct CallbackInput {
    pub provider: String,
    pub query: OAuthCallbackQuery,
    pub cookie_header: Option<String>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

// --------------------------------------------------------------------- 工具

/// `OAuthError` → `ApiError`。
///
/// 状态码的语义（前端按这个分流）：
/// - 404：提供方不存在或未配置（**不用 403**，403 等于告诉人家「这里有东西」）
/// - 400：`state` 伪造 / 重放 / 过期 / 跨 provider / 跨浏览器
/// - 502：提供方那一侧出了问题（换令牌、拉用户信息失败）
/// - 500：本机没配 `public_base_url`，是运维问题不是用户问题
fn map_oauth_error(err: OAuthError) -> ApiError {
    match err {
        OAuthError::UnknownProvider => ApiError::NotFound,
        OAuthError::MissingPublicBaseUrl => ApiError::BadRequest(
            "服务端未配置访问地址（public_base_url），第三方登录不可用".to_string(),
        ),
        OAuthError::InvalidState => ApiError::BadRequest(MSG_EXPIRED.to_string()),
        OAuthError::ProviderResponse | OAuthError::MissingSubject => {
            ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
        }
    }
}

/// 取出「已启用且已配齐凭据」的提供方。
///
/// 团队模式之外整组路由在功能上不存在（404），与云端 handoff 在团队模式下
/// 返回 404 是对称的：不给任何「这里还有另一个登录入口」的信号。
// ApiError 体积较大，但全仓库的 handler 都用它；与 password_routes.rs 的处理一致。
#[allow(clippy::result_large_err)]
fn require_provider(
    runtime: &LocalAuthRuntime,
    provider_id: &str,
) -> Result<ResolvedProvider, ApiError> {
    if runtime.mode() != ServerMode::Team {
        return Err(ApiError::NotFound);
    }
    // 先过白名单再谈别的：`provider_id` 来自 URL 路径。
    if provider_config(provider_id).is_none() {
        return Err(ApiError::NotFound);
    }
    resolve_provider(runtime.settings(), provider_id).ok_or(ApiError::NotFound)
}

fn derive_username(provider: &str, subject: &str) -> String {
    // 用 `subject` 的哈希而不是 `subject` 本身：`union_id` / `sub` 是对方那边的
    // 稳定标识，把它当成人人可见的用户名等于白送一份目录。哈希同样是「由
    // subject 派生」，且结果天然落在 `[a-z0-9-]` 里，不必再做字符集过滤。
    let digest = hash_session_token(&format!("{provider}:{subject}"));
    format!("{provider}-{}", &digest[..12])
}

fn candidate_username(provider: &str, subject: &str, attempt: u32) -> String {
    let base = derive_username(provider, subject);
    if attempt == 0 {
        base
    } else {
        format!("{base}-{attempt}")
    }
}

// --------------------------------------------------------------------- start

/// 生成 `state` 与 nonce，存进运行时，返回授权链接与要下发的 Cookie。
///
/// `bind_for_user_id` 非 `None` 表示这是「已登录用户绑定」，回调时会据此
/// 走绑定分支，并且**要求回调请求仍带着同一个用户的有效会话**。
#[allow(clippy::result_large_err)]
pub(crate) fn handle_start(
    runtime: &LocalAuthRuntime,
    provider_id: &str,
    bind_for_user_id: Option<Uuid>,
) -> Result<StartOutcome, ApiError> {
    let provider = require_provider(runtime, provider_id)?;
    // `redirect_uri` 只能从配置里的 `public_base_url` 拼出来。
    // 拿请求的 Host 头兜底等于允许攻击者把授权码定向到他的域名。
    let redirect =
        redirect_uri(runtime.public_base_url(), &provider.id).map_err(map_oauth_error)?;

    let state = generate_oauth_state();
    let nonce = generate_oauth_nonce();
    let verifier = provider.pkce.then(generate_pkce_verifier);

    let authorize_url = build_authorize_url(&provider, &state, &redirect, verifier.as_deref())
        .map_err(map_oauth_error)?;

    let now = Instant::now();
    runtime.oauth_states().insert(
        &state,
        PendingOAuthState::new(&provider.id, &nonce, verifier, bind_for_user_id, now),
        now,
    );

    Ok(StartOutcome {
        authorize_url,
        nonce_cookie: build_oauth_nonce_cookie(
            &nonce,
            STATE_TTL.as_secs(),
            runtime.secure_cookies(),
        ),
    })
}

// ------------------------------------------------------------------ callback

pub(crate) async fn handle_callback(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    http: &reqwest::Client,
    input: CallbackInput,
) -> Result<CallbackOutcome, ApiError> {
    let provider = require_provider(runtime, &input.provider)?;
    let ip_key = input.ip.as_deref().unwrap_or("");

    // 限速在做任何外发请求之前。
    if let Err(等待) =
        runtime
            .login_limiter()
            .check(OAUTH_CALLBACK_RATE_KEY, ip_key, Instant::now())
    {
        return Err(ApiError::TooManyRequests(format!(
            "尝试过于频繁，请 {} 秒后再试",
            等待.as_secs().max(1)
        )));
    }

    let result = callback_inner(pool, runtime, http, &provider, &input).await;
    match &result {
        Ok(_) => runtime
            .login_limiter()
            .reset(OAUTH_CALLBACK_RATE_KEY, ip_key),
        Err(_) => {
            runtime
                .login_limiter()
                .record_failure(OAUTH_CALLBACK_RATE_KEY, ip_key, Instant::now())
        }
    }
    result
}

async fn callback_inner(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    http: &reqwest::Client,
    provider: &ResolvedProvider,
    input: &CallbackInput,
) -> Result<CallbackOutcome, ApiError> {
    // **先校验 `state`，再看 `?error=`。** 反过来的话，任何人往回调地址上
    // 甩一个 `?error=x` 就能让服务端走进「授权失败」分支，而那条分支本该
    // 只对「确实发起过这次登录」的浏览器生效；同时也能保证一次真实的
    // 授权失败会把 `state` 消费掉，不留给攻击者再试。
    let state = input.query.state.as_deref().unwrap_or("");
    let cookie_header = input.cookie_header.as_deref();
    let presented_nonce = parse_non_empty_cookie(cookie_header, OAUTH_NONCE_COOKIE);

    // 一次性、未过期、同 provider；四条判据全在 `consume` 里。
    let now = Instant::now();
    let pending = runtime
        .oauth_states()
        .consume(state, &provider.id, now)
        .ok_or_else(|| ApiError::BadRequest(MSG_EXPIRED.to_string()))?;

    // 绑到发起方浏览器：别的浏览器（哪怕拿到了 URL 里的 state）没有这条 Cookie。
    let matched = presented_nonce
        .as_deref()
        .is_some_and(|nonce| pending.nonce_matches(nonce));
    if !matched {
        tracing::warn!(provider = %provider.id, "第三方登录回调的浏览器绑定不匹配");
        return Err(ApiError::BadRequest(MSG_EXPIRED.to_string()));
    }

    // 提供方回报了错误。**不读它的内容、不进日志、更不回显**：
    // 它整个来自 URL，是攻击者可控的。
    if input.query.error.is_some() {
        tracing::warn!(provider = %provider.id, "第三方授权失败");
        return Err(ApiError::BadRequest(MSG_AUTHORIZE_FAILED.to_string()));
    }

    let code = input
        .query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::BadRequest(MSG_AUTHORIZE_FAILED.to_string()))?;

    let redirect =
        redirect_uri(runtime.public_base_url(), &provider.id).map_err(map_oauth_error)?;

    // access token 从这里开始只活到本函数结束：不落库、不进日志、不进响应。
    let access_token = exchange_code(
        http,
        provider,
        code,
        &redirect,
        pending.code_verifier.as_deref(),
    )
    .await?;
    let identity = fetch_identity(http, provider, &access_token).await?;
    drop(access_token);

    match pending.bind_for_user_id {
        Some(user_id) => bind_identity(pool, provider, &identity, user_id, cookie_header).await,
        None => login_with_identity(pool, runtime, provider, &identity, input).await,
    }
}

/// 换令牌。三家的请求体格式不同（飞书 v3 form、Lark v2 JSON、Google form），
/// 由 `token_body_format` 决定；响应一律按扁平结构解析。
async fn exchange_code(
    http: &reqwest::Client,
    provider: &ResolvedProvider,
    code: &str,
    redirect: &str,
    code_verifier: Option<&str>,
) -> Result<String, ApiError> {
    let params = token_request_params(provider, code, redirect, code_verifier);
    let request = http.post(&provider.token_url);
    let request = match provider.token_body_format {
        // 本仓库的 `reqwest` 关掉了默认特性（`crates/server/Cargo.toml`），
        // 没有 `urlencoded`，所以 `RequestBuilder::form` 不存在。
        // 用 `url::form_urlencoded` 自己编码，**不要**为此给全仓库的 reqwest
        // 多开一个特性——那会影响所有 crate 的编译产物。
        TokenBodyFormat::Form => {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .finish();
            request
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
        }
        TokenBodyFormat::Json => {
            let body: serde_json::Map<String, serde_json::Value> = params
                .into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect();
            request.json(&body)
        }
    };

    let response = request.send().await.map_err(|err| {
        // 只记 `is_timeout` 这类布尔判据：err 里可能带上完整 URL（含 query）。
        tracing::warn!(
            provider = %provider.id,
            timeout = err.is_timeout(),
            "换取第三方令牌失败"
        );
        ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
    })?;
    let body = read_json_body(response, provider, "token").await?;
    parse_access_token(&body).map_err(map_oauth_error)
}

async fn fetch_identity(
    http: &reqwest::Client,
    provider: &ResolvedProvider,
    access_token: &str,
) -> Result<OAuthIdentity, ApiError> {
    let response = http
        .get(&provider.userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!(
                provider = %provider.id,
                timeout = err.is_timeout(),
                "拉取第三方用户信息失败"
            );
            ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
        })?;
    let body = read_json_body(response, provider, "userinfo").await?;
    extract_identity(provider, &body).map_err(map_oauth_error)
}

/// 读响应体并解析成 JSON。
///
/// - 非 2xx 一律当失败，但**只记状态码**，不记也不回显响应体。
/// - 有读取上限：对方返回一个无限流时不能把本进程吃到 OOM。
async fn read_json_body(
    response: reqwest::Response,
    provider: &ResolvedProvider,
    stage: &'static str,
) -> Result<serde_json::Value, ApiError> {
    use futures_util::StreamExt;

    let status = response.status();
    if !status.is_success() {
        tracing::warn!(
            provider = %provider.id,
            stage,
            status = status.as_u16(),
            "第三方接口返回非 2xx"
        );
        return Err(ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string()));
    }

    let mut collected: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            tracing::warn!(provider = %provider.id, stage, "读取第三方响应失败");
            ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
        })?;
        if collected.len() + chunk.len() > MAX_PROVIDER_BODY {
            tracing::warn!(provider = %provider.id, stage, "第三方响应体超限");
            return Err(ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string()));
        }
        collected.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&collected).map_err(|_| {
        // 解析失败时**不打印响应体**：它整个来自外部。
        tracing::warn!(provider = %provider.id, stage, "第三方响应不是合法 JSON");
        ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
    })
}

/// 登录分支。
async fn login_with_identity(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    provider: &ResolvedProvider,
    identity: &OAuthIdentity,
    input: &CallbackInput,
) -> Result<CallbackOutcome, ApiError> {
    let existing =
        LocalUserIdentities::find_by_provider_subject(pool, &provider.id, &identity.subject)
            .await
            .map_err(ApiError::from)?;

    let user = match existing {
        Some(link) => LocalUsers::find_by_id(pool, link.user_id)
            .await
            .map_err(ApiError::from)?
            // 身份还在但用户没了：级联删除应当保证不会发生，真发生了也不能建号。
            .ok_or_else(|| ApiError::BadRequest(MSG_EXPIRED.to_string()))?,
        None => signup_with_identity(pool, runtime, provider, identity).await?,
    };

    // **被停用的用户走第三方登录同样进不来。** 这一条必须在建会话之前。
    if user.status != LocalUserStatus::Active {
        tracing::warn!(provider = %provider.id, "已停用的账号尝试第三方登录");
        return Err(ApiError::Forbidden(MSG_DISABLED_USER.to_string()));
    }

    let outcome = start_session(
        pool,
        runtime,
        &user,
        input.user_agent.clone(),
        input.ip.clone(),
    )
    .await?;
    if let Err(err) = LocalUsers::touch_last_login(pool, user.id, Utc::now()).await {
        tracing::warn!(?err, "记录 last_login_at 失败");
    }
    Ok(CallbackOutcome {
        login: Some(outcome),
        redirect: LOGIN_REDIRECT,
    })
}

/// 未绑定身份的自助建号。
///
/// 两道闸的顺序是刻意的：
/// 1. **邮箱撞号一律拒绝**，无论允不允许自助注册。提供方返回的 email
///    未必经过实时验证，凭它自动登录成已有账号就是一条账号劫持路径；
///    正确做法是本人登录后在设置里主动绑定。
/// 2. `allow_oauth_signup` 默认关：关着时任何未绑定的身份都建不了号。
async fn signup_with_identity(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    provider: &ResolvedProvider,
    identity: &OAuthIdentity,
) -> Result<LocalUser, ApiError> {
    if let Some(email) = identity.email.as_deref()
        && LocalUsers::find_by_email(pool, email)
            .await
            .map_err(ApiError::from)?
            .is_some()
    {
        tracing::warn!(provider = %provider.id, "第三方身份的邮箱已被占用，拒绝自动合并");
        return Err(ApiError::Forbidden(MSG_EMAIL_TAKEN.to_string()));
    }

    if !runtime.allow_oauth_signup() {
        return Err(ApiError::Forbidden(MSG_SIGNUP_DISABLED.to_string()));
    }

    let display_name = identity
        .name
        .clone()
        .unwrap_or_else(|| derive_username(&provider.id, &identity.subject));

    let mut last_err = None;
    for attempt in 0..MAX_USERNAME_ATTEMPTS {
        let username = candidate_username(&provider.id, &identity.subject, attempt);
        let result = LocalUserIdentities::create_user_with_identity(
            pool,
            NewLocalUser {
                username,
                display_name: display_name.clone(),
                email: identity.email.clone(),
                // 第三方建号不设密码：设一个随机密码只会多一份可撞的凭据。
                password_hash: None,
                // 角色固定 member，**不从任何外部输入里读**。
                role: LocalUserRole::Member,
            },
            &provider.id,
            &identity.subject,
            identity.email.as_deref(),
        )
        .await;

        match result {
            Ok(user) => return Ok(user),
            // 用户名撞了就换一个后缀再试。
            Err(IdentityLinkError::User(db::models::local_user::LocalUserError::Conflict(msg))) => {
                last_err = Some(ApiError::Conflict(msg));
            }
            Err(IdentityLinkError::AlreadyLinked) => {
                // 并发：另一个请求刚刚把同一个身份绑走了。
                return Err(ApiError::Conflict(MSG_ALREADY_LINKED.to_string()));
            }
            Err(err) => return Err(map_identity_error(err)),
        }
    }
    Err(last_err.unwrap_or_else(|| ApiError::Conflict("无法为该账号分配用户名".to_string())))
}

/// 绑定分支：把第三方身份挂到**当前已登录**的账号上。
async fn bind_identity(
    pool: &SqlitePool,
    provider: &ResolvedProvider,
    identity: &OAuthIdentity,
    bind_for_user_id: Uuid,
    cookie_header: Option<&str>,
) -> Result<CallbackOutcome, ApiError> {
    // `state` 里记了「为谁绑」，但回调这一刻仍要求带着**同一个人**的有效会话：
    // 只信 state 的话，攻击者只要拿到一条绑定链接就能把自己的身份挂到受害者名下。
    let session_user = current_session_user(pool, cookie_header).await?;
    if session_user.id != bind_for_user_id {
        tracing::warn!("绑定回调的会话与发起绑定的用户不一致");
        return Err(ApiError::Unauthorized);
    }
    if session_user.status != LocalUserStatus::Active {
        return Err(ApiError::Forbidden(MSG_DISABLED_USER.to_string()));
    }

    let existing =
        LocalUserIdentities::find_by_provider_subject(pool, &provider.id, &identity.subject)
            .await
            .map_err(ApiError::from)?;
    match existing {
        // 已经绑在自己名下：幂等成功，不重复插行。
        Some(link) if link.user_id == session_user.id => {}
        // 已经属于别人：**绝不改绑**。
        Some(_) => {
            tracing::warn!(provider = %provider.id, "尝试绑定已属于他人的第三方身份");
            return Err(ApiError::Conflict(MSG_ALREADY_LINKED.to_string()));
        }
        None => {
            LocalUserIdentities::link(
                pool,
                session_user.id,
                &provider.id,
                &identity.subject,
                identity.email.as_deref(),
            )
            .await
            .map_err(map_identity_error)?;
        }
    }
    Ok(CallbackOutcome {
        login: None,
        redirect: BIND_REDIRECT,
    })
}

async fn current_session_user(
    pool: &SqlitePool,
    cookie_header: Option<&str>,
) -> Result<LocalUser, ApiError> {
    let token = session_token_from_header(cookie_header).ok_or(ApiError::Unauthorized)?;
    let session =
        LocalSessions::find_valid_by_token_hash(pool, &hash_session_token(&token), Utc::now())
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::Unauthorized)?;
    LocalUsers::find_by_id(pool, session.user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::Unauthorized)
}

fn map_identity_error(err: IdentityLinkError) -> ApiError {
    match err {
        IdentityLinkError::AlreadyLinked => ApiError::Conflict(MSG_ALREADY_LINKED.to_string()),
        IdentityLinkError::Validation(_) => {
            // 校验失败的内容来自提供方，不回显。
            ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string())
        }
        IdentityLinkError::User(err) => super::password_routes::map_local_user_error(err),
        IdentityLinkError::Database(err) => ApiError::from(err),
    }
}

// ------------------------------------------------------------- axum 外壳

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn cookie_header(headers: &HeaderMap) -> Option<String> {
    header_text(headers, header::COOKIE)
}

fn request_ip(runtime: &LocalAuthRuntime, headers: &HeaderMap, extensions: &Extensions) -> String {
    use axum::extract::ConnectInfo;

    client_ip(
        runtime.trust_proxy(),
        headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip()),
    )
}

/// 转义要拼进 HTML 文本节点的内容。
///
/// 调用方现在传的全是固定常量，但「现在是固定的」不是一道防线：
/// 一旦哪天有人把 query 里的东西传进来，这里是唯一拦得住的地方。
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

/// 出错时给浏览器的页面。
///
/// 回调是一次顶层导航，返回 JSON 会让用户看到一坨原始报文；但状态码必须
/// 保留，前端与监控都按它分流。文案只能来自本文件的固定常量。
pub(crate) fn error_page(err: &ApiError) -> Response<String> {
    let (status, message) = match err {
        ApiError::NotFound => (StatusCode::NOT_FOUND, MSG_UNKNOWN_PROVIDER),
        ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "请先登录再绑定第三方账号"),
        ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message.as_str()),
        ApiError::Forbidden(message) => (StatusCode::FORBIDDEN, message.as_str()),
        ApiError::Conflict(message) => (StatusCode::CONFLICT, message.as_str()),
        ApiError::TooManyRequests(message) => (StatusCode::TOO_MANY_REQUESTS, message.as_str()),
        ApiError::BadGateway(_) => (StatusCode::BAD_GATEWAY, MSG_PROVIDER_FAILED),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "登录失败，请稍后再试"),
    };
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN">
  <head><meta charset="utf-8"><title>登录失败</title></head>
  <body><h1>登录失败</h1><p>{}</p><p><a href="/">返回登录页</a></p></body>
</html>"#,
        html_escape(message)
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(body)
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(String::new())
                .expect("构造空响应不会失败")
        })
}

fn redirect_response(location: &str) -> Response<String> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(String::new())
        .unwrap_or_else(|_| error_page(&ApiError::BadRequest(MSG_EXPIRED.to_string())))
}

fn push_cookie(response: &mut Response<String>, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

pub(crate) async fn start(
    State(deployment): State<DeploymentImpl>,
    Path(provider): Path<String>,
) -> Response<String> {
    match handle_start(deployment.local_auth(), &provider, None) {
        Ok(outcome) => {
            let mut response = redirect_response(&outcome.authorize_url);
            push_cookie(&mut response, &outcome.nonce_cookie);
            response
        }
        Err(err) => error_page(&err),
    }
}

pub(crate) async fn callback(
    State(deployment): State<DeploymentImpl>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    extensions: Extensions,
    headers: HeaderMap,
) -> Response<String> {
    let runtime = deployment.local_auth();
    let secure = runtime.secure_cookies();
    let input = CallbackInput {
        provider,
        query,
        cookie_header: cookie_header(&headers),
        user_agent: header_text(&headers, header::USER_AGENT),
        ip: Some(request_ip(runtime, &headers, &extensions)),
    };

    let result = handle_callback(&deployment.db().pool, runtime, provider_client(), input).await;
    let mut response = match result {
        Ok(outcome) => {
            let mut response = redirect_response(outcome.redirect);
            if let Some(login) = outcome.login {
                push_cookie(&mut response, &login.session_cookie);
                push_cookie(&mut response, &login.csrf_cookie);
            }
            response
        }
        Err(err) => error_page(&err),
    };
    // nonce 用完即焚，无论成败。
    push_cookie(&mut response, &build_oauth_nonce_clear_cookie(secure));
    response
}

pub(crate) async fn bind(
    State(deployment): State<DeploymentImpl>,
    Path(provider): Path<String>,
    current: CurrentUser,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<OAuthBindStart>>), ApiError> {
    let outcome = handle_start(deployment.local_auth(), &provider, Some(current.id))?;
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&outcome.nonce_cookie) {
        headers.append(header::SET_COOKIE, value);
    }
    Ok((
        headers,
        ResponseJson(ApiResponse::success(OAuthBindStart {
            authorize_url: outcome.authorize_url,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Router, routing::get};
    use db::{
        models::local_user::{LocalUserRole, NewLocalUser},
        test_support::TestDb,
    };
    use serde_json::{Value, json};
    use services::services::server_settings::{OAuthClientCredentials, ServerSettings};
    use tokio::task::JoinHandle;

    use super::*;

    // ------------------------------------------------------- mock IdP
    //
    // 仓库里没有 HTTP mock 基建（`server` 的 dev-dependencies 只有 `tempfile`），
    // 也刻意**不引入 wiremock**：`crates/server` 本来就依赖 axum + tokio，
    // 用它们起一个 `127.0.0.1:0` 的测试服务器足够，且不多一份依赖。

    #[derive(Debug, Clone)]
    struct MockConfig {
        token_status: StatusCode,
        token_body: Value,
        userinfo_status: StatusCode,
        userinfo_body: Value,
        /// 拿到不匹配的 `code` 就返回 400，用来验证授权码确实被原样送过去了。
        expect_code: String,
    }

    impl Default for MockConfig {
        fn default() -> Self {
            Self {
                token_status: StatusCode::OK,
                token_body: json!({"code": 0, "access_token": "at-mock", "expires_in": 7200}),
                userinfo_status: StatusCode::OK,
                userinfo_body: json!({
                    "code": 0,
                    "msg": "success",
                    "data": {"union_id": "on_u1", "open_id": "ou_1", "name": "爱丽丝"}
                }),
                expect_code: "the-code".to_string(),
            }
        }
    }

    #[derive(Debug, Default)]
    struct MockRecord {
        /// 收到的 `/token` 请求体原文。
        token_bodies: Vec<String>,
        /// 收到的 `/userinfo` 的 Authorization 头。
        userinfo_auth: Vec<String>,
    }

    struct MockIdp {
        base_url: String,
        record: Arc<Mutex<MockRecord>>,
        handle: JoinHandle<()>,
    }

    impl Drop for MockIdp {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn 起_mock(config: MockConfig) -> MockIdp {
        let record = Arc::new(Mutex::new(MockRecord::default()));
        let state = (Arc::new(config), record.clone());

        let app = Router::new()
            .route(
                "/token",
                axum::routing::post({
                    let state = state.clone();
                    move |body: String| {
                        let (config, record) = state.clone();
                        async move {
                            record.lock().unwrap().token_bodies.push(body.clone());
                            if !body.contains(&config.expect_code) {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    axum::Json(json!({"error": "invalid_grant"})),
                                );
                            }
                            (config.token_status, axum::Json(config.token_body.clone()))
                        }
                    }
                }),
            )
            .route(
                "/userinfo",
                get({
                    let state = state.clone();
                    move |headers: HeaderMap| {
                        let (config, record) = state.clone();
                        async move {
                            let auth = headers
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            record.lock().unwrap().userinfo_auth.push(auth);
                            (
                                config.userinfo_status,
                                axum::Json(config.userinfo_body.clone()),
                            )
                        }
                    }
                }),
            )
            // 授权端点：真实提供方的页面，测试里不会真的打它。
            .route("/authorize", get(|| async { "authorize" }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock IdP 绑定端口失败");
        let addr = listener.local_addr().expect("取 mock IdP 地址失败");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        MockIdp {
            base_url: format!("http://{addr}"),
            record,
            handle,
        }
    }

    // ------------------------------------------------------- 测试脚手架

    fn 凭据(base: &str) -> OAuthClientCredentials {
        OAuthClientCredentials {
            client_id: "cli_x".to_string(),
            client_secret: "sec_x".to_string(),
            authorize_url: Some(format!("{base}/authorize")),
            token_url: Some(format!("{base}/token")),
            userinfo_url: Some(format!("{base}/userinfo")),
            scopes: None,
            pkce: None,
        }
    }

    fn 设置(mode: ServerMode, base: &str, allow_signup: bool) -> ServerSettings {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert("feishu".to_string(), 凭据(base));
        providers.insert("google".to_string(), 凭据(base));
        ServerSettings {
            mode,
            allow_oauth_signup: allow_signup,
            public_base_url: Some("https://k.example".to_string()),
            providers,
            ..ServerSettings::default()
        }
    }

    fn 运行时(settings: ServerSettings) -> LocalAuthRuntime {
        LocalAuthRuntime::new(settings, String::new())
    }

    fn 客户端() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("构造测试客户端失败")
    }

    async fn 建用户(
        db: &TestDb,
        username: &str,
        email: Option<&str>,
        role: LocalUserRole,
    ) -> LocalUser {
        LocalUsers::create(
            db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: email.map(str::to_string),
                password_hash: Some("$argon2id$x".to_string()),
                role,
            },
        )
        .await
        .expect("建用户失败")
    }

    /// 走一遍 `/start`，返回 `(state, nonce, Cookie 头)`。
    fn 发起(runtime: &LocalAuthRuntime, provider: &str) -> (String, String, String) {
        let outcome = handle_start(runtime, provider, None).expect("start 应成功");
        提取(&outcome)
    }

    fn 提取(outcome: &StartOutcome) -> (String, String, String) {
        let url = url::Url::parse(&outcome.authorize_url).expect("授权链接应合法");
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string())
            .expect("授权链接里应有 state");
        let nonce = outcome
            .nonce_cookie
            .split(';')
            .next()
            .and_then(|kv| kv.split_once('='))
            .map(|(_, v)| v.to_string())
            .expect("nonce Cookie 应可解析");
        let cookie = format!("{OAUTH_NONCE_COOKIE}={nonce}");
        (state, nonce, cookie)
    }

    fn 回调入参(provider: &str, code: &str, state: &str, cookie: &str) -> CallbackInput {
        CallbackInput {
            provider: provider.to_string(),
            query: OAuthCallbackQuery {
                code: Some(code.to_string()),
                state: Some(state.to_string()),
                error: None,
            },
            cookie_header: Some(cookie.to_string()),
            user_agent: Some("测试浏览器".to_string()),
            ip: Some("127.0.0.1".to_string()),
        }
    }

    async fn 用户数(db: &TestDb) -> usize {
        LocalUsers::find_all(db.pool()).await.unwrap().len()
    }

    async fn 会话数(db: &TestDb) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    async fn 身份数(db: &TestDb) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM local_user_identities")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    // ------------------------------------------------------------ start

    #[tokio::test]
    async fn start_返回授权链接与_nonce_cookie() {
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let outcome = handle_start(&runtime, "feishu", None).expect("应成功");

        assert!(outcome.authorize_url.starts_with(&idp.base_url));
        assert!(outcome.authorize_url.contains("client_id=cli_x"));
        assert!(
            outcome.authorize_url.contains(
                "redirect_uri=https%3A%2F%2Fk.example%2Fapi%2Flocal-auth%2Foauth%2Ffeishu%2Fcallback"
            ),
            "{}",
            outcome.authorize_url
        );
        assert!(outcome.nonce_cookie.contains("HttpOnly"));
        assert!(outcome.nonce_cookie.contains("SameSite=Lax"));
        // 客户端密钥绝不能出现在跳转链接里。
        assert!(!outcome.authorize_url.contains("sec_x"));
    }

    /// 个人模式下整组路由在功能上不存在。
    #[tokio::test]
    async fn 个人模式下_start_返回_404() {
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Personal, &idp.base_url, false));
        assert!(matches!(
            handle_start(&runtime, "feishu", None),
            Err(ApiError::NotFound)
        ));
    }

    /// **路径注入**：不在白名单里的 provider 一律 404，且原字符串不进任何字符串。
    #[tokio::test]
    async fn 非法_provider_一律_404() {
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        for provider in [
            "../x",
            "..",
            "%2e%2e%2f",
            "FEISHU",
            "feishu ",
            " feishu",
            "",
            "github",
            "lark", // 白名单里有，但本用例没配凭据
            "feishu/../google",
            "<script>",
        ] {
            let err = handle_start(&runtime, provider, None).expect_err("应被拒");
            assert!(matches!(err, ApiError::NotFound), "{provider:?} => {err:?}");
            // 渲染出来的页面里绝不能出现原字符串。
            let page = error_page(&err);
            assert!(
                !page.body().contains(provider) || provider.is_empty(),
                "{provider:?} 被回显进了页面"
            );
        }
    }

    /// 没配 `public_base_url` 时报错，**不能**拼出畸形 URL 或拿 Host 头兜底。
    #[tokio::test]
    async fn 没配_public_base_url_时_start_报错() {
        let idp = 起_mock(MockConfig::default()).await;
        let mut settings = 设置(ServerMode::Team, &idp.base_url, false);
        settings.public_base_url = None;
        let err = handle_start(&运行时(settings), "feishu", None).expect_err("应被拒");
        assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
    }

    #[tokio::test]
    async fn 每次_start_的_state_与_nonce_都不同() {
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let (state, nonce, _) = 发起(&runtime, "feishu");
            assert!(seen.insert(state.clone()), "state 重复");
            assert!(seen.insert(nonce), "nonce 与 state 撞了");
            assert_eq!(state.len(), 43, "state 应是 32 字节 base64url");
        }
    }

    // --------------------------------------------------------- 完整流程

    #[tokio::test]
    async fn 已绑定身份可以登录并建会话() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();

        let (state, _, cookie) = 发起(&runtime, "feishu");
        let outcome = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect("回调应成功");

        assert_eq!(outcome.redirect, "/");
        let login = outcome.login.expect("应建会话");
        assert_eq!(login.user.id, amy.id);
        assert!(login.session_cookie.starts_with("vk_session="));
        assert!(login.csrf_cookie.starts_with("vk_csrf="));
        assert_eq!(会话数(&db).await, 1);
        // 没有新建用户，也没有新建身份。
        assert_eq!(用户数(&db).await, 2, "只应有本机用户与 amy");
        assert_eq!(身份数(&db).await, 1);

        // 授权码确实被原样送去换令牌，且带上了 client_secret。
        let record = idp.record.lock().unwrap();
        assert_eq!(record.token_bodies.len(), 1);
        assert!(record.token_bodies[0].contains("the-code"));
        assert!(record.token_bodies[0].contains("sec_x"));
        // 用户信息用的是刚换来的令牌。
        assert_eq!(record.userinfo_auth, ["Bearer at-mock"]);
    }

    /// 令牌**不落库**：走完一整趟之后，库里任何一张表都不该出现它。
    #[tokio::test]
    async fn 回调不把令牌写进库里() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();

        let (state, _, cookie) = 发起(&runtime, "feishu");
        handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .unwrap();

        // 把认证相关的四张表**整表**读成文本再逐列扫敏感串。
        // 用 `quote()` 是因为 id 是 BLOB：`CAST(blob AS TEXT)` 可能不是合法
        // UTF-8，解码会失败；`quote()` 对任何类型都给出可打印的字面量。
        for table in [
            "local_users",
            "local_user_identities",
            "local_sessions",
            "local_invites",
        ] {
            let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
                sqlx::query_as(&format!("PRAGMA table_info({table})"))
                    .fetch_all(db.pool())
                    .await
                    .unwrap_or_else(|err| panic!("读取 {table} 表结构失败：{err}"));
            assert!(!columns.is_empty(), "{table} 不存在");
            let expr = columns
                .iter()
                .map(|c| format!("quote(\"{}\")", c.1))
                .collect::<Vec<_>>()
                .join("||'|'||");
            let rows: Vec<(String,)> = sqlx::query_as(&format!("SELECT {expr} FROM {table}"))
                .fetch_all(db.pool())
                .await
                .unwrap_or_else(|err| panic!("整表读取 {table} 失败：{err}"));
            let dump = rows.into_iter().map(|r| r.0).collect::<Vec<_>>().join("\n");
            for 禁词 in ["at-mock", "sec_x", "the-code", "cli_x"] {
                assert!(!dump.contains(禁词), "{table} 里出现了 {禁词}：{dump}");
            }
        }
        // 身份行里存的就只有 subject。
        let identity = LocalUserIdentities::find_by_provider_subject(db.pool(), "feishu", "on_u1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(identity.subject, "on_u1");
    }

    // --------------------------------------------------------- state 攻击

    #[tokio::test]
    async fn state_不存在时被拒且不建会话() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let (_, _, cookie) = 发起(&runtime, "feishu");

        for forged in ["", "forged", "AAAA", "%00"] {
            let err = handle_callback(
                db.pool(),
                &runtime,
                &客户端(),
                回调入参("feishu", "the-code", forged, &cookie),
            )
            .await
            .expect_err("伪造的 state 必须被拒");
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
                "{forged:?} => {err:?}"
            );
        }
        assert_eq!(会话数(&db).await, 0);
        assert_eq!(
            idp.record.lock().unwrap().token_bodies.len(),
            0,
            "不该发外部请求"
        );
    }

    /// **重放**：同一个 state 第二次必须失败。
    #[tokio::test]
    async fn state_用过一次之后被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();

        let (state, _, cookie) = 发起(&runtime, "feishu");
        handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect("第一次应成功");

        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect_err("重放必须被拒");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
            "{err:?}"
        );
        assert_eq!(会话数(&db).await, 1, "重放不得再建一条会话");
    }

    /// **过期**：10 分钟之后的回调必须失败。
    #[tokio::test]
    async fn state_过期后被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let (state, _, cookie) = 发起(&runtime, "feishu");

        // 直接把表里的条目按「已过期」重新插入，等价于时间前进 10 分钟。
        {
            let mut store = runtime.oauth_states();
            let 过期时刻 = Instant::now() - STATE_TTL - Duration::from_secs(1);
            let entry = store
                .consume(&state, "feishu", Instant::now())
                .expect("应还在");
            store.insert(
                &state,
                PendingOAuthState {
                    expires_at: 过期时刻,
                    ..entry
                },
                过期时刻,
            );
        }

        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect_err("过期的 state 必须被拒");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
            "{err:?}"
        );
        assert_eq!(会话数(&db).await, 0);
    }

    /// **跨 provider**：feishu 的 state 拿去 google 的回调必须失败。
    #[tokio::test]
    async fn state_属于另一个_provider_时被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let (state, _, cookie) = 发起(&runtime, "feishu");

        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("google", "the-code", &state, &cookie),
        )
        .await
        .expect_err("跨 provider 必须被拒");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
            "{err:?}"
        );
        // 原来的 state 不该被烧掉。
        assert!(
            runtime
                .oauth_states()
                .consume(&state, "feishu", Instant::now())
                .is_some()
        );
    }

    /// **跨浏览器**：另一台机器拿到了 URL 里的 state，但没有 nonce Cookie。
    #[tokio::test]
    async fn 没有_nonce_cookie_的回调被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();

        for 假 in [
            "",
            "vk_oauth=",
            "vk_oauth=wrong",
            "vk_session=abc",
            "vk_oauth=wrong; vk_session=abc",
        ] {
            let (state, _, _) = 发起(&runtime, "feishu");
            let err = handle_callback(
                db.pool(),
                &runtime,
                &客户端(),
                回调入参("feishu", "the-code", &state, 假),
            )
            .await
            .expect_err("没有正确 nonce 必须被拒");
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
                "{假:?} => {err:?}"
            );
        }
        assert_eq!(会话数(&db).await, 0);
    }

    /// 用另一次 `/start` 的 nonce 也不行：nonce 与 state 是成对的。
    #[tokio::test]
    async fn 另一次登录的_nonce_不能配这一次的_state() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let (state_a, _, _) = 发起(&runtime, "feishu");
        let (_, _, cookie_b) = 发起(&runtime, "feishu");

        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state_a, &cookie_b),
        )
        .await
        .expect_err("串了的 nonce 必须被拒");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m == MSG_EXPIRED),
            "{err:?}"
        );
    }

    // ----------------------------------------------------- 回调参数与反射

    /// **反射型 XSS**：`?error=` 的内容绝不能出现在响应体里。
    #[tokio::test]
    async fn 回调的_error_参数不被回显() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let (state, _, cookie) = 发起(&runtime, "feishu");

        let 攻击载荷 = "<script>alert(document.cookie)</script>";
        let mut input = 回调入参("feishu", "the-code", &state, &cookie);
        input.query.error = Some(攻击载荷.to_string());

        let err = handle_callback(db.pool(), &runtime, &客户端(), input)
            .await
            .expect_err("带 error 的回调必须被拒");
        let page = error_page(&err);
        assert_eq!(page.status(), StatusCode::BAD_REQUEST);
        for 片段 in ["<script>", "alert", "document.cookie", 攻击载荷] {
            assert!(
                !page.body().contains(片段),
                "页面回显了 {片段}：{}",
                page.body()
            );
        }
        assert_eq!(会话数(&db).await, 0);
        // state 已被消费掉，不能留着给攻击者再试。
        assert!(
            runtime
                .oauth_states()
                .consume(&state, "feishu", Instant::now())
                .is_none()
        );
    }

    /// 错误页的 Content-Type 与转义：任何文案都以 HTML 文本节点渲染。
    #[test]
    fn 错误页转义并带_content_type() {
        let page = error_page(&ApiError::BadRequest("<img src=x onerror=1>".to_string()));
        assert_eq!(
            page.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert!(!page.body().contains("<img"), "{}", page.body());
        assert!(page.body().contains("&lt;img"), "{}", page.body());
    }

    #[tokio::test]
    async fn 缺少_code_的回调被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));

        for code in [None, Some(String::new()), Some("   ".to_string())] {
            let (state, _, cookie) = 发起(&runtime, "feishu");
            let mut input = 回调入参("feishu", "x", &state, &cookie);
            input.query.code = code.clone();
            let err = handle_callback(db.pool(), &runtime, &客户端(), input)
                .await
                .expect_err("没有 code 必须被拒");
            assert!(
                matches!(err, ApiError::BadRequest(_)),
                "{code:?} => {err:?}"
            );
        }
        assert_eq!(idp.record.lock().unwrap().token_bodies.len(), 0);
    }

    // ------------------------------------------------------- 提供方故障

    async fn 跑一次(
        db: &TestDb,
        runtime: &LocalAuthRuntime,
        provider: &str,
    ) -> Result<CallbackOutcome, ApiError> {
        let (state, _, cookie) = 发起(runtime, provider);
        handle_callback(
            db.pool(),
            runtime,
            &客户端(),
            回调入参(provider, "the-code", &state, &cookie),
        )
        .await
    }

    #[tokio::test]
    async fn token_端点返回_500_时是_502() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            token_status: StatusCode::INTERNAL_SERVER_ERROR,
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
        assert!(matches!(err, ApiError::BadGateway(_)), "{err:?}");
        assert_eq!(会话数(&db).await, 0);
        assert_eq!(用户数(&db).await, 1, "不得建用户");
    }

    #[tokio::test]
    async fn token_端点返回_200_但没有_access_token_时是_502() {
        let db = TestDb::new().await;
        for body in [
            json!({"code": 0}),
            json!({"access_token": ""}),
            json!({"code": 0, "access_token": null}),
            json!("not an object"),
        ] {
            let idp = 起_mock(MockConfig {
                token_body: body.clone(),
                ..MockConfig::default()
            })
            .await;
            let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
            let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
            assert!(matches!(err, ApiError::BadGateway(_)), "{body} => {err:?}");
        }
        assert_eq!(会话数(&db).await, 0);
    }

    /// 扁平结构里的 `code != 0`：HTTP 200 但业务失败，必须识别出来。
    #[tokio::test]
    async fn token_端点返回扁平错误码时是_502() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            token_body: json!({"code": 99991400, "msg": "rate limited", "access_token": "at-x"}),
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
        assert!(matches!(err, ApiError::BadGateway(_)), "{err:?}");
        assert_eq!(会话数(&db).await, 0);
    }

    /// 授权码不对时 mock 返回 400，等价于「码已过期 / 被用过」。
    #[tokio::test]
    async fn 授权码不被提供方认可时是_502() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            expect_code: "only-this-code".to_string(),
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
        assert!(matches!(err, ApiError::BadGateway(_)), "{err:?}");
    }

    #[tokio::test]
    async fn userinfo_返回_401_时是_502() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            userinfo_status: StatusCode::UNAUTHORIZED,
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
        assert!(matches!(err, ApiError::BadGateway(_)), "{err:?}");
        assert_eq!(用户数(&db).await, 1, "不得建用户");
    }

    /// **缺 subject**：这是绑定键，缺了就绝不能建号。
    #[tokio::test]
    async fn userinfo_缺_subject_时是_502_且不建用户() {
        let db = TestDb::new().await;
        for body in [
            json!({"code": 0, "data": {"name": "爱丽丝", "email": "a@x.com"}}),
            json!({"code": 0, "data": {"union_id": "", "email": "a@x.com"}}),
            json!({"code": 0, "data": {"union_id": "   "}}),
            json!({"code": 0, "data": {"open_id": "ou_1"}}),
            json!({"code": 0}),
            json!({"code": 99991663, "data": {"union_id": "on_u1"}}),
        ] {
            let idp = 起_mock(MockConfig {
                userinfo_body: body.clone(),
                ..MockConfig::default()
            })
            .await;
            let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
            let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应失败");
            assert!(matches!(err, ApiError::BadGateway(_)), "{body} => {err:?}");
        }
        assert_eq!(用户数(&db).await, 1, "一个用户都不该建出来");
        assert_eq!(身份数(&db).await, 0);
        assert_eq!(会话数(&db).await, 0);
    }

    /// 缺 email 不影响登录：email 只是资料，不是绑定键。
    #[tokio::test]
    async fn userinfo_缺_email_仍可登录且_email_为空() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            userinfo_body: json!({"code": 0, "data": {"union_id": "on_u1", "name": "爱丽丝"}}),
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let outcome = 跑一次(&db, &runtime, "feishu").await.expect("应成功");
        let login = outcome.login.expect("应建会话");
        assert!(login.user.email.is_none());
    }

    // ----------------------------------------------------- 建号与撞号

    #[tokio::test]
    async fn 允许自助注册时建出_member_用户() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            userinfo_body: json!({
                "code": 0,
                "data": {"union_id": "on_u1", "name": "爱丽丝", "email": "alice@x.com"}
            }),
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let outcome = 跑一次(&db, &runtime, "feishu").await.expect("应成功");
        let login = outcome.login.expect("应建会话");

        assert_eq!(login.user.role, "member", "第三方建号只能是 member");
        assert_eq!(login.user.display_name, "爱丽丝");
        assert_eq!(login.user.email.as_deref(), Some("alice@x.com"));
        // 用户名由 subject 派生且规范化。
        assert!(
            login.user.username.starts_with("feishu-"),
            "{}",
            login.user.username
        );
        assert!(
            login
                .user
                .username
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "用户名未规范化：{}",
            login.user.username
        );
        assert!(
            !login.user.username.contains("on_u1"),
            "用户名不该直接暴露 union_id"
        );
        assert_eq!(身份数(&db).await, 1);
        // 建出来的号没有密码。
        assert!(
            LocalUsers::find_password_hash(db.pool(), login.user.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// 派生的用户名撞上已有账号时自动加后缀，不会撞唯一约束。
    #[tokio::test]
    async fn 用户名冲突时加后缀() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let 占位 = derive_username("feishu", "on_u1");
        建用户(&db, &占位, None, LocalUserRole::Member).await;

        let outcome = 跑一次(&db, &runtime, "feishu").await.expect("应成功");
        let login = outcome.login.expect("应建会话");
        assert_eq!(login.user.username, format!("{占位}-1"));
    }

    /// **不按 email 自动合并**：这是防账号劫持的关键。
    #[tokio::test]
    async fn 邮箱撞号时拒绝且不建号不建会话() {
        for allow_signup in [false, true] {
            let db = TestDb::new().await;
            let idp = 起_mock(MockConfig {
                userinfo_body: json!({
                    "code": 0,
                    "data": {"union_id": "on_u1", "name": "冒充者", "email": "Alice@X.com"}
                }),
                ..MockConfig::default()
            })
            .await;
            let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, allow_signup));
            let alice = 建用户(&db, "alice", Some("alice@x.com"), LocalUserRole::Admin).await;
            let 用户数前 = 用户数(&db).await;

            let err = 跑一次(&db, &runtime, "feishu")
                .await
                .expect_err("撞号必须被拒");
            assert!(
                matches!(&err, ApiError::Forbidden(m) if m == MSG_EMAIL_TAKEN),
                "allow_signup = {allow_signup} => {err:?}"
            );
            assert_eq!(error_page(&err).status(), StatusCode::FORBIDDEN);

            // 没登录成 alice、没建号、没建身份、没建会话。
            assert_eq!(用户数(&db).await, 用户数前, "不得新建用户");
            assert_eq!(身份数(&db).await, 0, "不得新建身份");
            assert_eq!(会话数(&db).await, 0, "不得建会话");
            assert!(
                LocalUserIdentities::list_for_user(db.pool(), alice.id)
                    .await
                    .unwrap()
                    .is_empty(),
                "不得把身份挂到 alice 名下"
            );
        }
    }

    /// 邮箱比对大小写不敏感：`Alice@X.com` 与 `alice@x.com` 是同一个人。
    #[tokio::test]
    async fn 邮箱撞号比对大小写不敏感() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig {
            userinfo_body: json!({
                "code": 0,
                "data": {"union_id": "on_u1", "email": "ALICE@X.COM"}
            }),
            ..MockConfig::default()
        })
        .await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        建用户(&db, "alice", Some("alice@x.com"), LocalUserRole::Member).await;
        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应被拒");
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == MSG_EMAIL_TAKEN),
            "{err:?}"
        );
    }

    /// `allow_oauth_signup` 默认关：未绑定的身份不能自助建号。
    #[tokio::test]
    async fn 关闭自助注册时未绑定身份被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        assert!(!runtime.allow_oauth_signup(), "默认必须是关的");

        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应被拒");
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == MSG_SIGNUP_DISABLED),
            "{err:?}"
        );
        assert_eq!(用户数(&db).await, 1);
        assert_eq!(会话数(&db).await, 0);
    }

    /// **被停用的用户**走第三方登录同样进不来。
    #[tokio::test]
    async fn 被停用用户的第三方登录被拒() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();
        LocalUsers::set_status(db.pool(), amy.id, LocalUserStatus::Disabled)
            .await
            .unwrap();

        let err = 跑一次(&db, &runtime, "feishu").await.expect_err("应被拒");
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == MSG_DISABLED_USER),
            "{err:?}"
        );
        assert_eq!(会话数(&db).await, 0, "停用的账号不得拿到会话");
    }

    /// 已绑定的身份再次回调，归属不会改到别人头上。
    #[tokio::test]
    async fn 已绑定的身份不会被改绑() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, true));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        建用户(&db, "bob", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();

        for _ in 0..3 {
            let outcome = 跑一次(&db, &runtime, "feishu").await.expect("应成功");
            assert_eq!(outcome.login.unwrap().user.id, amy.id);
        }
        assert_eq!(身份数(&db).await, 1, "不得插出第二行身份");
        assert_eq!(
            LocalUserIdentities::find_by_provider_subject(db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .unwrap()
                .user_id,
            amy.id
        );
    }

    // ------------------------------------------------------------ 绑定

    async fn 建会话(db: &TestDb, runtime: &LocalAuthRuntime, user: &LocalUser) -> String {
        let outcome = start_session(db.pool(), runtime, user, None, None)
            .await
            .expect("建会话失败");
        outcome
            .session_cookie
            .split(';')
            .next()
            .expect("会话 Cookie 应可解析")
            .to_string()
    }

    #[tokio::test]
    async fn 已登录用户可以绑定第三方账号() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        let session = 建会话(&db, &runtime, &amy).await;

        let start = handle_start(&runtime, "feishu", Some(amy.id)).expect("发起绑定应成功");
        let (state, nonce, _) = 提取(&start);
        let cookie = format!("{OAUTH_NONCE_COOKIE}={nonce}; {session}");

        let outcome = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect("绑定应成功");

        assert!(outcome.login.is_none(), "绑定不该再建一条会话");
        assert_eq!(outcome.redirect, "/?oauth=bound");
        assert_eq!(身份数(&db).await, 1);
        assert_eq!(
            LocalUserIdentities::list_for_user(db.pool(), amy.id)
                .await
                .unwrap()[0]
                .subject,
            "on_u1"
        );
    }

    /// 重复绑同一个身份是幂等的，不会插出第二行也不会报错。
    #[tokio::test]
    async fn 重复绑定同一身份是幂等的() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        let session = 建会话(&db, &runtime, &amy).await;

        for _ in 0..2 {
            let start = handle_start(&runtime, "feishu", Some(amy.id)).unwrap();
            let (state, nonce, _) = 提取(&start);
            let cookie = format!("{OAUTH_NONCE_COOKIE}={nonce}; {session}");
            handle_callback(
                db.pool(),
                &runtime,
                &客户端(),
                回调入参("feishu", "the-code", &state, &cookie),
            )
            .await
            .expect("绑定应成功");
        }
        assert_eq!(身份数(&db).await, 1);
    }

    /// **绑定不能把已属于他人的身份抢过来。**
    #[tokio::test]
    async fn 绑定不能抢走他人的身份() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        let bob = 建用户(&db, "bob", None, LocalUserRole::Member).await;
        LocalUserIdentities::link(db.pool(), amy.id, "feishu", "on_u1", None)
            .await
            .unwrap();
        let session = 建会话(&db, &runtime, &bob).await;

        let start = handle_start(&runtime, "feishu", Some(bob.id)).unwrap();
        let (state, nonce, _) = 提取(&start);
        let cookie = format!("{OAUTH_NONCE_COOKIE}={nonce}; {session}");

        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect_err("抢绑必须失败");
        assert!(
            matches!(&err, ApiError::Conflict(m) if m == MSG_ALREADY_LINKED),
            "{err:?}"
        );
        assert_eq!(error_page(&err).status(), StatusCode::CONFLICT);
        assert_eq!(
            LocalUserIdentities::find_by_provider_subject(db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .unwrap()
                .user_id,
            amy.id,
            "归属必须还在 amy 身上"
        );
        assert!(
            LocalUserIdentities::list_for_user(db.pool(), bob.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// 绑定回调必须仍带着**同一个人**的有效会话：
    /// 只信 state 的话，一条泄漏的绑定链接就能把攻击者的身份挂到受害者名下。
    #[tokio::test]
    async fn 绑定回调要求同一个人的有效会话() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        let bob = 建用户(&db, "bob", None, LocalUserRole::Member).await;
        let bob_session = 建会话(&db, &runtime, &bob).await;

        // 为 amy 发起的绑定，回调时却带着 bob 的会话（或根本没有会话）。
        for 会话 in ["".to_string(), bob_session.clone()] {
            let start = handle_start(&runtime, "feishu", Some(amy.id)).unwrap();
            let (state, nonce, _) = 提取(&start);
            let cookie = if 会话.is_empty() {
                format!("{OAUTH_NONCE_COOKIE}={nonce}")
            } else {
                format!("{OAUTH_NONCE_COOKIE}={nonce}; {会话}")
            };
            let err = handle_callback(
                db.pool(),
                &runtime,
                &客户端(),
                回调入参("feishu", "the-code", &state, &cookie),
            )
            .await
            .expect_err("会话不匹配必须被拒");
            assert!(matches!(err, ApiError::Unauthorized), "{会话:?} => {err:?}");
        }
        assert_eq!(身份数(&db).await, 0, "一条身份都不该被绑上");
    }

    #[tokio::test]
    async fn 绑定期间被停用的用户绑不上() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));
        let amy = 建用户(&db, "amy", None, LocalUserRole::Member).await;
        let session = 建会话(&db, &runtime, &amy).await;
        let start = handle_start(&runtime, "feishu", Some(amy.id)).unwrap();
        let (state, nonce, _) = 提取(&start);
        LocalUsers::set_status(db.pool(), amy.id, LocalUserStatus::Disabled)
            .await
            .unwrap();

        let cookie = format!("{OAUTH_NONCE_COOKIE}={nonce}; {session}");
        let err = handle_callback(
            db.pool(),
            &runtime,
            &客户端(),
            回调入参("feishu", "the-code", &state, &cookie),
        )
        .await
        .expect_err("停用的账号不该能绑定");
        // 会话查询本身就把停用用户挡掉了（SQL 里带 status = 'active'）。
        assert!(
            matches!(err, ApiError::Unauthorized | ApiError::Forbidden(_)),
            "{err:?}"
        );
        assert_eq!(身份数(&db).await, 0);
    }

    // ------------------------------------------------------------ 限速

    /// 回调失败会计数，连续失败到上限后直接 429，不再发外网请求。
    #[tokio::test]
    async fn 回调连续失败后被限速() {
        let db = TestDb::new().await;
        let idp = 起_mock(MockConfig::default()).await;
        let runtime = 运行时(设置(ServerMode::Team, &idp.base_url, false));

        let mut 命中限速 = false;
        for _ in 0..30 {
            let err = handle_callback(
                db.pool(),
                &runtime,
                &客户端(),
                回调入参("feishu", "the-code", "forged-state", "vk_oauth=x"),
            )
            .await
            .expect_err("伪造的 state 必然失败");
            if matches!(err, ApiError::TooManyRequests(_)) {
                命中限速 = true;
                break;
            }
        }
        assert!(命中限速, "连续失败必须触发限速");
    }

    // ------------------------------------------------------- 路由与契约

    #[test]
    fn 派生用户名稳定且合法() {
        let a = derive_username("feishu", "on_u1");
        assert_eq!(a, derive_username("feishu", "on_u1"), "必须稳定");
        assert_ne!(a, derive_username("google", "on_u1"), "不同提供方不得撞");
        assert_ne!(a, derive_username("feishu", "on_u2"));
        assert!(a.starts_with("feishu-"));
        assert!(
            a.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{a}"
        );
        // 后缀形式同样合法。
        for attempt in 0..MAX_USERNAME_ATTEMPTS {
            let name = candidate_username("feishu", "on_u1", attempt);
            assert!(name.len() <= 64, "{name}");
            db::models::local_user::validate_username(&name)
                .unwrap_or_else(|err| panic!("{name} 不是合法用户名：{err}"));
        }
    }

    /// 派生的用户名里绝不能出现 `subject` 本身。
    #[test]
    fn 派生用户名不泄露_subject() {
        for subject in ["on_u1", "1234567890", "ou_abcdef"] {
            let name = derive_username("feishu", subject);
            assert!(!name.contains(subject), "{name} 泄露了 {subject}");
        }
    }

    #[test]
    fn 错误映射的状态码语义固定() {
        use axum::response::IntoResponse;

        for (err, expected) in [
            (ApiError::NotFound, StatusCode::NOT_FOUND),
            (
                ApiError::BadRequest(MSG_EXPIRED.to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (
                ApiError::Forbidden(MSG_EMAIL_TAKEN.to_string()),
                StatusCode::FORBIDDEN,
            ),
            (
                ApiError::Conflict(MSG_ALREADY_LINKED.to_string()),
                StatusCode::CONFLICT,
            ),
            (
                ApiError::BadGateway(MSG_PROVIDER_FAILED.to_string()),
                StatusCode::BAD_GATEWAY,
            ),
            (ApiError::Unauthorized, StatusCode::UNAUTHORIZED),
        ] {
            assert_eq!(error_page(&err).status(), expected, "{err:?}");
            // 与 ApiError 自己的 JSON 响应保持同一个状态码，
            // 免得同一种失败在 HTML 与 JSON 两条路径上给出不同的码。
            let json_status = err.into_response().status();
            assert_eq!(json_status, expected);
        }
    }

    #[test]
    fn oauth_错误映射到预期的_api_错误() {
        assert!(matches!(
            map_oauth_error(OAuthError::UnknownProvider),
            ApiError::NotFound
        ));
        assert!(matches!(
            map_oauth_error(OAuthError::InvalidState),
            ApiError::BadRequest(_)
        ));
        assert!(matches!(
            map_oauth_error(OAuthError::ProviderResponse),
            ApiError::BadGateway(_)
        ));
        assert!(matches!(
            map_oauth_error(OAuthError::MissingSubject),
            ApiError::BadGateway(_)
        ));
    }

    /// 跳转目标是**同源常量**，不接受任何来自 URL 的重定向参数。
    #[test]
    fn 跳转目标是固定的同源路径() {
        for target in [LOGIN_REDIRECT, BIND_REDIRECT] {
            assert!(target.starts_with('/'), "{target}");
            assert!(!target.starts_with("//"), "{target} 是协议相对 URL");
            assert!(!target.contains("://"), "{target}");
        }
    }

    /// 源码层面的钉子：本文件不得把令牌写进日志。
    #[test]
    fn 源码里没有把令牌写进日志() {
        let source = include_str!("oauth_routes.rs");
        let 实现 = source.split("#[cfg(test)]").next().unwrap_or(source);
        for 行 in 实现.lines() {
            if 行.contains("tracing::") {
                for 禁词 in ["access_token", "code", "client_secret", "nonce", "state"] {
                    assert!(
                        !行.contains(&format!("%{禁词}")) && !行.contains(&format!("?{禁词}")),
                        "日志里带上了 {禁词}：{行}"
                    );
                }
            }
        }
    }
}
