//! 第三方登录（飞书 / Lark / Google）的提供方抽象、授权 URL 拼装、
//! 令牌与用户信息解析，以及 `state` 存储。
//!
//! # 事实与出处
//!
//! 下面这些是查过官方文档的（见 `docs/superpowers/specs/2026-09-16-team-mode-and-ui-design.md`
//! 第 6.4 节）：
//!
//! - **URL 参数名是 `client_id` 不是 `app_id`**。飞书开发者后台把它显示成
//!   "App ID"，但拼进授权链接的键名是 `client_id`。
//! - **飞书的 v2 换令牌端点已被官方标注弃用**，预置值用
//!   `https://accounts.feishu.cn/oauth/v3/token`（POST，form-urlencoded）；
//!   Lark 侧官方文档目前仍是 v2 且没有弃用提示（POST，JSON）。两边版本不同步，
//!   所以**端点一律做成可配置项**（`server.json` 的 `oauth.<id>.token_url`
//!   或 `VK_OAUTH_<ID>_TOKEN_URL`）。
//! - **飞书/Lark 的认证类接口是扁平结构**：`code` 与 `access_token` 同层，
//!   **不套** `{code,msg,data}` 信封。所以这里没有「通用剥 data 层」的逻辑，
//!   换令牌与拉用户信息分别用 [`parse_access_token`] 与 [`extract_identity`]，
//!   只有后者（`/open-apis/authen/v1/user_info` 这类 open-apis 接口）才带信封。
//! - **稳定身份标识**：飞书/Lark 用 `union_id`（`open_id` 是应用级的，换应用
//!   凭据就变；`user_id` 有复用风险；email/mobile 官方明文说「未经用户本人
//!   实时验证，不建议作为登录凭证」）；Google 用 `sub`（官方原文：
//!   `Don't use the email field as a unique identifier`）。
//! - 不硬编码任何令牌时长：有效期以接口返回的 `expires_in` 为准，本模块
//!   干脆连 `expires_in` 都不存——access token 用完即弃。
//!
//! # 待实测（**不得当作事实**）
//!
//! 1. 飞书/Lark 的 `redirect_uri` 是否强制 HTTPS、是否允许局域网裸 IP。
//!    Google 已明文禁止裸 IP 且要求 HTTPS（`localhost`/`127.0.0.1`/`[::1]` 除外）。
//! 2. Lark 是否也有 v3 令牌端点 → 用 `token_url` 覆盖项兜底。
//! 3. Lark 的 `user_info` 响应字段是否与飞书一致 → `subject_field` 等做成配置。
//! 4. Lark 是否支持 PKCE → 预置 `pkce = false`，飞书/Google 预置 `true`，
//!    都可用 `VK_OAUTH_<ID>_PKCE` 覆盖。
//! 5. 拿到 `union_id`/`name`/`email` 所需的最小权限点字符串 → 预置为空
//!    （不发 `scope`，用应用后台勾选的权限），必须时用 `VK_OAUTH_<ID>_SCOPES` 配。
//! 6. Google「Web application」与「Desktop app」客户端类型对 localhost 回调的
//!    支持范围（两份官方文档表述有出入）→ 只影响部署文档，不影响本模块。

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::{runtime::constant_time_eq, token::hash_session_token};
use crate::services::server_settings::{OAuthClientCredentials, ServerSettings};

/// `state` 有效期：10 分钟。授权码本身也是短时一次性的，没必要更长。
pub const STATE_TTL: Duration = Duration::from_secs(10 * 60);

/// 待回调的 `state` 条数上限。超过就淘汰最老的一条。
///
/// 没有上限就是一条免鉴权的内存耗尽路径：`/start` 不需要任何凭据，
/// 每打一次就往表里塞一条，10 分钟内能塞进多少全看攻击者的带宽。
pub const MAX_PENDING_STATES: usize = 10_000;

/// 回调路径前缀（含 `/api`）。`redirect_uri` 与 Cookie 的 `Path` 都从这里来。
pub const OAUTH_PATH_PREFIX: &str = "/api/local-auth/oauth";

/// `subject` 长度上限。对方返回一个几兆的字符串不该被原样写进库里。
pub const MAX_SUBJECT_LEN: usize = 255;

/// 换令牌请求体的格式。飞书 v3 是 form-urlencoded，Lark v2 是 JSON。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenBodyFormat {
    Form,
    Json,
}

/// 一个提供方的预置配置。全部字段都能被 `server.json` / 环境变量覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthProviderConfig {
    pub id: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub userinfo_url: &'static str,
    /// 空串表示不发 `scope` 参数（用应用后台勾选的权限）。
    pub scopes: &'static str,
    pub subject_field: &'static str,
    pub email_field: &'static str,
    pub name_field: &'static str,
    pub token_body_format: TokenBodyFormat,
    /// 用户信息响应是否包在 `{"code":0,"msg":"...","data":{...}}` 里。
    /// **只对用户信息接口生效**，换令牌接口一律按扁平结构解析。
    pub userinfo_envelope: bool,
    pub pkce: bool,
}

