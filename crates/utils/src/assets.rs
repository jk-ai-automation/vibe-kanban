use directories::ProjectDirs;
use rust_embed::RustEmbed;

const PROJECT_ROOT: &str = env!("CARGO_MANIFEST_DIR");

pub fn asset_dir() -> std::path::PathBuf {
    let path = if cfg!(debug_assertions) {
        std::path::PathBuf::from(PROJECT_ROOT).join("../../dev_assets")
    } else {
        prod_asset_dir_path()
    };

    // Ensure the directory exists
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("Failed to create asset directory");
    }

    path
    // ✔ macOS → ~/Library/Application Support/MyApp
    // ✔ Linux → ~/.local/share/myapp   (respects XDG_DATA_HOME)
    // ✔ Windows → %APPDATA%\Example\MyApp
}

pub fn prod_asset_dir_path() -> std::path::PathBuf {
    ProjectDirs::from("ai", "bloop", "vibe-kanban")
        .expect("OS didn't give us a home directory")
        .data_dir()
        .to_path_buf()
}

pub fn config_path() -> std::path::PathBuf {
    asset_dir().join("config.json")
}

pub fn profiles_path() -> std::path::PathBuf {
    asset_dir().join("profiles.json")
}

pub fn credentials_path() -> std::path::PathBuf {
    asset_dir().join("credentials.json")
}

pub fn trusted_keys_path() -> std::path::PathBuf {
    asset_dir().join("trusted_ed25519_public_keys.json")
}

pub fn server_signing_key_path() -> std::path::PathBuf {
    asset_dir().join("server_ed25519_signing_key")
}

pub fn relay_host_credentials_path() -> std::path::PathBuf {
    asset_dir().join("relay_host_credentials.json")
}

/// 服务端运行设置（运行模式、会话有效期、OAuth 凭据）。
/// 不存在时全部走默认值（个人版）；环境变量优先级高于本文件。
pub fn server_settings_path() -> std::path::PathBuf {
    asset_dir().join("server.json")
}

/// 本机令牌的请求头名。**小写**：`HeaderMap` 按小写存取。
///
/// 放在 `utils` 而不是 `services`，是因为发送端（`crates/mcp`）与校验端
/// （`crates/server` 经 `crates/services`）分属两棵依赖树，只有 `utils` 是
/// 两边都依赖的公共祖先。写成两份常量迟早会漂移，而漂移的表现是
/// 「团队模式下 MCP 静默 401」，极难定位。
/// `services::services::local_auth::token::MACHINE_TOKEN_HEADER` 转发到这里。
pub const MACHINE_TOKEN_HEADER: &str = "x-vk-machine-token";

/// 本机令牌（`X-VK-MACHINE-TOKEN`）的落盘位置。
///
/// 团队模式下它等价于「本机所有者」的免登录凭据，文件权限必须是 0600。
/// 生成与读取见 `services::services::local_auth::machine_token`。
pub fn machine_token_path() -> std::path::PathBuf {
    asset_dir().join("machine_token")
}

/// 同步读出本机令牌，给 MCP 这类不方便起 tokio 运行时的调用方用。
///
/// 读不到、为空、或只含空白一律返回 `None`——**绝不返回空串**：
/// 服务端对空令牌一律不匹配，返回空串只会让调用方带上一个必然被拒的头。
pub fn read_machine_token() -> Option<String> {
    let raw = std::fs::read_to_string(machine_token_path()).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(RustEmbed)]
#[folder = "../../assets/sounds"]
pub struct SoundAssets;

#[derive(RustEmbed)]
#[folder = "../../assets/scripts"]
pub struct ScriptAssets;
