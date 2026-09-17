use std::{net::IpAddr, sync::OnceLock};

use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode, header},
    response::Response,
};
use relay_client::RELAY_HEADER;
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
struct OriginKey {
    https: bool,
    host: String,
    port: u16,
}

impl OriginKey {
    fn from_origin(origin: &str) -> Option<Self> {
        let url = Url::parse(origin).ok()?;
        let https = match url.scheme() {
            "http" => false,
            "https" => true,
            _ => return None,
        };
        let host = normalize_host(url.host_str()?);
        let port = url.port_or_known_default()?;
        Some(Self { https, host, port })
    }

    fn from_host_header(host: &str, https: bool) -> Option<Self> {
        let authority: axum::http::uri::Authority = host.parse().ok()?;
        let host = normalize_host(authority.host());
        let port = authority.port_u16().unwrap_or_else(|| default_port(https));
        Some(Self { https, host, port })
    }
}

#[allow(clippy::result_large_err)]
pub fn validate_origin<B>(req: &mut Request<B>) -> Result<(), Response> {
    // Relay-proxied requests are authenticated through the relay's own session
    // system, so origin validation is not applicable.
    if is_relay_request(req) {
        return Ok(());
    }

    // 请求是否**必须**带 Origin。判据是「有没有借用浏览器的环境凭据」：
    // 带 Cookie 的写请求，以及带 Cookie 的 WebSocket 升级（CSWSH）。
    //
    // 不带 Cookie 的请求一律不强制 Origin：本机工具（MCP、脚本）用裸 HTTP
    // 客户端直连，既不带 Cookie 也不带 Origin，它们的鉴权由机器令牌负责
    // （`services::local_auth::token::MACHINE_TOKEN_HEADER`）。浏览器对
    // POST/PUT/PATCH/DELETE 与 WebSocket 握手**一定**会带 Origin，所以这条
    // 放行不给浏览器侧留口子。
    let origin_required = req.headers().contains_key(header::COOKIE)
        && (!is_safe_method(req.method()) || is_websocket_upgrade(req));

    // Origin 头在场但取不出可打印 ASCII（含非 ASCII 同形字伪装、控制字符）
    // 一律拒：否则它会退化成「没有 Origin」，绕过上面这条规则。
    if req.headers().contains_key(header::ORIGIN) && get_origin_header(req).is_none() {
        return Err(forbidden());
    }

    let Some(origin) = get_origin_header(req) else {
        return if origin_required {
            Err(forbidden())
        } else {
            Ok(())
        };
    };

    if origin.eq_ignore_ascii_case("null") {
        return Err(forbidden());
    }

    let host = get_host_header(req);

    // quick short-circuit same-origin check
    if host.is_some_and(|host| origin_matches_host(origin, host)) {
        return Ok(());
    }

    let Some(origin_key) = OriginKey::from_origin(origin) else {
        return Err(forbidden());
    };

    if allowed_origins()
        .iter()
        .any(|allowed| allowed == &origin_key)
    {
        return Ok(());
    }

    if let Some(host_key) =
        host.and_then(|host| OriginKey::from_host_header(host, origin_key.https))
        && host_key == origin_key
    {
        return Ok(());
    }

    Err(forbidden())
}

fn get_origin_header<B>(req: &Request<B>) -> Option<&str> {
    get_header(req, header::ORIGIN)
}

fn get_host_header<B>(req: &Request<B>) -> Option<&str> {
    get_header(req, header::HOST)
}

fn get_header<B>(req: &Request<B>, name: header::HeaderName) -> Option<&str> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// RFC 9110 §9.2.1 的安全方法。未知方法按**不安全**处理（fail-closed），
/// 免得哪天冒出一个自定义写方法就绕过整道门。
fn is_safe_method(method: &Method) -> bool {
    matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS" | "TRACE")
}

