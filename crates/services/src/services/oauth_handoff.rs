//! 云端 OAuth handoff 的待完成记录。
//!
//! # 这个模块在防什么
//!
//! `GET /api/auth/handoff/complete` 是一条**有副作用的 GET**：它拿 URL 里的
//! `handoff_id` + `app_code` 去云端换令牌，然后写凭据文件、改配置、拉起 relay
//! 与远端同步。会话 Cookie 是 `SameSite=Lax`（OAuth 回调是跨站顶层导航，
//! 必须用 Lax），而 Origin 强校验只覆盖写方法与 WebSocket 升级，**盖不住它**。
//!
//! 原有的唯一绑定是 PKCE 式的 `app_verifier`：它只存在发起方那台服务器的内存里，
//! 所以别人机器上发起的 handoff 换不动本机的凭据（见 `take` 的 `Unknown` 分支）。
//! 但那是「绑定到服务器进程」，不是「绑定到发起登录的那个浏览器」。于是本模块
//! 再加一道：
//!
//! - **nonce 绑定**：`handoff_init` 时生成一次性 nonce，通过 `Set-Cookie`
//!   （HttpOnly、SameSite=Lax、短有效期）下发给发起方浏览器；`complete` 时
//!   比对。`handoff_id` 与 `app_code` 会留在浏览器历史、代理日志、录屏里，
//!   nonce 不会——泄漏了 URL 也换不动凭据。
//! - **短 TTL**：原实现的待完成记录**永不过期也永不清理**，既是无界内存增长，
//!   也把可重放窗口拉到进程生命周期那么长。
//! - **一次性消费**：`take` 成功后记录即被移除，且**任何**取用（含失败）都会
//!   顺手清掉已过期的记录。
//!
//! # 明文 nonce 的去向
//!
//! 只出现在两个地方：生成它的那一刻，和发给浏览器的 `Set-Cookie` 头。
//! 这里只存 SHA-256。本模块不做任何 `tracing` 输出，就是为了不给它留出口。

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::services::server_settings::ServerMode;

/// 待完成 handoff 的存活时间。
///
/// 一次 OAuth 授权（选账号、输密码、过二次验证）十分钟绰绰有余；
/// 再长就只是在给重放留窗口。云端那侧的 handoff 也有自己的 TTL，
/// 两者取严即可，本机这一侧不需要跟云端对齐。
pub const HANDOFF_TTL_MINUTES: i64 = 10;

/// nonce 熵：32 字节，base64url 无填充后 43 个字符。
const NONCE_BYTES: usize = 32;

/// 取用失败的原因。
///
/// **会进日志与响应体，因此必须是编译期常量**：不能拼进 `handoff_id`、
/// nonce、`app_code` 或任何其它用户可控内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffRejection {
    /// 团队模式不支持云端链路（设计 §8.7）。
    DisabledInTeamMode,
    /// 本进程没有这条记录：别的机器上发起的，或者已经被消费过。
    Unknown,
    /// 记录还在但已超过 [`HANDOFF_TTL_MINUTES`]。
    Expired,
    /// 请求没带 nonce Cookie。最典型的跨站伪造回调。
    MissingNonce,
    /// 带了 nonce 但与记录不符。
    NonceMismatch,
}

impl HandoffRejection {
    /// 进日志用的固定 slug。
    pub fn as_str(self) -> &'static str {
        match self {
            HandoffRejection::DisabledInTeamMode => "handoff_disabled_in_team_mode",
            HandoffRejection::Unknown => "handoff_unknown",
            HandoffRejection::Expired => "handoff_expired",
            HandoffRejection::MissingNonce => "handoff_missing_nonce",
            HandoffRejection::NonceMismatch => "handoff_nonce_mismatch",
        }
    }

    /// 给用户看的文案。
    ///
    /// 四种失败**故意共用同一句话**：区分「不存在 / 过期 / nonce 不对」等于
    /// 告诉攻击者他猜对了哪一半。团队模式那条单独说，因为它不是攻击信号，
    /// 而是一个需要向管理员解释的部署事实。
    pub fn user_message(self) -> &'static str {
        match self {
            HandoffRejection::DisabledInTeamMode => {
                "Cloud sign-in is not available on this deployment."
            }
            _ => "This sign-in link is no longer valid. Please start over from the app.",
        }
    }
}

