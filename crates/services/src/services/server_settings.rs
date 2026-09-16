//! 服务端运行模式与 `asset_dir()/server.json` 设置。
//!
//! 解析全部做成纯函数：环境变量通过闭包注入，实现里**不直接调
//! `std::env::var`**，否则 Rust 默认并行跑测试时用例之间会互相污染。

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 服务端运行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerMode {
    /// 个人版：免登录，中间件注入本机用户，行为与历史版本一致。
    Personal,
    /// 团队版：`/api/*` 强制会话。
    Team,
}

impl ServerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerMode::Personal => "personal",
            ServerMode::Team => "team",
        }
    }
}

/// 本期支持的 OAuth 提供方 id。未列入的 key 一律忽略，
/// 避免 server.json 里写错名字却以为已生效。
pub const KNOWN_OAUTH_PROVIDERS: [&str; 3] = ["feishu", "lark", "google"];

/// 默认会话有效期（天）。
pub const DEFAULT_SESSION_TTL_DAYS: u32 = 30;

/// 会话有效期上限（天）。挡住「写个超大数当成永不过期」。
pub const MAX_SESSION_TTL_DAYS: u32 = 3650;

/// `server.json` 里的 OAuth 凭据。两项都可能缺失，缺一即视为未配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCredentialsFile {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// `asset_dir()/server.json` 的原始形态。全部字段可选，缺失即用默认值。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSettingsFile {
    pub mode: Option<String>,
    pub allow_oauth_signup: Option<bool>,
    pub session_ttl_days: Option<i64>,
    pub sqlite_wal: Option<bool>,
    pub trust_proxy: Option<bool>,
    pub public_base_url: Option<String>,
    pub oauth: Option<BTreeMap<String, OAuthCredentialsFile>>,
}

/// 一个提供方的完整凭据。只有 id 与 secret 都非空才会构造出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

/// 环境变量与配置文件合并后的最终设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    pub mode: ServerMode,
    pub allow_oauth_signup: bool,
    pub session_ttl_days: u32,
    pub sqlite_wal: bool,
    pub trust_proxy: bool,
    pub public_base_url: Option<String>,
    pub providers: BTreeMap<String, OAuthClientCredentials>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            mode: ServerMode::Personal,
            allow_oauth_signup: false,
            session_ttl_days: DEFAULT_SESSION_TTL_DAYS,
            sqlite_wal: true,
            trust_proxy: false,
            public_base_url: None,
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServerSettingsError {
    #[error("不支持的运行模式 {0:?}（合法值：personal / team）")]
    InvalidMode(String),
    #[error("不支持的会话有效期 {0:?}（需要 1..={MAX_SESSION_TTL_DAYS} 之间的整数天数）")]
    InvalidSessionTtl(String),
    #[error("读取配置文件 {path} 失败：{message}")]
    ReadFile { path: String, message: String },
    #[error("解析配置文件 {path} 失败：{message}")]
    ParseFile { path: String, message: String },
}

/// 读取 `server.json`。
///
/// - 文件不存在 → `Ok(None)`（个人版开箱即用，不需要这个文件）。
/// - 文件存在但读不动 / JSON 非法 → `Err`，由调用方让启动失败。
///   **不做静默降级**：把配置改坏却退回 personal（免登录）属于 fail-open。
pub async fn read_server_settings_file(
    path: &Path,
) -> Result<Option<ServerSettingsFile>, ServerSettingsError> {
    let raw = match tokio::fs::read(path).await {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(ServerSettingsError::ReadFile {
                path: path.display().to_string(),
                message: err.to_string(),
            });
        }
    };
    serde_json::from_slice::<ServerSettingsFile>(&raw)
        .map(Some)
        .map_err(|err| ServerSettingsError::ParseFile {
            path: path.display().to_string(),
            message: err.to_string(),
        })
}