/// 飞书。`accounts.feishu.cn` 是认证域，`open.feishu.cn` 是开放平台 API 域。
pub const FEISHU: OAuthProviderConfig = OAuthProviderConfig {
    id: "feishu",
    authorize_url: "https://accounts.feishu.cn/open-apis/authen/v1/authorize",
    // v2 已被官方标注弃用，这里用 v3。
    token_url: "https://accounts.feishu.cn/oauth/v3/token",
    userinfo_url: "https://open.feishu.cn/open-apis/authen/v1/user_info",
    scopes: "",
    subject_field: "union_id",
    email_field: "email",
    name_field: "name",
    token_body_format: TokenBodyFormat::Form,
    userinfo_envelope: true,
    pkce: true,
};

/// Lark（国际版）。官方文档目前仍是 v2 令牌端点且无弃用提示。
pub const LARK: OAuthProviderConfig = OAuthProviderConfig {
    id: "lark",
    authorize_url: "https://accounts.larksuite.com/open-apis/authen/v1/authorize",
    token_url: "https://open.larksuite.com/open-apis/authen/v2/oauth/token",
    userinfo_url: "https://open.larksuite.com/open-apis/authen/v1/user_info",
    scopes: "",
    subject_field: "union_id",
    email_field: "email",
    name_field: "name",
    token_body_format: TokenBodyFormat::Json,
    userinfo_envelope: true,
    // 待实测：Lark 是否支持 PKCE。默认不带，免得多一个参数就被拒。
    pkce: false,
};

/// Google（OIDC）。
pub const GOOGLE: OAuthProviderConfig = OAuthProviderConfig {
    id: "google",
    authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo",
    // Google 的 scope 是必填项，且这三个是标准 OIDC scope，不需要后台配。
    scopes: "openid email profile",
    // 官方原文：Don't use the email field as a unique identifier. Always use `sub`.
    subject_field: "sub",
    email_field: "email",
    name_field: "name",
    token_body_format: TokenBodyFormat::Form,
    userinfo_envelope: false,
    pkce: true,
};

/// 按 id 取预置配置。
///
/// **精确匹配**，不做大小写折叠也不做任何规整：`provider` 来自 URL 路径，
/// 这里是它进入任何字符串拼接之前的唯一一道闸。命不中就是 `None`，
/// 调用方必须据此返回 404，绝不能把原字符串拼进 URL、日志或 HTML。
pub fn provider_config(id: &str) -> Option<&'static OAuthProviderConfig> {
    match id {
        "feishu" => Some(&FEISHU),
        "lark" => Some(&LARK),
        "google" => Some(&GOOGLE),
        _ => None,
    }
}

/// 预置配置 + 运行期凭据 + 覆盖项，合成实际要用的那一份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    pub id: String,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub scopes: String,
    pub subject_field: String,
    pub email_field: String,
    pub name_field: String,
    pub token_body_format: TokenBodyFormat,
    pub userinfo_envelope: bool,
    pub pkce: bool,
}

impl OAuthProviderConfig {
    pub fn resolve(&self, creds: &OAuthClientCredentials) -> ResolvedProvider {
        let or_preset = |over: Option<&String>, preset: &str| {
            over.map(String::as_str).unwrap_or(preset).to_string()
        };
        ResolvedProvider {
            id: self.id.to_string(),
            client_id: creds.client_id.clone(),
            client_secret: creds.client_secret.clone(),
            authorize_url: or_preset(creds.authorize_url.as_ref(), self.authorize_url),
            token_url: or_preset(creds.token_url.as_ref(), self.token_url),
            userinfo_url: or_preset(creds.userinfo_url.as_ref(), self.userinfo_url),
            scopes: or_preset(creds.scopes.as_ref(), self.scopes),
            subject_field: self.subject_field.to_string(),
            email_field: self.email_field.to_string(),
            name_field: self.name_field.to_string(),
            token_body_format: self.token_body_format,
            userinfo_envelope: self.userinfo_envelope,
            pkce: creds.pkce.unwrap_or(self.pkce),
        }
    }
}

impl ResolvedProvider {
    /// 把三个端点的「协议 + 主机 + 端口」换成 `base` 的，路径不动。
    ///
    /// 给测试里的 mock IdP 用；生产上没人调它。解析不了的 URL 原样保留，
    /// 免得把一个畸形配置悄悄变成另一个畸形配置。
    #[must_use]
    pub fn with_base_url(mut self, base: &str) -> Self {
        let Ok(base) = url::Url::parse(base) else {
            return self;
        };
        for target in [
            &mut self.authorize_url,
            &mut self.token_url,
            &mut self.userinfo_url,
        ] {
            *target = rebase(target, &base);
        }
        self
    }
}

