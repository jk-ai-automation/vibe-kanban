//! 本地账号体系的会话 / 第三方身份 / 邀请码模型。
//!
//! 表结构见 `migrations/20260917000000_add_local_auth.sql`。
//!
//! 安全约定：
//! - 库里只存会话令牌的 SHA-256（`token_hash`），明文只发给浏览器。
//! - [`LocalSession`] 上**没有** `token_hash` 字段，会话对象不可能把哈希序列化出去。
//! - 「这张 Cookie 还能不能用」的全部判据都写在
//!   [`LocalSessions::find_valid_by_token_hash`] 的 SQL 里（未撤销、未过期、
//!   且用户仍是 `active`），不依赖调用方记得逐条检查。

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::db_retry::retry_on_busy;

/// `last_seen_at` 的写库节流窗口：距上次不足这个时长就不写。
/// 每请求写一次会让只读页面也产生写锁争抢。
pub const LAST_SEEN_THROTTLE: Duration = Duration::minutes(5);

/// User-Agent / IP 的存储长度上限。请求头由客户端完全控制，
/// 不设上限就等于让任何人往库里塞任意大的字符串。
pub const MAX_USER_AGENT_LEN: usize = 256;
pub const MAX_IP_LEN: usize = 64;

/// 过期会话清理任务的间隔。启动时先清一次，之后按这个间隔循环。
///
/// 单位是**小时**不是秒：写成 `from_secs(6)` 会让服务器每 6 秒扫一遍全表。
/// 有一条测试把它钉死。
pub const SESSION_CLEANUP_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(6 * 60 * 60);

/// 会话行。**不含 `token_hash`**，见模块文档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// 建会话入参。`token_hash` 由 `services::local_auth::token::hash_session_token`
/// 算好后传进来，本层不接触明文令牌。
#[derive(Debug, Clone)]
pub struct NewLocalSession {
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

pub struct LocalSessions;

impl LocalSessions {
    pub async fn create(
        pool: &SqlitePool,
        data: NewLocalSession,
    ) -> Result<LocalSession, sqlx::Error> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let user_agent = truncate(data.user_agent.as_deref(), MAX_USER_AGENT_LEN);
        let ip = truncate(data.ip.as_deref(), MAX_IP_LEN);
        let token_hash = data.token_hash;
        let user_id = data.user_id;
        let expires_at = data.expires_at;

        retry_on_busy(|| {
            let token_hash = token_hash.clone();
            let user_agent = user_agent.clone();
            let ip = ip.clone();
            async move {
                sqlx::query_as!(
                    LocalSession,
                    r#"INSERT INTO local_sessions
                           (id, user_id, token_hash, created_at, expires_at, last_seen_at,
                            user_agent, ip)
                       VALUES ($1, $2, $3, $4, $5, $4, $6, $7)
                       RETURNING id           AS "id!: Uuid",
                                 user_id      AS "user_id!: Uuid",
                                 created_at   AS "created_at!: DateTime<Utc>",
                                 expires_at   AS "expires_at!: DateTime<Utc>",
                                 last_seen_at AS "last_seen_at!: DateTime<Utc>""#,
                    id,
                    user_id,
                    token_hash,
                    now,
                    expires_at,
                    user_agent,
                    ip
                )
                .fetch_one(pool)
                .await
            }
        })
        .await
    }

