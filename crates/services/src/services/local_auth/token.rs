//! 会话令牌的生成 / 哈希，以及 Cookie 串的构造与解析。
//!
//! 明文令牌只出现在两个地方：生成它的那一刻，和发给浏览器的 `Set-Cookie` 头。
//! 库里只存 [`hash_session_token`] 的结果；日志与错误消息里一律不带明文
//! （本模块不做任何 `tracing` 输出，就是为了不给它留出口）。

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// 会话 Cookie 名。
pub const SESSION_COOKIE: &str = "vk_session";
/// CSRF 双提交 Cookie 名（**非** HttpOnly，前端要读它）。
pub const CSRF_COOKIE: &str = "vk_csrf";
/// CSRF 请求头名。HeaderMap 按小写存取，常量必须是小写。
pub const CSRF_HEADER: &str = "x-vk-csrf";
/// 本机进程（MCP）的免会话凭据请求头名。
pub const MACHINE_TOKEN_HEADER: &str = "x-vk-machine-token";

/// 令牌熵：32 字节。base64url 无填充后是 43 个字符。
const TOKEN_BYTES: usize = 32;

/// 生成会话令牌：32 字节操作系统随机数，base64url 无填充。
///
/// 字符集是 `[A-Za-z0-9_-]`，天然不含 `;` `=` `,` 与空白，
/// 拼进 `Set-Cookie` 不会产生属性注入。
pub fn generate_session_token() -> String {
    random_token()
}

/// 生成 CSRF 令牌。与会话令牌同源同长度，但是两个独立的随机值。
pub fn generate_csrf_token() -> String {
    random_token()
}

fn random_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 令牌的 SHA-256（小写十六进制）。库里存的就是它。
///
/// 这里用 SHA-256 而不是 Argon2 是刻意的：令牌本身就是 256 位均匀随机数，
/// 没有「弱口令」可言，慢哈希只会让每个请求都付出 19 MiB 的代价。
pub fn hash_session_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn max_age_seconds(ttl_days: u32) -> u64 {
    u64::from(ttl_days) * 24 * 60 * 60
}

/// 剔除 Cookie 值里的语法字符。
///
/// 令牌是本模块自己生成的，正常永远不会命中；这是给「有人把外部输入当令牌传进来」
/// 留的兜底，防止拼出第二条属性（`abc; Path=/evil`）或 HTTP 响应拆分（`\r\n`）。
fn sanitize_cookie_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~'))
        .collect()
}