/// 团队模式整组关掉云端 handoff。
///
/// 设计 §8.7 已明确「团队模式不支持云端 relay」，relay 请求在团队模式一律被拒；
/// 云端 OAuth handoff 是同一条链路的入口，必须一并关掉。
///
/// 不关会怎样：团队服务器上**任何**成员（包括非管理员）都能跑一遍
/// init + complete，把**自己的**云端账号凭据写进共享主机——随后
/// `sync_all_linked_workspaces` 会把团队的工作区同步进攻击者的云端账号，
/// `spawn_relay` 会用攻击者的身份把这台主机开成公网隧道。
pub fn cloud_handoff_allowed(mode: ServerMode) -> Result<(), HandoffRejection> {
    match mode {
        ServerMode::Personal => Ok(()),
        ServerMode::Team => Err(HandoffRejection::DisabledInTeamMode),
    }
}

/// 回调该不该要求 nonce Cookie。**只在 `handoff_init` 时决定**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonceBinding {
    /// 回调会落回**同一个**浏览器（网页版：`window.open` 开的弹窗与主窗口
    /// 共用 Cookie jar）。必须带上 nonce。
    Required(String),
    /// 回调会落进**系统浏览器**（Tauri 桌面版把新窗口请求交给系统浏览器打开，
    /// 见 `crates/tauri-app/src/main.rs` 的 `on_new_window`）。系统浏览器与
    /// 应用内 webview 不是同一个 Cookie jar，拿不到 init 时下发的 Cookie，
    /// 强要 nonce 会把桌面版登录 100% 打死。
    ///
    /// **这个豁免只能由 `handoff_init` 的请求体决定**（同源 + JSON，
    /// 跨站构造不出来），**绝不能**由 `complete` 的 query 决定——
    /// 那是攻击者可控的，等于把整道门做成一个开关。
    /// 被豁免的记录退回到原有的安全水位（仍需正确的 `handoff_id`
    /// 与只存在本进程的 `app_verifier`），不比现状更弱。
    SystemBrowser,
}

impl NonceBinding {
    fn hash(&self) -> Option<String> {
        match self {
            NonceBinding::Required(nonce) => Some(hash_nonce(nonce)),
            NonceBinding::SystemBrowser => None,
        }
    }
}

/// 取用成功后交还给调用方的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHandoff {
    pub provider: String,
    pub app_verifier: String,
}

#[derive(Debug, Clone)]
struct Entry {
    provider: String,
    app_verifier: String,
    /// `None` = [`NonceBinding::SystemBrowser`]。
    nonce_hash: Option<String>,
    expires_at: DateTime<Utc>,
}

/// 待完成 handoff 的内存表。
#[derive(Debug, Default)]
pub struct HandoffStore {
    entries: RwLock<HashMap<Uuid, Entry>>,
}

