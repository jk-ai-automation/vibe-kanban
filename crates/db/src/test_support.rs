//! 测试用的一次性 SQLite 数据库。
//! 每个 TestDb 拥有自己的临时目录，Drop 时自动删除。

use std::{future::Future, pin::Pin};

use sqlx::{
    Error as SqlxError, SqlitePool,
    sqlite::{SqliteConnection, SqliteJournalMode},
};
use tempfile::TempDir;

use crate::{DBService, journal_mode_from_env};

pub struct TestDb {
    pub db: DBService,
    _dir: TempDir,
}

impl TestDb {
    /// journal mode 走 `VK_SQLITE_WAL` 环境变量（默认 WAL），与生产路径一致。
    pub async fn new() -> Self {
        Self::new_with_journal(journal_mode_from_env()).await
    }

    /// 显式指定 journal mode，不受环境变量影响。用于对照测试「切 WAL / 退回 Delete
    /// 两条路径行为一致」。
    pub async fn new_with_journal(journal_mode: SqliteJournalMode) -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path_with_journal(&dir.path().join("test.sqlite"), journal_mode)
            .await
            .expect("初始化测试数据库失败");
        Self { db, _dir: dir }
    }

    /// 带 `after_connect` 钩子（例如 `EventService::create_hook` 装的变更钩子），
    /// journal mode 走环境变量，默认 WAL。
    pub async fn new_with_hook<F>(after_connect: F) -> Self
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            )
                -> Pin<Box<dyn Future<Output = Result<(), SqlxError>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        Self::new_with_journal_and_hook(journal_mode_from_env(), after_connect).await
    }

    /// 同 [`Self::new_with_hook`]，但显式指定 journal mode。
    /// 用来验证「变更钩子在 WAL 下和 Delete 下行为一致」（同一段测试代码跑两遍）。
    pub async fn new_with_journal_and_hook<F>(
        journal_mode: SqliteJournalMode,
        after_connect: F,
    ) -> Self
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            )
                -> Pin<Box<dyn Future<Output = Result<(), SqlxError>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path_with_after_connect(
            &dir.path().join("test.sqlite"),
            journal_mode,
            after_connect,
        )
        .await
        .expect("初始化测试数据库失败");
        Self { db, _dir: dir }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.db.pool
    }
}

#[cfg(test)]
mod tests {
    use super::TestDb;

    /// 列出某张表的列名，顺序与 SQLite 存储顺序一致。
    async fn column_names(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
        // table 来自测试常量，不接受外部输入；PRAGMA 不支持绑定参数。
        let sql = format!("PRAGMA table_info({table})");
        let rows: Vec<(i64, String)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .expect("读取表结构失败")
            .into_iter()
            .map(|r: (i64, String, String, i64, Option<String>, i64)| (r.0, r.1))
            .collect();
        rows.into_iter().map(|(_, name)| name).collect()
    }

    #[tokio::test]
    async fn 迁移后新表存在且_id_是第零列() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();

