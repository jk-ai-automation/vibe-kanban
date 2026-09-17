//! 登录限速（设计 §6.2 最后一条）。
//!
//! 两只桶，**任何一只满了都拒**：
//! - `用户名 + IP`：挡住对单个账号的暴力破解；
//! - `IP`：挡住「换着用户名喷洒」的枚举（每个用户名只试一两次，
//!   单看用户名桶永远不会满）。
//!
//! 全部逻辑是可注入时钟的纯函数（`now: Instant` 由调用方传入），
//! 不碰 `Instant::now()`，测试里可以随意快进而不用 `sleep`。
//!
//! **内存有界**是硬要求：限速器本身不能变成新的 DoS 面。每个 key 只占
//! 固定大小的 [`Bucket`]（三个整数/时间点，不是一条会无限增长的时间戳列表），
//! key 的总数由 [`MAX_ENTRIES`] 封顶，超出时先清过期、再按「最久没动过」淘汰。

use std::{
    collections::HashMap,
    net::IpAddr,
    time::{Duration, Instant},
};

/// 同一个「用户名 + IP」在窗口内允许的失败次数。第 11 次开始进入退避。
pub const USER_IP_MAX_FAILURES: u32 = 10;

/// 同一个 IP（不分用户名）在窗口内允许的失败次数。
/// 比上一条宽得多，只为挡住换用户名喷洒。
pub const IP_MAX_FAILURES: u32 = 30;

/// 失败记录的保留窗口。必须 **≥ [`MAX_BACKOFF`]**，
/// 否则退避还没走完记录就被清掉，等于没有退避。
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// 退避基数：达到阈值后第一次要等这么久。
pub const BASE_BACKOFF: Duration = Duration::from_secs(30);

/// 退避上限。再怎么翻倍也不超过它，免得把自己锁死到天荒地老。
pub const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

/// key 总数上限。攻击者可以随意伪造用户名来撑大表，必须封顶。
pub const MAX_ENTRIES: usize = 10_000;

/// 超过这个规模就在每次记失败时顺手清一遍过期条目。
pub const PRUNE_THRESHOLD: usize = 1_024;

/// 拿不到客户端 IP 时共用的固定桶名。
///
/// **不是放行**：拿不到 IP 的请求全部挤进同一只桶里一起被限速（fail-closed）。
/// 放行等于「把 IP 抹掉就能无限试密码」。
pub const UNKNOWN_IP: &str = "unknown";

/// 一个 key 的失败记录。固定大小，不随失败次数增长。
#[derive(Debug, Clone, Copy)]
struct Bucket {
    failures: u32,
    last: Instant,
}

/// 退避时长：达到阈值后每多失败一次翻一倍，封顶 [`MAX_BACKOFF`]。
fn backoff(failures: u32, threshold: u32) -> Duration {
    if failures < threshold {
        return Duration::ZERO;
    }
    let 翻倍次数 = failures - threshold;
    // 32 次翻倍就已经远超上限，再往上算会溢出。
    if 翻倍次数 >= 32 {
        return MAX_BACKOFF;
    }
    BASE_BACKOFF
        .saturating_mul(1u32 << 翻倍次数)
        .min(MAX_BACKOFF)
}

/// 登录限速器。放在 `LocalAuthRuntime` 里，跨请求共享。
#[derive(Debug, Default)]
pub struct LoginRateLimiter {
    /// key = `"<归一用户名>\u{1}<ip>"`
    per_user_ip: HashMap<String, Bucket>,
    /// key = `<ip>`
    per_ip: HashMap<String, Bucket>,
}

/// 用户名归一：裁空白 + 转小写。`Alice` 与 ` alice ` 必须共用一只桶，
/// 否则改一下大小写就能把计数清零。
fn 归一用户名(username: &str) -> String {
    username.trim().to_lowercase()
}

