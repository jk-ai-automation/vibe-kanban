use std::{str::FromStr, sync::Arc};

use sqlx::{
    ConnectOptions, Error, Pool, Sqlite, SqlitePool,
    migrate::MigrateError,
    sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions},
};
use utils::assets::asset_dir;

pub mod models;
pub mod test_support;

async fn run_migrations(pool: &Pool<Sqlite>) -> Result<(), Error> {
    use std::collections::HashSet;

    let migrator = sqlx::migrate!("./migrations");
    let mut processed_versions: HashSet<i64> = HashSet::new();

    loop {
        match migrator.run(pool).await {
            Ok(()) => return Ok(()),
            Err(MigrateError::VersionMismatch(version)) => {
                if cfg!(debug_assertions) {
                    // return the error in debug mode to catch migration issues early
                    return Err(sqlx::Error::Migrate(Box::new(
                        MigrateError::VersionMismatch(version),
                    )));
                }

                if !cfg!(windows) {
                    // On non-Windows platforms, we do not attempt to auto-fix checksum mismatches
                    return Err(sqlx::Error::Migrate(Box::new(
                        MigrateError::VersionMismatch(version),
                    )));
                }

                // Guard against infinite loop
                if !processed_versions.insert(version) {
                    return Err(sqlx::Error::Migrate(Box::new(
                        MigrateError::VersionMismatch(version),
                    )));
                }

                // On Windows, there can be checksum mismatches due to line ending differences
                // or other platform-specific issues. Update the stored checksum and retry.
                tracing::warn!(
                    "Migration version {} has checksum mismatch, updating stored checksum (likely platform-specific difference)",
                    version
                );

                // Find the migration with the mismatched version and get its current checksum
                if let Some(migration) = migrator.iter().find(|m| m.version == version) {
                    // Update the checksum in _sqlx_migrations to match the current file
                    sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                        .bind(&*migration.checksum)
                        .bind(version)
                        .execute(pool)
                        .await?;
                } else {
                    // Migration not found in current set, can't fix
                    return Err(sqlx::Error::Migrate(Box::new(
                        MigrateError::VersionMismatch(version),
                    )));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// 写锁等待时长。SQLite 拿不到写锁时先在驱动层等这么久再返回 BUSY，
/// 配合 [`models::db_retry::retry_on_busy`] 的应用层重试一起用。
pub const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 打开主库的连接参数。三条生产路径（普通、迁移、带 after_connect）共用，
/// 免得 busy_timeout 这类设置在某一条上漏掉。
fn main_db_options() -> Result<SqliteConnectOptions, Error> {
    let database_url = format!(
        "sqlite://{}",
        asset_dir().join("db.v2.sqlite").to_string_lossy()
    );
    Ok(SqliteConnectOptions::from_str(&database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete)
        .busy_timeout(BUSY_TIMEOUT))
}

#[derive(Clone)]
pub struct DBService {
    pub pool: Pool<Sqlite>,
}

impl DBService {
    pub async fn new() -> Result<DBService, Error> {
        let pool = SqlitePool::connect_with(main_db_options()?).await?;
        run_migrations(&pool).await?;
        Ok(DBService { pool })
    }

    /// 在指定路径创建并迁移一个独立数据库。
    /// 供测试与离线工具使用，不读取 asset_dir()，因此不会污染开发数据。
    pub async fn new_at_path(path: &std::path::Path) -> Result<DBService, Error> {
        // 直接给 filename，不拼 URL：路径里的空格、`#`、`?` 等字符不会被当成 URL 语法。
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePool::connect_with(options).await?;
        run_migrations(&pool).await?;
        Ok(DBService { pool })
    }

    pub async fn new_migration_pool() -> Result<Pool<Sqlite>, Error> {
        let options = main_db_options()?.disable_statement_logging();
        SqlitePoolOptions::new()
            .max_connections(64)
            .connect_with(options)
            .await
    }

    pub async fn new_with_after_connect<F>(after_connect: F) -> Result<DBService, Error>
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    {
        let pool = Self::create_pool(Some(Arc::new(after_connect))).await?;
        Ok(DBService { pool })
    }

    async fn create_pool<F>(after_connect: Option<Arc<F>>) -> Result<Pool<Sqlite>, Error>
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    {
        let options = main_db_options()?;

        let pool = if let Some(hook) = after_connect {
            SqlitePoolOptions::new()
                .after_connect(move |conn, _meta| {
                    let hook = hook.clone();
                    Box::pin(async move {
                        hook(conn).await?;
                        Ok(())
                    })
                })
                .connect_with(options)
                .await?
        } else {
            SqlitePool::connect_with(options).await?
        };

        run_migrations(&pool).await?;
        Ok(pool)
    }
}

#[cfg(test)]
mod tests {
    use super::{BUSY_TIMEOUT, main_db_options};
    use crate::test_support::TestDb;

    #[tokio::test]
    async fn 连接自带写锁等待时间() {
        let test_db = TestDb::new().await;
        let pragma: (i64,) = sqlx::query_as("PRAGMA busy_timeout")
            .fetch_one(test_db.pool())
            .await
            .expect("读取 busy_timeout 失败");
        assert_eq!(
            pragma.0,
            BUSY_TIMEOUT.as_millis() as i64,
            "连接必须带上写锁等待时间，否则并发写会直接 BUSY"
        );
    }

    #[test]
    fn 主库连接参数构造成功() {
        // 生产路径共用 main_db_options，这里只确认它能构造出来
        // （busy_timeout 的实际效果由上面的 PRAGMA 测试覆盖）。
        main_db_options().expect("主库连接参数必须可构造");
    }
}
