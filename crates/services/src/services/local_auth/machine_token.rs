//! 本机令牌（设计 §6.3 第 5 条）。
//!
//! 团队模式下 `/api/*` 全线要求会话，而 `vibe-kanban-mcp` 用裸 `reqwest`
//! 直连本地端口，既没有 Cookie 也没有 Origin —— 不给它一条等价凭据，
//! 编码智能体在团队模式下会全线 401。
//!
//! 这条凭据就是 `asset_dir()/machine_token`：32 字节随机数，base64url 无填充，
//! 文件权限 0600（只有本机当前用户读得到），随请求头
//! `X-VK-MACHINE-TOKEN` 发送。
//!
//! **空令牌绝不算匹配**（见 [`runtime::LocalAuthRuntime::machine_token_matches`]）。
//! 本模块因此把「文件存在但是空的 / 只有空白」当成「没有令牌」并重新生成：
//! 否则一次误清空文件就会退化成「空令牌配空请求头」的 fail-open。
//!
//! [`runtime::LocalAuthRuntime::machine_token_matches`]: super::runtime::LocalAuthRuntime::machine_token_matches

use std::path::Path;

use super::token::generate_session_token;

/// 读出本机令牌，没有就生成一个并落盘。
///
/// 返回的永远是非空令牌；出错（目录不可写等）时返回 `Err`，
/// **不返回空串**——空串会被上层当成「本机令牌未启用」而静默关掉这条通路。
pub async fn load_or_create_machine_token_at(path: &Path) -> std::io::Result<String> {
    if let Some(token) = read_machine_token_at(path).await? {
        // 文件可能是早期版本或手工创建的，权限未必对，每次启动都收紧一次。
        tighten_permissions(path).await?;
        return Ok(token);
    }

    let token = generate_session_token();
    write_private(path, &token).await?;

    // 并发启动时另一个进程可能刚写过；以磁盘上的那份为准，
    // 保证同一台机器上所有进程看到的令牌一致。
    match read_machine_token_at(path).await? {
        Some(existing) => Ok(existing),
        None => Ok(token),
    }
}