/// 是不是 WebSocket 升级握手。
///
/// 握手本身是 GET，`SameSite=Lax` 与「写方法才校验」都盖不住它，
/// 而浏览器会自动给跨站的 `new WebSocket(...)` 带上目标站点的 Cookie。
/// RFC 6455 规定 `Upgrade` 的取值按 ASCII 不区分大小写比较。
fn is_websocket_upgrade<B>(req: &Request<B>) -> bool {
    req.headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("websocket"))
}

fn is_relay_request<B>(req: &Request<B>) -> bool {
    req.headers()
        .get(RELAY_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "1")
}

fn forbidden() -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(|rest| rest.eq_ignore_ascii_case(host))
}

fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    let lower = trimmed.to_ascii_lowercase();
    if lower == "localhost" {
        return "localhost".to_string();
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        if ip.is_loopback() {
            return "localhost".to_string();
        }
        return ip.to_string();
    }
    lower
}

fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn allowed_origins() -> &'static Vec<OriginKey> {
    static ALLOWED: OnceLock<Vec<OriginKey>> = OnceLock::new();
    ALLOWED.get_or_init(|| {
        let value = match std::env::var("VK_ALLOWED_ORIGINS") {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };

        value
            .split(',')
            .filter_map(|origin| OriginKey::from_origin(origin.trim()))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use axum::http::{Request, header};

    use super::*;

    /// 攻击样例的请求构造器。默认是「GET + 无 Cookie」，
    /// 每个用例只显式打开它要考察的那一项。
    #[derive(Default, Clone)]
    struct 请求 {
        method: &'static str,
        origin: Option<&'static str>,
        host: Option<&'static str>,
        cookie: bool,
        upgrade: Option<&'static str>,
        relay: bool,
    }

    impl 请求 {
        fn 新(method: &'static str) -> Self {
            Self {
                method,
                host: Some("kanban.lan:8080"),
                ..Default::default()
            }
        }

        fn origin(mut self, value: &'static str) -> Self {
            self.origin = Some(value);
            self
        }

        fn host(mut self, value: Option<&'static str>) -> Self {
            self.host = value;
            self
        }

        fn 带_cookie(mut self) -> Self {
            self.cookie = true;
            self
        }

        fn upgrade(mut self, value: &'static str) -> Self {
            self.upgrade = Some(value);
            self
        }

        fn relay(mut self) -> Self {
            self.relay = true;
            self
        }

        fn 构造(&self) -> Request<Body> {
            let mut builder = Request::builder().uri("/test").method(self.method);
            if let Some(origin) = self.origin {
                builder = builder.header(header::ORIGIN, origin);
            }
            if let Some(host) = self.host {
                builder = builder.header(header::HOST, host);
            }
            if self.cookie {
                builder = builder.header(header::COOKIE, "vk_session=x");
            }
            if let Some(upgrade) = self.upgrade {
                builder = builder
                    .header(header::UPGRADE, upgrade)
                    .header(header::CONNECTION, "Upgrade");
            }
            if self.relay {
                builder = builder.header(RELAY_HEADER, "1");
            }
            builder.body(Body::empty()).unwrap()
        }

        fn 放行(&self) -> bool {
            validate_origin(&mut self.构造()).is_ok()
        }

        fn 被拒(&self) -> bool {
            is_forbidden(validate_origin(&mut self.构造()))
        }
    }

    /// 写方法全集，逐个方法都要按同一规则判定。
    const 写方法: [&str; 4] = ["POST", "PUT", "PATCH", "DELETE"];

    fn make_request(origin: Option<&str>, host: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/test").method("GET");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(host) = host {
            builder = builder.header(header::HOST, host);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn is_forbidden(result: Result<(), Response>) -> bool {
        matches!(result, Err(resp) if resp.status() == StatusCode::FORBIDDEN)
    }

    // ------------------------------------------------------- C1：写方法强校验

    /// 攻击面：`SameSite=Lax` 只挡住「浏览器自动带 Cookie 的跨站写」，
    /// 但一个能构造出「不带 Origin 的写请求」的客户端仍然能拿着 Cookie 打进来。
    /// 带 Cookie 就说明请求在借用浏览器的环境凭据，此时 Origin 必须在场。
    #[test]
    fn 带_cookie_的写请求缺少_origin_被拒() {
        for method in 写方法 {
            assert!(
                请求::新(method).带_cookie().被拒(),
                "{method} 缺 Origin 却带 Cookie，必须 403"
            );
        }
    }

    #[test]
    fn 带_cookie_的写请求_origin_跨站被拒() {
        for method in 写方法 {
            assert!(
                请求::新(method)
                    .带_cookie()
                    .origin("http://evil.example")
                    .被拒(),
                "{method} 跨站 Origin 必须 403"
            );
        }
    }

    #[test]
    fn 带_cookie_的写请求同源放行() {
        for method in 写方法 {
            assert!(
                请求::新(method)
                    .带_cookie()
                    .origin("http://kanban.lan:8080")
                    .放行(),
                "{method} 同源必须放行"
            );
        }
    }

    /// 本机工具（MCP、脚本）用裸 HTTP 客户端直连，不带 Cookie 也不带 Origin。
    /// 收紧后**绝不能**把它们打死——它们的鉴权由机器令牌（C5）负责。
    #[test]
    fn 不带_cookie_的写请求缺少_origin_仍放行() {
        for method in 写方法 {
            assert!(
                请求::新(method).放行(),
                "{method} 无 Cookie 无 Origin 应放行"
            );
        }
    }

    /// 顶层导航（点链接、地址栏）不带 Origin，读请求不能因此被打死。
    #[test]
    fn 带_cookie_的_get_与_head_请求缺少_origin_仍放行() {
        for method in ["GET", "HEAD", "OPTIONS"] {
            assert!(
                请求::新(method).带_cookie().放行(),
                "{method} 是读方法，不该强制 Origin"
            );
        }
    }

    // ----------------------------------------------------------- C1：CSWSH

    /// 浏览器不对 WebSocket 施加同源策略：跨站页面 `new WebSocket(...)` 会
    /// 自动带上目标站点的 Cookie。握手是 GET，所以必须单独拦。
    #[test]
    fn websocket_升级带_cookie_跨站被拒() {
        assert!(
            请求::新("GET")
                .带_cookie()
                .upgrade("websocket")
                .origin("http://evil.example")
                .被拒()
        );
    }

    #[test]
    fn websocket_升级带_cookie_缺少_origin_被拒() {
        assert!(请求::新("GET").带_cookie().upgrade("websocket").被拒());
    }

    #[test]
    fn websocket_升级带_cookie_同源放行() {
        assert!(
            请求::新("GET")
                .带_cookie()
                .upgrade("websocket")
                .origin("http://kanban.lan:8080")
                .放行()
        );
    }

    /// 不带 Cookie 的升级请求没有环境凭据可借，放行给本机工具。
    #[test]
    fn websocket_升级不带_cookie_缺少_origin_放行() {
        assert!(请求::新("GET").upgrade("websocket").放行());
    }

    /// `Upgrade` 头的取值大小写由客户端决定，RFC 6455 规定按 ASCII 不区分大小写比较。
    /// 写成大小写敏感的比较等于给攻击者留了一个 `Upgrade: WebSocket` 的绕过。
    #[test]
    fn upgrade_头大小写不敏感() {
        for upgrade in ["websocket", "WebSocket", "WEBSOCKET", "  websocket  "] {
            assert!(
                请求::新("GET").带_cookie().upgrade(upgrade).被拒(),
                "Upgrade: {upgrade:?} 应与小写同样处理"
            );
        }
        // 不是 websocket 的升级（例如 h2c）不走 WS 规则。
        assert!(请求::新("GET").带_cookie().upgrade("h2c").放行());
    }

    // ------------------------------------------------- C1：Origin 伪装攻击样例

    /// 子域伪装：`evil-app.test` 与 `app.test` 只差一个前缀，
    /// 任何基于「包含 / 后缀」的比较都会被它打穿。
    #[test]
    fn 子域与前后缀伪装全部被拒() {
        for origin in [
            "http://evil-app.test",
            "http://app.test.evil.example",
            "http://evilapp.test",
            "http://app.test.evil",
            "http://xapp.test",
            "http://app.testx",
        ] {
            assert!(
                请求::新("POST")
                    .带_cookie()
                    .host(Some("app.test"))
                    .origin(origin)
                    .被拒(),
                "{origin} 不是 app.test 的同源"
            );
        }
    }

    #[test]
    fn 端口不同被拒() {
        for (origin, host) in [
            ("http://app.test:8080", "app.test:9090"),
            ("http://app.test", "app.test:8080"),
            ("http://app.test:8080", "app.test"),
        ] {
            assert!(
                请求::新("POST")
                    .带_cookie()
                    .host(Some(host))
                    .origin(origin)
                    .被拒(),
                "{origin} vs {host}"
            );
        }
    }

    /// 主机名按 ASCII 不区分大小写，放行是正确行为——用一条测试钉住，
    /// 免得收紧时手滑改成大小写敏感而误伤正常浏览器。
    #[test]
    fn 主机名大小写不影响同源判定() {
        for (origin, host) in [
            ("http://APP.TEST:8080", "app.test:8080"),
            ("http://app.test:8080", "APP.TEST:8080"),
            ("HTTP://app.test:8080", "app.test:8080"),
        ] {
            assert!(
                请求::新("POST")
                    .带_cookie()
                    .host(Some(host))
                    .origin(origin)
                    .放行(),
                "{origin} vs {host}"
            );
        }
    }

    /// 尾点是 DNS 里的合法「绝对域名」写法，某些解析栈会把 `app.test.`
    /// 与 `app.test` 当成同一台机器。这里保持 fail-closed：不相等就拒。
    #[test]
    fn 尾点域名被拒() {
        assert!(
            请求::新("POST")
                .带_cookie()
                .host(Some("app.test"))
                .origin("http://app.test.")
                .被拒()
        );
    }

    /// `null` 来自 sandboxed iframe、`file://` 页面与部分重定向链，
    /// 是最典型的「伪装成无源」攻击。
    #[test]
    fn null_与非_http_协议的_origin_被拒() {
        for origin in [
            "null",
            "NULL",
            "file://",
            "file:///etc/passwd",
            "data:text/html,x",
            "chrome-extension://abcdefg",
            "ws://app.test:8080",
            "javascript:alert(1)",
        ] {
            assert!(
                请求::新("POST")
                    .带_cookie()
                    .host(Some("app.test:8080"))
                    .origin(origin)
                    .被拒(),
                "{origin} 必须被拒"
            );
        }
    }

    /// 同形字攻击：西里尔字母 `а` 与拉丁 `a` 在视觉上一致，
    /// Punycode 后是完全不同的主机名，必须拒。
    #[test]
    fn 非_ascii_与_punycode_伪装被拒() {
        for origin in [
            "http://аpp.test",        // 首字母是西里尔 U+0430
            "http://xn--pp-8lc.test", // 上一行的 Punycode 形式
            "http://app.tеst",        // e 是西里尔 U+0435
            "http://app%2Etest.evil.example",
        ] {
            assert!(
                请求::新("POST")
                    .带_cookie()
                    .host(Some("app.test"))
                    .origin(origin)
                    .被拒(),
                "{origin} 必须被拒"
            );
        }
    }

    /// relay 请求整条跳过 Origin 校验（它的鉴权由 relay 自己的签名体系负责，
    /// 且团队模式下会话层会直接拒掉 relay）。这条行为不能被 C1 改坏。
    #[test]
    fn relay_头仍然整体跳过() {
        assert!(
            请求::新("POST")
                .relay()
                .带_cookie()
                .origin("http://evil.example")
                .放行()
        );
        assert!(请求::新("POST").relay().带_cookie().放行());
        assert!(
            请求::新("GET")
                .relay()
                .带_cookie()
                .upgrade("websocket")
                .放行()
        );
    }

    /// 缺 Host 头时无从判定同源，带 Cookie 的写请求必须 fail-closed。
    #[test]
    fn 缺少_host_头的写请求被拒() {
        assert!(
            请求::新("POST")
                .带_cookie()
                .host(None)
                .origin("http://app.test")
                .被拒()
        );
        assert!(请求::新("POST").带_cookie().host(None).被拒());
    }

    /// 「没有 GET 写操作」是 `SameSite=Lax` 的安全前提。
    /// `registered_endpoints()` 覆盖 `/api/local` 这一组，先把它钉住；
    /// 其余路由由 §12.13 的人工扫描兜底。
    #[test]
    fn 本地路由没有用_get_做写操作() {
        use crate::routes::local_projects::{registered_endpoints, router};

        let _ = router();
        let 写动词 = ["create", "update", "delete", "bulk", "move", "reorder"];
        for (path, methods) in registered_endpoints() {
            if !methods.contains("GET") {
                continue;
            }
            for verb in 写动词 {
                assert!(
                    !path.to_ascii_lowercase().contains(verb),
                    "{path} 用 GET 暴露了疑似写操作（{verb}）；\
                     SameSite=Lax 下跨站顶层 GET 导航会带上会话 Cookie"
                );
            }
        }
    }

    #[test]
    fn no_origin_header_allows_request() {
        let mut req = make_request(None, Some("example.com"));
        assert!(validate_origin(&mut req).is_ok());
    }

    #[test]
    fn null_origin_is_forbidden() {
        for null in ["null", "NULL", "Null"] {
            let mut req = make_request(Some(null), Some("example.com"));
            assert!(is_forbidden(validate_origin(&mut req)));
        }
    }

    #[test]
    fn same_origin_allows_request() {
        // HTTP, HTTPS, with port, case-insensitive
        let cases = [
            ("http://example.com", "example.com"),
            ("https://example.com", "example.com"),
            ("http://example.com:8080", "example.com:8080"),
            ("http://EXAMPLE.COM", "example.com"),
        ];
        for (origin, host) in cases {
            let mut req = make_request(Some(origin), Some(host));
            assert!(validate_origin(&mut req).is_ok(), "{origin} vs {host}");
        }
    }

    #[test]
    fn cross_origin_forbidden() {
        let cases = [
            ("http://unknown.com", "example.com"),         // different host
            ("http://example.com:8080", "example.com:80"), // different port
            ("ftp://example.com", "example.com"),          // non-http scheme
            ("not-a-valid-url", "example.com"),            // invalid URL
            ("http://example.com", ""),                    // missing host (invalid)
        ];
        for (origin, host) in cases {
            let host_opt = if host.is_empty() { None } else { Some(host) };
            let mut req = make_request(Some(origin), host_opt);
            assert!(is_forbidden(validate_origin(&mut req)), "{origin}");
        }
    }

    #[test]
    fn loopback_addresses_normalized_and_equivalent() {
        // All loopback forms normalize to "localhost"
        assert_eq!(
            OriginKey::from_origin("http://localhost:3000")
                .unwrap()
                .host,
            "localhost"
        );
        assert_eq!(
            OriginKey::from_origin("http://127.0.0.1:3000")
                .unwrap()
                .host,
            "localhost"
        );
        assert_eq!(
            OriginKey::from_origin("http://[::1]:3000").unwrap().host,
            "localhost"
        );

        // Cross-loopback requests should be allowed
        let mut req = make_request(Some("http://127.0.0.1:3000"), Some("[::1]:3000"));
        assert!(validate_origin(&mut req).is_ok());
    }

    #[test]
    fn default_ports_handled_correctly() {
        assert_eq!(
            OriginKey::from_origin("http://example.com").unwrap().port,
            80
        );
        assert_eq!(
            OriginKey::from_origin("https://example.com").unwrap().port,
            443
        );

        // Explicit default port matches implicit
        let mut req = make_request(Some("http://example.com:80"), Some("example.com"));
        assert!(validate_origin(&mut req).is_ok());
    }
}