    /// 按令牌哈希取一条**当前可用**的会话。
    ///
    /// 四条判据全部写在 SQL 里：哈希精确相等、未撤销、未过期、用户是 `active`。
    /// 时间比较用 RFC3339 字符串——迁移里所有时间戳都是这个格式，
    /// sqlx 绑定 `DateTime<Utc>` 也是这个格式，因此字典序即时间序。
    pub async fn find_valid_by_token_hash(
        pool: &SqlitePool,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<LocalSession>, sqlx::Error> {
        sqlx::query_as!(
            LocalSession,
            r#"SELECT s.id           AS "id!: Uuid",
                      s.user_id      AS "user_id!: Uuid",
                      s.created_at   AS "created_at!: DateTime<Utc>",
                      s.expires_at   AS "expires_at!: DateTime<Utc>",
                      s.last_seen_at AS "last_seen_at!: DateTime<Utc>"
               FROM local_sessions s
               JOIN local_users u ON u.id = s.user_id
               WHERE s.token_hash = $1
                 AND s.revoked_at IS NULL
                 AND s.expires_at > $2
                 AND u.status = 'active'"#,
            token_hash,
            now
        )
        .fetch_optional(pool)
        .await
    }

    /// 刷新 `last_seen_at`，距上次不足 [`LAST_SEEN_THROTTLE`] 时不写库。
    ///
    /// **滑动续期的语义就到此为止：只推进 `last_seen_at`，不延长 `expires_at`。**
    /// `expires_at` 保持登录那一刻定下的绝对时刻，好处是「撤销」「过期」两件事
    /// 都能从一行数据直接推理出来；真要做无限续期，得另加写库节流并重新论证
    /// 「一张永不过期的 Cookie 被偷了怎么办」。（计划 §12 第 18 条的结论。）
    ///
    /// 返回 `true` 表示这次确实写了库。已撤销 / 已过期的会话一律不刷新。
    pub async fn touch_last_seen(
        pool: &SqlitePool,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let cutoff = now - LAST_SEEN_THROTTLE;
        let done = retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE local_sessions
                   SET last_seen_at = $2
                   WHERE id = $1
                     AND revoked_at IS NULL
                     AND expires_at > $2
                     AND last_seen_at <= $3"#,
                id,
                now,
                cutoff
            )
            .execute(pool)
            .await
        })
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// 撤销一条会话。返回 `true` 表示本次确实改动了一行（重复撤销返回 `false`）。
    pub async fn revoke(
        pool: &SqlitePool,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE local_sessions SET revoked_at = $2
                   WHERE id = $1 AND revoked_at IS NULL"#,
                id,
                now
            )
            .execute(pool)
            .await
        })
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// 撤销某个用户的全部会话（改密、停用、管理员重置密码时用）。返回撤销条数。
    pub async fn revoke_all_for_user(
        pool: &SqlitePool,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let done = retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE local_sessions SET revoked_at = $2
                   WHERE user_id = $1 AND revoked_at IS NULL"#,
                user_id,
                now
            )
            .execute(pool)
            .await
        })
        .await?;
        Ok(done.rows_affected())
    }

    /// 同 [`Self::revoke_all_for_user`]，但保留一条（自己改密时不把自己踢下线）。
    pub async fn revoke_all_for_user_except(
        pool: &SqlitePool,
        user_id: Uuid,
        keep_session_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let done = retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE local_sessions SET revoked_at = $3
                   WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL"#,
                user_id,
                keep_session_id,
                now
            )
            .execute(pool)
            .await
        })
        .await?;
        Ok(done.rows_affected())
    }

    /// 物理删除已过期的会话行（启动时与定时任务调用）。返回删除条数。
    pub async fn delete_expired(pool: &SqlitePool, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let done = retry_on_busy(|| async move {
            sqlx::query!(r#"DELETE FROM local_sessions WHERE expires_at <= $1"#, now)
                .execute(pool)
                .await
        })
        .await?;
        Ok(done.rows_affected())
    }

    /// 同 [`Self::delete_expired`]，但用当前时刻。
    ///
    /// 给后台清理任务用：`crates/local-deployment` 不依赖 chrono，
    /// 与其为了一句 `Utc::now()` 给它加一个依赖，不如把时间源留在本层。
    pub async fn delete_expired_now(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
        Self::delete_expired(pool, Utc::now()).await
    }
}

fn truncate(raw: Option<&str>, max_chars: usize) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().take(max_chars).collect())
}