/// 合并配置文件与环境变量，环境变量优先。
///
/// `env` 是注入的查找闭包（生产传 `&|k| std::env::var(k).ok()`），
/// 这样测试不必改进程环境，也就不会在并行跑用例时互相污染。
pub fn load_server_settings(
    file: Option<ServerSettingsFile>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<ServerSettings, ServerSettingsError> {
    let file = file.unwrap_or_default();

    let mode = match env_str(env, "VK_MODE").or_else(|| non_blank(file.mode.as_deref())) {
        Some(raw) => parse_mode(&raw)?,
        None => ServerMode::Personal,
    };

    let session_ttl_days = match env_str(env, "VK_SESSION_TTL_DAYS") {
        Some(raw) => parse_ttl_days(&raw)?,
        None => match file.session_ttl_days {
            Some(days) => parse_ttl_days(&days.to_string())?,
            None => DEFAULT_SESSION_TTL_DAYS,
        },
    };

    let allow_oauth_signup = flag(
        env_str(env, "VK_ALLOW_OAUTH_SIGNUP"),
        file.allow_oauth_signup,
        false,
    );
    let sqlite_wal = flag(env_str(env, "VK_SQLITE_WAL"), file.sqlite_wal, true);
    let trust_proxy = flag(env_str(env, "VK_TRUST_PROXY"), file.trust_proxy, false);

    let public_base_url =
        env_str(env, "VK_PUBLIC_BASE_URL").or_else(|| non_blank(file.public_base_url.as_deref()));

    let file_oauth = file.oauth.unwrap_or_default();
    let mut providers = BTreeMap::new();
    for provider in KNOWN_OAUTH_PROVIDERS {
        let from_file = file_oauth.get(provider);
        let upper = provider.to_ascii_uppercase();
        let client_id = env_str(env, &format!("VK_OAUTH_{upper}_CLIENT_ID"))
            .or_else(|| non_blank(from_file.and_then(|c| c.client_id.as_deref())));
        let client_secret = env_str(env, &format!("VK_OAUTH_{upper}_CLIENT_SECRET"))
            .or_else(|| non_blank(from_file.and_then(|c| c.client_secret.as_deref())));
        // 缺一不可：半配置的提供方会在登录页露出一个必然失败的按钮。
        if let (Some(client_id), Some(client_secret)) = (client_id, client_secret) {
            providers.insert(
                provider.to_string(),
                OAuthClientCredentials {
                    client_id,
                    client_secret,
                },
            );
        }
    }

    Ok(ServerSettings {
        mode,
        allow_oauth_signup,
        session_ttl_days,
        sqlite_wal,
        trust_proxy,
        public_base_url,
        providers,
    })
}

fn parse_mode(raw: &str) -> Result<ServerMode, ServerSettingsError> {
    let trimmed = raw.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "personal" => Ok(ServerMode::Personal),
        "team" => Ok(ServerMode::Team),
        _ => Err(ServerSettingsError::InvalidMode(trimmed.to_string())),
    }
}

fn parse_ttl_days(raw: &str) -> Result<u32, ServerSettingsError> {
    let trimmed = raw.trim();
    let invalid = || ServerSettingsError::InvalidSessionTtl(trimmed.to_string());
    let days: u32 = trimmed.parse().map_err(|_| invalid())?;
    if days == 0 || days > MAX_SESSION_TTL_DAYS {
        return Err(invalid());
    }
    Ok(days)
}

/// 环境变量取值：去两侧空白，空串按「未设置」处理。
fn env_str(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    env(key).and_then(|v| non_blank(Some(v.as_str())))
}