fn rebase(target: &str, base: &url::Url) -> String {
    let Ok(mut parsed) = url::Url::parse(target) else {
        return target.to_string();
    };
    if parsed.set_scheme(base.scheme()).is_err() {
        return target.to_string();
    }
    if parsed.set_host(base.host_str()).is_err() {
        return target.to_string();
    }
    if parsed.set_port(base.port()).is_err() {
        return target.to_string();
    }
    parsed.to_string()
}

/// 从 [`ServerSettings`] 里取出「已配齐凭据且 id 合法」的提供方。
///
/// 两道闸缺一不可：id 不在预置表里 → `None`（挡路径注入）；
/// 凭据没配齐 → `None`（挡登录页上那个必然失败的按钮）。
pub fn resolve_provider(settings: &ServerSettings, id: &str) -> Option<ResolvedProvider> {
    let preset = provider_config(id)?;
    let creds = settings.providers.get(id)?;
    Some(preset.resolve(creds))
}

// ------------------------------------------------------------------- 错误

/// 第三方登录的失败原因。
///
/// 每条都是**固定文案**：它们会被直接渲染给用户，任何来自提供方或 URL 的
/// 字符串都不得进入这里（云端 handoff 那条路径就因为回显 `?error=`
/// 开过一个反射型 XSS）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OAuthError {
    #[error("登录方式不可用")]
    UnknownProvider,
    #[error("服务端未配置访问地址（public_base_url），第三方登录不可用")]
    MissingPublicBaseUrl,
    #[error("登录已过期，请重试")]
    InvalidState,
    #[error("第三方登录服务返回了异常响应")]
    ProviderResponse,
    #[error("第三方登录没有返回可用的用户标识")]
    MissingSubject,
}

// --------------------------------------------------------------- URL 拼装

/// 回调地址：`{public_base_url}/api/local-auth/oauth/{provider}/callback`。
///
/// - `public_base_url` 未配置 → `Err`。**不能**拿请求的 `Host` 头兜底：
///   那是攻击者可控的，等于把授权码定向到他的域名。
/// - 末尾斜杠会被规整掉，否则拼出 `https://k.example//api/...`。
/// - `provider` 必须先过 [`provider_config`]，这里只接受预置 id。
pub fn redirect_uri(public_base_url: Option<&str>, provider: &str) -> Result<String, OAuthError> {
    if provider_config(provider).is_none() {
        return Err(OAuthError::UnknownProvider);
    }
    let base = public_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or(OAuthError::MissingPublicBaseUrl)?;
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return Err(OAuthError::MissingPublicBaseUrl);
    }
    Ok(format!("{base}{OAUTH_PATH_PREFIX}/{provider}/callback"))
}

/// PKCE 的 `code_challenge`（S256）。
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 授权跳转地址。
///
/// 所有参数都经 `url` 的 query 序列化，`&`、`=`、空格等一律被百分号编码；
/// 手写 `format!` 拼 query 是这里唯一不能做的事。
pub fn build_authorize_url(
    provider: &ResolvedProvider,
    state: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<String, OAuthError> {
    let mut url =
        url::Url::parse(&provider.authorize_url).map_err(|_| OAuthError::UnknownProvider)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &provider.client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("state", state);
        if !provider.scopes.trim().is_empty() {
            query.append_pair("scope", provider.scopes.trim());
        }
        if let Some(verifier) = code_verifier {
            query.append_pair("code_challenge", &pkce_challenge(verifier));
            query.append_pair("code_challenge_method", "S256");
        }
    }
    Ok(url.to_string())
}

/// 换令牌的请求参数。**每次请求现算，不缓存、不落库、不进日志。**
pub fn token_request_params(
    provider: &ResolvedProvider,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Vec<(String, String)> {
    let mut params = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("client_id".to_string(), provider.client_id.clone()),
        ("client_secret".to_string(), provider.client_secret.clone()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
    ];
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier".to_string(), verifier.to_string()));
    }
    params
}

// ----------------------------------------------------------------- 响应解析

/// 从换令牌响应里取 `access_token`。
///
/// 三家的响应形状不同，但都是**扁平**的，所以一套逻辑够用，不需要
/// 「按提供方选解析器」这种会随对方改版而腐烂的分支：
/// - Google / 飞书 v3：标准 OAuth2，失败时带 `error`；
/// - Lark v2：扁平结构，`code` 与 `access_token` 同层，`code != 0` 即失败。
///
/// **这里刻意不读 `expires_in` / `refresh_token`**：access token 用完即弃，
/// 存下来只会多一处泄漏面，而本系统的会话有自己的 TTL。
pub fn parse_access_token(body: &Value) -> Result<String, OAuthError> {
    if let Some(code) = body.get("code") {
        let failed = match code {
            Value::Number(n) => n.as_i64() != Some(0),
            Value::String(s) => !matches!(s.trim(), "" | "0"),
            Value::Null => false,
            _ => true,
        };
        if failed {
            return Err(OAuthError::ProviderResponse);
        }
    }
    if body.get("error").is_some_and(|v| !v.is_null()) {
        return Err(OAuthError::ProviderResponse);
    }
    let token = body
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return Err(OAuthError::ProviderResponse);
    }
    Ok(token.to_string())
}