#[cfg(test)]
mod session_tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::{
        models::{
            local_project::DEFAULT_USER_ID,
            local_user::{LocalUserRole, LocalUserStatus, LocalUsers, NewLocalUser},
        },
        test_support::TestDb,
    };

    async fn 建用户(test_db: &TestDb, username: &str) -> Uuid {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: None,
                password_hash: Some("$argon2id$x".to_string()),
                role: LocalUserRole::Member,
            },
        )
        .await
        .expect("建用户失败")
        .id
    }

    async fn 建会话(test_db: &TestDb, user_id: Uuid, hash: &str, ttl: Duration) -> LocalSession {
        LocalSessions::create(
            test_db.pool(),
            NewLocalSession {
                user_id,
                token_hash: hash.to_string(),
                expires_at: Utc::now() + ttl,
                user_agent: Some("测试浏览器".to_string()),
                ip: Some("127.0.0.1".to_string()),
            },
        )
        .await
        .expect("建会话失败")
    }

    #[tokio::test]
    async fn 创建会话后可按令牌哈希查到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;

        let found = LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-a", Utc::now())
            .await
            .unwrap()
            .expect("应命中");
        assert_eq!(found.id, session.id);
        assert_eq!(found.user_id, user_id);
    }

    #[tokio::test]
    async fn 伪造的令牌哈希查不到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;

        for forged in ["hash-b", "hash-", "hash-a ", "HASH-A", "", "%"] {
            assert!(
                LocalSessions::find_valid_by_token_hash(test_db.pool(), forged, Utc::now())
                    .await
                    .unwrap()
                    .is_none(),
                "{forged:?} 不应命中任何会话"
            );
        }
    }

    #[tokio::test]
    async fn 过期会话查不到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        建会话(&test_db, user_id, "hash-a", -Duration::hours(1)).await;

        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-a", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
    }

    /// 过期判定用的是 RFC3339 字符串比较，必须在「带小数秒」的时间戳上也成立。
    #[tokio::test]
    async fn 过期判定精确到亚秒() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let expires_at = Utc::now() + Duration::milliseconds(500);
        LocalSessions::create(
            test_db.pool(),
            NewLocalSession {
                user_id,
                token_hash: "hash-ms".to_string(),
                expires_at,
                user_agent: None,
                ip: None,
            },
        )
        .await
        .unwrap();

        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                "hash-ms",
                expires_at - Duration::milliseconds(1)
            )
            .await
            .unwrap()
            .is_some(),
            "到期前 1 毫秒仍应有效"
        );
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-ms", expires_at)
                .await
                .unwrap()
                .is_none(),
            "到期时刻应失效（判定是严格大于）"
        );
    }

    #[tokio::test]
    async fn 已撤销会话查不到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;

        assert!(
            LocalSessions::revoke(test_db.pool(), session.id, Utc::now())
                .await
                .unwrap()
        );
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-a", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
        // 重复撤销是幂等的，返回 false 表示「本次没有改动任何行」。
        assert!(
            !LocalSessions::revoke(test_db.pool(), session.id, Utc::now())
                .await
                .unwrap()
        );
    }

    /// 停用用户后，他手里那张仍未过期的 Cookie 必须立刻失效。
    /// 这条判定放在 SQL 里（JOIN local_users 且要求 status='active'），
    /// 不依赖调用方记得再查一次用户状态。
    #[tokio::test]
    async fn 停用用户的有效会话查不到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-a", Utc::now())
                .await
                .unwrap()
                .is_some()
        );

        LocalUsers::set_status(test_db.pool(), user_id, LocalUserStatus::Disabled)
            .await
            .unwrap();
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-a", Utc::now())
                .await
                .unwrap()
                .is_none(),
            "被停用用户的会话必须立刻失效"
        );
    }

    #[tokio::test]
    async fn 多设备会话互不影响且登出只撤销当前会话() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let 手机 = 建会话(&test_db, user_id, "hash-phone", Duration::days(30)).await;
        建会话(&test_db, user_id, "hash-laptop", Duration::days(30)).await;

        LocalSessions::revoke(test_db.pool(), 手机.id, Utc::now())
            .await
            .unwrap();

        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-phone", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-laptop", Utc::now())
                .await
                .unwrap()
                .is_some(),
            "登出一台设备不得把另一台也踢下线"
        );
    }

    #[tokio::test]
    async fn 撤销用户全部会话() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy").await;
        let bob = 建用户(&test_db, "bob").await;
        建会话(&test_db, amy, "hash-a1", Duration::days(30)).await;
        建会话(&test_db, amy, "hash-a2", Duration::days(30)).await;
        建会话(&test_db, bob, "hash-b1", Duration::days(30)).await;

        let revoked = LocalSessions::revoke_all_for_user(test_db.pool(), amy, Utc::now())
            .await
            .unwrap();
        assert_eq!(revoked, 2);

        for hash in ["hash-a1", "hash-a2"] {
            assert!(
                LocalSessions::find_valid_by_token_hash(test_db.pool(), hash, Utc::now())
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-b1", Utc::now())
                .await
                .unwrap()
                .is_some(),
            "不得波及别的用户"
        );
    }

    /// 改密场景：撤销除当前会话外的全部会话（当前浏览器不被自己踢下线）。
    #[tokio::test]
    async fn 撤销除指定会话外的全部会话() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy").await;
        let 保留 = 建会话(&test_db, amy, "hash-keep", Duration::days(30)).await;
        建会话(&test_db, amy, "hash-drop1", Duration::days(30)).await;
        建会话(&test_db, amy, "hash-drop2", Duration::days(30)).await;

        let revoked =
            LocalSessions::revoke_all_for_user_except(test_db.pool(), amy, 保留.id, Utc::now())
                .await
                .unwrap();
        assert_eq!(revoked, 2);
        assert!(
            LocalSessions::find_valid_by_token_hash(test_db.pool(), "hash-keep", Utc::now())
                .await
                .unwrap()
                .is_some()
        );
        for hash in ["hash-drop1", "hash-drop2"] {
            assert!(
                LocalSessions::find_valid_by_token_hash(test_db.pool(), hash, Utc::now())
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn last_seen_at_写库有五分钟节流() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;

        let 原值: String =
            sqlx::query_scalar("SELECT last_seen_at FROM local_sessions WHERE id = ?1")
                .bind(session.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();

        // 距上次不足 5 分钟：不写库。
        assert!(
            !LocalSessions::touch_last_seen(
                test_db.pool(),
                session.id,
                Utc::now() + Duration::minutes(4)
            )
            .await
            .unwrap()
        );
        let 未变: String =
            sqlx::query_scalar("SELECT last_seen_at FROM local_sessions WHERE id = ?1")
                .bind(session.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(未变, 原值, "节流窗口内不应写库");

        // 超过 5 分钟：写库。
        let 新时刻 = Utc::now() + Duration::minutes(6);
        assert!(
            LocalSessions::touch_last_seen(test_db.pool(), session.id, 新时刻)
                .await
                .unwrap()
        );
        let 已变: String =
            sqlx::query_scalar("SELECT last_seen_at FROM local_sessions WHERE id = ?1")
                .bind(session.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_ne!(已变, 原值);
    }

    /// 滑动续期的语义：只推进 `last_seen_at`，**不延长 `expires_at`**。
    #[tokio::test]
    async fn 续期不会延长过期时间() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;
        let 原过期: String =
            sqlx::query_scalar("SELECT expires_at FROM local_sessions WHERE id = ?1")
                .bind(session.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();

        LocalSessions::touch_last_seen(test_db.pool(), session.id, Utc::now() + Duration::hours(1))
            .await
            .unwrap();

        let 新过期: String =
            sqlx::query_scalar("SELECT expires_at FROM local_sessions WHERE id = ?1")
                .bind(session.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(新过期, 原过期, "expires_at 必须是固定的绝对时刻");
    }

    #[tokio::test]
    async fn 已撤销会话不再刷新_last_seen_at() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;
        LocalSessions::revoke(test_db.pool(), session.id, Utc::now())
            .await
            .unwrap();

        assert!(
            !LocalSessions::touch_last_seen(
                test_db.pool(),
                session.id,
                Utc::now() + Duration::hours(1)
            )
            .await
            .unwrap(),
            "撤销后的会话不应再被刷新"
        );
    }

    #[tokio::test]
    async fn 清理过期会话只删过期的() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        建会话(&test_db, user_id, "hash-old1", -Duration::days(1)).await;
        建会话(&test_db, user_id, "hash-old2", -Duration::seconds(1)).await;
        建会话(&test_db, user_id, "hash-live", Duration::days(30)).await;

        let deleted = LocalSessions::delete_expired(test_db.pool(), Utc::now())
            .await
            .unwrap();
        assert_eq!(deleted, 2);

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    /// 防手滑：`from_secs(6)` 会让服务器每 6 秒扫一遍全表。
    #[test]
    fn 清理任务的间隔常量是六小时() {
        assert_eq!(
            SESSION_CLEANUP_INTERVAL,
            std::time::Duration::from_secs(6 * 3600)
        );
    }

    /// 清理必须只按 `expires_at` 判定，**不能**顺手把「已撤销但未过期」
    /// 的行也删掉：撤销记录还要留着给「这张 Cookie 为什么失效」的排查用，
    /// 而且删掉它会让被撤销的令牌哈希重新可用（唯一索引空出来了）。
    #[tokio::test]
    async fn 清理不碰已撤销但未过期的会话() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let 已撤销 = 建会话(&test_db, user_id, "hash-revoked", Duration::days(30)).await;
        LocalSessions::revoke(test_db.pool(), 已撤销.id, Utc::now())
            .await
            .unwrap();
        建会话(&test_db, user_id, "hash-live", Duration::days(30)).await;

        assert_eq!(
            LocalSessions::delete_expired(test_db.pool(), Utc::now())
                .await
                .unwrap(),
            0,
            "没有过期行时不该删任何东西"
        );
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(left, 2);
    }

    /// 边界：`expires_at` 正好等于 `now` 的行算过期（SQL 是 `<=`）。
    #[tokio::test]
    async fn 清理的时间边界是闭区间() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        let session = 建会话(&test_db, user_id, "hash-edge", Duration::days(1)).await;
        assert_eq!(
            LocalSessions::delete_expired(test_db.pool(), session.expires_at)
                .await
                .unwrap(),
            1,
            "expires_at == now 必须算过期"
        );
    }

    /// 大量会话时清理仍然是一条 SQL，不会因为条数多而漏删或超时。
    #[tokio::test]
    async fn 大量过期会话一次清干净() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        for i in 0..500 {
            建会话(
                &test_db,
                user_id,
                &format!("hash-old-{i}"),
                -Duration::days(1),
            )
            .await;
        }
        for i in 0..50 {
            建会话(
                &test_db,
                user_id,
                &format!("hash-live-{i}"),
                Duration::days(30),
            )
            .await;
        }
        assert_eq!(
            LocalSessions::delete_expired(test_db.pool(), Utc::now())
                .await
                .unwrap(),
            500
        );
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(left, 50, "有效会话一条都不能少");
    }

    /// `LocalSession` 上不能有 `token_hash`，否则会话对象一旦被序列化，
    /// 攻击者拿到哈希就能直接在库里对账（甚至离线撞库）。
    #[tokio::test]
    async fn 会话对象序列化后不含令牌哈希() {
        let test_db = TestDb::new().await;
        let session = 建会话(&test_db, DEFAULT_USER_ID, "hash-secret", Duration::days(30)).await;
        let json = serde_json::to_string(&session).unwrap();
        assert!(!json.contains("token"), "不得含 token 字段：{json}");
        assert!(!json.contains("hash-secret"), "不得含哈希值：{json}");
    }

    #[tokio::test]
    async fn 同一令牌哈希不能建两条会话() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy").await;
        建会话(&test_db, user_id, "hash-a", Duration::days(30)).await;
        let err = LocalSessions::create(
            test_db.pool(),
            NewLocalSession {
                user_id,
                token_hash: "hash-a".to_string(),
                expires_at: Utc::now() + Duration::days(30),
                user_agent: None,
                ip: None,
            },
        )
        .await
        .expect_err("令牌哈希撞车必须报错，不能静默复用别人的会话");
        assert!(crate::models::db_retry::is_unique_violation(&err));
    }

    #[tokio::test]
    async fn 用户代理与_ip_可以为空() {
        let test_db = TestDb::new().await;
        LocalSessions::create(
            test_db.pool(),
            NewLocalSession {
                user_id: DEFAULT_USER_ID,
                token_hash: "hash-anon".to_string(),
                expires_at: Utc::now() + Duration::days(30),
                user_agent: None,
                ip: None,
            },
        )
        .await
        .expect("UA / IP 缺失不应阻止建会话");
    }

    #[tokio::test]
    async fn 超长的用户代理会被截断而不是报错() {
        let test_db = TestDb::new().await;
        LocalSessions::create(
            test_db.pool(),
            NewLocalSession {
                user_id: DEFAULT_USER_ID,
                token_hash: "hash-long-ua".to_string(),
                expires_at: Utc::now() + Duration::days(30),
                user_agent: Some("U".repeat(10_000)),
                ip: Some("I".repeat(10_000)),
            },
        )
        .await
        .expect("超长请求头不应把建会话打挂");

        let ua: String = sqlx::query_scalar(
            "SELECT user_agent FROM local_sessions WHERE token_hash = 'hash-long-ua'",
        )
        .fetch_one(test_db.pool())
        .await
        .unwrap();
        assert!(ua.chars().count() <= MAX_USER_AGENT_LEN);
    }
}