fn non_blank(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// 开关项：环境变量 > 配置文件 > 默认值。
///
/// 环境变量只认 `1`/`true`（不区分大小写）为真、`0`/`false` 为假；
/// 其余写法一律回落到 `default`，这样拼错不会意外打开一个防护开关。
fn flag(env_value: Option<String>, file_value: Option<bool>, default: bool) -> bool {
    if let Some(raw) = env_value {
        return match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" => true,
            "0" | "false" => false,
            _ => default,
        };
    }
    file_value.unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// 构造一个「环境变量查找闭包」。
    fn env_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn load(
        file: Option<ServerSettingsFile>,
        pairs: &[(&str, &str)],
    ) -> Result<ServerSettings, ServerSettingsError> {
        let map = env_of(pairs);
        load_server_settings(file, &|k| map.get(k).cloned())
    }

    #[test]
    fn 模式默认是个人版() {
        let settings = load(None, &[]).expect("默认设置应合法");
        assert_eq!(settings.mode, ServerMode::Personal);
        assert!(!settings.allow_oauth_signup);
        assert_eq!(settings.session_ttl_days, 30);
        assert!(settings.sqlite_wal);
        assert!(!settings.trust_proxy);
        assert!(settings.public_base_url.is_none());
        assert!(settings.providers.is_empty());
    }

    #[test]
    fn 环境变量覆盖配置文件() {
        let file = ServerSettingsFile {
            mode: Some("personal".to_string()),
            ..Default::default()
        };
        let settings = load(Some(file), &[("VK_MODE", "team")]).expect("应合法");
        assert_eq!(settings.mode, ServerMode::Team);
    }

    #[test]
    fn 配置文件在没有环境变量时生效() {
        let file = ServerSettingsFile {
            mode: Some("team".to_string()),
            session_ttl_days: Some(7),
            ..Default::default()
        };
        let settings = load(Some(file), &[]).expect("应合法");
        assert_eq!(settings.mode, ServerMode::Team);
        assert_eq!(settings.session_ttl_days, 7);
    }

    #[test]
    fn 非法模式值报错() {
        let err = load(None, &[("VK_MODE", "teams")]).expect_err("应报错");
        assert_eq!(err, ServerSettingsError::InvalidMode("teams".to_string()));
    }

    #[test]
    fn 配置文件里的非法模式值同样报错而不是静默退回个人版() {
        // fail-open 检查：改坏 server.json 必须让启动失败，不能悄悄降级成免登录。
        let file = ServerSettingsFile {
            mode: Some("Teams".to_string()),
            ..Default::default()
        };
        let err = load(Some(file), &[]).expect_err("应报错");
        assert_eq!(err, ServerSettingsError::InvalidMode("Teams".to_string()));
    }

    #[test]
    fn 模式大小写与空格不敏感() {
        assert_eq!(
            load(None, &[("VK_MODE", " TEAM ")]).unwrap().mode,
            ServerMode::Team
        );
        assert_eq!(
            load(None, &[("VK_MODE", "Personal")]).unwrap().mode,
            ServerMode::Personal
        );
    }

    #[test]
    fn 空的模式环境变量按未设置处理() {
        let file = ServerSettingsFile {
            mode: Some("team".to_string()),
            ..Default::default()
        };
        // 有些启动脚本会写 VK_MODE=，空串不应把 team 顶掉也不应报错。
        assert_eq!(
            load(Some(file), &[("VK_MODE", "   ")]).unwrap().mode,
            ServerMode::Team
        );
    }

    #[test]
    fn oauth_凭据缺一不可() {
        let settings = load(None, &[("VK_OAUTH_FEISHU_CLIENT_ID", "cli_x")]).unwrap();
        assert!(
            settings.providers.is_empty(),
            "只配了 client_id 的提供方不得进入半配置状态：{:?}",
            settings.providers
        );

        let settings = load(
            None,
            &[
                ("VK_OAUTH_FEISHU_CLIENT_ID", "cli_x"),
                ("VK_OAUTH_FEISHU_CLIENT_SECRET", "   "),
            ],
        )
        .unwrap();
        assert!(settings.providers.is_empty(), "空白 secret 等同于未配置");

        let settings = load(
            None,
            &[
                ("VK_OAUTH_FEISHU_CLIENT_ID", "cli_x"),
                ("VK_OAUTH_FEISHU_CLIENT_SECRET", "sec_x"),
            ],
        )
        .unwrap();
        assert_eq!(settings.providers.len(), 1);
        let creds = settings.providers.get("feishu").expect("feishu 应已配置");
        assert_eq!(creds.client_id, "cli_x");
        assert_eq!(creds.client_secret, "sec_x");
    }

    #[test]
    fn oauth_环境变量覆盖配置文件的凭据() {
        let mut oauth = BTreeMap::new();
        oauth.insert(
            "google".to_string(),
            OAuthCredentialsFile {
                client_id: Some("from-file".to_string()),
                client_secret: Some("file-secret".to_string()),
            },
        );
        let file = ServerSettingsFile {
            oauth: Some(oauth),
            ..Default::default()
        };
        let settings = load(Some(file), &[("VK_OAUTH_GOOGLE_CLIENT_ID", "from-env")]).unwrap();
        let creds = settings.providers.get("google").expect("google 应已配置");
        assert_eq!(creds.client_id, "from-env");
        assert_eq!(creds.client_secret, "file-secret");
    }

    #[test]
    fn 未知提供方不会被读进来() {
        let mut oauth = BTreeMap::new();
        oauth.insert(
            "github".to_string(),
            OAuthCredentialsFile {
                client_id: Some("a".to_string()),
                client_secret: Some("b".to_string()),
            },
        );
        let file = ServerSettingsFile {
            oauth: Some(oauth),
            ..Default::default()
        };
        assert!(load(Some(file), &[]).unwrap().providers.is_empty());
    }

    #[test]
    fn session_ttl_days_非法值报错() {
        assert_eq!(
            load(None, &[("VK_SESSION_TTL_DAYS", "0")]).expect_err("应报错"),
            ServerSettingsError::InvalidSessionTtl("0".to_string())
        );
        assert_eq!(
            load(None, &[("VK_SESSION_TTL_DAYS", "abc")]).expect_err("应报错"),
            ServerSettingsError::InvalidSessionTtl("abc".to_string())
        );
        assert_eq!(
            load(None, &[("VK_SESSION_TTL_DAYS", "-1")]).expect_err("应报错"),
            ServerSettingsError::InvalidSessionTtl("-1".to_string())
        );
        // 上限：避免「永不过期」的会话。
        assert_eq!(
            load(None, &[("VK_SESSION_TTL_DAYS", "3651")]).expect_err("应报错"),
            ServerSettingsError::InvalidSessionTtl("3651".to_string())
        );
        assert_eq!(
            load(None, &[("VK_SESSION_TTL_DAYS", " 14 ")])
                .unwrap()
                .session_ttl_days,
            14
        );
    }

    #[test]
    fn 配置文件里的非法_ttl_同样报错() {
        let file = ServerSettingsFile {
            session_ttl_days: Some(0),
            ..Default::default()
        };
        assert_eq!(
            load(Some(file), &[]).expect_err("应报错"),
            ServerSettingsError::InvalidSessionTtl("0".to_string())
        );
    }

    #[test]
    fn trust_proxy_只认_1_和_true() {
        for raw in ["0", "false", "yes", "on", "", "  "] {
            assert!(
                !load(None, &[("VK_TRUST_PROXY", raw)]).unwrap().trust_proxy,
                "{raw:?} 不应被当成 true"
            );
        }
        for raw in ["1", "true", "TRUE", " True "] {
            assert!(
                load(None, &[("VK_TRUST_PROXY", raw)]).unwrap().trust_proxy,
                "{raw:?} 应被当成 true"
            );
        }
    }

    #[test]
    fn sqlite_wal_默认开启且可以用_0_关掉() {
        assert!(load(None, &[]).unwrap().sqlite_wal);
        assert!(!load(None, &[("VK_SQLITE_WAL", "0")]).unwrap().sqlite_wal);
        assert!(
            !load(None, &[("VK_SQLITE_WAL", "false")])
                .unwrap()
                .sqlite_wal
        );
        assert!(load(None, &[("VK_SQLITE_WAL", "1")]).unwrap().sqlite_wal);
    }

    #[test]
    fn public_base_url_去空白且空串视为未配置() {
        assert_eq!(
            load(None, &[("VK_PUBLIC_BASE_URL", " https://k.lan ")])
                .unwrap()
                .public_base_url
                .as_deref(),
            Some("https://k.lan")
        );
        assert!(
            load(None, &[("VK_PUBLIC_BASE_URL", "  ")])
                .unwrap()
                .public_base_url
                .is_none()
        );
    }

    #[tokio::test]
    async fn 配置文件不存在时返回_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        assert!(read_server_settings_file(&path).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn 配置文件非法_json_时报错而不是静默降级() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        tokio::fs::write(&path, b"{ mode: team ").await.unwrap();
        let err = read_server_settings_file(&path)
            .await
            .expect_err("非法 JSON 必须让启动失败");
        assert!(matches!(err, ServerSettingsError::ParseFile { .. }));
    }

    #[tokio::test]
    async fn 配置文件可以只写一部分字段() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        tokio::fs::write(&path, br#"{"mode":"team"}"#)
            .await
            .unwrap();
        let file = read_server_settings_file(&path)
            .await
            .unwrap()
            .expect("文件存在时应返回 Some");
        assert_eq!(file.mode.as_deref(), Some("team"));
        assert!(file.session_ttl_days.is_none());
    }
}
