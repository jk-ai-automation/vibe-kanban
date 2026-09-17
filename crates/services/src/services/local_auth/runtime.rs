//! `LocalAuthRuntime`：进程内的本地认证状态。
//!
//! 名字刻意避开 `AuthContext`——那个名字已被云端 OAuth
//! （`services::services::auth::AuthContext`）占用。

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use super::{oauth::OAuthStateStore, rate_limit::LoginRateLimiter};
use crate::services::server_settings::{ServerMode, ServerSettings};

/// 首启初始化令牌的有效期：30 分钟。
///
/// 计时用 [`Instant`]（单调时钟）而不是墙钟：改系统时间不能把它续期。
pub const SETUP_TOKEN_TTL: Duration = Duration::from_secs(30 * 60);

/// 进程内的一次性初始化令牌。
///
/// **刻意不落库**：进程一重启就作废，重启时会重新打印一条新链接。
/// 这样「忘了关掉的服务器上还挂着一张能建管理员的票」不会长期存在。
#[derive(Debug, Clone)]
struct SetupToken {
    value: String,
    expires_at: Instant,
}

/// 本地认证运行时。`Deployment::local_auth()` 返回它的引用。
///
/// 目前只持有设置与本机令牌。登录限速器（C3）、OAuth state 存储（E）、
/// 首启一次性令牌（D）会在各自的任务里加进来——这里**不预留空壳字段**，
/// 因为未使用的私有字段会让 `clippy -D warnings` 直接失败。
#[derive(Clone)]
pub struct LocalAuthRuntime {
    settings: Arc<ServerSettings>,
    machine_token: Arc<str>,
    login_limiter: Arc<Mutex<LoginRateLimiter>>,
    setup_token: Arc<Mutex<Option<SetupToken>>>,
    oauth_states: Arc<Mutex<OAuthStateStore>>,
}

impl LocalAuthRuntime {
    /// `machine_token` 为空串表示「本机令牌尚未启用」，
    /// 此时 [`Self::machine_token_matches`] 对任何输入都返回 false。
    pub fn new(settings: ServerSettings, machine_token: String) -> Self {
        Self {
            settings: Arc::new(settings),
            machine_token: Arc::from(machine_token.as_str()),
            login_limiter: Arc::new(Mutex::new(LoginRateLimiter::new())),
            setup_token: Arc::new(Mutex::new(None)),
            oauth_states: Arc::new(Mutex::new(OAuthStateStore::new())),
        }
    }

    /// 第三方登录的 `state` 表。跨请求共享（`/start` 写、`/callback` 读），
    /// 所以必须挂在运行时上；**不要**在持锁期间 `await`。
    pub fn oauth_states(&self) -> MutexGuard<'_, OAuthStateStore> {
        self.oauth_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 成员实际访问的地址。第三方登录的 `redirect_uri` 从它拼出来——
    /// **不能**用请求的 `Host` 头，那是攻击者可控的。
    pub fn public_base_url(&self) -> Option<&str> {
        self.settings.public_base_url.as_deref()
    }

    /// 登录限速器。跨请求共享，**不要**在持锁期间 `await`。
    ///
    /// 锁中毒（某个线程在持锁时 panic）时取回内部值继续用：限速状态本身
    /// 不是一致性敏感的数据，而「锁中毒就 panic」会让整台服务器登不上去。
    pub fn login_limiter(&self) -> MutexGuard<'_, LoginRateLimiter> {
        self.login_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 要不要采信 `X-Forwarded-For`。默认 false，见 `rate_limit::client_ip`。
    pub fn trust_proxy(&self) -> bool {
        self.settings.trust_proxy
    }

    pub fn mode(&self) -> ServerMode {
        self.settings.mode
    }

    pub fn settings(&self) -> &ServerSettings {
        &self.settings
    }

    pub fn session_ttl_days(&self) -> u32 {
        self.settings.session_ttl_days
    }