        for table in [
            "local_projects",
            "project_statuses",
            "issues",
            "project_tags",
            "issue_tags",
            "issue_comments",
        ] {
            let columns = column_names(pool, table).await;
            assert!(!columns.is_empty(), "表 {table} 不存在");
            assert_eq!(columns[0], "id", "表 {table} 的 id 必须是第 0 列");
        }
    }

    #[tokio::test]
    async fn workspaces_新增_issue_id_列且外键为_set_null() {
        let test_db = TestDb::new().await;
        let columns = column_names(test_db.pool(), "workspaces").await;
        assert!(
            columns.contains(&"issue_id".to_string()),
            "workspaces 缺少 issue_id 列，实际列：{columns:?}"
        );

        let fks: Vec<(String, String)> = sqlx::query_as(
            "SELECT \"table\", on_delete FROM pragma_foreign_key_list('workspaces')",
        )
        .fetch_all(test_db.pool())
        .await
        .expect("读取外键失败");

        assert!(
            fks.iter()
                .any(|(table, on_delete)| table == "issues" && on_delete == "SET NULL"),
            "workspaces.issue_id 必须是 ON DELETE SET NULL，实际外键：{fks:?}"
        );
    }

    #[tokio::test]
    async fn 遗留表未被删除() {
        let test_db = TestDb::new().await;
        let legacy = column_names(test_db.pool(), "projects").await;
        assert!(!legacy.is_empty(), "遗留 projects 表不应被本迁移删除");
    }

    /// 在「只跑到旧迁移」的库上再跑一次全量迁移，验证新迁移能在既有数据上应用，
    /// 且重复应用不会报错（sqlx 按版本号记账，已应用的不会重跑）。
    #[tokio::test]
    async fn 新迁移可以应用在已有旧迁移的库上并且可重复执行() {
        use sqlx::{Row, sqlite::SqliteConnectOptions};

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("legacy.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("连接失败");

        // 只跑到 20260916000000 为止，模拟升级前的库。
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
            .await
            .expect("读取迁移目录失败");
        let legacy: Vec<_> = migrator
            .iter()
            .filter(|m| m.version <= 20260916000000)
            .cloned()
            .collect();
        let mut legacy_migrator =
            sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await
                .expect("读取迁移目录失败");
        legacy_migrator.migrations = legacy.into();
        legacy_migrator.run(&pool).await.expect("旧迁移应成功");

        // 造一点旧数据：项目 + 状态列 + 两条需求（时间戳走旧的 DEFAULT 格式）。
        let project_id = uuid::Uuid::new_v4();
        let status_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO local_projects (id, organization_id, name) VALUES (?1, ?2, 'Legacy Project')")
            .bind(project_id)
            .bind(uuid::Uuid::from_u128(1))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO project_statuses (id, project_id, name) VALUES (?1, ?2, '待开发')",
        )
        .bind(status_id)
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();
        for number in [1_i64, 7] {
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(project_id)
            .bind(number)
            .bind(format!("LP-{number}"))
            .bind(status_id)
            .bind(format!("旧需求 {number}"))
            .execute(&pool)
            .await
            .unwrap();
        }

        // 升级到最新。
        migrator.run(&pool).await.expect("新迁移应能应用在旧库上");

        let row = sqlx::query(
            "SELECT simple_id_prefix, next_issue_number, created_at FROM local_projects WHERE id = ?1",
        )
        .bind(project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.get::<String, _>("simple_id_prefix"),
            "LP",
            "前缀应沿用已有需求的 simple_id 前缀"
        );
        assert_eq!(
            row.get::<i64, _>("next_issue_number"),
            8,
            "编号游标应回填成 MAX(issue_number) + 1"
        );
        assert!(
            row.get::<String, _>("created_at").contains('T'),
            "旧格式时间戳应被规整成 RFC3339"
        );
        let issue_created: String =
            sqlx::query_scalar("SELECT created_at FROM issues WHERE project_id = ?1 LIMIT 1")
                .bind(project_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(issue_created.contains('T'), "需求的时间戳也应被规整");

        // 重复执行：已应用的迁移不会重跑，数据也不会被二次改写。
        migrator.run(&pool).await.expect("重复执行迁移应成功");
        let again: i64 =
            sqlx::query_scalar("SELECT next_issue_number FROM local_projects WHERE id = ?1")
                .bind(project_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(again, 8, "重复执行迁移不得改动已回填的数据");
    }

    /// 团队版认证迁移（`20260917000000_add_local_auth.sql`）只新增表和一条本机用户，
    /// **不得**改写历史需求 / 评论上已有的 `creator_user_id` / `author_id`——
    /// 即便这些值指向一个迁移后并不存在的用户 id，也必须原样保留。
    #[tokio::test]
    async fn 认证迁移不改写历史需求的创建人() {
        use sqlx::Row;

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("legacy_auth.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("连接失败");

        // 只跑到 20260916010000（团队版认证迁移之前），模拟升级前的库。
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
            .await
            .expect("读取迁移目录失败");
        let legacy: Vec<_> = migrator
            .iter()
            .filter(|m| m.version <= 20260916010000)
            .cloned()
            .collect();
        let mut legacy_migrator =
            sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await
                .expect("读取迁移目录失败");
        legacy_migrator.migrations = legacy.into();
        legacy_migrator.run(&pool).await.expect("旧迁移应成功");

        let project_id = uuid::Uuid::new_v4();
        let status_id = uuid::Uuid::new_v4();
        let issue_id = uuid::Uuid::new_v4();
        // 一个迁移后必然不存在的用户 id：任何「回填/改写」都会把它换成别的值，
        // 只有「一行不改」才能让下面的断言通过。
        let stale_user_id = uuid::Uuid::from_u128(0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF);

        sqlx::query(
            "INSERT INTO local_projects (id, organization_id, name) VALUES (?1, ?2, 'Legacy Project')",
        )
        .bind(project_id)
        .bind(uuid::Uuid::from_u128(1))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO project_statuses (id, project_id, name) VALUES (?1, ?2, '待开发')",
        )
        .bind(status_id)
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title, creator_user_id) \
             VALUES (?1, ?2, 1, 'LP-1', ?3, '旧需求', ?4)",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(status_id)
        .bind(stale_user_id)
        .execute(&pool)
        .await
        .unwrap();
        let comment_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO issue_comments (id, issue_id, author_id, message) VALUES (?1, ?2, ?3, '旧评论')",
        )
        .bind(comment_id)
        .bind(issue_id)
        .bind(stale_user_id)
        .execute(&pool)
        .await
        .unwrap();

        // 升级到最新（含 20260917000000_add_local_auth.sql）。
        migrator.run(&pool).await.expect("认证迁移应能应用在旧库上");

        let issue_creator: Vec<u8> =
            sqlx::query("SELECT creator_user_id FROM issues WHERE id = ?1")
                .bind(issue_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("creator_user_id");
        assert_eq!(
            issue_creator,
            stale_user_id.as_bytes().to_vec(),
            "认证迁移不得改写历史需求的 creator_user_id"
        );

        let comment_author: Vec<u8> =
            sqlx::query("SELECT author_id FROM issue_comments WHERE id = ?1")
                .bind(comment_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("author_id");
        assert_eq!(
            comment_author,
            stale_user_id.as_bytes().to_vec(),
            "认证迁移不得改写历史评论的 author_id"
        );

        // 新迁移应插入本机用户行，但不应影响上面两条历史记录。
        let local_user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(local_user_count, 1, "应只有迁移写入的那一条本机用户");
    }

    /// `20260917010000_add_workspace_created_by_user_id.sql`：新增列的位置与外键约定。
    #[tokio::test]
    async fn workspaces_新增_created_by_user_id_列且外键为_set_null() {
        let test_db = TestDb::new().await;
        let columns = column_names(test_db.pool(), "workspaces").await;
        assert!(
            columns.contains(&"created_by_user_id".to_string()),
            "workspaces 缺少 created_by_user_id 列，实际列：{columns:?}"
        );
        assert_eq!(columns[0], "id", "加列后 id 仍必须是第 0 列");

        let fks: Vec<(String, String)> = sqlx::query_as(
            "SELECT \"table\", on_delete FROM pragma_foreign_key_list('workspaces')",
        )
        .fetch_all(test_db.pool())
        .await
        .expect("读取外键失败");
        assert!(
            fks.iter()
                .any(|(table, on_delete)| table == "local_users" && on_delete == "SET NULL"),
            "workspaces.created_by_user_id 必须是 ON DELETE SET NULL，实际外键：{fks:?}"
        );
    }

    /// 老库（只跑到 20260917000000，即本迁移之前）已有的工作区行升级后必须
    /// 回填为默认用户，且迁移可重复执行、不改写已回填的数据。
    #[tokio::test]
    async fn 新迁移给已有工作区行回填默认用户且可重复执行() {
        use sqlx::Row;

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("legacy_workspaces.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("连接失败");

        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
            .await
            .expect("读取迁移目录失败");
        let legacy: Vec<_> = migrator
            .iter()
            .filter(|m| m.version <= 20260917000000)
            .cloned()
            .collect();
        let mut legacy_migrator =
            sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await
                .expect("读取迁移目录失败");
        legacy_migrator.migrations = legacy.into();
        legacy_migrator.run(&pool).await.expect("旧迁移应成功");

        let workspace_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces (id, branch, name) VALUES (?1, 'vk/legacy', '旧工作区')")
            .bind(workspace_id)
            .execute(&pool)
            .await
            .unwrap();

        // 升级到最新（含新迁移）。
        migrator.run(&pool).await.expect("新迁移应能应用在旧库上");

        let created_by: Vec<u8> =
            sqlx::query("SELECT created_by_user_id FROM workspaces WHERE id = ?1")
                .bind(workspace_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("created_by_user_id");
        assert_eq!(
            created_by,
            uuid::Uuid::from_u128(2).as_bytes().to_vec(),
            "老库里已有的工作区行必须回填为本机用户"
        );

        // 重复执行：已应用的迁移不会重跑（sqlx 按版本号记账），回填结果不变。
        migrator.run(&pool).await.expect("重复执行迁移应成功");
        let again: Vec<u8> =
            sqlx::query("SELECT created_by_user_id FROM workspaces WHERE id = ?1")
                .bind(workspace_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("created_by_user_id");
        assert_eq!(again, created_by, "重复执行迁移不得改动已回填的数据");
    }

    /// 创建者账号被删除时工作区本身（分支、执行历史）不应被连带删除——
    /// 与 workspaces.issue_id、local_invites.created_by 的既有约定一致。
    #[tokio::test]
    async fn 创建者账号被删除后工作区不被连带删除() {
        use sqlx::Row;

        let test_db = TestDb::new().await;
        let user_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO local_users (id, username, display_name) VALUES (?1, 'alice', 'Alice')",
        )
        .bind(user_id)
        .execute(test_db.pool())
        .await
        .unwrap();

        let workspace_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workspaces (id, branch, name, created_by_user_id) VALUES (?1, 'vk/alice', 'Alice 的工作区', ?2)",
        )
        .bind(workspace_id)
        .bind(user_id)
        .execute(test_db.pool())
        .await
        .unwrap();

        sqlx::query("DELETE FROM local_users WHERE id = ?1")
            .bind(user_id)
            .execute(test_db.pool())
            .await
            .unwrap();

        let row = sqlx::query("SELECT created_by_user_id FROM workspaces WHERE id = ?1")
            .bind(workspace_id)
            .fetch_optional(test_db.pool())
            .await
            .unwrap();
        let row = row.expect("工作区不应随创建者账号被删除而被级联删除");
        let created_by: Option<Vec<u8>> = row.get("created_by_user_id");
        assert_eq!(created_by, None, "创建者被删除后 created_by_user_id 应置空");
    }
}
