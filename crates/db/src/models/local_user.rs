//! 本地用户模型。表结构见 `migrations/20260917000000_add_local_auth.sql`。
//!
//! 安全约定：[`LocalUser`] 上**没有** `password_hash` 字段，哈希只能经
//! [`LocalUsers::find_password_hash`] 单独取。这样任何 `Json(user)` 都不可能
//! 把密码哈希漏到响应里。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, Type};
use thiserror::Error;
use uuid::Uuid;

use super::db_retry::{is_unique_violation, retry_on_busy};

/// 用户名长度上限。
pub const MAX_USERNAME_LEN: usize = 64;
/// 显示名长度上限。
pub const MAX_DISPLAY_NAME_LEN: usize = 100;
/// 邮箱长度上限。
pub const MAX_EMAIL_LEN: usize = 254;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, ts_rs::TS)]
#[sqlx(type_name = "local_user_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum LocalUserRole {
    Admin,
    Member,
}

impl LocalUserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalUserRole::Admin => "admin",
            LocalUserRole::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, ts_rs::TS)]
#[sqlx(type_name = "local_user_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum LocalUserStatus {
    Active,
    Disabled,
}

impl LocalUserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalUserStatus::Active => "active",
            LocalUserStatus::Disabled => "disabled",
        }
    }
}

/// 对外可见的用户行。**不含 `password_hash`**，见模块文档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct LocalUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: LocalUserRole,
    pub status: LocalUserStatus,
    pub avatar_color: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// 建号入参。`password_hash` 由 `services::local_auth::password` 算好后传进来，
/// 本层不接触明文密码。
#[derive(Debug, Clone)]
pub struct NewLocalUser {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub role: LocalUserRole,
}

#[derive(Debug, Error)]
pub enum LocalUserError {
    #[error("用户名只允许 1-64 个小写字母、数字与 . - _")]
    InvalidUsername,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("用户不存在")]
    NotFound,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// 用户名规范化：去两侧空白 + 转小写。
///
/// 只做 ASCII 小写化：非 ASCII 字符会在 [`validate_username`] 里被整体拒绝，
/// 所以这里不需要（也不应该）走 Unicode 大小写折叠——`İ`→`i̇` 这类映射会让
/// 「规范化两次结果不同」，给同形字攻击留口子。
pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// 用户名字符集：`[a-z0-9._-]`，长度 1..=64。
///
/// 只放行 ASCII 是刻意的：西里尔 `а`（U+0430）与拉丁 `a` 在界面上完全一样，
/// 放进来就能冒充别人；零宽字符同理。
pub fn validate_username(normalized: &str) -> Result<(), LocalUserError> {
    if normalized.is_empty() || normalized.len() > MAX_USERNAME_LEN {
        return Err(LocalUserError::InvalidUsername);
    }
    if !normalized
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(LocalUserError::InvalidUsername);
    }
    Ok(())
}

/// 邮箱规范化：去空白 + 转小写；空串视为未填。
pub fn normalize_email(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_lowercase())
}

pub struct LocalUsers;