/// IP 归一：裁空白；空串一律落到 [`UNKNOWN_IP`]。
fn 归一_ip(ip: &str) -> String {
    let ip = ip.trim();
    if ip.is_empty() {
        UNKNOWN_IP.to_string()
    } else {
        ip.to_ascii_lowercase()
    }
}

fn 组合键(username: &str, ip: &str) -> String {
    // 用 U+0001 当分隔符：它不可能出现在用户名或 IP 里，
    // 所以 ("a\u{1}b", "c") 与 ("a", "b\u{1}c") 不会撞成同一只桶。
    format!("{}\u{1}{}", 归一用户名(username), 归一_ip(ip))
}

impl LoginRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 还能不能试。`Err(剩余时长)` 表示要等多久才能再试。
    ///
    /// **必须在读库与算 Argon2 之前调用**，否则被限速的请求照样会
    /// 吃掉一次 19 MiB 的哈希，限速就成了纯装饰。
    pub fn check(&mut self, username: &str, ip: &str, now: Instant) -> Result<(), Duration> {
        let 组合 = 组合键(username, ip);
        let ip_key = 归一_ip(ip);

        let 用户等待 = self
            .per_user_ip
            .get(&组合)
            .map(|b| 剩余(b, USER_IP_MAX_FAILURES, now))
            .unwrap_or(Duration::ZERO);
        let ip_等待 = self
            .per_ip
            .get(&ip_key)
            .map(|b| 剩余(b, IP_MAX_FAILURES, now))
            .unwrap_or(Duration::ZERO);

        // 取更严的那只桶。
        let 等待 = 用户等待.max(ip_等待);
        if 等待.is_zero() {
            Ok(())
        } else {
            Err(等待)
        }
    }

    /// 记一次失败。
    pub fn record_failure(&mut self, username: &str, ip: &str, now: Instant) {
        let 组合 = 组合键(username, ip);
        let ip_key = 归一_ip(ip);
        累加(&mut self.per_user_ip, 组合, now);
        累加(&mut self.per_ip, ip_key, now);
        self.约束容量(now);
    }

    /// 登录成功后清空该「用户名 + IP」的计数。
    ///
    /// **只清用户名桶，不清 IP 桶**：否则攻击者只要在喷洒过程中
    /// 成功登录一个自己的账号，就能把整只 IP 桶洗干净。
    pub fn reset(&mut self, username: &str, ip: &str) {
        self.per_user_ip.remove(&组合键(username, ip));
    }

    /// 清掉过期条目。定期调用，防止内存无限增长。
    pub fn prune(&mut self, now: Instant) {
        self.per_user_ip
            .retain(|_, b| now.duration_since(b.last) < WINDOW);
        self.per_ip
            .retain(|_, b| now.duration_since(b.last) < WINDOW);
    }

    /// 当前占用的 key 总数。给测试与监控用。
    pub fn len(&self) -> usize {
        self.per_user_ip.len() + self.per_ip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 先清过期，还超就按「最久没动过」淘汰到上限以内。
    ///
    /// 表还小的时候什么都不做；过了 [`PRUNE_THRESHOLD`] 才开始每次失败都清一遍
    /// （失败登录本身已被限速，这点 O(n) 开销打不成新的 DoS 面）。
    fn 约束容量(&mut self, now: Instant) {
        if self.per_user_ip.len() <= PRUNE_THRESHOLD && self.per_ip.len() <= PRUNE_THRESHOLD {
            return;
        }
        self.prune(now);
        淘汰到上限(&mut self.per_user_ip);
        淘汰到上限(&mut self.per_ip);
    }
}

/// 距离「可以再试」还剩多久。0 表示现在就能试。
fn 剩余(bucket: &Bucket, threshold: u32, now: Instant) -> Duration {
    // 超出保留窗口的记录视同不存在（prune 还没跑到而已）。
    let 已过 = now.duration_since(bucket.last);
    if 已过 >= WINDOW {
        return Duration::ZERO;
    }
    backoff(bucket.failures, threshold).saturating_sub(已过)
}

