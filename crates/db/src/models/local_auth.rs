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
use crate::models::local_user::LocalUserRole;

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

// -------------------------------------------------------------- 第三方身份

/// `provider` 的存储长度上限。它只可能是 `feishu`/`lark`/`google`，
/// 上限纯粹是「万一上层漏了校验」的兜底。
pub const MAX_PROVIDER_LEN: usize = 32;
/// `subject`（`union_id` / `sub`）的存储长度上限。
/// 对方返回一个几兆的字符串不该被原样写进库里。
pub const MAX_SUBJECT_LEN: usize = 255;

/// 一条第三方身份绑定。
///
/// **表里没有任何令牌字段**（见迁移 `20260917000000_add_local_auth.sql`）：
/// access token 换完用户信息就丢，绝不落库。这里存的 `subject` 是提供方那边
/// 的**稳定标识**（飞书/Lark 的 `union_id`、Google 的 `sub`），
/// `email` 只是备查资料，**不是**绑定键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalUserIdentity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityLinkError {
    #[error("{0}")]
    Validation(String),
    /// `(provider, subject)` 已经绑在**别的**账号上。
    /// 这是防账号劫持的关键：一个第三方身份只能属于一个本地账号。
    #[error("该第三方账号已绑定到其他用户")]
    AlreadyLinked,
    #[error(transparent)]
    User(#[from] crate::models::local_user::LocalUserError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// 解绑失败的原因。
///
/// 与 [`IdentityLinkError`] 分开是刻意的：解绑只有三种结局，
/// 混进 link 那一套会让「把自己锁在门外」这条最关键的判据淹没在
/// 一堆与它无关的分支里。
#[derive(Debug, thiserror::Error)]
pub enum IdentityUnlinkError {
    /// 当前用户名下没有这个提供方的绑定。
    ///
    /// **别人名下有**也走这一条：解绑按 `user_id` 圈定范围，
    /// 「有但不是你的」与「根本没有」对调用方必须长得一模一样，
    /// 否则这个接口就成了一台「某人绑没绑飞书」的查询机。
    #[error("未找到该第三方账号绑定")]
    NotFound,
    /// 解了就再也登不进来：账号没有密码，且这是最后一个已绑定身份。
    #[error("这是该账号唯一的登录方式，解绑后将无法登录")]
    LastLoginMethod,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub struct LocalUserIdentities;

impl LocalUserIdentities {
    /// 按 `(provider, subject)` 查。这是登录时**唯一**的查找路径——
    /// 刻意没有「按 email 查身份」的方法，免得有人顺手用它做自动合并。
    pub async fn find_by_provider_subject(
        pool: &SqlitePool,
        provider: &str,
        subject: &str,
    ) -> Result<Option<LocalUserIdentity>, sqlx::Error> {
        if provider.is_empty() || subject.is_empty() {
            return Ok(None);
        }
        sqlx::query_as!(
            LocalUserIdentity,
            r#"SELECT id         AS "id!: Uuid",
                      user_id    AS "user_id!: Uuid",
                      provider   AS "provider!",
                      subject    AS "subject!",
                      email      AS "email?",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM local_user_identities
               WHERE provider = $1 AND subject = $2"#,
            provider,
            subject
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn list_for_user(
        pool: &SqlitePool,
        user_id: Uuid,
    ) -> Result<Vec<LocalUserIdentity>, sqlx::Error> {
        sqlx::query_as!(
            LocalUserIdentity,
            r#"SELECT id         AS "id!: Uuid",
                      user_id    AS "user_id!: Uuid",
                      provider   AS "provider!",
                      subject    AS "subject!",
                      email      AS "email?",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM local_user_identities
               WHERE user_id = $1
               ORDER BY provider ASC"#,
            user_id
        )
        .fetch_all(pool)
        .await
    }

    /// 解除某个账号在某个提供方上的绑定。
    ///
    /// 两条判据都写在**同一条 `DELETE` 里**，不是「先查后删」：
    /// 1. `user_id` 进 `WHERE`，所以 A 不可能删掉 B 的行；
    /// 2. 「解完还留得下登录方式」也进 `WHERE`——账号有密码，
    ///    或者名下还有别的提供方。分成两步的话，无密码 + 两个身份的
    ///    账号并发解绑两个不同提供方时，两边都会读到「还有另一个」，
    ///    然后各删各的，人就被锁在门外了。
    ///
    /// 删不掉时再查一次是为了分辨原因，这一次读不影响正确性：
    /// 真有并发把行删走了，报「未找到」正是想要的结果（幂等）。
    pub async fn unlink(
        pool: &SqlitePool,
        user_id: Uuid,
        provider: &str,
    ) -> Result<(), IdentityUnlinkError> {
        let provider = provider.trim().to_string();
        // 越界的 provider 不可能在表里，直接当未找到；不下发给数据库。
        if provider.is_empty() || provider.len() > MAX_PROVIDER_LEN {
            return Err(IdentityUnlinkError::NotFound);
        }

        let affected = {
            let provider = provider.clone();
            retry_on_busy(|| {
                let provider = provider.clone();
                async move {
                    sqlx::query!(
                        r#"DELETE FROM local_user_identities
                           WHERE user_id = $1
                             AND provider = $2
                             AND (EXISTS (SELECT 1 FROM local_users u
                                          WHERE u.id = $1 AND u.password_hash IS NOT NULL)
                                  OR EXISTS (SELECT 1 FROM local_user_identities other
                                             WHERE other.user_id = $1
                                               AND other.provider <> $2))"#,
                        user_id,
                        provider
                    )
                    .execute(pool)
                    .await
                }
            })
            .await?
            .rows_affected()
        };
        if affected > 0 {
            return Ok(());
        }

        let still_there: Option<i64> = sqlx::query_scalar!(
            r#"SELECT 1 AS "one!: i64" FROM local_user_identities
               WHERE user_id = $1 AND provider = $2"#,
            user_id,
            provider
        )
        .fetch_optional(pool)
        .await?;
        if still_there.is_some() {
            Err(IdentityUnlinkError::LastLoginMethod)
        } else {
            Err(IdentityUnlinkError::NotFound)
        }
    }

    /// 把一个第三方身份绑到某个账号上。
    ///
    /// 并发判据是表上的 `(provider, subject)` 唯一索引，**不是**「先查再插」：
    /// 两个请求同时为同一个身份绑不同账号时，后到的那个必然拿到唯一约束冲突。
    pub async fn link(
        pool: &SqlitePool,
        user_id: Uuid,
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<LocalUserIdentity, IdentityLinkError> {
        let prepared = PreparedIdentity::new(user_id, provider, subject, email)?;
        let result = retry_on_busy(|| {
            let prepared = prepared.clone();
            async move { prepared.insert(pool).await }
        })
        .await;
        result.map_err(map_identity_insert_error)
    }

    /// 建号 + 绑定，同一个事务。
    ///
    /// 必须是事务：建完用户才失败会留下一个**没有任何凭据**的账号
    /// （没密码也没身份），它既登不进来也没人知道该删掉。
    pub async fn create_user_with_identity(
        pool: &SqlitePool,
        user: crate::models::local_user::NewLocalUser,
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<crate::models::local_user::LocalUser, IdentityLinkError> {
        use crate::models::local_user::{PreparedUser, map_insert_error};

        // 两份参数都在开事务之前校验完，事务里不会再因为参数非法而半途失败。
        let prepared_user = PreparedUser::new(user)?;
        let mut tx = pool.begin().await?;

        let created = match prepared_user.insert(&mut *tx).await {
            Ok(created) => created,
            Err(err) => {
                tx.rollback().await?;
                return Err(IdentityLinkError::User(map_insert_error(err)));
            }
        };
        let prepared_identity = match PreparedIdentity::new(created.id, provider, subject, email) {
            Ok(prepared) => prepared,
            Err(err) => {
                tx.rollback().await?;
                return Err(err);
            }
        };
        if let Err(err) = prepared_identity.insert(&mut *tx).await {
            tx.rollback().await?;
            return Err(map_identity_insert_error(err));
        }
        tx.commit().await?;
        Ok(created)
    }
}

/// 校验过的绑定入参，可以插进连接池或事务。
#[derive(Debug, Clone)]
struct PreparedIdentity {
    id: Uuid,
    user_id: Uuid,
    provider: String,
    subject: String,
    email: Option<String>,
    now: DateTime<Utc>,
}

impl PreparedIdentity {
    fn new(
        user_id: Uuid,
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<Self, IdentityLinkError> {
        let provider = provider.trim();
        let subject = subject.trim();
        if provider.is_empty() || provider.len() > MAX_PROVIDER_LEN {
            return Err(IdentityLinkError::Validation("提供方非法".to_string()));
        }
        if subject.is_empty() || subject.chars().count() > MAX_SUBJECT_LEN {
            return Err(IdentityLinkError::Validation(
                "第三方用户标识非法".to_string(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            user_id,
            provider: provider.to_string(),
            subject: subject.to_string(),
            email: crate::models::local_user::normalize_email(email),
            now: Utc::now(),
        })
    }

    async fn insert<'e, E>(&self, executor: E) -> Result<LocalUserIdentity, sqlx::Error>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        sqlx::query_as!(
            LocalUserIdentity,
            r#"INSERT INTO local_user_identities (id, user_id, provider, subject, email, created_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id         AS "id!: Uuid",
                         user_id    AS "user_id!: Uuid",
                         provider   AS "provider!",
                         subject    AS "subject!",
                         email      AS "email?",
                         created_at AS "created_at!: DateTime<Utc>""#,
            self.id,
            self.user_id,
            self.provider,
            self.subject,
            self.email,
            self.now
        )
        .fetch_one(executor)
        .await
    }
}

fn map_identity_insert_error(err: sqlx::Error) -> IdentityLinkError {
    use super::db_retry::is_unique_violation;

    if is_unique_violation(&err) {
        IdentityLinkError::AlreadyLinked
    } else {
        IdentityLinkError::Database(err)
    }
}

// ------------------------------------------------------------------ 邀请码

/// 邀请码默认有效期（天）。管理员建码时不填就用它。
pub const DEFAULT_INVITE_TTL_DAYS: i64 = 7;

/// 邀请码行。**不含 `code_hash`**，理由同 [`LocalSession`]：
/// 管理员页面会直接 `Json(invites)`，带上哈希等于把离线撞库的原料送出去。
/// 明文邀请码只在创建那一次返回给调用方，之后库里、响应里都再也见不到。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalInvite {
    pub id: Uuid,
    pub role: LocalUserRole,
    pub created_by: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub used_by: Option<Uuid>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 建邀请入参。`code_hash` 由调用方（`services::local_auth::token::hash_session_token`）
/// 算好后传进来，本层不接触明文邀请码。
#[derive(Debug, Clone)]
pub struct NewLocalInvite {
    pub code_hash: String,
    pub role: LocalUserRole,
    pub created_by: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
}

/// 邀请注册的用户资料。
///
/// **刻意没有 `role` 字段**：新用户的角色只能来自邀请码本身。
/// 请求体里的 `role=admin` 在类型层面就无处落脚，不依赖任何 handler 记得忽略它。
#[derive(Debug, Clone)]
pub struct InviteRegistration {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password_hash: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteRedeemError {
    /// 邀请码不存在 / 已过期 / 已被用过。
    ///
    /// **三种情况共用一个变体是安全要求**：分开就等于给攻击者一个
    /// 「这个码存不存在」的预言机，可以拿来枚举。
    #[error("邀请码无效或已过期")]
    InvalidCode,
    #[error(transparent)]
    User(#[from] crate::models::local_user::LocalUserError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub struct LocalInvites;

impl LocalInvites {
    pub async fn create(
        pool: &SqlitePool,
        data: NewLocalInvite,
    ) -> Result<LocalInvite, sqlx::Error> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let NewLocalInvite {
            code_hash,
            role,
            created_by,
            expires_at,
        } = data;

        retry_on_busy(|| {
            let code_hash = code_hash.clone();
            async move {
                sqlx::query_as!(
                    LocalInvite,
                    r#"INSERT INTO local_invites
                           (id, code_hash, role, created_by, expires_at, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6)
                       RETURNING id         AS "id!: Uuid",
                                 role       AS "role!: LocalUserRole",
                                 created_by AS "created_by?: Uuid",
                                 expires_at AS "expires_at!: DateTime<Utc>",
                                 used_by    AS "used_by?: Uuid",
                                 used_at    AS "used_at?: DateTime<Utc>",
                                 created_at AS "created_at!: DateTime<Utc>""#,
                    id,
                    code_hash,
                    role,
                    created_by,
                    expires_at,
                    now
                )
                .fetch_one(pool)
                .await
            }
        })
        .await
    }

    /// 按哈希取一条**当前可用**的邀请：未使用、未过期。
    /// 时间比较用 RFC3339 字符串，理由同 [`LocalSessions::find_valid_by_token_hash`]。
    pub async fn find_usable_by_code_hash(
        pool: &SqlitePool,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<LocalInvite>, sqlx::Error> {
        sqlx::query_as!(
            LocalInvite,
            r#"SELECT id         AS "id!: Uuid",
                      role       AS "role!: LocalUserRole",
                      created_by AS "created_by?: Uuid",
                      expires_at AS "expires_at!: DateTime<Utc>",
                      used_by    AS "used_by?: Uuid",
                      used_at    AS "used_at?: DateTime<Utc>",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM local_invites
               WHERE code_hash = $1
                 AND used_at IS NULL
                 AND expires_at > $2"#,
            code_hash,
            now
        )
        .fetch_optional(pool)
        .await
    }

    /// 全部邀请（含已使用、已过期），按创建时间倒序。管理员页面用。
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<LocalInvite>, sqlx::Error> {
        sqlx::query_as!(
            LocalInvite,
            r#"SELECT id         AS "id!: Uuid",
                      role       AS "role!: LocalUserRole",
                      created_by AS "created_by?: Uuid",
                      expires_at AS "expires_at!: DateTime<Utc>",
                      used_by    AS "used_by?: Uuid",
                      used_at    AS "used_at?: DateTime<Utc>",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM local_invites
               ORDER BY created_at DESC"#
        )
        .fetch_all(pool)
        .await
    }

    /// 删除一条邀请（作废）。返回受影响行数，0 表示本来就不存在。
    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let done = retry_on_busy(|| async move {
            sqlx::query!(r#"DELETE FROM local_invites WHERE id = $1"#, id)
                .execute(pool)
                .await
        })
        .await?;
        Ok(done.rows_affected())
    }

    /// 兑换邀请码：建号 + 消费邀请，**全程一个事务**。
    ///
    /// 顺序是「先建号、后消费邀请」：
    /// - 建号先做，用户名唯一索引就成了这一路的占位；重名时整体回滚，
    ///   邀请码**不会**被一次失败的注册烧掉。
    /// - 消费邀请用条件更新 `WHERE id = ? AND used_at IS NULL`，
    ///   靠 `rows_affected` 判胜负，而不是「先查后写」——后者在两个请求
    ///   同时兑换同一个码时会双双通过检查。
    ///
    /// 新用户的角色只能来自 `invite.role`，见 [`InviteRegistration`]。
    pub async fn redeem(
        pool: &SqlitePool,
        code_hash: &str,
        registration: InviteRegistration,
        now: DateTime<Utc>,
    ) -> Result<crate::models::local_user::LocalUser, InviteRedeemError> {
        use crate::models::local_user::{NewLocalUser, PreparedUser, map_insert_error};

        // 先读一次拿角色。这一次读**不是**并发判据（判据是下面的条件更新），
        // 只是为了知道该建 admin 还是 member；顺带让「码根本不存在」走快速失败。
        let invite = Self::find_usable_by_code_hash(pool, code_hash, now)
            .await?
            .ok_or(InviteRedeemError::InvalidCode)?;

        // 参数校验放在开事务之前：事务里不该再因为「用户名有空格」而回滚。
        let prepared = PreparedUser::new(NewLocalUser {
            username: registration.username,
            display_name: registration.display_name,
            email: registration.email,
            password_hash: registration.password_hash,
            role: invite.role,
        })?;

        let mut tx = pool.begin().await?;

        let user = match prepared.insert(&mut *tx).await {
            Ok(user) => user,
            Err(err) => {
                tx.rollback().await?;
                return Err(InviteRedeemError::User(map_insert_error(err)));
            }
        };

        let consumed = sqlx::query!(
            r#"UPDATE local_invites
               SET used_by = $2, used_at = $3
               WHERE id = $1 AND used_at IS NULL AND expires_at > $3"#,
            invite.id,
            user.id,
            now
        )
        .execute(&mut *tx)
        .await?;

        if consumed.rows_affected() != 1 {
            // 别人抢先一步用掉了这个码：把刚建的用户一起回滚。
            tx.rollback().await?;
            return Err(InviteRedeemError::InvalidCode);
        }

        tx.commit().await?;
        Ok(user)
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

#[cfg(test)]
mod invite_tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::{
        models::{
            local_project::DEFAULT_USER_ID,
            local_user::{LocalUserError, LocalUserRole, LocalUsers, NewLocalUser},
        },
        test_support::TestDb,
    };

    async fn 建邀请(
        test_db: &TestDb,
        code_hash: &str,
        role: LocalUserRole,
        ttl: Duration,
    ) -> LocalInvite {
        LocalInvites::create(
            test_db.pool(),
            NewLocalInvite {
                code_hash: code_hash.to_string(),
                role,
                created_by: Some(DEFAULT_USER_ID),
                expires_at: Utc::now() + ttl,
            },
        )
        .await
        .expect("建邀请失败")
    }

    fn 注册(username: &str) -> InviteRegistration {
        InviteRegistration {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some("$argon2id$x".to_string()),
        }
    }

    /// 邀请码明文与哈希都不能出现在邀请对象上：列表接口直接 `Json(invite)`
    /// 就会把它漏给任何管理员页面的旁观者（以及日志）。
    #[tokio::test]
    async fn 邀请对象序列化后不含邀请码哈希() {
        let test_db = TestDb::new().await;
        let invite = 建邀请(
            &test_db,
            "hash-secret-code",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        let json = serde_json::to_string(&invite).unwrap();
        assert!(!json.contains("hash-secret-code"), "不得含哈希值：{json}");
        assert!(!json.contains("code"), "不得含 code 字段：{json}");
    }

    #[tokio::test]
    async fn 未过期未使用的邀请可以按哈希查到() {
        let test_db = TestDb::new().await;
        let invite = 建邀请(
            &test_db,
            "hash-ok",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        let found = LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-ok", Utc::now())
            .await
            .unwrap()
            .expect("应查到");
        assert_eq!(found.id, invite.id);
        assert_eq!(found.role, LocalUserRole::Member);
        assert!(found.used_at.is_none());
    }

    #[tokio::test]
    async fn 过期邀请查不到() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-expired",
            LocalUserRole::Member,
            -Duration::seconds(1),
        )
        .await;
        assert!(
            LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-expired", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 已使用邀请查不到() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-used",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        LocalInvites::redeem(test_db.pool(), "hash-used", 注册("amy"), Utc::now())
            .await
            .expect("首次兑换应成功");
        assert!(
            LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-used", Utc::now())
                .await
                .unwrap()
                .is_none(),
            "一次性消费：用过就不能再用"
        );
    }

    #[tokio::test]
    async fn 不存在的哈希查不到() {
        let test_db = TestDb::new().await;
        assert!(
            LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-ghost", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
    }

    /// 兑换成功后邀请行上要留下「谁用的、什么时候用的」。
    #[tokio::test]
    async fn 兑换成功后记录使用者与使用时间() {
        let test_db = TestDb::new().await;
        建邀请(&test_db, "hash-r", LocalUserRole::Member, Duration::days(7)).await;
        let user = LocalInvites::redeem(test_db.pool(), "hash-r", 注册("amy"), Utc::now())
            .await
            .unwrap();
        let invite = LocalInvites::find_all(test_db.pool())
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(invite.used_by, Some(user.id));
        assert!(invite.used_at.is_some());
    }

    /// **角色来自邀请，不来自注册请求。** [`InviteRegistration`] 上刻意没有
    /// `role` 字段——请求体里传 `role=admin` 在类型层面就无处落脚。
    #[tokio::test]
    async fn 兑换产生的角色来自邀请() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-admin",
            LocalUserRole::Admin,
            Duration::days(7),
        )
        .await;
        建邀请(
            &test_db,
            "hash-member",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        let a = LocalInvites::redeem(test_db.pool(), "hash-admin", 注册("boss"), Utc::now())
            .await
            .unwrap();
        let m = LocalInvites::redeem(test_db.pool(), "hash-member", 注册("amy"), Utc::now())
            .await
            .unwrap();
        assert_eq!(a.role, LocalUserRole::Admin);
        assert_eq!(m.role, LocalUserRole::Member);
    }

    /// 枚举防护的基础：三种失败在模型层就是**同一个**错误变体，
    /// 调用方想区分也区分不了。
    #[tokio::test]
    async fn 不存在_过期_已用_三种失败返回同一个错误变体() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-e1",
            LocalUserRole::Member,
            -Duration::seconds(1),
        )
        .await;
        建邀请(
            &test_db,
            "hash-e2",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        LocalInvites::redeem(test_db.pool(), "hash-e2", 注册("amy"), Utc::now())
            .await
            .unwrap();

        for (hash, 场景) in [
            ("hash-ghost", "不存在"),
            ("hash-e1", "已过期"),
            ("hash-e2", "已使用"),
        ] {
            let err = LocalInvites::redeem(test_db.pool(), hash, 注册("bob"), Utc::now())
                .await
                .expect_err("{场景} 应失败");
            assert!(
                matches!(err, InviteRedeemError::InvalidCode),
                "{场景} 应是 InvalidCode，实际 {err:?}"
            );
        }
    }

    /// 并发兑换：两个请求拿同一个码同时注册，只能有一个建号成功。
    /// 靠的是条件更新 `WHERE used_at IS NULL` 的 `rows_affected`，不是「先查后写」。
    #[tokio::test]
    async fn 并发兑换同一邀请只有一个成功() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-race",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;

        let pool_a = test_db.pool().clone();
        let pool_b = test_db.pool().clone();
        let a = tokio::spawn(async move {
            LocalInvites::redeem(&pool_a, "hash-race", 注册("racer-a"), Utc::now()).await
        });
        let b = tokio::spawn(async move {
            LocalInvites::redeem(&pool_b, "hash-race", 注册("racer-b"), Utc::now()).await
        });
        let (ra, rb) = (a.await.unwrap(), b.await.unwrap());
        let 成功数 = [&ra, &rb].iter().filter(|r| r.is_ok()).count();
        assert_eq!(成功数, 1, "同一邀请码只能兑换一次：{ra:?} / {rb:?}");

        let 用户数: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM local_users WHERE username LIKE 'racer-%'")
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(用户数, 1, "失败的那一路不得留下用户行");
    }

    /// 用户名冲突必须整体回滚：邀请码不能被一次失败的注册烧掉。
    #[tokio::test]
    async fn 用户名冲突时邀请码未被消费() {
        let test_db = TestDb::new().await;
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: "amy".to_string(),
                display_name: "amy".to_string(),
                email: None,
                password_hash: None,
                role: LocalUserRole::Member,
            },
        )
        .await
        .unwrap();
        建邀请(
            &test_db,
            "hash-conflict",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;

        let err = LocalInvites::redeem(test_db.pool(), "hash-conflict", 注册("amy"), Utc::now())
            .await
            .expect_err("重名应失败");
        assert!(
            matches!(err, InviteRedeemError::User(LocalUserError::Conflict(_))),
            "应是冲突错误，实际 {err:?}"
        );
        assert!(
            LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-conflict", Utc::now())
                .await
                .unwrap()
                .is_some(),
            "注册失败不得消费邀请码"
        );
    }

    /// 非法用户名同样不能烧掉邀请码。
    #[tokio::test]
    async fn 非法用户名时邀请码未被消费() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-bad",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        let err = LocalInvites::redeem(test_db.pool(), "hash-bad", 注册("ad min"), Utc::now())
            .await
            .expect_err("非法用户名应失败");
        assert!(matches!(
            err,
            InviteRedeemError::User(LocalUserError::InvalidUsername)
        ));
        assert!(
            LocalInvites::find_usable_by_code_hash(test_db.pool(), "hash-bad", Utc::now())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn 列表含已用邀请并按创建时间倒序() {
        let test_db = TestDb::new().await;
        for (i, hash) in ["hash-1", "hash-2", "hash-3"].iter().enumerate() {
            LocalInvites::create(
                test_db.pool(),
                NewLocalInvite {
                    code_hash: (*hash).to_string(),
                    role: LocalUserRole::Member,
                    created_by: Some(DEFAULT_USER_ID),
                    expires_at: Utc::now() + Duration::days(7),
                },
            )
            .await
            .unwrap();
            // 让 created_at 严格递增，避免同毫秒导致排序不稳定。
            let _ = i;
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        LocalInvites::redeem(test_db.pool(), "hash-1", 注册("amy"), Utc::now())
            .await
            .unwrap();
        let all = LocalInvites::find_all(test_db.pool()).await.unwrap();
        assert_eq!(all.len(), 3, "已使用的邀请仍要出现在列表里");
        assert!(
            all[0].created_at >= all[1].created_at && all[1].created_at >= all[2].created_at,
            "应按创建时间倒序"
        );
    }

    #[tokio::test]
    async fn 删除邀请返回受影响行数() {
        let test_db = TestDb::new().await;
        let invite = 建邀请(
            &test_db,
            "hash-del",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        assert_eq!(
            LocalInvites::delete(test_db.pool(), invite.id)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            LocalInvites::delete(test_db.pool(), invite.id)
                .await
                .unwrap(),
            0,
            "重复删除返回 0，调用方据此报 404"
        );
        assert_eq!(
            LocalInvites::delete(test_db.pool(), Uuid::new_v4())
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn 同一个哈希不能建两条邀请() {
        let test_db = TestDb::new().await;
        建邀请(
            &test_db,
            "hash-dup",
            LocalUserRole::Member,
            Duration::days(7),
        )
        .await;
        let err = LocalInvites::create(
            test_db.pool(),
            NewLocalInvite {
                code_hash: "hash-dup".to_string(),
                role: LocalUserRole::Member,
                created_by: Some(DEFAULT_USER_ID),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .expect_err("哈希撞车必须报错");
        assert!(crate::models::db_retry::is_unique_violation(&err));
    }

    #[test]
    fn 邀请码默认有效期是七天() {
        assert_eq!(DEFAULT_INVITE_TTL_DAYS, 7);
    }
}

#[cfg(test)]
mod identity_tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        models::local_user::{
            LocalUserRole, LocalUserStatus, LocalUsers, NewLocalUser, normalize_email,
        },
        test_support::TestDb,
    };

    async fn 建用户(test_db: &TestDb, username: &str, email: Option<&str>) -> Uuid {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: email.map(str::to_string),
                password_hash: Some("$argon2id$x".to_string()),
                role: LocalUserRole::Member,
            },
        )
        .await
        .expect("建用户失败")
        .id
    }

    #[tokio::test]
    async fn 绑定后可按_provider_与_subject_查到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        let identity =
            LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", Some("A@X.com"))
                .await
                .expect("绑定失败");
        assert_eq!(identity.user_id, user_id);
        assert_eq!(identity.email.as_deref(), Some("a@x.com"), "邮箱应被规范化");

        let found =
            LocalUserIdentities::find_by_provider_subject(test_db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .expect("应命中");
        assert_eq!(found.id, identity.id);
    }

    /// 查找是精确匹配：换个提供方、换个大小写、带空白都不该命中，
    /// 否则「飞书的 union_id」就能拿去冒充「Lark 的 union_id」。
    #[tokio::test]
    async fn 查找不做任何模糊匹配() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();

        for (provider, subject) in [
            ("lark", "on_u1"),
            ("google", "on_u1"),
            ("Feishu", "on_u1"),
            ("feishu", "on_u"),
            ("feishu", "ON_U1"),
            ("feishu", "on_u1 "),
            ("feishu", ""),
            ("", "on_u1"),
            ("feishu", "%"),
            ("feishu", "on_u1' OR '1'='1"),
        ] {
            assert!(
                LocalUserIdentities::find_by_provider_subject(test_db.pool(), provider, subject)
                    .await
                    .unwrap()
                    .is_none(),
                "({provider:?}, {subject:?}) 不应命中"
            );
        }
    }

    /// **一个第三方身份只能属于一个本地账号。** 否则拿到别人 union_id 的人
    /// 只要绑到自己账号上，就能从自己这边登进受害者的身份。
    #[tokio::test]
    async fn 同一身份不能绑到两个账号() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        let bob = 建用户(&test_db, "bob", None).await;

        LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", None)
            .await
            .expect("首次绑定应成功");
        let err = LocalUserIdentities::link(test_db.pool(), bob, "feishu", "on_u1", None)
            .await
            .expect_err("第二次绑定必须失败");
        assert!(matches!(err, IdentityLinkError::AlreadyLinked), "{err:?}");

        // 归属没有被改掉。
        let found =
            LocalUserIdentities::find_by_provider_subject(test_db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(found.user_id, amy, "归属必须还在第一个账号上");
    }

    /// 重复绑同一个身份到**同一个**账号也算冲突（唯一索引不看 user_id），
    /// 上层据此走「已经绑过了」的分支，而不是插出第二行。
    #[tokio::test]
    async fn 重复绑定同一账号也会冲突() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", None)
            .await
            .unwrap();
        assert!(matches!(
            LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", None).await,
            Err(IdentityLinkError::AlreadyLinked)
        ));
    }

    #[tokio::test]
    async fn 同一账号可以绑多个提供方() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        for provider in ["feishu", "google", "lark"] {
            LocalUserIdentities::link(test_db.pool(), amy, provider, "sub-1", None)
                .await
                .unwrap_or_else(|err| panic!("{provider} 绑定失败：{err}"));
        }
        let list = LocalUserIdentities::list_for_user(test_db.pool(), amy)
            .await
            .unwrap();
        assert_eq!(
            list.iter().map(|i| i.provider.as_str()).collect::<Vec<_>>(),
            ["feishu", "google", "lark"]
        );
    }

    #[tokio::test]
    async fn 非法的_provider_或_subject_被拒() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        let 超长 = "x".repeat(MAX_SUBJECT_LEN + 1);
        for (provider, subject) in [
            ("", "s"),
            ("   ", "s"),
            ("feishu", ""),
            ("feishu", "   "),
            ("feishu", 超长.as_str()),
            (&"p".repeat(MAX_PROVIDER_LEN + 1), "s"),
        ] {
            let err = LocalUserIdentities::link(test_db.pool(), amy, provider, subject, None)
                .await
                .expect_err("非法入参必须被拒");
            assert!(matches!(err, IdentityLinkError::Validation(_)), "{err:?}");
        }
        assert!(
            LocalUserIdentities::list_for_user(test_db.pool(), amy)
                .await
                .unwrap()
                .is_empty(),
            "被拒的入参不得写进库里"
        );
    }

    /// 删用户时身份行必须跟着走（`ON DELETE CASCADE`），
    /// 否则留下的孤儿行会让同一个 union_id 再也绑不上任何账号。
    #[tokio::test]
    async fn 删用户时身份级联删除() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", None)
            .await
            .unwrap();
        sqlx::query("DELETE FROM local_users WHERE id = ?1")
            .bind(amy)
            .execute(test_db.pool())
            .await
            .unwrap();
        assert!(
            LocalUserIdentities::find_by_provider_subject(test_db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 建号与绑定在同一个事务里() {
        let test_db = TestDb::new().await;
        let user = LocalUserIdentities::create_user_with_identity(
            test_db.pool(),
            NewLocalUser {
                username: "feishu-abc".to_string(),
                display_name: "爱丽丝".to_string(),
                email: Some("alice@x.com".to_string()),
                password_hash: None,
                role: LocalUserRole::Member,
            },
            "feishu",
            "on_u1",
            Some("alice@x.com"),
        )
        .await
        .expect("建号 + 绑定应成功");
        assert_eq!(user.role, LocalUserRole::Member);
        assert_eq!(user.status, LocalUserStatus::Active);
        assert!(
            LocalUsers::find_password_hash(test_db.pool(), user.id)
                .await
                .unwrap()
                .is_none(),
            "第三方建号不该有密码"
        );
        assert_eq!(
            LocalUserIdentities::find_by_provider_subject(test_db.pool(), "feishu", "on_u1")
                .await
                .unwrap()
                .unwrap()
                .user_id,
            user.id
        );
    }

    /// 身份插不进去时用户也必须回滚：否则会留下一个既没密码也没身份的
    /// 幽灵账号——登不进来，也没人知道该删它。
    #[tokio::test]
    async fn 绑定失败时新建的用户被回滚() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", None)
            .await
            .unwrap();
        let 建号前 = LocalUsers::find_all(test_db.pool()).await.unwrap().len();

        let err = LocalUserIdentities::create_user_with_identity(
            test_db.pool(),
            NewLocalUser {
                username: "feishu-new".to_string(),
                display_name: "新人".to_string(),
                email: None,
                password_hash: None,
                role: LocalUserRole::Member,
            },
            "feishu",
            "on_u1",
            None,
        )
        .await
        .expect_err("身份已被占用，建号必须整体失败");
        assert!(matches!(err, IdentityLinkError::AlreadyLinked), "{err:?}");

        assert_eq!(
            LocalUsers::find_all(test_db.pool()).await.unwrap().len(),
            建号前,
            "失败的建号不得留下用户行"
        );
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "feishu-new")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// 身份表里**没有任何令牌列**。这条测试是「令牌不落库」在 schema 层的钉子：
    /// 有人加了 `access_token` 列，这里立刻红。
    #[tokio::test]
    async fn 身份表里没有任何令牌列() {
        let test_db = TestDb::new().await;
        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(local_user_identities)")
                .fetch_all(test_db.pool())
                .await
                .unwrap();
        let columns: Vec<String> = rows.into_iter().map(|r| r.1.to_lowercase()).collect();
        assert_eq!(
            columns,
            [
                "id",
                "user_id",
                "provider",
                "subject",
                "email",
                "created_at"
            ]
        );
        for 禁词 in ["token", "secret", "credential", "password", "refresh"] {
            assert!(
                !columns.iter().any(|c| c.contains(禁词)),
                "身份表不得有 {禁词} 列：{columns:?}"
            );
        }
    }

    /// 序列化出去的身份行里也不能有令牌。
    #[tokio::test]
    async fn 身份行序列化后不含令牌字段() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        let identity =
            LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_u1", Some("a@x.com"))
                .await
                .unwrap();
        let json = serde_json::to_string(&identity).unwrap();
        for 禁词 in ["token", "secret", "password", "refresh"] {
            assert!(!json.contains(禁词), "序列化泄露了 {禁词}：{json}");
        }
    }

    // ------------------------------------------------------- 按邮箱查找

    #[tokio::test]
    async fn 按邮箱查找大小写与空白不敏感() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", Some("Amy@Example.COM")).await;
        for raw in ["amy@example.com", "AMY@EXAMPLE.COM", "  Amy@Example.com  "] {
            assert_eq!(
                LocalUsers::find_by_email(test_db.pool(), raw)
                    .await
                    .unwrap()
                    .map(|u| u.id),
                Some(amy),
                "{raw:?} 应命中"
            );
        }
    }

    #[tokio::test]
    async fn 按邮箱查找空值与不存在都返回_none() {
        let test_db = TestDb::new().await;
        建用户(&test_db, "amy", Some("amy@example.com")).await;
        for raw in ["", "   ", "bob@example.com", "%", "amy@example.co"] {
            assert!(
                LocalUsers::find_by_email(test_db.pool(), raw)
                    .await
                    .unwrap()
                    .is_none(),
                "{raw:?} 不应命中"
            );
        }
        assert_eq!(normalize_email(Some("  ")), None);
    }

    // ------------------------------------------------------------ 解绑

    /// 把用户的密码抹掉，模拟「只靠第三方登录」的账号。
    async fn 去掉密码(test_db: &TestDb, user_id: Uuid) {
        LocalUsers::set_password_hash(test_db.pool(), user_id, None)
            .await
            .expect("清空密码失败");
    }

    async fn 身份数(test_db: &TestDb, user_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM local_user_identities WHERE user_id = ?1")
            .bind(user_id)
            .fetch_one(test_db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn 有密码时可以解绑最后一个身份() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();

        LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu")
            .await
            .expect("有密码兜底时应允许解绑");
        assert_eq!(身份数(&test_db, user_id).await, 0);
    }

    /// 最重要的一条：没有密码且只剩一个身份时解绑会把人锁在门外。
    #[tokio::test]
    async fn 无密码时不能解绑唯一的身份() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();
        去掉密码(&test_db, user_id).await;

        let err = LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu")
            .await
            .expect_err("唯一登录方式不该能解绑");
        assert!(matches!(err, IdentityUnlinkError::LastLoginMethod));
        assert_eq!(身份数(&test_db, user_id).await, 1, "绑定必须完好无损");
    }

    /// 无密码但还有第二个身份时，解掉其中一个是允许的。
    #[tokio::test]
    async fn 无密码但有两个身份时可以解绑其中一个() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();
        LocalUserIdentities::link(test_db.pool(), user_id, "google", "sub-1", None)
            .await
            .unwrap();
        去掉密码(&test_db, user_id).await;

        LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu")
            .await
            .expect("还有另一个身份时应允许解绑");
        assert_eq!(身份数(&test_db, user_id).await, 1);

        // 剩下的那个就解不掉了。
        let err = LocalUserIdentities::unlink(test_db.pool(), user_id, "google")
            .await
            .expect_err("剩最后一个时应被挡下");
        assert!(matches!(err, IdentityUnlinkError::LastLoginMethod));
        assert_eq!(身份数(&test_db, user_id).await, 1);
    }

    /// 解绑按 `user_id` 圈定范围：A 解不掉 B 的绑定。
    #[tokio::test]
    async fn 解绑不能跨用户() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        let bob = 建用户(&test_db, "bob", None).await;
        LocalUserIdentities::link(test_db.pool(), bob, "feishu", "on_bob", None)
            .await
            .unwrap();

        let err = LocalUserIdentities::unlink(test_db.pool(), amy, "feishu")
            .await
            .expect_err("不该动别人的绑定");
        assert!(matches!(err, IdentityUnlinkError::NotFound));
        assert_eq!(身份数(&test_db, bob).await, 1, "B 的绑定必须完好");
    }

    #[tokio::test]
    async fn 解绑不存在的绑定报未找到() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        for provider in [
            "feishu",
            "google",
            "lark",
            "",
            "   ",
            "x".repeat(200).as_str(),
        ] {
            let err = LocalUserIdentities::unlink(test_db.pool(), user_id, provider)
                .await
                .expect_err("不存在的绑定应报未找到");
            assert!(
                matches!(err, IdentityUnlinkError::NotFound),
                "provider = {provider:?}"
            );
        }
    }

    /// 重复解绑同一条：第二次是「未找到」而不是 panic。
    #[tokio::test]
    async fn 重复解绑同一条不会_panic() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();

        LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu")
            .await
            .expect("第一次应成功");
        let err = LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu")
            .await
            .expect_err("第二次应报未找到");
        assert!(matches!(err, IdentityUnlinkError::NotFound));
    }

    /// 并发解绑同一条：恰好一个成功，另一个报未找到，不会 panic。
    #[tokio::test]
    async fn 并发解绑同一条恰好成功一次() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu"),
            LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu"),
        );
        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "恰好一个成功"
        );
        assert_eq!(身份数(&test_db, user_id).await, 0);
    }

    /// 无密码 + 两个身份时并发解绑两个不同提供方：
    /// 数据库层的判据是单条 SQL，必须保证至少留下一个身份。
    #[tokio::test]
    async fn 并发解绑两个不同身份也不会把人锁在门外() {
        let test_db = TestDb::new().await;
        let user_id = 建用户(&test_db, "amy", None).await;
        LocalUserIdentities::link(test_db.pool(), user_id, "feishu", "on_u1", None)
            .await
            .unwrap();
        LocalUserIdentities::link(test_db.pool(), user_id, "google", "sub-1", None)
            .await
            .unwrap();
        去掉密码(&test_db, user_id).await;

        let _ = tokio::join!(
            LocalUserIdentities::unlink(test_db.pool(), user_id, "feishu"),
            LocalUserIdentities::unlink(test_db.pool(), user_id, "google"),
        );
        assert_eq!(
            身份数(&test_db, user_id).await,
            1,
            "无论并发怎么交错，都必须留下至少一个登录方式"
        );
    }

    #[tokio::test]
    async fn 列出当前用户的绑定只含自己的() {
        let test_db = TestDb::new().await;
        let amy = 建用户(&test_db, "amy", None).await;
        let bob = 建用户(&test_db, "bob", None).await;
        LocalUserIdentities::link(test_db.pool(), amy, "google", "sub-amy", None)
            .await
            .unwrap();
        LocalUserIdentities::link(test_db.pool(), amy, "feishu", "on_amy", None)
            .await
            .unwrap();
        LocalUserIdentities::link(test_db.pool(), bob, "lark", "on_bob", None)
            .await
            .unwrap();

        let list = LocalUserIdentities::list_for_user(test_db.pool(), amy)
            .await
            .expect("列表应成功");
        let providers: Vec<&str> = list.iter().map(|it| it.provider.as_str()).collect();
        assert_eq!(providers, ["feishu", "google"], "只含自己的，按提供方排序");
    }
}