impl LocalUsers {
    pub async fn create(
        pool: &SqlitePool,
        data: NewLocalUser,
    ) -> Result<LocalUser, LocalUserError> {
        let prepared = PreparedUser::new(data)?;
        let result = retry_on_busy(|| {
            let prepared = prepared.clone();
            async move { prepared.insert(pool).await }
        })
        .await;
        result.map_err(map_insert_error)
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<LocalUser>, sqlx::Error> {
        sqlx::query_as!(
            LocalUser,
            r#"SELECT id            AS "id!: Uuid",
                      username      AS "username!",
                      display_name  AS "display_name!",
                      email         AS "email?",
                      role          AS "role!: LocalUserRole",
                      status        AS "status!: LocalUserStatus",
                      avatar_color  AS "avatar_color!",
                      created_at    AS "created_at!: DateTime<Utc>",
                      updated_at    AS "updated_at!: DateTime<Utc>",
                      last_login_at AS "last_login_at?: DateTime<Utc>"
               FROM local_users
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    /// 按用户名查找。入参会先经 [`normalize_username`]，因此 `LOCAL` 与 ` local `
    /// 命中同一行；不做任何前缀 / 模糊匹配。
    pub async fn find_by_username(
        pool: &SqlitePool,
        raw_username: &str,
    ) -> Result<Option<LocalUser>, sqlx::Error> {
        let username = normalize_username(raw_username);
        sqlx::query_as!(
            LocalUser,
            r#"SELECT id            AS "id!: Uuid",
                      username      AS "username!",
                      display_name  AS "display_name!",
                      email         AS "email?",
                      role          AS "role!: LocalUserRole",
                      status        AS "status!: LocalUserStatus",
                      avatar_color  AS "avatar_color!",
                      created_at    AS "created_at!: DateTime<Utc>",
                      updated_at    AS "updated_at!: DateTime<Utc>",
                      last_login_at AS "last_login_at?: DateTime<Utc>"
               FROM local_users
               WHERE username = $1"#,
            username
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<LocalUser>, sqlx::Error> {
        sqlx::query_as!(
            LocalUser,
            r#"SELECT id            AS "id!: Uuid",
                      username      AS "username!",
                      display_name  AS "display_name!",
                      email         AS "email?",
                      role          AS "role!: LocalUserRole",
                      status        AS "status!: LocalUserStatus",
                      avatar_color  AS "avatar_color!",
                      created_at    AS "created_at!: DateTime<Utc>",
                      updated_at    AS "updated_at!: DateTime<Utc>",
                      last_login_at AS "last_login_at?: DateTime<Utc>"
               FROM local_users
               ORDER BY username ASC"#
        )
        .fetch_all(pool)
        .await
    }

    /// 密码哈希的唯一出口。用户不存在或没设密码都返回 `None`。
    pub async fn find_password_hash(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<Option<String>> =
            sqlx::query_scalar!(r#"SELECT password_hash FROM local_users WHERE id = $1"#, id)
                .fetch_optional(pool)
                .await?;
        Ok(row.flatten())
    }

    pub async fn set_password_hash(
        pool: &SqlitePool,
        id: Uuid,
        password_hash: Option<&str>,
    ) -> Result<(), LocalUserError> {
        Self::update_one(move || {
            let password_hash = password_hash.map(str::to_string);
            let now = Utc::now();
            async move {
                sqlx::query!(
                    r#"UPDATE local_users SET password_hash = $2, updated_at = $3 WHERE id = $1"#,
                    id,
                    password_hash,
                    now
                )
                .execute(pool)
                .await
            }
        })
        .await
    }

    pub async fn set_role(
        pool: &SqlitePool,
        id: Uuid,
        role: LocalUserRole,
    ) -> Result<(), LocalUserError> {
        Self::update_one(move || {
            let now = Utc::now();
            async move {
                sqlx::query!(
                    r#"UPDATE local_users SET role = $2, updated_at = $3 WHERE id = $1"#,
                    id,
                    role,
                    now
                )
                .execute(pool)
                .await
            }
        })
        .await
    }

    pub async fn set_status(
        pool: &SqlitePool,
        id: Uuid,
        status: LocalUserStatus,
    ) -> Result<(), LocalUserError> {
        Self::update_one(move || {
            let now = Utc::now();
            async move {
                sqlx::query!(
                    r#"UPDATE local_users SET status = $2, updated_at = $3 WHERE id = $1"#,
                    id,
                    status,
                    now
                )
                .execute(pool)
                .await
            }
        })
        .await
    }

    pub async fn update_profile(
        pool: &SqlitePool,
        id: Uuid,
        display_name: &str,
        email: Option<&str>,
    ) -> Result<(), LocalUserError> {
        let display_name: String = display_name
            .trim()
            .chars()
            .take(MAX_DISPLAY_NAME_LEN)
            .collect();
        if display_name.is_empty() {
            return Err(LocalUserError::Validation("显示名不能为空".to_string()));
        }
        let email = normalize_email(email);

        let result = retry_on_busy(|| {
            let display_name = display_name.clone();
            let email = email.clone();
            let now = Utc::now();
            async move {
                sqlx::query!(
                    r#"UPDATE local_users
                       SET display_name = $2, email = $3, updated_at = $4
                       WHERE id = $1"#,
                    id,
                    display_name,
                    email,
                    now
                )
                .execute(pool)
                .await
            }
        })
        .await;

        match result {
            Ok(done) if done.rows_affected() == 0 => Err(LocalUserError::NotFound),
            Ok(_) => Ok(()),
            Err(err) if is_unique_violation(&err) => {
                Err(LocalUserError::Conflict("邮箱已被占用".to_string()))
            }
            Err(err) => Err(LocalUserError::Database(err)),
        }
    }

    pub async fn touch_last_login(
        pool: &SqlitePool,
        id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), LocalUserError> {
        Self::update_one(move || async move {
            sqlx::query!(
                r#"UPDATE local_users SET last_login_at = $2 WHERE id = $1"#,
                id,
                at
            )
            .execute(pool)
            .await
        })
        .await
    }

    /// 「能真正登录进来的管理员」数量：`role=admin` 且 `status=active`，
    /// 并且至少有一种凭据（密码哈希或已绑定的第三方身份）。
    ///
    /// 迁移写入的本机用户是 admin 但没有密码，所以团队模式首启时这里是 0，
    /// `/api/local-auth/bootstrap` 据此判断要不要走初始化向导。
    pub async fn count_login_capable_admins(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!: i64"
               FROM local_users u
               WHERE u.role = 'admin'
                 AND u.status = 'active'
                 AND (u.password_hash IS NOT NULL
                      OR EXISTS (SELECT 1 FROM local_user_identities i WHERE i.user_id = u.id))"#
        )
        .fetch_one(pool)
        .await
    }

    /// 同 [`Self::count_login_capable_admins`]，但排除某一个用户。
    ///
    /// 成员管理用它回答「把这个人降权 / 停用之后，还剩下能登录的管理员吗」。
    /// 判据必须和 `count_login_capable_admins` 完全一致（含「至少有一种凭据」），
    /// 否则迁移写入的那个没有密码的本机 admin 会被当成「还剩一个」，
    /// 于是唯一的真管理员被降权，谁也登不进来，也没人能改回来。
    pub async fn count_login_capable_admins_except(
        pool: &SqlitePool,
        exclude_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!: i64"
               FROM local_users u
               WHERE u.role = 'admin'
                 AND u.status = 'active'
                 AND u.id <> $1
                 AND (u.password_hash IS NOT NULL
                      OR EXISTS (SELECT 1 FROM local_user_identities i WHERE i.user_id = u.id))"#,
            exclude_id
        )
        .fetch_one(pool)
        .await
    }

    /// 单行 UPDATE 的公共外壳：带 busy 重试，打不中行一律 `NotFound`
    /// （不能静默成功，否则「改了不存在的用户」会被当成改成功）。
    async fn update_one<F, Fut>(run: F) -> Result<(), LocalUserError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error>>,
    {
        match retry_on_busy(run).await {
            Ok(done) if done.rows_affected() == 0 => Err(LocalUserError::NotFound),
            Ok(_) => Ok(()),
            Err(err) if is_unique_violation(&err) => {
                Err(LocalUserError::Conflict("唯一约束冲突".to_string()))
            }
            Err(err) => Err(LocalUserError::Database(err)),
        }
    }
}

/// 校验并补全过的建号入参，随时可以插进任意执行器（连接池或事务）。
///
/// 拆出来是为了让「邀请码注册」能把建号与消费邀请放进**同一个事务**：
/// [`LocalUsers::create`] 只接受 `&SqlitePool`，在事务里用不了。
/// 校验（用户名字符集、邮箱长度）在 [`Self::new`] 里就做完，
/// 因此事务开启之后不会再因为参数非法而半途失败。
#[derive(Debug, Clone)]
pub(crate) struct PreparedUser {
    id: Uuid,
    username: String,
    display_name: String,
    email: Option<String>,
    password_hash: Option<String>,
    role: LocalUserRole,
    status: LocalUserStatus,
    avatar_color: String,
    now: DateTime<Utc>,
}

impl PreparedUser {
    pub(crate) fn new(data: NewLocalUser) -> Result<Self, LocalUserError> {
        let username = normalize_username(&data.username);
        validate_username(&username)?;

        let display_name: String = data
            .display_name
            .trim()
            .chars()
            .take(MAX_DISPLAY_NAME_LEN)
            .collect();
        let display_name = if display_name.is_empty() {
            username.clone()
        } else {
            display_name
        };

        let email = normalize_email(data.email.as_deref());
        if let Some(email) = &email
            && email.chars().count() > MAX_EMAIL_LEN
        {
            return Err(LocalUserError::Validation("邮箱过长".to_string()));
        }

        let avatar_color = avatar_color_for(&username);
        Ok(Self {
            id: Uuid::new_v4(),
            username,
            display_name,
            email,
            password_hash: data.password_hash,
            role: data.role,
            status: LocalUserStatus::Active,
            avatar_color,
            now: Utc::now(),
        })
    }

    pub(crate) async fn insert<'e, E>(&self, executor: E) -> Result<LocalUser, sqlx::Error>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        sqlx::query_as!(
            LocalUser,
            r#"INSERT INTO local_users
                   (id, username, display_name, email, password_hash, role, status,
                    avatar_color, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
               RETURNING id            AS "id!: Uuid",
                         username      AS "username!",
                         display_name  AS "display_name!",
                         email         AS "email?",
                         role          AS "role!: LocalUserRole",
                         status        AS "status!: LocalUserStatus",
                         avatar_color  AS "avatar_color!",
                         created_at    AS "created_at!: DateTime<Utc>",
                         updated_at    AS "updated_at!: DateTime<Utc>",
                         last_login_at AS "last_login_at?: DateTime<Utc>""#,
            self.id,
            self.username,
            self.display_name,
            self.email,
            self.password_hash,
            self.role,
            self.status,
            self.avatar_color,
            self.now
        )
        .fetch_one(executor)
        .await
    }
}

/// 建号 INSERT 的错误映射：唯一约束冲突 → 409，其余原样往上抛。
pub(crate) fn map_insert_error(err: sqlx::Error) -> LocalUserError {
    if is_unique_violation(&err) {
        LocalUserError::Conflict("用户名或邮箱已被占用".to_string())
    } else {
        LocalUserError::Database(err)
    }
}

/// 头像底色：按用户名哈希到一组固定色，保证同一用户每次都是同一个颜色。
fn avatar_color_for(username: &str) -> String {
    const PALETTE: [&str; 8] = [
        "#6366f1", "#0ea5e9", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#14b8a6",
    ];
    let sum: u32 = username.bytes().map(u32::from).sum();
    PALETTE[(sum as usize) % PALETTE.len()].to_string()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{models::local_project::DEFAULT_USER_ID, test_support::TestDb};

    fn 新用户(username: &str) -> NewLocalUser {
        NewLocalUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: None,
            role: LocalUserRole::Member,
        }
    }

    #[test]
    fn 用户名规范化为小写并去空白() {
        assert_eq!(normalize_username(" Alice "), "alice");
        assert_eq!(normalize_username("LOCAL"), "local");
        assert_eq!(normalize_username("a.b-c_1"), "a.b-c_1");
    }

    #[test]
    fn 用户名字符集受限() {
        // 空、超长、含空格、含 NUL、西里尔同形字、含 @ 一律拒绝。
        for bad in [
            "",
            "   ",
            &"a".repeat(65),
            "ad min",
            "admin\u{0}",
            "аdmin", // 首字母是西里尔 U+0430，看起来和 ASCII a 一样
            "admin@x",
            "ad\nmin", // 内部换行（两侧的空白会被 trim 掉，属于合法输入）
            "ad/min",
            "管理员",
            "admin\u{200b}", // 零宽空格
        ] {
            assert!(
                matches!(
                    validate_username(&normalize_username(bad)),
                    Err(LocalUserError::InvalidUsername)
                ),
                "{bad:?} 应被拒绝"
            );
        }
        for good in ["a", "a.b-c_1", "local", &"a".repeat(64)] {
            validate_username(&normalize_username(good))
                .unwrap_or_else(|e| panic!("{good:?} 应合法：{e}"));
        }
    }

    #[tokio::test]
    async fn 大小写不同的用户名撞唯一约束() {
        let test_db = TestDb::new().await;
        LocalUsers::create(test_db.pool(), 新用户("Alice"))
            .await
            .expect("建 Alice 失败");
        let err = LocalUsers::create(test_db.pool(), 新用户("alice"))
            .await
            .expect_err("同名（忽略大小写）必须冲突");
        assert!(
            matches!(err, LocalUserError::Conflict(_)),
            "应是 409 冲突而不是 500，实际：{err:?}"
        );
    }

    #[tokio::test]
    async fn 非法用户名在写库前就被拒绝() {
        let test_db = TestDb::new().await;
        let err = LocalUsers::create(test_db.pool(), 新用户("ad min"))
            .await
            .expect_err("非法用户名必须被拒");
        assert!(matches!(err, LocalUserError::InvalidUsername));
    }

    #[tokio::test]
    async fn 固定本机用户可按_id_查到() {
        let test_db = TestDb::new().await;
        let user = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .expect("迁移应写入本机用户");
        assert_eq!(user.username, "local");
        assert_eq!(user.role, LocalUserRole::Admin);
        assert_eq!(user.status, LocalUserStatus::Active);
    }

    #[tokio::test]
    async fn 按用户名查找忽略大小写() {
        let test_db = TestDb::new().await;
        let user = LocalUsers::find_by_username(test_db.pool(), "  LOCAL ")
            .await
            .unwrap()
            .expect("应命中本机用户");
        assert_eq!(user.id, DEFAULT_USER_ID);
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "loca")
                .await
                .unwrap()
                .is_none(),
            "不得做前缀匹配"
        );
    }

