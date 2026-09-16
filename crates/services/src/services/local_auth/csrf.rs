//! 双提交 CSRF 校验（设计 §6.3 第 3 条）。
//!
//! 登录时下发两条 Cookie：`vk_session`（HttpOnly）与 `vk_csrf`（**非** HttpOnly）。
//! 前端写操作时用 JS 读出 `vk_csrf` 回填 `X-VK-CSRF` 头，服务端比对两者。
//! 跨站页面读不到目标站点的 Cookie，因此伪造不出正确的头。
//!
//! 这里刻意只收 `&str` 形态的方法名，不收 `http::Method`：`crates/services`
//! 不依赖 axum / http，把纯判定放在这里可以不给 `crates/server` 新增依赖
//! （`subtle` 已是本 crate 的依赖）。

use subtle::ConstantTimeEq;

/// 校验失败的原因。会进日志与 403 响应体，必须是编译期常量，
/// 不能拼进任何用户可控的内容（令牌本身尤其不能进日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrfError {
    /// 有 Cookie 但没有 `X-VK-CSRF` 头。最常见的跨站写。
    MissingHeader,
    /// 有头但没有 `vk_csrf` Cookie。
    MissingCookie,
    /// 两边都在但至少一边是空值。**必须拒**：否则攻击者送两个空值就能通过。
    Empty,
    /// 两边都在、都非空，但不相等。
    Mismatch,
}

impl CsrfError {
    pub fn as_str(self) -> &'static str {
        match self {
            CsrfError::MissingHeader => "csrf_missing_header",
            CsrfError::MissingCookie => "csrf_missing_cookie",
            CsrfError::Empty => "csrf_empty",
            CsrfError::Mismatch => "csrf_mismatch",
        }
    }
}

/// 这个方法要不要做 CSRF 校验。
///
/// 安全方法（RFC 9110 §9.2.1）不校验：`GET` / `HEAD` / `OPTIONS` / `TRACE`。
/// 其余一律校验，**包括未知方法**——fail-closed，免得哪天冒出一个
/// 自定义写方法就绕过了整道门。
pub fn requires_csrf(method: &str) -> bool {
    !matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS" | "TRACE"
    )
}

/// 双提交比对。
///
/// `cookie` 是 `vk_csrf` Cookie 的值，`header` 是 `X-VK-CSRF` 头的值。
/// 两者都裁掉首尾空白后按**常量时间**比较——空白不是秘密，令牌内容是。
pub fn csrf_check(
    method: &str,
    cookie: Option<&str>,
    header: Option<&str>,
) -> Result<(), CsrfError> {
    if !requires_csrf(method) {
        return Ok(());
    }

    // 先判 Cookie：两边都缺时报「缺 Cookie」，因为没有 Cookie 就根本谈不上双提交。
    let Some(cookie) = cookie else {
        return Err(CsrfError::MissingCookie);
    };
    let Some(header) = header else {
        return Err(CsrfError::MissingHeader);
    };

    let cookie = cookie.trim();
    let header = header.trim();
    if cookie.is_empty() || header.is_empty() {
        return Err(CsrfError::Empty);
    }

    if tokens_match(cookie, header) {
        Ok(())
    } else {
        Err(CsrfError::Mismatch)
    }
}