/// 读出令牌；文件不存在、为空、或只含空白都返回 `Ok(None)`。
pub async fn read_machine_token_at(path: &Path) -> std::io::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => Ok(sanitize(&raw)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// 裁掉首尾空白；空的一律当作「没有令牌」。
fn sanitize(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 以 0600 写入。先建父目录，再写文件。
async fn write_private(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, token).await?;
    tighten_permissions(path).await
}

/// 把文件权限收紧到 0600。非 Unix 平台是空操作
/// （Windows 的 ACL 默认就把用户目录限制在当前用户）。
async fn tighten_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(path, perms).await?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 临时目录() -> tempfile::TempDir {
        tempfile::tempdir().expect("建临时目录失败")
    }

    #[tokio::test]
    async fn 首次调用生成并落盘且两次结果相同() {
        let dir = 临时目录();
        let path = dir.path().join("machine_token");

        let 第一次 = load_or_create_machine_token_at(&path).await.unwrap();
        assert!(path.exists(), "令牌必须落盘，否则重启就换一把");
        let 第二次 = load_or_create_machine_token_at(&path).await.unwrap();
        assert_eq!(第一次, 第二次, "同一个文件必须返回同一个令牌");

        let 磁盘上 = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(磁盘上.trim(), 第一次);
    }

    #[tokio::test]
    async fn 令牌是_43_字符_base64url() {
        let dir = 临时目录();
        let token = load_or_create_machine_token_at(&dir.path().join("machine_token"))
            .await
            .unwrap();
        assert_eq!(token.len(), 43, "32 字节 base64url 无填充是 43 个字符");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "令牌含非 base64url 字符：{token}"
        );
    }

    /// 令牌要塞进 HTTP 请求头，含空白或控制字符会让 `HeaderValue` 构造失败，
    /// 表现为「MCP 静默不带这个头」——最难查的那种失效。
    #[tokio::test]
    async fn 令牌可以直接当请求头值() {
        let dir = 临时目录();
        let token = load_or_create_machine_token_at(&dir.path().join("machine_token"))
            .await
            .unwrap();
        assert!(!token.chars().any(char::is_whitespace), "{token:?}");
        assert!(
            token.chars().all(|c| c.is_ascii_graphic()),
            "只能是可打印 ASCII：{token:?}"
        );
    }

    #[tokio::test]
    async fn 两个不同目录生成不同令牌() {
        let a = 临时目录();
        let b = 临时目录();
        let 令牌_a = load_or_create_machine_token_at(&a.path().join("machine_token"))
            .await
            .unwrap();
        let 令牌_b = load_or_create_machine_token_at(&b.path().join("machine_token"))
            .await
            .unwrap();
        assert_ne!(令牌_a, 令牌_b, "随机源有问题");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn 文件权限是_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = 临时目录();
        let path = dir.path().join("machine_token");
        load_or_create_machine_token_at(&path).await.unwrap();

        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "本机令牌等价于免登录凭据，同机其它用户不能读，实际 {:o}",
            mode & 0o777
        );
    }

    /// 已存在但权限过宽的文件（比如用户手工 `echo > machine_token`）
    /// 必须在启动时被收紧，而不是放着不管。
    #[cfg(unix)]
    #[tokio::test]
    async fn 已存在文件的过宽权限会被收紧() {
        use std::os::unix::fs::PermissionsExt;

        let dir = 临时目录();
        let path = dir.path().join("machine_token");
        tokio::fs::write(&path, "preexisting-token-value")
            .await
            .unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        let token = load_or_create_machine_token_at(&path).await.unwrap();
        assert_eq!(token, "preexisting-token-value", "不该丢掉已有令牌");
        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "实际 {:o}", mode & 0o777);
    }

    /// **fail-open 防线**：文件被清空 → 空令牌 → 服务端拿空串去比对。
    /// 必须当作「没有令牌」重新生成，绝不能返回空串。
    #[tokio::test]
    async fn 空文件被当作没有令牌并重新生成() {
        let dir = 临时目录();
        let path = dir.path().join("machine_token");
        tokio::fs::write(&path, "").await.unwrap();

        let token = load_or_create_machine_token_at(&path).await.unwrap();
        assert!(!token.is_empty(), "绝不能返回空串");
        assert_eq!(token.len(), 43);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap().trim(),
            token,
            "新令牌必须写回磁盘"
        );
    }

    #[tokio::test]
    async fn 只含空白的文件同样重新生成() {
        for 空白 in ["   ", "\n", "\r\n", "\t\t", " \n \t "] {
            let dir = 临时目录();
            let path = dir.path().join("machine_token");
            tokio::fs::write(&path, 空白).await.unwrap();

            let token = load_or_create_machine_token_at(&path).await.unwrap();
            assert_eq!(token.len(), 43, "{空白:?} 应被当作没有令牌");
        }
    }

    /// 文件末尾的换行（编辑器会自动加）不该被当成令牌的一部分，
    /// 否则服务端存的和 MCP 发的差一个 `\n`，永远对不上。
    #[tokio::test]
    async fn 首尾空白会被裁掉() {
        let dir = 临时目录();
        let path = dir.path().join("machine_token");
        tokio::fs::write(&path, "  token-with-spaces  \n")
            .await
            .unwrap();
        assert_eq!(
            load_or_create_machine_token_at(&path).await.unwrap(),
            "token-with-spaces"
        );
    }

    #[tokio::test]
    async fn 父目录不存在时会被建出来() {
        let dir = 临时目录();
        let path = dir.path().join("a").join("b").join("machine_token");
        let token = load_or_create_machine_token_at(&path).await.unwrap();
        assert_eq!(token.len(), 43);
        assert!(path.exists());
    }

    /// 全链路：落盘的令牌 → `LocalAuthRuntime` → 请求头比对。
    /// 任何一环（裁空白、大小写、长度）对不上，团队模式下 MCP 就会静默 401。
    #[tokio::test]
    async fn 落盘的令牌能被运行时匹配() {
        use crate::services::{
            local_auth::runtime::LocalAuthRuntime,
            server_settings::{ServerMode, ServerSettings},
        };

        let dir = 临时目录();
        let path = dir.path().join("machine_token");
        let token = load_or_create_machine_token_at(&path).await.unwrap();

        let rt = LocalAuthRuntime::new(
            ServerSettings {
                mode: ServerMode::Team,
                ..ServerSettings::default()
            },
            token.clone(),
        );
        assert!(rt.machine_token_matches(&token), "自己生成的令牌必须匹配");
        assert!(!rt.machine_token_matches(""), "空头绝不匹配");
        assert!(!rt.machine_token_matches(&token[..token.len() - 1]));
        assert!(!rt.machine_token_matches(&format!("{token}x")));
    }

    /// 请求头名必须是小写：`HeaderMap` 按小写存取，写成大写会永远取不到。
    /// 常量定义在 `utils` 里，发送端（crates/mcp）与校验端共用同一个字面量。
    #[test]
    fn 请求头常量两端共用且是小写() {
        use super::super::token::MACHINE_TOKEN_HEADER;
        assert_eq!(MACHINE_TOKEN_HEADER, utils::assets::MACHINE_TOKEN_HEADER);
        assert_eq!(MACHINE_TOKEN_HEADER, MACHINE_TOKEN_HEADER.to_lowercase());
        assert_eq!(MACHINE_TOKEN_HEADER, "x-vk-machine-token");
    }

    #[tokio::test]
    async fn 读不到文件时返回_none_而不是报错() {
        let dir = 临时目录();
        assert_eq!(
            read_machine_token_at(&dir.path().join("nope"))
                .await
                .unwrap(),
            None
        );
    }
}