    #[tokio::test]
    async fn 停用用户仍可查到但状态是_disabled() {
        let test_db = TestDb::new().await;
        let user = LocalUsers::create(test_db.pool(), 新用户("bob"))
            .await
            .unwrap();
        LocalUsers::set_status(test_db.pool(), user.id, LocalUserStatus::Disabled)
            .await
            .unwrap();
        let again = LocalUsers::find_by_id(test_db.pool(), user.id)
            .await
            .unwrap()
            .expect("停用不是删除");
        assert_eq!(again.status, LocalUserStatus::Disabled);
    }

    /// 密码哈希绝不能出现在 `LocalUser` 上，否则任何 `Json(user)` 都会把它漏出去。
    #[tokio::test]
    async fn 用户结构体序列化后不含密码哈希() {
        let test_db = TestDb::new().await;
        let mut data = 新用户("carol");
        data.password_hash = Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string());
        let user = LocalUsers::create(test_db.pool(), data).await.unwrap();

        let json = serde_json::to_string(&user).unwrap();
        assert!(
            !json.contains("password"),
            "序列化结果不得含 password 字段：{json}"
        );
        assert!(!json.contains("argon2"), "序列化结果不得含哈希串：{json}");

        // 哈希只能通过专门的取值函数拿到。
        let hash = LocalUsers::find_password_hash(test_db.pool(), user.id)
            .await
            .unwrap()
            .expect("哈希应已写入");
        assert!(hash.starts_with("$argon2id$"));
    }

    #[tokio::test]
    async fn 没有密码的用户取哈希返回_none() {
        let test_db = TestDb::new().await;
        assert!(
            LocalUsers::find_password_hash(test_db.pool(), DEFAULT_USER_ID)
                .await
                .unwrap()
                .is_none(),
            "个人版本机用户没有密码"
        );
        assert!(
            LocalUsers::find_password_hash(test_db.pool(), Uuid::new_v4())
                .await
                .unwrap()
                .is_none(),
            "不存在的用户也返回 None 而不是报错"
        );
    }

    #[tokio::test]
    async fn 列表按用户名排序() {
        let test_db = TestDb::new().await;
        for name in ["zoe", "amy", "mike"] {
            LocalUsers::create(test_db.pool(), 新用户(name))
                .await
                .unwrap();
        }
        let names: Vec<String> = LocalUsers::find_all(test_db.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|u| u.username)
            .collect();
        assert_eq!(names, vec!["amy", "local", "mike", "zoe"]);
    }

    #[tokio::test]
    async fn 邮箱规范化并且唯一() {
        let test_db = TestDb::new().await;
        let mut data = 新用户("dave");
        data.email = Some("  Dave@Example.COM ".to_string());
        let user = LocalUsers::create(test_db.pool(), data).await.unwrap();
        assert_eq!(user.email.as_deref(), Some("dave@example.com"));

        let mut dup = 新用户("dave2");
        dup.email = Some("DAVE@example.com".to_string());
        let err = LocalUsers::create(test_db.pool(), dup)
            .await
            .expect_err("同一邮箱不得建两个号");
        assert!(matches!(err, LocalUserError::Conflict(_)));
    }

    #[tokio::test]
    async fn 空邮箱视为未填并且可以有多个() {
        let test_db = TestDb::new().await;
        for name in ["e1", "e2"] {
            let mut data = 新用户(name);
            data.email = Some("   ".to_string());
            let user = LocalUsers::create(test_db.pool(), data).await.unwrap();
            assert!(user.email.is_none());
        }
    }

    #[tokio::test]
    async fn 改密码与改角色() {
        let test_db = TestDb::new().await;
        let user = LocalUsers::create(test_db.pool(), 新用户("frank"))
            .await
            .unwrap();

        LocalUsers::set_password_hash(test_db.pool(), user.id, Some("$argon2id$new"))
            .await
            .unwrap();
        assert_eq!(
            LocalUsers::find_password_hash(test_db.pool(), user.id)
                .await
                .unwrap()
                .as_deref(),
            Some("$argon2id$new")
        );

        LocalUsers::set_role(test_db.pool(), user.id, LocalUserRole::Admin)
            .await
            .unwrap();
        assert_eq!(
            LocalUsers::find_by_id(test_db.pool(), user.id)
                .await
                .unwrap()
                .unwrap()
                .role,
            LocalUserRole::Admin
        );

        // 清空密码（只留第三方登录）。
        LocalUsers::set_password_hash(test_db.pool(), user.id, None)
            .await
            .unwrap();
        assert!(
            LocalUsers::find_password_hash(test_db.pool(), user.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 改动不存在的用户返回_notfound_而不是静默成功() {
        let test_db = TestDb::new().await;
        let ghost = Uuid::new_v4();
        assert!(matches!(
            LocalUsers::set_status(test_db.pool(), ghost, LocalUserStatus::Disabled).await,
            Err(LocalUserError::NotFound)
        ));
        assert!(matches!(
            LocalUsers::set_role(test_db.pool(), ghost, LocalUserRole::Admin).await,
            Err(LocalUserError::NotFound)
        ));
        assert!(matches!(
            LocalUsers::set_password_hash(test_db.pool(), ghost, Some("x")).await,
            Err(LocalUserError::NotFound)
        ));
    }

    #[tokio::test]
    async fn 记录最近登录时间() {
        let test_db = TestDb::new().await;
        assert!(
            LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
                .await
                .unwrap()
                .unwrap()
                .last_login_at
                .is_none()
        );
        let now = chrono::Utc::now();
        LocalUsers::touch_last_login(test_db.pool(), DEFAULT_USER_ID, now)
            .await
            .unwrap();
        let stored = LocalUsers::find_by_id(test_db.pool(), DEFAULT_USER_ID)
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .expect("应已记录");
        assert!((stored - now).num_seconds().abs() < 2);
    }

    /// 首启向导的判据：迁移写入的本机用户没有密码也没有第三方身份，
    /// 因此团队模式下必须先创建管理员。
    #[tokio::test]
    async fn 可登录的管理员计数() {
        let test_db = TestDb::new().await;
        assert_eq!(
            LocalUsers::count_login_capable_admins(test_db.pool())
                .await
                .unwrap(),
            0,
            "本机用户没有密码，不算可登录的管理员"
        );

        // 普通成员设了密码也不算。
        let mut member = 新用户("grace");
        member.password_hash = Some("$argon2id$x".to_string());
        LocalUsers::create(test_db.pool(), member).await.unwrap();
        assert_eq!(
            LocalUsers::count_login_capable_admins(test_db.pool())
                .await
                .unwrap(),
            0
        );

        LocalUsers::set_password_hash(test_db.pool(), DEFAULT_USER_ID, Some("$argon2id$y"))
            .await
            .unwrap();
        assert_eq!(
            LocalUsers::count_login_capable_admins(test_db.pool())
                .await
                .unwrap(),
            1
        );

        // 停用后又不算了。
        LocalUsers::set_status(test_db.pool(), DEFAULT_USER_ID, LocalUserStatus::Disabled)
            .await
            .unwrap();
        assert_eq!(
            LocalUsers::count_login_capable_admins(test_db.pool())
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn 只绑了第三方身份的管理员也算可登录() {
        let test_db = TestDb::new().await;
        sqlx::query(
            "INSERT INTO local_user_identities (id, user_id, provider, subject) \
             VALUES (?1, ?2, 'feishu', 'ou_admin')",
        )
        .bind(Uuid::new_v4())
        .bind(DEFAULT_USER_ID)
        .execute(test_db.pool())
        .await
        .unwrap();
        assert_eq!(
            LocalUsers::count_login_capable_admins(test_db.pool())
                .await
                .unwrap(),
            1
        );
    }

    #[test]
    fn 角色与状态的字符串映射与库里一致() {
        assert_eq!(LocalUserRole::Admin.as_str(), "admin");
        assert_eq!(LocalUserRole::Member.as_str(), "member");
        assert_eq!(LocalUserStatus::Active.as_str(), "active");
        assert_eq!(LocalUserStatus::Disabled.as_str(), "disabled");
    }
}