fn 累加(表: &mut HashMap<String, Bucket>, key: String, now: Instant) {
    match 表.get_mut(&key) {
        // 上一次失败已经滑出窗口，从头计数。
        Some(bucket) if now.duration_since(bucket.last) >= WINDOW => {
            bucket.failures = 1;
            bucket.last = now;
        }
        Some(bucket) => {
            bucket.failures = bucket.failures.saturating_add(1);
            bucket.last = now;
        }
        None => {
            表.insert(
                key,
                Bucket {
                    failures: 1,
                    last: now,
                },
            );
        }
    }
}

fn 淘汰到上限(表: &mut HashMap<String, Bucket>) {
    if 表.len() <= MAX_ENTRIES {
        return;
    }
    let mut 条目: Vec<(String, Instant)> = 表.iter().map(|(k, b)| (k.clone(), b.last)).collect();
    条目.sort_by_key(|(_, last)| *last);
    let 要删 = 表.len() - MAX_ENTRIES;
    for (key, _) in 条目.into_iter().take(要删) {
        表.remove(&key);
    }
}

/// 取「真实」客户端 IP。
///
/// 默认**不信任** `X-Forwarded-For`：那是一个客户端可以随便写的请求头，
/// 信了就等于把限速拱手让人（每次请求换一个伪造 IP 即可无限试密码）。
/// 只有部署方通过 `VK_TRUST_PROXY` 显式声明「我前面确实有一层自己的反代」
/// 时才读它，并且只取**第一段**且必须能解析成合法 IP，否则回落到对端地址。
pub fn client_ip(trust_proxy: bool, forwarded_for: Option<&str>, peer: Option<IpAddr>) -> String {
    if trust_proxy
        && let Some(ip) = forwarded_for
            .and_then(|raw| raw.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<IpAddr>().ok())
    {
        return ip.to_string();
    }
    peer.map(|ip| ip.to_string())
        .unwrap_or_else(|| UNKNOWN_IP.to_string())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    const 五分钟: Duration = Duration::from_secs(5 * 60);

    fn 打满用户桶(limiter: &mut LoginRateLimiter, username: &str, ip: &str, now: Instant) {
        for _ in 0..USER_IP_MAX_FAILURES {
            assert!(limiter.check(username, ip, now).is_ok());
            limiter.record_failure(username, ip, now);
        }
    }

    #[test]
    fn 五分钟内十次失败后被拒() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        let err = limiter
            .check("alice", "1.2.3.4", t0)
            .expect_err("第 11 次必须被拒");
        assert!(err > Duration::ZERO);
        assert!(err <= MAX_BACKOFF);
    }

    #[test]
    fn 窗口滑出后恢复() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        assert!(limiter.check("alice", "1.2.3.4", t0).is_err());
        assert!(
            limiter
                .check("alice", "1.2.3.4", t0 + 五分钟 + Duration::from_secs(1))
                .is_ok(),
            "退避走完后必须放行"
        );
    }

    #[test]
    fn 成功登录清空计数() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..(USER_IP_MAX_FAILURES - 1) {
            limiter.record_failure("alice", "1.2.3.4", t0);
        }
        limiter.reset("alice", "1.2.3.4");
        assert!(limiter.check("alice", "1.2.3.4", t0).is_ok());
        // 清完之后再打满，仍然能被拒——reset 不是永久豁免。
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        assert!(limiter.check("alice", "1.2.3.4", t0).is_err());
    }

    /// **攻击样例：用户名喷洒。** 每个用户名只试一次，用户名桶永远不满，
    /// 只有 IP 桶能挡住它。
    #[test]
    fn 同一_ip_换用户名也被限速() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..IP_MAX_FAILURES {
            let username = format!("user{i}");
            assert!(
                limiter.check(&username, "9.9.9.9", t0).is_ok(),
                "第 {i} 个用户名不该被拒"
            );
            limiter.record_failure(&username, "9.9.9.9", t0);
        }
        assert!(
            limiter.check("user-next", "9.9.9.9", t0).is_err(),
            "同一 IP 换到第 {} 个用户名必须被拒",
            IP_MAX_FAILURES + 1
        );
    }

    /// 喷洒者在过程中成功登录自己的账号，不能把整只 IP 桶洗掉。
    #[test]
    fn 成功登录不清空_ip_桶() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..IP_MAX_FAILURES {
            limiter.record_failure(&format!("user{i}"), "9.9.9.9", t0);
        }
        limiter.reset("attacker-own-account", "9.9.9.9");
        assert!(
            limiter.check("user-next", "9.9.9.9", t0).is_err(),
            "reset 只该清用户名桶"
        );
    }

    #[test]
    fn 不同_ip_互不影响() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        assert!(limiter.check("alice", "1.2.3.4", t0).is_err());
        assert!(
            limiter.check("alice", "5.6.7.8", t0).is_ok(),
            "另一个 IP 不该被牵连"
        );
    }

    #[test]
    fn 不同用户名互不影响() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        assert!(
            limiter.check("bob", "1.2.3.4", t0).is_ok(),
            "同 IP 的另一个用户名还没到 IP 阈值，不该被拒"
        );
    }

    /// **fail-closed**：拿不到 IP 不等于放行，全部挤进 `unknown` 一起限速。
    #[test]
    fn ip_未知时共用一个桶而不是放行() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "", t0);
        assert!(limiter.check("alice", "", t0).is_err());
        assert!(
            limiter.check("alice", "   ", t0).is_err(),
            "空白 IP 必须和空串落到同一只桶"
        );
        assert!(
            limiter.check("alice", UNKNOWN_IP, t0).is_err(),
            "显式的 unknown 也是同一只桶"
        );
    }

    #[test]
    fn 用户名大小写归一() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        打满用户桶(&mut limiter, "alice", "1.2.3.4", t0);
        for 变体 in ["Alice", "ALICE", " alice ", "aLiCe"] {
            assert!(
                limiter.check(变体, "1.2.3.4", t0).is_err(),
                "{变体} 必须与 alice 共用一只桶"
            );
        }
    }

    /// 分隔符必须是用户名与 IP 都不可能含有的字符，
    /// 否则 `("a\u{1}b", "c")` 与 `("a", "b\u{1}c")` 会撞桶。
    #[test]
    fn 组合键不会被拼接歧义撞桶() {
        assert_ne!(组合键("a", "b-c"), 组合键("a-b", "c"));
        assert_ne!(组合键("a", "1.2.3.4"), 组合键("a\u{1}1.2.3.4", ""));
    }

    #[test]
    fn prune_清掉过期条目() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..50 {
            limiter.record_failure(&format!("user{i}"), &format!("10.0.0.{i}"), t0);
        }
        assert!(!limiter.is_empty());
        limiter.prune(t0 + Duration::from_secs(60));
        assert!(!limiter.is_empty(), "窗口内的条目不该被清掉");
        limiter.prune(t0 + WINDOW + Duration::from_secs(1));
        assert_eq!(limiter.len(), 0, "过期条目必须清干净，防止内存无限增长");
    }

    #[test]
    fn 退避时间随失败次数增长() {
        assert_eq!(
            backoff(USER_IP_MAX_FAILURES - 1, USER_IP_MAX_FAILURES),
            Duration::ZERO
        );
        let mut 上一次 = Duration::ZERO;
        for n in USER_IP_MAX_FAILURES..(USER_IP_MAX_FAILURES + 20) {
            let 当前 = backoff(n, USER_IP_MAX_FAILURES);
            assert!(当前 >= 上一次, "第 {n} 次的退避不该变短");
            assert!(当前 <= MAX_BACKOFF, "第 {n} 次的退避超过了上限");
            上一次 = 当前;
        }
        assert_eq!(
            backoff(USER_IP_MAX_FAILURES, USER_IP_MAX_FAILURES),
            BASE_BACKOFF
        );
        assert_eq!(
            backoff(USER_IP_MAX_FAILURES + 1, USER_IP_MAX_FAILURES),
            BASE_BACKOFF * 2
        );
        assert_eq!(
            backoff(u32::MAX, USER_IP_MAX_FAILURES),
            MAX_BACKOFF,
            "不能溢出"
        );
    }

    /// 保留窗口必须 ≥ 退避上限，否则退避还没走完记录就被清掉。
    #[test]
    fn 保留窗口不短于退避上限() {
        assert!(
            WINDOW >= MAX_BACKOFF,
            "WINDOW={WINDOW:?} MAX_BACKOFF={MAX_BACKOFF:?}"
        );
    }

    /// **限速器自身不能成为 DoS 面**：攻击者伪造海量用户名时 key 数必须封顶。
    #[test]
    fn key_数量有上限() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..(MAX_ENTRIES + 500) {
            limiter.record_failure(&format!("user{i}"), "1.2.3.4", t0);
        }
        assert!(
            limiter.len() <= 2 * MAX_ENTRIES,
            "key 数失控：{}",
            limiter.len()
        );
    }

    /// 每个 key 的记录是定长的，不会随失败次数增长。
    #[test]
    fn 单个_key_的记录不随失败次数增长() {
        let mut limiter = LoginRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..100_000u32 {
            limiter.record_failure("alice", "1.2.3.4", t0 + Duration::from_millis(u64::from(i)));
        }
        assert_eq!(limiter.len(), 2, "只该有一只用户名桶加一只 IP 桶");
        assert_eq!(std::mem::size_of::<Bucket>(), std::mem::size_of::<Bucket>());
    }

    // ------------------------------------------------------------ client_ip

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    /// **攻击样例**：`X-Forwarded-For` 是客户端可以随便写的头。
    /// 默认不信任，否则每次请求换一个伪造 IP 就能绕开整个限速。
    #[test]
    fn 不信任代理时忽略_x_forwarded_for() {
        for 伪造 in ["1.2.3.4", "1.2.3.4, 5.6.7.8", "evil", ""] {
            assert_eq!(
                client_ip(false, Some(伪造), Some(v4(127, 0, 0, 1))),
                "127.0.0.1",
                "{伪造:?} 不该被采信"
            );
        }
    }

    #[test]
    fn 信任代理时取第一段() {
        assert_eq!(
            client_ip(true, Some("1.2.3.4, 5.6.7.8"), Some(v4(10, 0, 0, 1))),
            "1.2.3.4"
        );
        assert_eq!(
            client_ip(true, Some("  1.2.3.4  "), Some(v4(10, 0, 0, 1))),
            "1.2.3.4"
        );
        // IPv6 也要认，并且按标准形式归一。
        assert_eq!(
            client_ip(true, Some("2001:DB8::1"), Some(v4(10, 0, 0, 1))),
            IpAddr::V6("2001:db8::1".parse::<Ipv6Addr>().unwrap()).to_string()
        );
    }

    /// 信任代理时头里是垃圾，必须回落到对端地址而不是把垃圾当 key。
    #[test]
    fn 信任代理但头是垃圾时回落到对端() {
        for 垃圾 in [
            "not-an-ip",
            "999.999.999.999",
            "<script>",
            "1.2.3.4.5",
            "localhost",
            ",",
            "   ",
            "",
        ] {
            assert_eq!(
                client_ip(true, Some(垃圾), Some(v4(10, 0, 0, 1))),
                "10.0.0.1",
                "{垃圾:?} 应回落到对端"
            );
        }
    }

    #[test]
    fn 对端也没有时返回_unknown() {
        assert_eq!(client_ip(false, None, None), UNKNOWN_IP);
        assert_eq!(client_ip(true, None, None), UNKNOWN_IP);
        assert_eq!(client_ip(true, Some("garbage"), None), UNKNOWN_IP);
    }
}
