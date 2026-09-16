//! 本地账号体系的会话 / 第三方身份 / 邀请码模型。
//!
//! 表结构见 `migrations/20260917000000_add_local_auth.sql`。

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