    pub fn allow_oauth_signup(&self) -> bool {
        self.settings.allow_oauth_signup
    }

    /// Cookie 要不要带 `Secure`。
    ///
    /// 判据是「成员实际访问的地址是不是 HTTPS」，也就是 `public_base_url` 的协议。
    /// 不能看请求本身：反代之后到达本进程的永远是明文 HTTP。
    /// 没配 `public_base_url` 时按明文处理——给明文站点发 `Secure` Cookie
    /// 会让浏览器直接丢弃它，表现为「登录成功但立刻又退回登录页」。
    pub fn secure_cookies(&self) -> bool {
        self.settings
            .public_base_url
            .as_deref()
            .map(|url| url.trim().to_ascii_lowercase().starts_with("https://"))
            .unwrap_or(false)
    }

    /// 已配齐凭据的 OAuth 提供方 id，按字典序。未配置的不会出现在登录页。
    pub fn available_providers(&self) -> Vec<String> {
        self.settings.providers.keys().cloned().collect()
    }

    pub fn machine_token(&self) -> &str {
        &self.machine_token
    }

    /// 常量时间比较本机令牌。
    ///
    /// 空的运行时令牌（本机令牌尚未启用）或空的请求令牌一律不匹配——
    /// 否则「两边都没配」就会变成「谁都能免会话访问」。
    pub fn machine_token_matches(&self, candidate: &str) -> bool {
        constant_time_eq(&self.machine_token, candidate)
    }

    // ------------------------------------------------- 首启初始化令牌

    fn setup_slot(&self) -> MutexGuard<'_, Option<SetupToken>> {
        self.setup_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 生成一张新的初始化令牌，覆盖旧的（旧的当场失效）。返回明文。
    ///
    /// 明文**只**在这里和启动日志里出现一次；不落库、不进错误消息。
    pub fn issue_setup_token(&self, value: String, now: Instant) -> String {
        let mut slot = self.setup_slot();
        *slot = Some(SetupToken {
            value: value.clone(),
            expires_at: now + SETUP_TOKEN_TTL,
        });
        value
    }

    /// 校验但**不消费**。供 `GET /api/local-auth/setup` 判断要不要渲染向导。
    pub fn setup_token_valid(&self, candidate: &str, now: Instant) -> bool {
        let slot = self.setup_slot();
        match slot.as_ref() {
            Some(token) if token.expires_at > now => constant_time_eq(&token.value, candidate),
            _ => false,
        }
    }

    /// 校验并**消费**。返回 true 表示这次用掉了令牌，之后再也验不过。
    ///
    /// 校验失败不会清掉令牌：否则任何人乱打一次就能把管理员手里那张链接作废。
    pub fn consume_setup_token(&self, candidate: &str, now: Instant) -> bool {
        let mut slot = self.setup_slot();
        let 命中 = match slot.as_ref() {
            Some(token) if token.expires_at > now => constant_time_eq(&token.value, candidate),
            _ => false,
        };
        if 命中 {
            *slot = None;
        }
        命中
    }

    /// 丢弃当前令牌（初始化完成后调用，双保险）。
    pub fn clear_setup_token(&self) {
        *self.setup_slot() = None;
    }
}