/// `vk_session=<token>; HttpOnly; SameSite=Lax; Path=/; Max-Age=<ttl>[; Secure]`
///
/// **用 `Lax` 而不是 `Strict`**：OAuth 提供方跳回来的那次跨站顶层导航带不上
/// Strict Cookie，登录会表现为「回调成功但页面仍未登录」。Lax 的安全前提
/// （GET 不做写操作 + Origin 强校验 + 双提交令牌）由任务 C 补齐。
pub fn build_session_cookie(token: &str, ttl_days: u32, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        sanitize_cookie_value(token),
        max_age_seconds(ttl_days)
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// CSRF Cookie。**不加 HttpOnly**：前端要用 JS 读出来回填 `X-VK-CSRF` 头。
pub fn build_csrf_cookie(token: &str, ttl_days: u32, secure: bool) -> String {
    let mut cookie = format!(
        "{CSRF_COOKIE}={}; SameSite=Lax; Path=/; Max-Age={}",
        sanitize_cookie_value(token),
        max_age_seconds(ttl_days)
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 登出用：同名同 Path 的空值 Cookie，`Max-Age=0` 让浏览器立刻删掉。
/// 属性必须与下发时一致，否则浏览器会认为是另一条 Cookie 而删不掉。
pub fn build_session_clear_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0").to_string();
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn build_csrf_clear_cookie(secure: bool) -> String {
    let mut cookie = format!("{CSRF_COOKIE}=; SameSite=Lax; Path=/; Max-Age=0").to_string();
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 从 `Cookie` 请求头里取出指定名字的值。
///
/// 规则（每条都有对应测试）：
/// - 按 `;` 切分，每段裁两侧空白，用**第一个** `=` 切成键/值；
/// - 键裁空白后**精确相等**才算命中，大小写敏感，不做前缀/后缀匹配；
/// - 值裁两侧空白后原样返回（不解 URL 编码、不去引号）；
/// - 同名多条取**第一条**（攻击者能从子域名注入第二条同名 Cookie）；
/// - 没有 `=` 的段整体忽略。
pub fn parse_cookie(header: Option<&str>, name: &str) -> Option<String> {
    let header = header?;
    for part in header.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim() == name {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// 同 [`parse_cookie`]，但把空值也当作「没有」。
/// 中间件取会话令牌时必须用这个：`vk_session=` 不是一张有效的 Cookie。
pub fn parse_non_empty_cookie(header: Option<&str>, name: &str) -> Option<String> {
    parse_cookie(header, name).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn 令牌是_43_字符的_base64url() {
        let token = generate_session_token();
        assert_eq!(token.len(), 43, "32 字节 base64url 无填充是 43 个字符");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "令牌含非 base64url 字符：{token}"
        );
        assert!(!token.contains('='), "不应带填充");
    }

    #[test]
    fn 令牌每次都不同且不含_cookie_分隔符() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let token = generate_session_token();
            assert!(
                !token.contains(';') && !token.contains('=') && !token.contains(','),
                "令牌不得含 Cookie 分隔符：{token}"
            );
            assert!(
                !token.chars().any(char::is_whitespace),
                "令牌不得含空白：{token}"
            );
            assert!(seen.insert(token), "1000 次生成出现重复，随机源有问题");
        }
    }

    #[test]
    fn 哈希是_64_位小写十六进制且稳定() {
        let hash = hash_session_token("abc");
        assert_eq!(hash.len(), 64);
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(hash, hash_session_token("abc"), "同一输入必须稳定");
        assert_ne!(hash, hash_session_token("abd"));
        assert_ne!(hash, hash_session_token("abc "), "不得对令牌做 trim");
        assert_ne!(hash, hash_session_token("ABC"), "必须大小写敏感");

        // 已知向量：SHA-256("abc")
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn 空令牌也能算出哈希但上层必须当作无会话() {
        // hash_session_token 是纯函数，不负责判空；判空在 parse_cookie 的调用方。
        assert_eq!(hash_session_token("").len(), 64);
    }

    #[test]
    fn 会话_cookie_串包含全部属性() {
        assert_eq!(
            build_session_cookie("abc", 30, false),
            "vk_session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000"
        );
        assert_eq!(
            build_session_cookie("abc", 30, true),
            "vk_session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000; Secure"
        );
        assert_eq!(
            build_session_cookie("abc", 1, false),
            "vk_session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400"
        );
    }

    /// CSRF Cookie 必须**不是** HttpOnly（前端要读它回填请求头），
    /// 其余属性与会话 Cookie 一致。
    #[test]
    fn csrf_cookie_不是_httponly() {
        let cookie = build_csrf_cookie("tok", 30, false);
        assert_eq!(cookie, "vk_csrf=tok; SameSite=Lax; Path=/; Max-Age=2592000");
        assert!(!cookie.contains("HttpOnly"));
        assert!(
            build_csrf_cookie("tok", 30, true).ends_with("; Secure"),
            "HTTPS 下必须带 Secure"
        );
    }

    #[test]
    fn 登出_cookie_立即过期() {
        let cleared = build_session_clear_cookie(false);
        assert!(cleared.starts_with("vk_session=; "));
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("HttpOnly"));
        assert!(cleared.contains("Path=/"));
        assert!(cleared.contains("SameSite=Lax"));

        let csrf = build_csrf_clear_cookie(false);
        assert!(csrf.starts_with("vk_csrf=; "));
        assert!(csrf.contains("Max-Age=0"));
        assert!(!csrf.contains("HttpOnly"));
    }

    /// 令牌是我们自己生成的，正常不会含 Cookie 语法字符；但万一哪天有调用方
    /// 把外部输入塞进来，也不能让它拼出第二条属性（`abc; Path=/evil`）。
    #[test]
    fn 令牌里的_cookie_语法字符会被剔除() {
        let cookie = build_session_cookie("abc; Path=/evil; Domain=attacker.test", 30, false);
        // 属性个数与内容必须与正常情况完全一致，一条都不能多出来。
        let 属性: Vec<&str> = cookie.split("; ").collect();
        assert_eq!(
            &属性[1..],
            ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"],
            "多出了注入的属性：{cookie}"
        );
        let 值 = 属性[0].strip_prefix("vk_session=").expect("首段应是键值对");
        assert!(
            !值.contains(';') && !值.contains('=') && !值.contains('/') && !值.contains(' '),
            "值里仍留着 Cookie 语法字符：{值:?}"
        );

        // 换行同样要挡：拼进响应头就是 HTTP 响应拆分。
        let cookie = build_session_cookie("abc\r\nSet-Cookie: evil=1", 30, false);
        assert!(!cookie.contains('\r') && !cookie.contains('\n'), "{cookie}");
    }

    #[test]
    fn parse_cookie_基本用例() {
        assert_eq!(
            parse_cookie(Some("a=1; vk_session=xyz; b=2"), SESSION_COOKIE).as_deref(),
            Some("xyz")
        );
        assert_eq!(
            parse_cookie(Some("vk_session=xyz"), SESSION_COOKIE).as_deref(),
            Some("xyz")
        );
        assert_eq!(parse_cookie(None, SESSION_COOKIE), None);
        assert_eq!(parse_cookie(Some(""), SESSION_COOKIE), None);
        assert_eq!(parse_cookie(Some("   "), SESSION_COOKIE), None);
    }

    #[test]
    fn parse_cookie_不做前缀或后缀匹配() {
        assert_eq!(
            parse_cookie(Some("vk_session_evil=bad"), SESSION_COOKIE),
            None
        );
        assert_eq!(parse_cookie(Some("xvk_session=bad"), SESSION_COOKIE), None);
        assert_eq!(parse_cookie(Some("vk_sessio=bad"), SESSION_COOKIE), None);
        assert_eq!(
            parse_cookie(Some("__Host-vk_session=bad"), SESSION_COOKIE),
            None
        );
    }

    #[test]
    fn parse_cookie_名字大小写敏感() {
        assert_eq!(parse_cookie(Some("VK_SESSION=x"), SESSION_COOKIE), None);
        assert_eq!(parse_cookie(Some("Vk_Session=x"), SESSION_COOKIE), None);
    }

    /// 攻击者可以从子域名给父域写一个同名 Cookie，浏览器会把两条都发上来。
    /// 取第一条是 RFC 6265 的既定行为（更具体的路径在前），这里用测试把它钉死，
    /// 免得哪天实现换成「取最后一条」而悄悄改变可被覆盖的方向。
    #[test]
    fn parse_cookie_同名多条取第一条() {
        assert_eq!(
            parse_cookie(Some("vk_session=a; vk_session=b"), SESSION_COOKIE).as_deref(),
            Some("a")
        );
        assert_eq!(
            parse_cookie(Some("x=1; vk_session=a; y=2; vk_session=b"), SESSION_COOKIE).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn parse_cookie_裁掉两侧空白() {
        assert_eq!(
            parse_cookie(Some("   vk_session   =   x   "), SESSION_COOKIE).as_deref(),
            Some("x")
        );
        assert_eq!(
            parse_cookie(Some("a=1;vk_session=x;b=2"), SESSION_COOKIE).as_deref(),
            Some("x"),
            "分号后没有空格也要能解析"
        );
    }

    /// 空值必须能被区分出来；上层要把 `Some("")` 当作「无会话」。
    #[test]
    fn parse_cookie_空值返回空串() {
        assert_eq!(
            parse_cookie(Some("vk_session="), SESSION_COOKIE).as_deref(),
            Some("")
        );
        assert_eq!(
            parse_cookie(Some("vk_session=   "), SESSION_COOKIE).as_deref(),
            Some("")
        );
        // 便捷入口：空值直接按「没有」处理。
        assert_eq!(
            parse_non_empty_cookie(Some("vk_session="), SESSION_COOKIE),
            None
        );
        assert_eq!(
            parse_non_empty_cookie(Some("vk_session=x"), SESSION_COOKIE).as_deref(),
            Some("x")
        );
    }

    #[test]
    fn parse_cookie_畸形输入不会_panic() {
        for header in [
            ";;;",
            "=",
            "=x",
            "vk_session",
            "vk_session;",
            "a=1; ; vk_session=x",
            "vk_session=x=y",
            "\u{0}vk_session=x",
            &"a=1; ".repeat(5000),
        ] {
            let _ = parse_cookie(Some(header), SESSION_COOKIE);
        }
        // 值里含 '=' 时按第一个 '=' 切分，剩下的都算值。
        assert_eq!(
            parse_cookie(Some("vk_session=x=y"), SESSION_COOKIE).as_deref(),
            Some("x=y")
        );
        // 没有 '=' 的段整体忽略。
        assert_eq!(parse_cookie(Some("vk_session"), SESSION_COOKIE), None);
    }

    #[test]
    fn csrf_令牌与会话令牌互不通用() {
        let session = generate_session_token();
        let csrf = generate_csrf_token();
        assert_ne!(session, csrf);
        assert_eq!(csrf.len(), 43);
    }

    #[test]
    fn 请求头常量是小写() {
        // axum / http 的 HeaderMap 按小写存取，常量写成大写会永远取不到。
        assert_eq!(CSRF_HEADER, CSRF_HEADER.to_ascii_lowercase());
        assert_eq!(
            MACHINE_TOKEN_HEADER,
            MACHINE_TOKEN_HEADER.to_ascii_lowercase()
        );
    }
}