impl HandoffStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记下一条待完成的 handoff。`now` 由调用方传入，测试才能控制时间。
    pub async fn insert(
        &self,
        handoff_id: Uuid,
        provider: String,
        app_verifier: String,
        binding: &NonceBinding,
        now: DateTime<Utc>,
    ) {
        let entry = Entry {
            provider,
            app_verifier,
            nonce_hash: binding.hash(),
            expires_at: now + Duration::minutes(HANDOFF_TTL_MINUTES),
        };
        let mut entries = self.entries.write().await;
        // 顺手做一次清扫：没有后台任务，过期记录只能靠每次取用带走，
        // 否则被放弃的登录会在内存里一直堆着。
        entries.retain(|_, e| e.expires_at > now);
        entries.insert(handoff_id, entry);
    }

    /// 消费一条 handoff。
    ///
    /// `presented_nonce` 是请求 Cookie 里的 nonce（没有就是 `None`）。
    /// 成功即一次性消费：同一条记录第二次取用必然是 [`HandoffRejection::Unknown`]。
    ///
    /// **校验失败时记录也会被移除**：留着只会让攻击者拿同一个 `handoff_id`
    /// 反复试 nonce。正常用户重新点一次登录即可。
    pub async fn take(
        &self,
        handoff_id: &Uuid,
        presented_nonce: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<PendingHandoff, HandoffRejection> {
        let mut entries = self.entries.write().await;
        entries.retain(|_, e| e.expires_at > now);

        let entry = entries
            .remove(handoff_id)
            .ok_or(HandoffRejection::Unknown)?;

        if entry.expires_at <= now {
            return Err(HandoffRejection::Expired);
        }

        if let Some(expected_hash) = entry.nonce_hash.as_deref() {
            let presented = presented_nonce
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or(HandoffRejection::MissingNonce)?;
            if !constant_time_eq(&hash_nonce(presented), expected_hash) {
                return Err(HandoffRejection::NonceMismatch);
            }
        }

        Ok(PendingHandoff {
            provider: entry.provider,
            app_verifier: entry.app_verifier,
        })
    }

    /// [`Self::insert`] 的「用当前时间」版本，供生产调用方使用。
    pub async fn insert_now(
        &self,
        handoff_id: Uuid,
        provider: String,
        app_verifier: String,
        binding: &NonceBinding,
    ) {
        self.insert(handoff_id, provider, app_verifier, binding, Utc::now())
            .await;
    }

    /// [`Self::take`] 的「用当前时间」版本，供生产调用方使用。
    pub async fn take_now(
        &self,
        handoff_id: &Uuid,
        presented_nonce: Option<&str>,
    ) -> Result<PendingHandoff, HandoffRejection> {
        self.take(handoff_id, presented_nonce, Utc::now()).await
    }

    /// 当前记录条数。只给测试用。
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

/// 生成一次性 nonce：32 字节操作系统随机数，base64url 无填充。
///
/// 字符集是 `[A-Za-z0-9_-]`，天然不含 `;` `=` `,` 与空白，
/// 拼进 `Set-Cookie` 不会产生属性注入。
pub fn generate_handoff_nonce() -> String {
    let mut bytes = [0u8; NONCE_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_nonce(nonce: &str) -> String {
    let digest = Sha256::digest(nonce.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 常量时间比较。
///
/// 比的是两个 SHA-256 十六进制串，长度恒为 64，所以长度短路这条分支
/// 在正常路径上永远不会命中；留着是因为 `ct_eq` 要求等长切片，
/// 而长度本身不是秘密。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 此刻() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn 绑定(nonce: &str) -> NonceBinding {
        NonceBinding::Required(nonce.to_string())
    }

    async fn 存一条(store: &HandoffStore, id: Uuid, binding: &NonceBinding, now: DateTime<Utc>) {
        store
            .insert(id, "google".into(), "verifier-abc".into(), binding, now)
            .await;
    }

    // ------------------------------------------------ 团队模式：整组不可达

    #[test]
    fn 团队模式禁用云端_handoff() {
        assert_eq!(
            cloud_handoff_allowed(ServerMode::Team),
            Err(HandoffRejection::DisabledInTeamMode)
        );
    }

    #[test]
    fn 个人模式允许云端_handoff() {
        assert_eq!(cloud_handoff_allowed(ServerMode::Personal), Ok(()));
    }

    /// 穷举，免得以后加了第三种模式默认落进「允许」。
    #[test]
    fn 模式判定全覆盖() {
        for (mode, 期望) in [
            (ServerMode::Personal, Ok(())),
            (ServerMode::Team, Err(HandoffRejection::DisabledInTeamMode)),
        ] {
            assert_eq!(cloud_handoff_allowed(mode), 期望, "{mode:?}");
        }
    }

    // ------------------------------------------------ 个人模式：正常流程

    /// 硬要求：个人模式下 init → complete 必须照常走通，
    /// 并且原样交还 provider 与 app_verifier（后面要拿去云端 redeem）。
    #[tokio::test]
    async fn 个人模式正常流程成功() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();

        存一条(&store, id, &绑定(&nonce), 此刻()).await;

        let got = store.take(&id, Some(&nonce), 此刻()).await.unwrap();
        assert_eq!(
            got,
            PendingHandoff {
                provider: "google".into(),
                app_verifier: "verifier-abc".into(),
            }
        );
    }

    /// TTL 内的任意时刻都应成功，边界上（刚好 10 分钟差一秒）也算成功。
    #[tokio::test]
    async fn ttl_之内都成功() {
        for 秒 in [0, 1, 60, HANDOFF_TTL_MINUTES * 60 - 1] {
            let store = HandoffStore::new();
            let id = Uuid::new_v4();
            let nonce = generate_handoff_nonce();
            存一条(&store, id, &绑定(&nonce), 此刻()).await;
            assert!(
                store
                    .take(&id, Some(&nonce), 此刻() + Duration::seconds(秒))
                    .await
                    .is_ok(),
                "{秒} 秒后应仍然有效"
            );
        }
    }

    /// 桌面版（系统浏览器）拿不到 Cookie，必须放行——否则桌面登录全挂。
    #[tokio::test]
    async fn 系统浏览器绑定不要求_nonce() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        存一条(&store, id, &NonceBinding::SystemBrowser, 此刻()).await;
        assert!(store.take(&id, None, 此刻()).await.is_ok());
    }

    /// 但豁免不免 TTL，也不免一次性消费。
    #[tokio::test]
    async fn 系统浏览器绑定仍受_ttl_与一次性约束() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        存一条(&store, id, &NonceBinding::SystemBrowser, 此刻()).await;
        assert!(store.take(&id, None, 此刻()).await.is_ok());
        assert_eq!(
            store.take(&id, None, 此刻()).await,
            Err(HandoffRejection::Unknown)
        );

        let id2 = Uuid::new_v4();
        存一条(&store, id2, &NonceBinding::SystemBrowser, 此刻()).await;
        assert_eq!(
            store
                .take(
                    &id2,
                    None,
                    此刻() + Duration::minutes(HANDOFF_TTL_MINUTES + 1)
                )
                .await,
            Err(HandoffRejection::Unknown)
        );
    }

    // ------------------------------------------------ 攻击样例

    /// **攻击样例 1**：跨站页面把受害者导航到 `complete?handoff_id=…&app_code=…`。
    /// 浏览器会带上 `SameSite=Lax` 的 Cookie，但攻击者的链接**没有**
    /// 对应的 nonce Cookie（它是 init 时才下发的）。必须拒。
    #[tokio::test]
    async fn 无_nonce_cookie_被拒() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        存一条(&store, id, &绑定(&generate_handoff_nonce()), 此刻()).await;
        assert_eq!(
            store.take(&id, None, 此刻()).await,
            Err(HandoffRejection::MissingNonce)
        );
    }

    /// 空值 / 纯空白的 Cookie 等同于没有，**不能**当成「带了一个空 nonce」
    /// 然后去和空哈希比——那样送个 `vk_handoff=` 就过了。
    #[tokio::test]
    async fn 空_nonce_当作没有() {
        for 空 in ["", "   ", "\t", "\n"] {
            let store = HandoffStore::new();
            let id = Uuid::new_v4();
            存一条(&store, id, &绑定(&generate_handoff_nonce()), 此刻()).await;
            assert_eq!(
                store.take(&id, Some(空), 此刻()).await,
                Err(HandoffRejection::MissingNonce),
                "{空:?}"
            );
        }
    }

    /// **攻击样例 2**：攻击者猜 / 塞一个自己的 nonce。
    #[tokio::test]
    async fn nonce_不匹配被拒() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let 真 = generate_handoff_nonce();
        存一条(&store, id, &绑定(&真), 此刻()).await;
        assert_eq!(
            store
                .take(&id, Some(&generate_handoff_nonce()), 此刻())
                .await,
            Err(HandoffRejection::NonceMismatch)
        );
    }

    /// 只差一个字符、以及长度不同的 nonce，都必须拒。
    #[tokio::test]
    async fn nonce_近似值被拒() {
        let 真 = generate_handoff_nonce();
        let mut 改一位 = 真.clone();
        let 末位 = 改一位.pop().unwrap();
        改一位.push(if 末位 == 'A' { 'B' } else { 'A' });

        for 假 in [
            改一位,
            真[..真.len() - 1].to_string(),
            format!("{真}A"),
            真.to_uppercase(),
        ] {
            if 假 == 真 {
                continue;
            }
            let store = HandoffStore::new();
            let id = Uuid::new_v4();
            存一条(&store, id, &绑定(&真), 此刻()).await;
            assert_eq!(
                store.take(&id, Some(&假), 此刻()).await,
                Err(HandoffRejection::NonceMismatch),
                "{假:?}"
            );
        }
    }

    /// **攻击样例 3**：拿一个过期的（被放弃的）登录记录来重放。
    #[tokio::test]
    async fn nonce_过期被拒() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        存一条(&store, id, &绑定(&nonce), 此刻()).await;

        // 过期记录先被清扫掉，因此报的是 Unknown（对外文案与其它失败一致）。
        assert_eq!(
            store
                .take(
                    &id,
                    Some(&nonce),
                    此刻() + Duration::minutes(HANDOFF_TTL_MINUTES) + Duration::seconds(1)
                )
                .await,
            Err(HandoffRejection::Unknown)
        );
    }

    /// 恰好到期的那一秒就算过期（`expires_at > now` 是严格大于）。
    #[tokio::test]
    async fn 恰好到期算过期() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        存一条(&store, id, &绑定(&nonce), 此刻()).await;
        assert!(
            store
                .take(
                    &id,
                    Some(&nonce),
                    此刻() + Duration::minutes(HANDOFF_TTL_MINUTES)
                )
                .await
                .is_err()
        );
    }

    /// **攻击样例 4**：nonce 已被正常流程消费，攻击者重放同一个 URL。
    #[tokio::test]
    async fn nonce_已消费不可重放() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        存一条(&store, id, &绑定(&nonce), 此刻()).await;

        assert!(store.take(&id, Some(&nonce), 此刻()).await.is_ok());
        assert_eq!(
            store.take(&id, Some(&nonce), 此刻()).await,
            Err(HandoffRejection::Unknown)
        );
    }

    /// **攻击样例 5**：攻击者在**自己**机器上发起 handoff，拿到
    /// `handoff_id` + `app_code` 后诱导受害者访问。受害者这台服务器上
    /// 根本没有这条记录（`app_verifier` 只存在发起方进程内存里）。
    /// 这条是既有防线，用测试钉住，防止以后把 store 改成全局共享。
    #[tokio::test]
    async fn 别处发起的_handoff_在本机无记录() {
        let store = HandoffStore::new();
        assert_eq!(
            store.take(&Uuid::new_v4(), Some("whatever"), 此刻()).await,
            Err(HandoffRejection::Unknown)
        );
    }

    /// 一次失败的尝试也会带走记录，攻击者不能拿同一个 id 反复试 nonce。
    #[tokio::test]
    async fn 失败尝试也会消费掉记录() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        存一条(&store, id, &绑定(&nonce), 此刻()).await;

        assert_eq!(
            store.take(&id, Some("wrong"), 此刻()).await,
            Err(HandoffRejection::NonceMismatch)
        );
        // 即使后来送对了 nonce 也没用了。
        assert_eq!(
            store.take(&id, Some(&nonce), 此刻()).await,
            Err(HandoffRejection::Unknown)
        );
    }

    // ------------------------------------------------ 内存与不泄漏

    /// 过期记录不会无限堆积：原实现没有任何清理，被放弃的登录会一直占内存。
    #[tokio::test]
    async fn 过期记录会被清扫() {
        let store = HandoffStore::new();
        for _ in 0..50 {
            存一条(&store, Uuid::new_v4(), &NonceBinding::SystemBrowser, 此刻()).await;
        }
        assert_eq!(store.len().await, 50);

        // 一次晚到的取用就把全部过期记录带走。
        let _ = store
            .take(
                &Uuid::new_v4(),
                None,
                此刻() + Duration::minutes(HANDOFF_TTL_MINUTES + 1),
            )
            .await;
        assert!(store.is_empty().await);
    }

    /// 拒绝原因是编译期常量，绝不含 nonce / handoff_id / app_code。
    #[test]
    fn 拒绝原因是固定常量() {
        for r in [
            HandoffRejection::DisabledInTeamMode,
            HandoffRejection::Unknown,
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            let slug = r.as_str();
            assert!(slug.is_ascii() && !slug.is_empty());
            assert!(
                slug.starts_with("handoff_"),
                "{slug} 应带统一前缀，方便日志检索"
            );
        }
    }

    /// 对外文案不区分四种失败——区分等于告诉攻击者猜对了哪一半。
    #[test]
    fn 四种失败共用同一句对外文案() {
        let 基准 = HandoffRejection::Unknown.user_message();
        for r in [
            HandoffRejection::Unknown,
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            assert_eq!(r.user_message(), 基准, "{r:?} 不该有独立文案");
        }
        assert_ne!(
            HandoffRejection::DisabledInTeamMode.user_message(),
            基准,
            "团队模式是部署事实，应单独说明"
        );
    }

    /// 文案里不能出现任何看起来像凭据的东西。
    #[test]
    fn 对外文案不含敏感词() {
        for r in [
            HandoffRejection::DisabledInTeamMode,
            HandoffRejection::Unknown,
            HandoffRejection::Expired,
            HandoffRejection::MissingNonce,
            HandoffRejection::NonceMismatch,
        ] {
            let msg = r.user_message().to_ascii_lowercase();
            for 敏感 in [
                "nonce",
                "token",
                "verifier",
                "app_code",
                "handoff_id",
                "cookie",
            ] {
                assert!(!msg.contains(敏感), "{r:?} 的文案泄漏了 {敏感}");
            }
        }
    }

    // ------------------------------------------------ nonce 本身

    #[test]
    fn nonce_长度与字符集() {
        let nonce = generate_handoff_nonce();
        assert_eq!(nonce.len(), 43, "32 字节 base64url 无填充 = 43 字符");
        assert!(
            nonce
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{nonce} 含有会破坏 Set-Cookie 的字符"
        );
    }

    #[test]
    fn nonce_每次都不同() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            assert!(seen.insert(generate_handoff_nonce()), "nonce 出现重复");
        }
    }

    /// 存的是哈希不是明文：内存里捞到记录也换不出可用的 nonce。
    #[tokio::test]
    async fn 记录里存的是哈希() {
        let store = HandoffStore::new();
        let id = Uuid::new_v4();
        let nonce = generate_handoff_nonce();
        存一条(&store, id, &绑定(&nonce), 此刻()).await;

        let entries = store.entries.read().await;
        let hash = entries[&id].nonce_hash.as_deref().unwrap();
        assert_ne!(hash, nonce);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, &hash_nonce(&nonce));
    }

    // ------------------------------------------------ 常量时间比较

    #[test]
    fn 常量时间比较的正反例() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ABC"));
        // 长度不同先短路：长度不是秘密（哈希恒为 64 字符）。
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("abcd", "abc"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    /// 等长输入必须走满整个比较，不能在第一个不同字节处提前返回。
    /// 纯函数测不了时序，这里退而求其次：钉住「等长时结果只由内容决定」，
    /// 并确认前缀相同、仅末位不同的两串也判否（即没有做前缀短路）。
    #[test]
    fn 等长比较不做前缀短路() {
        let a = "a".repeat(64);
        let mut b = a.clone();
        b.pop();
        b.push('b');
        assert!(!constant_time_eq(&a, &b));

        let mut c = a.clone();
        c.remove(0);
        c.insert(0, 'b');
        assert!(!constant_time_eq(&a, &c));
    }
}