/// 常量时间字符串比较。
///
/// 两侧任一为空一律不匹配：「都没配」绝不能变成「谁都能过」。
/// 长度不同时提前返回——长度本来就能从别处观察到，不是秘密。
/// 全仓库的令牌比对都必须走这里，**不得**用 `==`：`==` 在第一个不同字节
/// 就短路返回，攻击者可以按字节逐位试出正确令牌。
pub fn constant_time_eq(expected: &str, candidate: &str) -> bool {
    use subtle::ConstantTimeEq;

    if expected.is_empty() || candidate.is_empty() {
        return false;
    }
    let expected = expected.as_bytes();
    let candidate = candidate.as_bytes();
    if expected.len() != candidate.len() {
        return false;
    }
    expected.ct_eq(candidate).into()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::services::server_settings::OAuthClientCredentials;

    fn 设置(mode: ServerMode, base_url: Option<&str>) -> ServerSettings {
        ServerSettings {
            mode,
            public_base_url: base_url.map(str::to_string),
            ..ServerSettings::default()
        }
    }

    #[test]
    fn 个人模式下运行时报告_personal() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Personal, None), String::new());
        assert_eq!(rt.mode(), ServerMode::Personal);
        assert_eq!(rt.session_ttl_days(), 30);
        assert!(!rt.allow_oauth_signup());
    }

    #[test]
    fn secure_cookies_由_public_base_url_的协议决定() {
        for (url, expected) in [
            (Some("https://kanban.example.com"), true),
            (Some("  HTTPS://Kanban.Example.Com  "), true),
            (Some("http://192.168.1.10:8080"), false),
            (Some("://nonsense"), false),
            (Some("httpsfoo://x"), false),
            (None, false),
        ] {
            let rt = LocalAuthRuntime::new(设置(ServerMode::Team, url), String::new());
            assert_eq!(rt.secure_cookies(), expected, "url = {url:?}");
        }
    }

    #[test]
    fn providers_为空时返回空数组() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        assert!(rt.available_providers().is_empty());
    }

    #[test]
    fn providers_按字典序返回() {
        let mut providers = BTreeMap::new();
        for id in ["lark", "feishu", "google"] {
            providers.insert(
                id.to_string(),
                OAuthClientCredentials {
                    client_id: "id".to_string(),
                    client_secret: "secret".to_string(),
                    ..OAuthClientCredentials::default()
                },
            );
        }
        let rt = LocalAuthRuntime::new(
            ServerSettings {
                providers,
                ..设置(ServerMode::Team, None)
            },
            String::new(),
        );
        assert_eq!(rt.available_providers(), ["feishu", "google", "lark"]);
    }

    /// 本机令牌未启用（空串）时，任何请求头都不能被当成本机身份，
    /// 尤其是「请求头也为空」这种最容易写错的情况。
    #[test]
    fn 空的本机令牌永不匹配() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        for candidate in ["", " ", "anything", "\0"] {
            assert!(!rt.machine_token_matches(candidate), "{candidate:?}");
        }
    }

    #[test]
    fn 本机令牌精确匹配() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), "s3cret-token".to_string());
        assert!(rt.machine_token_matches("s3cret-token"));
        for wrong in [
            "",
            "s3cret",
            "s3cret-token ",
            " s3cret-token",
            "S3CRET-TOKEN",
            "s3cret-tokeN",
            "s3cret-token\0",
        ] {
            assert!(!rt.machine_token_matches(wrong), "{wrong:?} 不应匹配");
        }
    }

    // --------------------------------------------- 首启初始化令牌

    #[test]
    fn 没发过令牌时任何输入都不匹配() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        for candidate in ["", " ", "anything", "\0"] {
            assert!(!rt.setup_token_valid(candidate, now), "{candidate:?}");
            assert!(!rt.consume_setup_token(candidate, now), "{candidate:?}");
        }
    }

    #[test]
    fn 令牌发出后可校验但空串与错值不行() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        assert!(rt.setup_token_valid(&token, now));
        for wrong in [
            "",
            " ",
            "s3tup",
            "s3tup-token ",
            " s3tup-token",
            "S3TUP-TOKEN",
            "s3tup-tokeN",
            "s3tup-token\0",
        ] {
            assert!(!rt.setup_token_valid(wrong, now), "{wrong:?} 不应匹配");
        }
    }

    /// 校验不消费：`GET /setup` 可以被前端反复调用，不会把令牌用掉。
    #[test]
    fn 校验不消费令牌() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        for _ in 0..5 {
            assert!(rt.setup_token_valid(&token, now));
        }
        assert!(rt.consume_setup_token(&token, now), "校验过后仍应可消费");
    }

    /// **一次性**：用过之后立刻失效，哪怕 30 分钟还没到。
    #[test]
    fn 令牌用后立即失效() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        assert!(rt.consume_setup_token(&token, now));
        assert!(!rt.consume_setup_token(&token, now), "第二次必须失败");
        assert!(!rt.setup_token_valid(&token, now), "用过之后校验也不过");
    }

    #[test]
    fn 令牌三十分钟后过期() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        assert_eq!(SETUP_TOKEN_TTL, Duration::from_secs(30 * 60));
        assert!(rt.setup_token_valid(&token, now + SETUP_TOKEN_TTL - Duration::from_secs(1)));
        assert!(
            !rt.setup_token_valid(&token, now + SETUP_TOKEN_TTL),
            "刚好到点就应过期"
        );
        assert!(!rt.consume_setup_token(&token, now + SETUP_TOKEN_TTL + Duration::from_secs(1)));
    }

    /// 猜错不能把管理员手里那张链接作废——否则是一条零成本的拒绝服务。
    #[test]
    fn 猜错令牌不会作废正确的令牌() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        for _ in 0..20 {
            assert!(!rt.consume_setup_token("guess", now));
        }
        assert!(rt.consume_setup_token(&token, now), "正确令牌必须还能用");
    }

    #[test]
    fn 重新发令牌会让旧令牌失效() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let 旧 = rt.issue_setup_token("old-token".to_string(), now);
        let 新 = rt.issue_setup_token("new-token".to_string(), now);
        assert!(!rt.setup_token_valid(&旧, now), "旧令牌必须当场失效");
        assert!(rt.setup_token_valid(&新, now));
    }

    #[test]
    fn 可以主动丢弃令牌() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        rt.clear_setup_token();
        assert!(!rt.setup_token_valid(&token, now));
    }

    /// 克隆出来的运行时共享同一张令牌（axum 每个请求都拿到一份 clone）——
    /// 否则「A 请求消费掉」在 B 请求里看不见，一次性就形同虚设。
    #[test]
    fn 克隆的运行时共享同一张令牌() {
        let rt = LocalAuthRuntime::new(设置(ServerMode::Team, None), String::new());
        let now = Instant::now();
        let token = rt.issue_setup_token("s3tup-token".to_string(), now);
        let 副本 = rt.clone();
        assert!(副本.setup_token_valid(&token, now));
        assert!(副本.consume_setup_token(&token, now));
        assert!(
            !rt.setup_token_valid(&token, now),
            "另一份 clone 里也必须已失效"
        );
    }

    /// **令牌比对必须是常量时间。** 全部走 `constant_time_eq`（内部用
    /// `subtle::ConstantTimeEq`），源码里不得出现对令牌值的 `==` 比较。
    #[test]
    fn 令牌比对走常量时间实现() {
        let source = include_str!("runtime.rs");
        let 实现 = source
            .split("fn constant_time_eq")
            .nth(1)
            .expect("必须有 constant_time_eq");
        assert!(
            实现.contains("ConstantTimeEq") && 实现.contains("ct_eq"),
            "constant_time_eq 必须用 subtle::ConstantTimeEq"
        );
        // 三处比对都要经过它，一处都不能用 == 短路比较。
        for 调用点 in [
            "machine_token_matches",
            "setup_token_valid",
            "consume_setup_token",
        ] {
            let 段 = source
                .split(&format!("fn {调用点}"))
                .nth(1)
                .unwrap_or_else(|| panic!("找不到 {调用点}"));
            let 函数体 = 段.split("\n    pub fn ").next().unwrap_or(段);
            assert!(
                函数体.contains("constant_time_eq("),
                "{调用点} 必须走 constant_time_eq"
            );
            assert!(
                !函数体.contains("== candidate") && !函数体.contains("candidate =="),
                "{调用点} 不得用 == 比对令牌"
            );
        }
    }
}