/// 从用户信息响应里抽出身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthIdentity {
    /// 绑定键。飞书/Lark 是 `union_id`，Google 是 `sub`。
    pub subject: String,
    /// 仅用于「撞号提示」与建号时填资料，**不是**绑定键。
    pub email: Option<String>,
    pub name: Option<String>,
}

/// 解析用户信息。
///
/// `userinfo_envelope = true` 时先剥 `{"code":0,"data":{...}}`——这一层
/// **只**存在于 `/open-apis/...` 这类接口，认证域的换令牌接口没有它。
pub fn extract_identity(
    provider: &ResolvedProvider,
    body: &Value,
) -> Result<OAuthIdentity, OAuthError> {
    let payload = if provider.userinfo_envelope {
        if let Some(code) = body.get("code").and_then(Value::as_i64)
            && code != 0
        {
            return Err(OAuthError::ProviderResponse);
        }
        body.get("data").ok_or(OAuthError::ProviderResponse)?
    } else {
        body
    };

    let subject = payload
        .get(&provider.subject_field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if subject.is_empty() {
        return Err(OAuthError::MissingSubject);
    }
    if subject.chars().count() > MAX_SUBJECT_LEN {
        return Err(OAuthError::MissingSubject);
    }

    let email = payload
        .get(&provider.email_field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_lowercase);
    let name = payload
        .get(&provider.name_field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().take(100).collect());

    Ok(OAuthIdentity {
        subject: subject.to_string(),
        email,
        name,
    })
}

// ---------------------------------------------------------------- state 存储

/// 一条待回调的 `state`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOAuthState {
    /// 发起时的提供方 id。回调必须来自同一个提供方。
    pub provider: String,
    /// 绑定到发起方浏览器的一次性 nonce 的 SHA-256。
    /// 存哈希而不是明文：内存转储里也拿不到能直接用的值。
    pub nonce_hash: String,
    pub code_verifier: Option<String>,
    /// 非 `None` 表示这是「已登录用户绑定第三方账号」，不是登录。
    pub bind_for_user_id: Option<Uuid>,
    pub expires_at: Instant,
}

impl PendingOAuthState {
    pub fn new(
        provider: &str,
        nonce: &str,
        code_verifier: Option<String>,
        bind_for_user_id: Option<Uuid>,
        now: Instant,
    ) -> Self {
        Self {
            provider: provider.to_string(),
            nonce_hash: hash_session_token(nonce),
            code_verifier,
            bind_for_user_id,
            expires_at: now + STATE_TTL,
        }
    }

    /// 常量时间比对 nonce。
    pub fn nonce_matches(&self, candidate: &str) -> bool {
        constant_time_eq(&self.nonce_hash, &hash_session_token(candidate))
    }
}

/// 进程内的 `state` 表。**刻意不落库**：进程一重启，所有半截的登录流程都作废。
///
/// 表的键是 `state` 的 SHA-256，不是 `state` 本身：这样查表全程不对秘密做
/// 字符串比较，内存里也没有一份能直接拿去回调的明文。
#[derive(Debug, Default)]
pub struct OAuthStateStore {
    entries: HashMap<String, PendingOAuthState>,
    /// 插入顺序，用于超额时淘汰最老的一条。
    order: VecDeque<String>,
}

