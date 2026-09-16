//! 测试用的一次性 SQLite 数据库。
//! 每个 TestDb 拥有自己的临时目录，Drop 时自动删除。

use sqlx::SqlitePool;
use tempfile::TempDir;

use crate::DBService;

pub struct TestDb {
    pub db: DBService,
    _dir: TempDir,
}

impl TestDb {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path(&dir.path().join("test.sqlite"))
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
}
