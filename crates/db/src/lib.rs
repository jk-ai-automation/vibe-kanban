use std::{future::Future, path::Path, pin::Pin, str::FromStr, sync::Arc};

use sqlx::{
    ConnectOptions, Error, Pool, Sqlite,
    migrate::MigrateError,
    sqlite::{
        SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions,
        SqliteSynchronous,
    },
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

/// 连接池上限。团队版多人并发读写时，读连接不应被写连接饿死；
/// 读写分离暂不做（见设计文档 §9），先给一个显式上限，避免依赖 sqlx 默认值（10）。
pub const MAX_CONNECTIONS: u32 = 16;

/// 环境变量名：设为 `0`/`false`（大小写不敏感）时回退到 `journal_mode=DELETE`。
/// 默认（未设置或其它任意值）走 WAL。
///
/// **已知缺口**：Windows 下 WAL 的文件锁行为本机无法验证（本仓库开发环境是 macOS），
/// 保留这个回退开关就是为了在 Windows 上出现异常时可以不改代码直接退回原行为。
pub const VK_SQLITE_WAL_ENV: &str = "VK_SQLITE_WAL";

/// 纯函数：把环境变量的原始值解析成 journal mode。抽出来是为了不依赖进程环境就能测试
/// `None`/`"0"`/`"false"`/`"1"`/`"garbage"` 等各种取值（`journal_mode_from_env` 只是它的薄封装）。
fn journal_mode_from(value: Option<&str>) -> SqliteJournalMode {
    match value {
        Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => SqliteJournalMode::Delete,
        _ => SqliteJournalMode::Wal,
    }
}

fn journal_mode_from_env() -> SqliteJournalMode {
    journal_mode_from(std::env::var(VK_SQLITE_WAL_ENV).ok().as_deref())
}

/// 把合并后的 `sqlite_wal` 开关翻译成 journal mode。
///
/// 「合并后」指 `server.json` 的 `sqlite_wal` 字段与 `VK_SQLITE_WAL` 环境变量已经
/// 在 `server_settings` 里合并过一次（环境变量优先）。生产路径必须走这里，不能再自己
/// 读一次环境变量——否则 `server.json` 里写的 `"sqlite_wal": false` 会被静默忽略，
/// 而配置文件里躺着一个不生效的字段是最难排查的那类问题。
///
/// `journal_mode_from_env` 只留给不读 `server.json` 的离线工具与测试。
fn journal_mode_for(sqlite_wal: bool) -> SqliteJournalMode {
    if sqlite_wal {
        SqliteJournalMode::Wal
    } else {
        SqliteJournalMode::Delete
    }
}

/// 三条生产/测试路径共用的连接参数拼装：busy_timeout、journal_mode、synchronous
/// 都在这里一起设，免得某一条路径漏掉。
fn configure(
    options: SqliteConnectOptions,
    journal_mode: SqliteJournalMode,
) -> SqliteConnectOptions {
    options
        .create_if_missing(true)
        .journal_mode(journal_mode)
        // WAL 下用 NORMAL 而不是默认 FULL：WAL 模式本身已经保证崩溃后数据库不损坏，
        // NORMAL 只在 checkpoint 时 fsync，显著减少写延迟；断电时可能丢最后几条已提交
        // 事务（不会损坏库），这是 WAL 官方推荐的搭配。Delete 模式下这个设置同样适用。
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(BUSY_TIMEOUT)
}

/// 打开主库的连接参数（走 `asset_dir()` 下的固定路径）。
fn main_db_options(journal_mode: SqliteJournalMode) -> Result<SqliteConnectOptions, Error> {
    let database_url = format!(
        "sqlite://{}",
        asset_dir().join("db.v2.sqlite").to_string_lossy()
    );
    Ok(configure(
        SqliteConnectOptions::from_str(&database_url)?,
        journal_mode,
    ))
}

/// 打开指定路径库的连接参数。供测试与离线工具使用，不读取 asset_dir()。
/// 直接给 filename，不拼 URL：路径里的空格、`#`、`?` 等字符不会被当成 URL 语法。
fn path_db_options(path: &Path, journal_mode: SqliteJournalMode) -> SqliteConnectOptions {
    configure(SqliteConnectOptions::new().filename(path), journal_mode)
}

type AfterConnectHook = Arc<
    dyn for<'a> Fn(
            &'a mut SqliteConnection,
        ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct DBService {
    pub pool: Pool<Sqlite>,
}

impl DBService {
    /// 打开主库。journal mode 取自合并后的 `sqlite_wal` 开关，见 [`journal_mode_for`]。
    ///
    /// 刻意**没有**一个不带参数的版本：主库只有这一条生产路径，多一个读环境变量的
    /// 重载就意味着某天有人调了它，然后 `server.json` 里的 `sqlite_wal` 又悄悄失效。
    pub async fn new_with_wal(sqlite_wal: bool) -> Result<DBService, Error> {
        let pool = Self::create_pool(main_db_options(journal_mode_for(sqlite_wal))?, None).await?;
        Ok(DBService { pool })
    }

    /// 在指定路径创建并迁移一个独立数据库，journal mode 走 `VK_SQLITE_WAL` 环境变量。
    /// 供测试与离线工具使用，不读取 asset_dir()，因此不会污染开发数据。
    pub async fn new_at_path(path: &Path) -> Result<DBService, Error> {
        Self::new_at_path_with_journal(path, journal_mode_from_env()).await
    }

    /// 同 [`Self::new_at_path`]，但显式指定 journal mode，不受环境变量影响。
    /// 用于测试「切换 WAL / 回退 Delete 两条路径都要覆盖」。
    pub async fn new_at_path_with_journal(
        path: &Path,
        journal_mode: SqliteJournalMode,
    ) -> Result<DBService, Error> {
        let pool = Self::create_pool(path_db_options(path, journal_mode), None).await?;
        Ok(DBService { pool })
    }

    pub async fn new_migration_pool(sqlite_wal: bool) -> Result<Pool<Sqlite>, Error> {
        let options = main_db_options(journal_mode_for(sqlite_wal))?.disable_statement_logging();
        SqlitePoolOptions::new()
            .max_connections(64)
            .connect_with(options)
            .await
    }

    /// 带变更钩子地打开主库。journal mode 同 [`Self::new_with_wal`]。
    pub async fn new_with_after_connect_and_wal<F>(
        sqlite_wal: bool,
        after_connect: F,
    ) -> Result<DBService, Error>
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    {
        let pool = Self::create_pool(
            main_db_options(journal_mode_for(sqlite_wal))?,
            Some(Arc::new(after_connect)),
        )
        .await?;
        Ok(DBService { pool })
    }

    /// 同 [`Self::new_with_after_connect`]，但连接指定路径的库并显式指定 journal mode。
    /// 供测试验证「变更钩子在 WAL / Delete 两种模式下行为一致」，无法用 `new_with_after_connect`
    /// 是因为它固定读 `asset_dir()`。
    pub async fn new_at_path_with_after_connect<F>(
        path: &Path,
        journal_mode: SqliteJournalMode,
        after_connect: F,
    ) -> Result<DBService, Error>
    where
        F: for<'a> Fn(
                &'a mut SqliteConnection,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    {
        let pool = Self::create_pool(
            path_db_options(path, journal_mode),
            Some(Arc::new(after_connect)),
        )
        .await?;
        Ok(DBService { pool })
    }

    async fn create_pool(
        options: SqliteConnectOptions,
        after_connect: Option<AfterConnectHook>,
    ) -> Result<Pool<Sqlite>, Error> {
        let pool = if let Some(hook) = after_connect {
            SqlitePoolOptions::new()
                .max_connections(MAX_CONNECTIONS)
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
            SqlitePoolOptions::new()
                .max_connections(MAX_CONNECTIONS)
                .connect_with(options)
                .await?
        };

        run_migrations(&pool).await?;
        Ok(pool)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqliteJournalMode;

    use super::{
        BUSY_TIMEOUT, DBService, MAX_CONNECTIONS, journal_mode_for, journal_mode_from,
        main_db_options,
    };
    use crate::test_support::TestDb;

    async fn journal_mode_of(pool: &sqlx::SqlitePool) -> String {
        let pragma: (String,) = sqlx::query_as("PRAGMA journal_mode")
            .fetch_one(pool)
            .await
            .expect("读取 journal_mode 失败");
        pragma.0
    }

    #[test]
    fn 合并后的_sqlite_wal_开关决定_journal_mode() {
        // `server.json` 的 `sqlite_wal` 字段一度只被解析、没被建库路径消费，
        // 表现是配置文件里写了 `"sqlite_wal": false` 却仍然跑在 WAL 上。
        assert_eq!(journal_mode_for(true), SqliteJournalMode::Wal);
        assert_eq!(
            journal_mode_for(false),
            SqliteJournalMode::Delete,
            "server.json 里关掉 WAL 必须真的生效，不能只认环境变量"
        );
    }

    #[tokio::test]
    async fn sqlite_wal_关掉后建出来的库真的是_delete() {
        // 把上面那条纯函数断言接到真实 PRAGMA 上：光有映射不够，还要确认
        // 这个 journal mode 确实被带进了连接参数。
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path_with_journal(
            &dir.path().join("test.sqlite"),
            journal_mode_for(false),
        )
        .await
        .expect("按 server.json 的 sqlite_wal=false 初始化应成功");
        assert_eq!(journal_mode_of(&db.pool).await, "delete");
    }

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
        main_db_options(SqliteJournalMode::Wal).expect("主库连接参数必须可构造");
    }

    #[tokio::test]
    async fn 默认走_wal() {
        // TestDb::new() 不显式传参数，走 journal_mode_from_env()：
        // 测试进程没设 VK_SQLITE_WAL，因此默认应该是 WAL。
        let test_db = TestDb::new().await;
        assert_eq!(
            journal_mode_of(test_db.pool()).await,
            "wal",
            "默认应切到 WAL，团队版并发写场景下读不应被写阻塞"
        );
    }

    #[tokio::test]
    async fn 可回退到_delete() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path_with_journal(
            &dir.path().join("test.sqlite"),
            SqliteJournalMode::Delete,
        )
        .await
        .expect("按 Delete 模式初始化应成功");
        assert_eq!(
            journal_mode_of(&db.pool).await,
            "delete",
            "VK_SQLITE_WAL=0 时必须能退回原来的 Delete 模式"
        );
    }

    #[tokio::test]
    async fn wal_下_synchronous_是_normal() {
        let test_db = TestDb::new().await;
        let pragma: (i64,) = sqlx::query_as("PRAGMA synchronous")
            .fetch_one(test_db.pool())
            .await
            .expect("读取 synchronous 失败");
        assert_eq!(
            pragma.0, 1,
            "WAL 下 synchronous 应为 NORMAL（SQLite 内部编号 1），而不是默认 FULL(2)"
        );
    }

    #[tokio::test]
    async fn 连接池上限是_16() {
        let test_db = TestDb::new().await;
        assert_eq!(
            test_db.pool().options().get_max_connections(),
            MAX_CONNECTIONS,
            "连接池必须显式设上限，不能依赖 sqlx 默认值"
        );
    }

    #[test]
    fn journal_mode_from_未设置时走_wal() {
        assert_eq!(journal_mode_from(None), SqliteJournalMode::Wal);
    }

    #[test]
    fn journal_mode_from_0_时走_delete() {
        assert_eq!(journal_mode_from(Some("0")), SqliteJournalMode::Delete);
    }

    #[test]
    fn journal_mode_from_false_时走_delete() {
        assert_eq!(journal_mode_from(Some("false")), SqliteJournalMode::Delete);
        assert_eq!(journal_mode_from(Some("FALSE")), SqliteJournalMode::Delete);
        assert_eq!(journal_mode_from(Some("False")), SqliteJournalMode::Delete);
    }

    #[test]
    fn journal_mode_from_1_时走_wal() {
        assert_eq!(journal_mode_from(Some("1")), SqliteJournalMode::Wal);
    }

    #[test]
    fn journal_mode_from_无法识别的值时走_wal() {
        // 未知取值不应静默回退到 Delete：宁可让人明显看到"没生效"，也不要在解析错误时
        // 悄悄丢失并发优势。
        assert_eq!(journal_mode_from(Some("garbage")), SqliteJournalMode::Wal);
    }

    /// 把「WAL 下必须用 sqlite3 .backup / VACUUM INTO 备份，不能直接 cp」这条约束钉在测试里。
    ///
    /// 做法：写一行不 checkpoint 的数据，只复制主库文件（不复制 `-wal` 边车文件），
    /// 打开副本后断言这行数据不在——证明「裸 cp」会丢数据。
    #[tokio::test]
    async fn wal_下直接_cp_主库文件会丢数据() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let main_path = dir.path().join("main.sqlite");
        let db = DBService::new_at_path_with_journal(&main_path, SqliteJournalMode::Wal)
            .await
            .expect("初始化主库失败");

        let project_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO local_projects (id, organization_id, name) VALUES (?1, ?2, 'WAL 备份测试')",
        )
        .bind(project_id)
        .bind(uuid::Uuid::from_u128(1))
        .execute(&db.pool)
        .await
        .expect("写入测试数据失败");

        // 确认这条数据确实还留在 -wal 边车文件里，没有被自动 checkpoint 掉，
        // 否则下面的"裸 cp 丢数据"就无法稳定复现。
        let wal_path = {
            let mut p = main_path.clone().into_os_string();
            p.push("-wal");
            std::path::PathBuf::from(p)
        };
        assert!(
            wal_path.exists() && std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0) > 0,
            "预期未 checkpoint 的写入应停留在 -wal 文件里，若此断言失败说明 sqlx 提前自动 \
             checkpoint 了，需要把本测试换成直接断言 -wal 文件存在（见 F3 步骤说明）"
        );

        // 只复制主库文件，模拟运维「裸 cp 数据库文件」的错误备份方式。
        let copy_path = dir.path().join("copy.sqlite");
        std::fs::copy(&main_path, &copy_path).expect("复制主库文件失败");

        // 用独立连接打开副本（Delete 模式即可，只是为了不再产生新的 -wal），确认数据缺失。
        let copy_db = DBService::new_at_path_with_journal(&copy_path, SqliteJournalMode::Delete)
            .await
            .expect("打开副本失败");
        let found: Option<(uuid::Uuid,)> =
            sqlx::query_as("SELECT id FROM local_projects WHERE id = ?1")
                .bind(project_id)
                .fetch_optional(&copy_db.pool)
                .await
                .expect("查询副本失败");
        assert!(
            found.is_none(),
            "裸 cp 的副本不应包含未 checkpoint 的数据；若这里失败说明裸 cp 备份不再丢数据，\
             需要重新评估 F3 的备份约束是否还成立"
        );

        drop(db);
    }
}
