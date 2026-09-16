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
}