#[cfg(test)]
mod schema_tests {
    use uuid::Uuid;

    use crate::{models::local_project::DEFAULT_USER_ID, test_support::TestDb};

    async fn column_names(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
        // table 来自测试常量，不接受外部输入；PRAGMA 不支持绑定参数。
        let sql = format!("PRAGMA table_info({table})");
        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .expect("读取表结构失败");
        rows.into_iter().map(|r| r.1).collect()
    }

    #[tokio::test]
    async fn 认证四张表存在且_id_是第零列() {
        let test_db = TestDb::new().await;
        for table in [
            "local_users",
            "local_sessions",
            "local_user_identities",
            "local_invites",
        ] {
            let columns = column_names(test_db.pool(), table).await;
            assert!(!columns.is_empty(), "表 {table} 不存在");
            assert_eq!(columns[0], "id", "表 {table} 的 id 必须是第 0 列");
        }
    }

    /// 生产连接是否开启外键，直接决定 `ON DELETE CASCADE` 会不会生效。
    /// sqlx 的 SqliteConnectOptions 默认 `foreign_keys=ON`，这里把它钉死：
    /// 一旦有人显式关掉，撤销用户时会话就会变成孤儿行。
    #[tokio::test]
    async fn 测试库与生产库一样默认开启外键() {
        let test_db = TestDb::new().await;
        let (on,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(test_db.pool())
            .await
            .expect("读取 foreign_keys 失败");
        assert_eq!(on, 1, "外键必须开启，否则 ON DELETE CASCADE 是空话");
    }

    #[tokio::test]
    async fn 迁移写入固定本机用户() {
        let test_db = TestDb::new().await;
        let row: (Vec<u8>, String, String, String, String) =
            sqlx::query_as("SELECT id, username, display_name, role, status FROM local_users")
                .fetch_one(test_db.pool())
                .await
                .expect("固定本机用户应已写入");

        assert_eq!(
            row.0,
            DEFAULT_USER_ID.as_bytes().to_vec(),
            "本机用户 id 必须与 DEFAULT_USER_ID 一致，否则历史 creator_user_id 对不上"
        );
        assert_eq!(row.1, "local");
        assert_eq!(row.3, "admin");
        assert_eq!(row.4, "active");
        assert!(!row.2.is_empty());
    }

    /// 本机用户的 id 能按 Uuid 解出来，说明 sqlx 的 Uuid ↔ 16 字节 BLOB 编码
    /// 与迁移里的 `X'...'` 字面量一致。
    #[tokio::test]
    async fn 本机用户可以按_uuid_绑定查到() {
        let test_db = TestDb::new().await;
        let id: Uuid = sqlx::query_scalar("SELECT id FROM local_users WHERE username = 'local'")
            .fetch_one(test_db.pool())
            .await
            .expect("查询失败");
        assert_eq!(id, DEFAULT_USER_ID);

        let hex: String = sqlx::query_scalar("SELECT hex(id) FROM local_users WHERE id = ?1")
            .bind(DEFAULT_USER_ID)
            .fetch_one(test_db.pool())
            .await
            .expect("按 Uuid 绑定应能命中");
        assert_eq!(hex, "00000000000000000000000000000002");
    }

    /// `randomblob(16)` 生成的 BLOB 能被 sqlx 解码成 Uuid（Uuid::from_slice 只看长度）。
    #[tokio::test]
    async fn randomblob_16_可以解码成_uuid() {
        let test_db = TestDb::new().await;
        let id: Uuid = sqlx::query_scalar("SELECT randomblob(16)")
            .fetch_one(test_db.pool())
            .await
            .expect("randomblob(16) 应能解码成 Uuid");
        assert_ne!(id, Uuid::nil());
    }

    #[tokio::test]
    async fn 用户名唯一约束在库层生效() {
        let test_db = TestDb::new().await;
        let err = sqlx::query(
            "INSERT INTO local_users (id, username, display_name) VALUES (?1, 'local', 'X')",
        )
        .bind(Uuid::new_v4())
        .execute(test_db.pool())
        .await
        .expect_err("重复用户名必须被唯一索引拦下");
        assert!(
            crate::models::db_retry::is_unique_violation(&err),
            "应是唯一约束错误，实际：{err}"
        );
    }

    /// 唯一索引是大小写敏感的（SQLite 默认 BINARY 排序规则）；
    /// 「Alice 与 alice 视为同一人」必须由应用层的 normalize_username 保证。
    #[tokio::test]
    async fn 用户名唯一索引大小写敏感由应用层负责规范化() {
        let test_db = TestDb::new().await;
        sqlx::query(
            "INSERT INTO local_users (id, username, display_name) VALUES (?1, 'Alice', 'A')",
        )
        .bind(Uuid::new_v4())
        .execute(test_db.pool())
        .await
        .expect("插入 Alice 应成功");
        sqlx::query(
            "INSERT INTO local_users (id, username, display_name) VALUES (?1, 'alice', 'a')",
        )
        .bind(Uuid::new_v4())
        .execute(test_db.pool())
        .await
        .expect("库层不会拦下大小写不同的用户名");
    }

    #[tokio::test]
    async fn 邮箱唯一索引忽略_null() {
        let test_db = TestDb::new().await;
        for _ in 0..2 {
            sqlx::query(
                "INSERT INTO local_users (id, username, display_name, email) \
                 VALUES (?1, ?2, 'X', NULL)",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4().to_string())
            .execute(test_db.pool())
            .await
            .expect("多条 email 为空的用户不应互相冲突");
        }
    }

    #[tokio::test]
    async fn 删除用户时级联删除会话与身份() {
        let test_db = TestDb::new().await;
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO local_users (id, username, display_name) VALUES (?1, 'bob', 'Bob')",
        )
        .bind(user_id)
        .execute(test_db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO local_sessions (id, user_id, token_hash, expires_at) \
             VALUES (?1, ?2, 'hash', '2099-01-01T00:00:00+00:00')",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(test_db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO local_user_identities (id, user_id, provider, subject) \
             VALUES (?1, ?2, 'feishu', 'ou_x')",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(test_db.pool())
        .await
        .unwrap();

        sqlx::query("DELETE FROM local_users WHERE id = ?1")
            .bind(user_id)
            .execute(test_db.pool())
            .await
            .expect("删除用户失败");

        let sessions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM local_sessions WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        let identities: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM local_user_identities WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(sessions, 0, "会话应被级联删除");
        assert_eq!(identities, 0, "第三方身份应被级联删除");
    }

    #[tokio::test]
    async fn 同一提供方的_subject_唯一() {
        let test_db = TestDb::new().await;
        sqlx::query(
            "INSERT INTO local_user_identities (id, user_id, provider, subject) \
             VALUES (?1, ?2, 'feishu', 'ou_dup')",
        )
        .bind(Uuid::new_v4())
        .bind(DEFAULT_USER_ID)
        .execute(test_db.pool())
        .await
        .unwrap();

        let err = sqlx::query(
            "INSERT INTO local_user_identities (id, user_id, provider, subject) \
             VALUES (?1, ?2, 'feishu', 'ou_dup')",
        )
        .bind(Uuid::new_v4())
        .bind(DEFAULT_USER_ID)
        .execute(test_db.pool())
        .await
        .expect_err("(provider, subject) 必须唯一");
        assert!(crate::models::db_retry::is_unique_violation(&err));

        // 换一个 provider 就不冲突了。
        sqlx::query(
            "INSERT INTO local_user_identities (id, user_id, provider, subject) \
             VALUES (?1, ?2, 'lark', 'ou_dup')",
        )
        .bind(Uuid::new_v4())
        .bind(DEFAULT_USER_ID)
        .execute(test_db.pool())
        .await
        .expect("不同 provider 的同名 subject 应能共存");
    }

    #[tokio::test]
    async fn 会话令牌哈希唯一() {
        let test_db = TestDb::new().await;
        for _ in 0..1 {
            sqlx::query(
                "INSERT INTO local_sessions (id, user_id, token_hash, expires_at) \
                 VALUES (?1, ?2, 'same-hash', '2099-01-01T00:00:00+00:00')",
            )
            .bind(Uuid::new_v4())
            .bind(DEFAULT_USER_ID)
            .execute(test_db.pool())
            .await
            .unwrap();
        }
        let err = sqlx::query(
            "INSERT INTO local_sessions (id, user_id, token_hash, expires_at) \
             VALUES (?1, ?2, 'same-hash', '2099-01-01T00:00:00+00:00')",
        )
        .bind(Uuid::new_v4())
        .bind(DEFAULT_USER_ID)
        .execute(test_db.pool())
        .await
        .expect_err("token_hash 必须唯一");
        assert!(crate::models::db_retry::is_unique_violation(&err));
    }

    /// 时间戳 DEFAULT 必须写出 RFC3339（与 sqlx 绑定 `DateTime<Utc>` 的格式一致），
    /// 否则同一列里会混进 `YYYY-MM-DD HH:MM:SS.SSS`，字符串比较（如
    /// `expires_at > $now`）会直接失效——20260916010000 那条迁移就是在补这个坑。
    #[tokio::test]
    async fn 时间戳默认值是_rfc3339() {
        use chrono::{DateTime, Utc};

        let test_db = TestDb::new().await;
        let raw: String =
            sqlx::query_scalar("SELECT created_at FROM local_users WHERE username = 'local'")
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert!(raw.contains('T'), "created_at 应是 RFC3339，实际：{raw}");
        assert!(raw.ends_with("+00:00"), "应带 +00:00 偏移，实际：{raw}");
        DateTime::parse_from_rfc3339(&raw).expect("应能按 RFC3339 解析");

        // 默认值也要能被 sqlx 解成 DateTime<Utc>。
        let parsed: DateTime<Utc> =
            sqlx::query_scalar("SELECT created_at FROM local_users WHERE username = 'local'")
                .fetch_one(test_db.pool())
                .await
                .expect("sqlx 应能把默认时间戳解码成 DateTime<Utc>");
        assert!(parsed <= Utc::now());
    }
}