/// 常量时间比较两个令牌。
///
/// `ct_eq` 要求等长切片，长度不同时直接返回 false——长度本来就不是秘密
/// （令牌长度是固定的 43 字符，攻击者早就知道）。
fn tokens_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 安全方法不校验，否则顶层导航会被 403 打死。
    #[test]
    fn 读方法不校验() {
        for method in [
            "GET", "HEAD", "OPTIONS", "TRACE", "get", "head", "options", " Get ",
        ] {
            assert_eq!(csrf_check(method, None, None), Ok(()), "{method}");
            assert!(!requires_csrf(method), "{method}");
        }
    }

    #[test]
    fn 写方法两者一致通过() {
        for method in ["POST", "PUT", "PATCH", "DELETE", "post", "Delete"] {
            assert_eq!(
                csrf_check(method, Some("abc"), Some("abc")),
                Ok(()),
                "{method}"
            );
            assert!(requires_csrf(method), "{method}");
        }
    }

    /// 最典型的跨站写：攻击者的页面能让浏览器自动带上 Cookie，
    /// 但读不到 Cookie 的值，因此加不上这个头。
    #[test]
    fn 写方法缺头被拒() {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert_eq!(
                csrf_check(method, Some("abc"), None),
                Err(CsrfError::MissingHeader),
                "{method}"
            );
        }
    }

    #[test]
    fn 写方法缺_cookie_被拒() {
        assert_eq!(
            csrf_check("POST", None, Some("abc")),
            Err(CsrfError::MissingCookie)
        );
    }

    #[test]
    fn 写方法两者都缺被拒() {
        assert!(csrf_check("POST", None, None).is_err());
    }

    /// **攻击样例**：若实现写成「两边相等即通过」，攻击者送 `vk_csrf=` 与
    /// `X-VK-CSRF:` 两个空值就能通过。空值必须单独拒掉。
    #[test]
    fn 两者都是空串被拒() {
        for (cookie, header) in [
            ("", ""),
            ("", "abc"),
            ("abc", ""),
            ("   ", "   "),
            ("\t", "\n"),
        ] {
            assert!(
                matches!(
                    csrf_check("POST", Some(cookie), Some(header)),
                    Err(CsrfError::Empty) | Err(CsrfError::Mismatch)
                ),
                "cookie={cookie:?} header={header:?} 必须被拒"
            );
        }
        // 两边都空必须报 Empty 而不是 Mismatch——语义要准确。
        assert_eq!(
            csrf_check("POST", Some(""), Some("")),
            Err(CsrfError::Empty)
        );
        assert_eq!(
            csrf_check("POST", Some("  "), Some("  ")),
            Err(CsrfError::Empty)
        );
    }

    #[test]
    fn 不一致被拒() {
        assert_eq!(
            csrf_check("POST", Some("abc"), Some("abd")),
            Err(CsrfError::Mismatch)
        );
        // 大小写敏感：base64url 令牌区分大小写。
        assert_eq!(
            csrf_check("POST", Some("abc"), Some("ABC")),
            Err(CsrfError::Mismatch)
        );
    }

    /// 长度不同必须被拒，且不能因为提前 `return` 泄露「前缀是对的」。
    /// 长度本身不是秘密（固定 43 字符），所以按长度短路是可以的。
    #[test]
    fn 长度不同被拒() {
        for (cookie, header) in [
            ("abc", "abcd"),
            ("abcd", "abc"),
            ("a", "aaaaaaaaaaaaaaaaaaaa"),
            (
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            ),
        ] {
            assert_eq!(
                csrf_check("POST", Some(cookie), Some(header)),
                Err(CsrfError::Mismatch),
                "{cookie} vs {header}"
            );
        }
    }

    /// 首尾空白不是秘密，裁掉后再比；但裁完是空的仍然算空。
    #[test]
    fn 裁掉首尾空白后比较() {
        assert_eq!(csrf_check("POST", Some("  abc  "), Some("abc")), Ok(()));
        assert_eq!(csrf_check("POST", Some("abc"), Some("\tabc\n")), Ok(()));
    }

    /// 未知方法一律按写方法处理（fail-closed）。
    #[test]
    fn 未知方法按写方法处理() {
        for method in ["PROPFIND", "LOCK", "FOO", "", "  "] {
            assert!(requires_csrf(method), "{method:?} 应被当作写方法");
            assert!(csrf_check(method, None, None).is_err(), "{method:?}");
        }
    }

    /// 拒绝原因是固定字符串，不含任何用户输入（会进日志）。
    #[test]
    fn 拒绝原因是固定常量() {
        assert_eq!(CsrfError::MissingHeader.as_str(), "csrf_missing_header");
        assert_eq!(CsrfError::MissingCookie.as_str(), "csrf_missing_cookie");
        assert_eq!(CsrfError::Empty.as_str(), "csrf_empty");
        assert_eq!(CsrfError::Mismatch.as_str(), "csrf_mismatch");
    }

    /// 真实令牌长度（43 字符 base64url）下的正反例，防止实现里写死短串假设。
    #[test]
    fn 真实长度令牌的正反例() {
        let token = crate::services::local_auth::token::generate_csrf_token();
        assert_eq!(token.len(), 43);
        assert_eq!(csrf_check("POST", Some(&token), Some(&token)), Ok(()));

        let 另一个 = crate::services::local_auth::token::generate_csrf_token();
        assert_eq!(
            csrf_check("POST", Some(&token), Some(&另一个)),
            Err(CsrfError::Mismatch)
        );

        // 只差最后一个字符。
        let mut 改一位 = token.clone();
        let 末位 = 改一位.pop().unwrap();
        改一位.push(if 末位 == 'A' { 'B' } else { 'A' });
        assert_eq!(
            csrf_check("POST", Some(&token), Some(&改一位)),
            Err(CsrfError::Mismatch)
        );
    }
}
