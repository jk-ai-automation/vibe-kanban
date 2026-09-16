//! `LocalAuthRuntime`：进程内的本地认证状态。
//!
//! 名字刻意避开 `AuthContext`——那个名字已被云端 OAuth
//! （`services::services::auth::AuthContext`）占用。

use std::sync::Arc;

use crate::services::server_settings::{ServerMode, ServerSettings};

/// 本地认证运行时。`Deployment::local_auth()` 返回它的引用。
///
/// 目前只持有设置与本机令牌。登录限速器（C3）、OAuth state 存储（E）、
/// 首启一次性令牌（D）会在各自的任务里加进来——这里**不预留空壳字段**，
/// 因为未使用的私有字段会让 `clippy -D warnings` 直接失败。
#[derive(Clone)]
pub struct LocalAuthRuntime {
    settings: Arc<ServerSettings>,
    machine_token: Arc<str>,
}

impl LocalAuthRuntime {
    /// `machine_token` 为空串表示「本机令牌尚未启用」，
    /// 此时 [`Self::machine_token_matches`] 对任何输入都返回 false。
    pub fn new(settings: ServerSettings, machine_token: String) -> Self {
        Self {
            settings: Arc::new(settings),
            machine_token: Arc::from(machine_token.as_str()),
        }
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
        use subtle::ConstantTimeEq;

        if self.machine_token.is_empty() || candidate.is_empty() {
            return false;
        }
        let expected = self.machine_token.as_bytes();
        let got = candidate.as_bytes();
        if expected.len() != got.len() {
            return false;
        }
        expected.ct_eq(got).into()
    }
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
}