impl OAuthStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, state: &str, entry: PendingOAuthState, now: Instant) {
        self.prune(now);
        while self.entries.len() >= MAX_PENDING_STATES {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.entries.remove(&oldest);
                }
                None => break,
            }
        }
        let key = hash_session_token(state);
        if self.entries.insert(key.clone(), entry).is_none() {
            self.order.push_back(key);
        }
    }

    /// 取出并**销毁**一条 `state`。
    ///
    /// 四道判据全部不满足才返回 `Some`：存在、未过期、提供方一致。
    /// 提供方不一致时**不销毁**——`state` 是秘密，不该让一次猜错的
    /// 回调把合法用户的登录流程打断。
    pub fn consume(
        &mut self,
        state: &str,
        provider: &str,
        now: Instant,
    ) -> Option<PendingOAuthState> {
        if state.is_empty() {
            return None;
        }
        let key = hash_session_token(state);
        let entry = self.entries.get(&key)?;
        if entry.expires_at <= now {
            self.entries.remove(&key);
            return None;
        }
        if entry.provider != provider {
            return None;
        }
        let entry = self.entries.remove(&key);
        self.order.retain(|k| k != &key);
        entry
    }

    /// 丢掉所有已过期的条目。
    pub fn prune(&mut self, now: Instant) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.retain(|_, entry| entry.expires_at > now);
        let live = &self.entries;
        self.order.retain(|key| live.contains_key(key));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn 凭据(client_id: &str) -> OAuthClientCredentials {
        OAuthClientCredentials {
            client_id: client_id.to_string(),
            client_secret: "secret".to_string(),
            ..OAuthClientCredentials::default()
        }
    }

    fn 设置(ids: &[&str]) -> ServerSettings {
        let mut providers = BTreeMap::new();
        for id in ids {
            providers.insert((*id).to_string(), 凭据("cli_x"));
        }
        ServerSettings {
            public_base_url: Some("https://k.example".to_string()),
            providers,
            ..ServerSettings::default()
        }
    }

    fn 解析(id: &str) -> ResolvedProvider {
        provider_config(id).unwrap().resolve(&凭据("cli_x"))
    }

    // ------------------------------------------------------- 预置配置

    #[test]
    fn 三个提供方都有预置配置() {
        for id in ["feishu", "lark", "google"] {
            let config = provider_config(id).unwrap_or_else(|| panic!("{id} 应有预置配置"));
            assert_eq!(config.id, id);
            for url in [config.authorize_url, config.token_url, config.userinfo_url] {
                assert!(
                    url.starts_with("https://"),
                    "{id} 的端点必须是 HTTPS：{url}"
                );
                assert!(
                    url::Url::parse(url).is_ok(),
                    "{id} 的端点不是合法 URL：{url}"
                );
            }
        }
    }

    /// 绑定键必须是稳定标识：飞书/Lark 的 `union_id`、Google 的 `sub`。
    /// **不得**是 `open_id`（应用级，换应用凭据就变）、`user_id`（会被复用）
    /// 或 email（官方明说不该当登录凭证）。
    #[test]
    fn 绑定键用的是稳定标识() {
        assert_eq!(FEISHU.subject_field, "union_id");
        assert_eq!(LARK.subject_field, "union_id");
        assert_eq!(GOOGLE.subject_field, "sub");
        for config in [FEISHU, LARK, GOOGLE] {
            assert_ne!(config.subject_field, "open_id");
            assert_ne!(config.subject_field, "user_id");
            assert_ne!(config.subject_field, config.email_field);
        }
    }

    /// 飞书的 v2 换令牌端点已被官方弃用。
    #[test]
    fn 飞书用_v3_换令牌端点() {
        assert_eq!(
            FEISHU.token_url,
            "https://accounts.feishu.cn/oauth/v3/token"
        );
        assert!(!FEISHU.token_url.contains("/v2/"));
    }

    #[test]
    fn 未知提供方返回_none() {
        for id in [
            "github",
            "../../etc",
            "..",
            "feishu/",
            "/feishu",
            "FEISHU",
            "Feishu",
            " feishu",
            "feishu ",
            "",
            "feishu%2f..",
            "\u{0}feishu",
        ] {
            assert!(provider_config(id).is_none(), "{id:?} 不应命中任何提供方");
        }
    }

    #[test]
    fn 未配置凭据的提供方解析不出来() {
        let settings = 设置(&["feishu"]);
        assert!(resolve_provider(&settings, "feishu").is_some());
        for id in ["google", "lark", "github", "../../etc"] {
            assert!(
                resolve_provider(&settings, id).is_none(),
                "{id} 不该解析出提供方"
            );
        }
    }

    #[test]
    fn 覆盖项生效且未覆盖的用预置值() {
        let creds = OAuthClientCredentials {
            client_id: "cli".to_string(),
            client_secret: "sec".to_string(),
            token_url: Some("https://mock.test/token".to_string()),
            scopes: Some("contact:user.base:readonly".to_string()),
            pkce: Some(true),
            ..OAuthClientCredentials::default()
        };
        let resolved = LARK.resolve(&creds);
        assert_eq!(resolved.token_url, "https://mock.test/token");
        assert_eq!(resolved.scopes, "contact:user.base:readonly");
        assert!(resolved.pkce, "覆盖项应能把 Lark 的 PKCE 打开");
        assert_eq!(resolved.authorize_url, LARK.authorize_url);
        assert_eq!(resolved.userinfo_url, LARK.userinfo_url);
    }

    #[test]
    fn with_base_url_只换协议主机端口() {
        let resolved = 解析("feishu").with_base_url("http://127.0.0.1:53421");
        assert_eq!(
            resolved.authorize_url,
            "http://127.0.0.1:53421/open-apis/authen/v1/authorize"
        );
        assert_eq!(resolved.token_url, "http://127.0.0.1:53421/oauth/v3/token");
        assert_eq!(
            resolved.userinfo_url,
            "http://127.0.0.1:53421/open-apis/authen/v1/user_info"
        );
    }

    // ------------------------------------------------------ redirect_uri

    #[test]
    fn redirect_uri_由_public_base_url_拼出() {
        assert_eq!(
            redirect_uri(Some("https://k.example"), "feishu").unwrap(),
            "https://k.example/api/local-auth/oauth/feishu/callback"
        );
    }

    #[test]
    fn redirect_uri_末尾斜杠被规整() {
        for base in [
            "https://k.example/",
            "https://k.example///",
            " https://k.example/ ",
        ] {
            assert_eq!(
                redirect_uri(Some(base), "google").unwrap(),
                "https://k.example/api/local-auth/oauth/google/callback",
                "base = {base:?}"
            );
        }
    }

    /// 没配 `public_base_url` 就必须报错。**不能**拿请求的 Host 头兜底：
    /// 那是攻击者可控的，等于把授权码定向到他的域名。
    #[test]
    fn 没配_public_base_url_时报错而不是拼出畸形_url() {
        for base in [None, Some(""), Some("   "), Some("/")] {
            assert_eq!(
                redirect_uri(base, "feishu"),
                Err(OAuthError::MissingPublicBaseUrl),
                "base = {base:?}"
            );
        }
    }

    #[test]
    fn redirect_uri_拒绝未知提供方() {
        assert_eq!(
            redirect_uri(Some("https://k.example"), "../../etc/passwd"),
            Err(OAuthError::UnknownProvider)
        );
    }

    // ------------------------------------------------------ authorize_url

    #[test]
    fn 授权_url_里的_state_与_redirect_uri_被正确编码() {
        let resolved = 解析("feishu");
        let url = build_authorize_url(
            &resolved,
            "a&b=c",
            "https://k.example/api/local-auth/oauth/feishu/callback",
            None,
        )
        .unwrap();
        assert!(url.contains("state=a%26b%3Dc"), "state 未被编码：{url}");
        assert!(
            url.contains(
                "redirect_uri=https%3A%2F%2Fk.example%2Fapi%2Flocal-auth%2Foauth%2Ffeishu%2Fcallback"
            ),
            "redirect_uri 未被编码：{url}"
        );
        assert!(url.contains("client_id=cli_x"));
        assert!(url.contains("response_type=code"));
        // 参数名是 client_id，不是后台显示的 App ID。
        assert!(!url.contains("app_id="), "参数名必须是 client_id：{url}");
    }

    #[test]
    fn 空_scope_不发_scope_参数而_google_发() {
        let feishu =
            build_authorize_url(&解析("feishu"), "s", "https://k.example/cb", None).unwrap();
        assert!(!feishu.contains("scope="), "飞书预置为空时不应发 scope");

        let google =
            build_authorize_url(&解析("google"), "s", "https://k.example/cb", None).unwrap();
        assert!(
            google.contains("scope=openid+email+profile"),
            "Google 的 scope 是必填：{google}"
        );
    }

    #[test]
    fn pkce_挑战是_s256_且不回显_verifier() {
        let url = build_authorize_url(
            &解析("google"),
            "s",
            "https://k.example/cb",
            Some("verifier-x"),
        )
        .unwrap();
        assert!(url.contains("code_challenge_method=S256"));
        assert!(
            !url.contains("verifier-x"),
            "verifier 绝不能出现在授权链接里：{url}"
        );
        // 已知向量：base64url(SHA-256("verifier-x"))
        assert_eq!(
            pkce_challenge("verifier-x"),
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier-x"))
        );
        assert_eq!(pkce_challenge("verifier-x").len(), 43);
    }

    #[test]
    fn 换令牌参数带凭据与授权码() {
        let params = token_request_params(
            &解析("feishu"),
            "the-code",
            "https://k.example/cb",
            Some("v"),
        );
        let map: BTreeMap<_, _> = params.into_iter().collect();
        assert_eq!(map["grant_type"], "authorization_code");
        assert_eq!(map["client_id"], "cli_x");
        assert_eq!(map["client_secret"], "secret");
        assert_eq!(map["code"], "the-code");
        assert_eq!(map["code_verifier"], "v");
        // 应用一律是机密客户端，PKCE 不能替代 client_secret。
        assert!(!map["client_secret"].is_empty());
    }

    // ---------------------------------------------------------- 响应解析

    #[test]
    fn 标准响应能取出_access_token() {
        assert_eq!(
            parse_access_token(&json!({"access_token": "at-1", "expires_in": 7200})).unwrap(),
            "at-1"
        );
    }

    /// 飞书/Lark 的认证接口是扁平的：`code` 与 `access_token` 同层。
    #[test]
    fn 扁平结构里的_code_为零时能取出令牌() {
        assert_eq!(
            parse_access_token(&json!({"code": 0, "access_token": "at-2", "expires_in": 7200}))
                .unwrap(),
            "at-2"
        );
    }

    #[test]
    fn 扁平结构里的_code_非零一律失败() {
        for body in [
            json!({"code": 99991400, "msg": "rate limited"}),
            json!({"code": 20001, "access_token": "at-3"}),
            json!({"code": "1", "access_token": "at-3"}),
        ] {
            assert_eq!(
                parse_access_token(&body),
                Err(OAuthError::ProviderResponse),
                "body = {body}"
            );
        }
    }

    #[test]
    fn 缺少_access_token_一律失败() {
        for body in [
            json!({}),
            json!({"code": 0}),
            json!({"access_token": ""}),
            json!({"access_token": "   "}),
            json!({"access_token": 123}),
            json!({"access_token": null}),
            json!({"error": "invalid_grant", "error_description": "bad code"}),
            json!({"error": "invalid_grant", "access_token": "at"}),
        ] {
            assert_eq!(
                parse_access_token(&body),
                Err(OAuthError::ProviderResponse),
                "body = {body}"
            );
        }
    }

    /// **不解析也不返回 `expires_in`**：令牌用完即弃，不存任何时长。
    #[test]
    fn 令牌解析不涉及任何硬编码时长() {
        let source = include_str!("oauth.rs");
        let 实现 = source
            .split("pub fn parse_access_token")
            .nth(1)
            .and_then(|s| s.split("\n/// ").next())
            .expect("必须有 parse_access_token");
        assert!(
            !实现.contains("expires_in"),
            "parse_access_token 不该读 expires_in"
        );
    }

    #[test]
    fn 带信封的用户信息能被解析() {
        let identity = extract_identity(
            &解析("feishu"),
            &json!({
                "code": 0,
                "msg": "success",
                "data": {
                    "union_id": "on_union_1",
                    "open_id": "ou_open_1",
                    "name": "爱丽丝",
                    "email": "Alice@Example.COM"
                }
            }),
        )
        .unwrap();
        assert_eq!(identity.subject, "on_union_1");
        assert_eq!(identity.email.as_deref(), Some("alice@example.com"));
        assert_eq!(identity.name.as_deref(), Some("爱丽丝"));
    }

    #[test]
    fn 不带信封的用户信息能被解析() {
        let identity = extract_identity(
            &解析("google"),
            &json!({"sub": "1122334455", "email": "bob@example.com", "name": "Bob"}),
        )
        .unwrap();
        assert_eq!(identity.subject, "1122334455");
    }

    /// 绑定键取的是 `union_id`，即使响应里同时有 `open_id`。
    #[test]
    fn 有_open_id_时也只取_union_id() {
        let identity = extract_identity(
            &解析("lark"),
            &json!({"code": 0, "data": {"union_id": "u1", "open_id": "o1"}}),
        )
        .unwrap();
        assert_eq!(identity.subject, "u1");
        assert_ne!(identity.subject, "o1");
    }

    #[test]
    fn 缺_subject_或_subject_为空时报错() {
        for body in [
            json!({"code": 0, "data": {}}),
            json!({"code": 0, "data": {"union_id": ""}}),
            json!({"code": 0, "data": {"union_id": "   "}}),
            json!({"code": 0, "data": {"union_id": null}}),
            json!({"code": 0, "data": {"union_id": 12345}}),
            json!({"code": 0, "data": {"open_id": "ou_1"}}),
        ] {
            assert_eq!(
                extract_identity(&解析("feishu"), &body),
                Err(OAuthError::MissingSubject),
                "body = {body}"
            );
        }
    }

    #[test]
    fn 超长_subject_被拒() {
        let long = "x".repeat(MAX_SUBJECT_LEN + 1);
        assert_eq!(
            extract_identity(&解析("google"), &json!({"sub": long})),
            Err(OAuthError::MissingSubject)
        );
        let ok = "x".repeat(MAX_SUBJECT_LEN);
        assert!(extract_identity(&解析("google"), &json!({"sub": ok})).is_ok());
    }

    #[test]
    fn 信封缺失或_code_非零时报错() {
        for body in [
            json!({"code": 0}),
            json!({"code": 99991663, "data": {"union_id": "u1"}}),
            json!({"union_id": "u1"}),
        ] {
            assert_eq!(
                extract_identity(&解析("feishu"), &body),
                Err(OAuthError::ProviderResponse),
                "body = {body}"
            );
        }
    }

    #[test]
    fn 缺_email_不影响取身份() {
        let identity =
            extract_identity(&解析("google"), &json!({"sub": "s1", "name": "Bob"})).unwrap();
        assert_eq!(identity.subject, "s1");
        assert!(identity.email.is_none());
    }

    // ---------------------------------------------------------- state 存储

    fn 待回调(provider: &str, nonce: &str, now: Instant) -> PendingOAuthState {
        PendingOAuthState::new(provider, nonce, None, None, now)
    }

    #[test]
    fn state_可以被消费一次() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        let entry = store.consume("st-1", "feishu", now).expect("第一次应命中");
        assert_eq!(entry.provider, "feishu");
        assert!(store.is_empty());
    }

    /// **重放**：同一个 `state` 第二次必须落空。
    #[test]
    fn state_是一次性的() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        assert!(store.consume("st-1", "feishu", now).is_some());
        for _ in 0..3 {
            assert!(
                store.consume("st-1", "feishu", now).is_none(),
                "重放必须被拒"
            );
        }
    }

    /// **伪造**：没发过的 `state` 一律落空。
    #[test]
    fn 伪造的_state_不命中() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        for forged in ["", "st-2", "st-1 ", " st-1", "ST-1", "st", "%", "\u{0}"] {
            assert!(
                store.consume(forged, "feishu", now).is_none(),
                "{forged:?} 不应命中"
            );
        }
        assert!(
            store.consume("st-1", "feishu", now).is_some(),
            "猜错不该把正确的 state 烧掉"
        );
    }

    #[test]
    fn state_十分钟后过期() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        assert_eq!(STATE_TTL, Duration::from_secs(600));
        assert!(
            store
                .consume("st-1", "feishu", now + STATE_TTL - Duration::from_secs(1))
                .is_some()
        );

        let mut store = OAuthStateStore::new();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        assert!(
            store.consume("st-1", "feishu", now + STATE_TTL).is_none(),
            "刚好到点就应过期"
        );
    }

    /// **跨提供方**：拿 feishu 的 `state` 去 google 的回调必须落空。
    #[test]
    fn state_与_provider_绑定() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        store.insert("st-1", 待回调("feishu", "n1", now), now);
        assert!(store.consume("st-1", "google", now).is_none());
        assert!(store.consume("st-1", "lark", now).is_none());
        assert!(
            store.consume("st-1", "feishu", now).is_some(),
            "换错提供方不该把 state 烧掉"
        );
    }

    /// **跨浏览器**：另一台机器拿到 `state` 也没有 nonce，比对必须失败。
    #[test]
    fn nonce_把_state_绑到发起方浏览器() {
        let now = Instant::now();
        let entry = 待回调("feishu", "nonce-a", now);
        assert!(entry.nonce_matches("nonce-a"));
        for wrong in ["", "nonce-b", "nonce-a ", " nonce-a", "NONCE-A", "nonce"] {
            assert!(!entry.nonce_matches(wrong), "{wrong:?} 不应匹配");
        }
    }

    /// nonce 只存哈希：内存里没有一份能直接拿去回调的明文。
    #[test]
    fn state_表里不存_nonce_明文() {
        let now = Instant::now();
        let entry = 待回调("feishu", "nonce-a", now);
        assert_ne!(entry.nonce_hash, "nonce-a");
        assert_eq!(entry.nonce_hash.len(), 64);
        assert_eq!(entry.nonce_hash, hash_session_token("nonce-a"));
    }

    #[test]
    fn 过期条目会被清掉() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        for i in 0..5 {
            store.insert(&format!("st-{i}"), 待回调("feishu", "n", now), now);
        }
        assert_eq!(store.len(), 5);
        store.prune(now + STATE_TTL);
        assert_eq!(store.len(), 0);
    }

    /// **内存耗尽**：`/start` 免鉴权，没有上限就是一条零成本的打法。
    #[test]
    fn state_表有容量上限且淘汰最老的() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        for i in 0..MAX_PENDING_STATES {
            store.insert(&format!("st-{i}"), 待回调("feishu", "n", now), now);
        }
        assert_eq!(store.len(), MAX_PENDING_STATES);

        store.insert("st-new", 待回调("feishu", "n", now), now);
        assert_eq!(store.len(), MAX_PENDING_STATES, "不得超过上限");
        assert!(
            store.consume("st-0", "feishu", now).is_none(),
            "最老的一条应已被淘汰"
        );
        assert!(store.consume("st-new", "feishu", now).is_some());
    }

    /// 消费之后插入顺序表也要跟着收缩，否则它会无限增长。
    #[test]
    fn 消费后顺序表同步收缩() {
        let mut store = OAuthStateStore::new();
        let now = Instant::now();
        for i in 0..100 {
            store.insert(&format!("st-{i}"), 待回调("feishu", "n", now), now);
            assert!(store.consume(&format!("st-{i}"), "feishu", now).is_some());
        }
        assert_eq!(store.len(), 0);
        assert_eq!(store.order.len(), 0, "顺序表必须同步收缩");
    }

    /// 错误文案是固定常量，不含任何来自提供方或 URL 的内容。
    #[test]
    fn 错误文案里没有可控内容() {
        for err in [
            OAuthError::UnknownProvider,
            OAuthError::MissingPublicBaseUrl,
            OAuthError::InvalidState,
            OAuthError::ProviderResponse,
            OAuthError::MissingSubject,
        ] {
            let message = err.to_string();
            assert!(!message.is_empty());
            for bad in ['<', '>', '"', '&', '\''] {
                assert!(!message.contains(bad), "{message} 含有 HTML 敏感字符 {bad}");
            }
        }
    }
}
