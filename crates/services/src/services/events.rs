use std::{str::FromStr, sync::Arc};

use db::{
    DBService,
    models::{
        execution_process::ExecutionProcess, scratch::Scratch, session::Session,
        workspace::Workspace,
    },
};
use sqlx::{Error as SqlxError, Sqlite, SqlitePool, decode::Decode, sqlite::SqliteOperation};
use tokio::sync::RwLock;
use utils::msg_store::MsgStore;
use uuid::Uuid;

#[path = "events/patches.rs"]
pub mod patches;
#[path = "events/streams.rs"]
mod streams;
#[path = "events/types.rs"]
pub mod types;

pub use patches::{
    execution_process_patch, issue_comment_patch, issue_patch, pipeline_run_patch,
    pipeline_stage_run_patch, project_status_patch, scratch_patch, workspace_patch,
};
pub use types::{EventError, EventPatch, EventPatchInner, HookTables, RecordTypes};

/// 提交屏障池只有 1 个连接（sqlx 池按先来先得分配）。与业务主池、钩子反查池都分开，
/// 排队的屏障不会占用业务连接。
const COMMIT_BARRIER_MAX_CONNECTIONS: u32 = 1;

/// 变更钩子专用的屏障池：同一数据库文件、同样的连接参数（含 busy_timeout），不装钩子。
/// 懒连接，建池时不开连接；整个钩子生命周期内复用。
fn commit_barrier_pool(query_pool: &SqlitePool) -> SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(COMMIT_BARRIER_MAX_CONNECTIONS)
        .connect_lazy_with((*query_pool.connect_options()).clone())
}

/// 提交屏障：只作「触发钩子的写者已结束」的同步点。
///
/// commit hook 在提交真正完成之前触发，所以要等：`BEGIN IMMEDIATE` 要拿写锁，会一直等到
/// 那个写事务提交或回滚（最多等 busy_timeout）。拿到后立即回滚释放写锁，不在持锁期间
/// 反查，别的写者因此不必等推送结束。
async fn wait_for_commit(barrier_pool: &SqlitePool) -> Result<(), SqlxError> {
    let barrier = barrier_pool.begin_with("BEGIN IMMEDIATE").await?;
    barrier.rollback().await
}

/// 非删除的行变更。删除在 preupdate 钩子里就地生成 remove 补丁（之后读不到旧行了）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowChange {
    Insert,
    Update,
}

/// 一个连接上、当前事务里攒下的变更。提交时整体取走推送，回滚时清空。
#[derive(Default)]
struct PendingChanges {
    removals: Vec<json_patch::Patch>,
    changes: Vec<(HookTables, i64, RowChange)>,
}

impl PendingChanges {
    fn is_empty(&self) -> bool {
        self.removals.is_empty() && self.changes.is_empty()
    }

    /// 按 (表, rowid) 去重，保持首次出现的顺序；同一行只要插入过就按插入推 add。
    fn deduped_changes(&self) -> Vec<(HookTables, i64, RowChange)> {
        let mut index: std::collections::HashMap<(HookTables, i64), usize> =
            std::collections::HashMap::new();
        let mut out: Vec<(HookTables, i64, RowChange)> = Vec::new();
        for &(table, rowid, change) in &self.changes {
            match index.get(&(table, rowid)) {
                Some(&i) => {
                    if change == RowChange::Insert {
                        out[i].2 = RowChange::Insert;
                    }
                }
                None => {
                    index.insert((table, rowid), out.len());
                    out.push((table, rowid, change));
                }
            }
        }
        out
    }
}

type PendingBuffer = Arc<std::sync::Mutex<PendingChanges>>;

/// 钩子回调里取缓冲。回调里绝不能 panic（commit hook 里 panic 会把提交变成回滚），
/// 所以中毒的锁直接接着用。
fn lock_pending(pending: &PendingBuffer) -> std::sync::MutexGuard<'_, PendingChanges> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 删除前（preupdate）按旧行生成 remove 补丁。
fn removal_patch(preupdate: &sqlx::sqlite::PreupdateHookResult<'_>) -> Option<json_patch::Patch> {
    let id = || {
        preupdate
            .get_old_column_value(0)
            .ok()
            .and_then(|value| <Uuid as Decode<Sqlite>>::decode(value).ok())
    };
    match preupdate.table {
        "workspaces" => id().map(workspace_patch::remove),
        "execution_processes" => id().map(execution_process_patch::remove),
        "scratch" => {
            // 复合键：id（第 0 列）+ scratch_type（第 1 列）
            let scratch_id = id()?;
            let type_val = preupdate.get_old_column_value(1).ok()?;
            let type_str = <String as Decode<Sqlite>>::decode(type_val).ok()?;
            Some(scratch_patch::remove(scratch_id, &type_str))
        }
        "issues" => id().map(issue_patch::remove),
        "project_statuses" => id().map(project_status_patch::remove),
        "issue_comments" => id().map(issue_comment_patch::remove),
        "pipeline_runs" => id().map(pipeline_run_patch::remove),
        "pipeline_stage_runs" => id().map(pipeline_stage_run_patch::remove),
        _ => None,
    }
}

/// 推送消费者共用的上下文。
#[derive(Clone)]
struct HookContext {
    msg_store: Arc<MsgStore>,
    /// 反查用的库（不装钩子）。
    db: DBService,
    barrier_pool: SqlitePool,
    #[cfg(test)]
    probe: Option<Arc<HookProbe>>,
}

impl HookContext {
    fn new(msg_store: Arc<MsgStore>, db: DBService) -> Self {
        Self {
            msg_store,
            barrier_pool: commit_barrier_pool(&db.pool),
            db,
            #[cfg(test)]
            probe: None,
        }
    }
}

/// 测试探针：统计过屏障的次数，并可在推送前人为延迟。
#[cfg(test)]
#[derive(Default)]
struct HookProbe {
    barrier_count: std::sync::atomic::AtomicUsize,
    publish_delay_ms: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
pub struct EventService {
    msg_store: Arc<MsgStore>,
    db: DBService,
    #[allow(dead_code)]
    entry_count: Arc<RwLock<usize>>,
}

impl EventService {
    /// Creates a new EventService that will work with a DBService configured with hooks
    pub fn new(db: DBService, msg_store: Arc<MsgStore>, entry_count: Arc<RwLock<usize>>) -> Self {
        Self {
            msg_store,
            db,
            entry_count,
        }
    }

    async fn push_workspace_update_for_session(
        pool: &SqlitePool,
        msg_store: Arc<MsgStore>,
        session_id: Uuid,
    ) -> Result<(), SqlxError> {
        if let Some(session) = Session::find_by_id(pool, session_id).await?
            && let Some(workspace_with_status) =
                Workspace::find_by_id_with_status(pool, session.workspace_id).await?
        {
            msg_store.push_patch(workspace_patch::replace(&workspace_with_status));
        }
        Ok(())
    }

    /// Creates the hook function that should be used with DBService::new_with_after_connect
    ///
    /// 必须在 tokio 运行时内调用：这里会启动推送消费者任务。`_entry_count` 只为兼容调用方保留
    /// （旧的 `/entries` 兜底格式已删除）。
    pub fn create_hook(
        msg_store: Arc<MsgStore>,
        _entry_count: Arc<RwLock<usize>>,
        db_service: DBService,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        Self::hook_from_context(HookContext::new(msg_store, db_service))
    }

    /// 同 [`Self::create_hook`]，但带测试探针（统计屏障次数、人为延迟推送）。
    #[cfg(test)]
    fn create_hook_with_probe(
        msg_store: Arc<MsgStore>,
        db_service: DBService,
        probe: Arc<HookProbe>,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        let mut context = HookContext::new(msg_store, db_service);
        context.probe = Some(probe);
        Self::hook_from_context(context)
    }

    /// 同 [`Self::create_hook`]，但屏障池指向一个打不开的路径：用来测「屏障失败时降级推送」。
    #[cfg(test)]
    fn create_hook_with_broken_barrier(
        msg_store: Arc<MsgStore>,
        db_service: DBService,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        let mut context = HookContext::new(msg_store, db_service);
        context.barrier_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_lazy_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename("/vk-不存在的目录/commit-barrier.sqlite")
                    .create_if_missing(false),
            );
        Self::hook_from_context(context)
    }

    /// 流程：preupdate / update 钩子只把变更记进本连接的缓冲；commit hook 触发时整体取走，
    /// 送进无界通道；唯一的消费者任务按通道顺序串行推送（等提交屏障 → 先删除 → 按行去重
    /// 反查）。rollback hook 清空缓冲，回滚掉的写入不推任何补丁。
    ///
    /// 顺序：commit hook 在提交过程中、仍持有写锁时触发，各连接的提交被写锁互斥，所以
    /// 通道里的顺序就是提交顺序。消费者在所有发送端（即各连接上的钩子）drop 后自然退出；
    /// 运行时关闭后 send 失败只丢弃，不 panic。
    fn hook_from_context(
        context: HookContext,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<PendingChanges>();
        tokio::spawn(Self::publish_loop(context, receiver));
        move |conn: &mut sqlx::sqlite::SqliteConnection| {
            let sender = sender.clone();
            Box::pin(async move {
                let mut handle = conn.lock_handle().await?;
                // 每个连接一份缓冲：同一连接上的钩子回调是串行的，不同连接互不干扰。
                let pending: PendingBuffer = Arc::default();

                handle.set_preupdate_hook({
                    let pending = pending.clone();
                    move |preupdate: sqlx::sqlite::PreupdateHookResult<'_>| {
                        if preupdate.operation != SqliteOperation::Delete {
                            return;
                        }
                        if let Some(patch) = removal_patch(&preupdate) {
                            lock_pending(&pending).removals.push(patch);
                        }
                    }
                });

                handle.set_update_hook({
                    let pending = pending.clone();
                    move |hook: sqlx::sqlite::UpdateHookResult<'_>| {
                        let change = match hook.operation {
                            SqliteOperation::Insert => RowChange::Insert,
                            SqliteOperation::Update => RowChange::Update,
                            // 删除已在 preupdate 里记下 remove。
                            _ => return,
                        };
                        if let Ok(table) = HookTables::from_str(hook.table) {
                            lock_pending(&pending)
                                .changes
                                .push((table, hook.rowid, change));
                        }
                    }
                });

                handle.set_commit_hook({
                    let pending = pending.clone();
                    move || {
                        let batch = std::mem::take(&mut *lock_pending(&pending));
                        // 接收端只在运行时关闭时消失，此时丢弃即可。
                        if !batch.is_empty() && sender.send(batch).is_err() {
                            tracing::debug!("变更推送消费者已退出，丢弃本次提交的推送");
                        }
                        // true = 不否决提交。
                        true
                    }
                });

                handle.set_rollback_hook(move || {
                    *lock_pending(&pending) = PendingChanges::default();
                });

                Ok(())
            })
        }
    }

    /// 推送消费者：按提交顺序逐批推送。
    async fn publish_loop(
        context: HookContext,
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<PendingChanges>,
    ) {
        while let Some(batch) = receiver.recv().await {
            Self::publish_batch(&context, batch).await;
        }
    }

    /// 推送一次提交攒下的变更：等提交屏障 → 先推删除 → 按行去重反查并推送。
    ///
    /// 屏障只作「写者已提交」的同步点，拿到写锁即释放，反查期间不占写锁。
    /// 一致性：同一行最终一致——每次反查都在各自的提交之后，推送串行且按提交顺序，
    /// 最后一个推送读到的是所有提交之后的数据。不保证同一次推送内的多个读（含
    /// find_by_id_with_status、push_workspace_update_for_session 这类二次查询）是同一快照：
    /// 推送期间别的写者可以提交，读到的只会更新，不会更旧。
    async fn publish_batch(context: &HookContext, batch: PendingChanges) {
        let changes = batch.deduped_changes();
        let committed = wait_for_commit(&context.barrier_pool).await;
        for patch in batch.removals {
            context.msg_store.push_patch(patch);
        }
        if let Err(e) = &committed {
            // 降级而不是丢弃：commit hook 已经触发，提交大概率成功；屏障失败（屏障池连不上、
            // 超过 busy_timeout 等）时照常反查推送，最坏退回改造前的语义——可能读到提交前的
            // 旧值。丢弃则会让客户端一直停在旧状态，直到重连，比读到旧值更糟。
            tracing::warn!(
                "等待写事务提交失败，本次提交的 {} 条变更降级为不过屏障直接反查推送，\
                 可能读到提交前的值: {}",
                changes.len(),
                e
            );
        }

        #[cfg(test)]
        if let Some(probe) = &context.probe {
            if committed.is_ok() {
                probe
                    .barrier_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let delay = probe
                .publish_delay_ms
                .load(std::sync::atomic::Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }

        for (table, rowid, change) in changes {
            Self::push_row_change(context, table, rowid, change).await;
        }
    }

    /// 按 rowid 反查一行并推对应补丁。
    async fn push_row_change(
        context: &HookContext,
        table: HookTables,
        rowid: i64,
        change: RowChange,
    ) {
        let db = &context.db;
        let msg_store = &context.msg_store;
        let record_type: RecordTypes = match table {
            HookTables::Workspaces => match Workspace::find_by_rowid(&db.pool, rowid).await {
                Ok(Some(workspace)) => RecordTypes::Workspace(workspace),
                Ok(None) => RecordTypes::DeletedWorkspace { rowid },
                Err(e) => {
                    tracing::error!("Failed to fetch workspace: {:?}", e);
                    return;
                }
            },
            HookTables::ExecutionProcesses => {
                match ExecutionProcess::find_by_rowid(&db.pool, rowid).await {
                    Ok(Some(process)) => RecordTypes::ExecutionProcess(process),
                    Ok(None) => RecordTypes::DeletedExecutionProcess {
                        rowid,
                        session_id: None,
                        process_id: None,
                    },
                    Err(e) => {
                        tracing::error!("Failed to fetch execution_process: {:?}", e);
                        return;
                    }
                }
            }
            HookTables::Scratch => match Scratch::find_by_rowid(&db.pool, rowid).await {
                Ok(Some(scratch)) => RecordTypes::Scratch(scratch),
                Ok(None) => RecordTypes::DeletedScratch {
                    rowid,
                    scratch_id: None,
                    scratch_type: None,
                },
                Err(e) => {
                    tracing::error!("Failed to fetch scratch: {:?}", e);
                    return;
                }
            },
            HookTables::Issues => {
                match db::models::issue::Issues::find_by_rowid(&db.pool, rowid).await {
                    Ok(Some(issue)) => RecordTypes::Issue(issue),
                    Ok(None) => RecordTypes::DeletedIssue { rowid },
                    Err(e) => {
                        tracing::error!("读取 issue rowid={} 失败: {}", rowid, e);
                        return;
                    }
                }
            }
            HookTables::ProjectStatuses => {
                match db::models::local_project_status::ProjectStatuses::find_by_rowid(
                    &db.pool, rowid,
                )
                .await
                {
                    Ok(Some(status)) => RecordTypes::ProjectStatus(status),
                    Ok(None) => RecordTypes::DeletedProjectStatus { rowid },
                    Err(e) => {
                        tracing::error!("读取 project_status rowid={} 失败: {}", rowid, e);
                        return;
                    }
                }
            }
            HookTables::IssueComments => {
                match db::models::issue_side::IssueComments::find_by_rowid(&db.pool, rowid).await {
                    Ok(Some(comment)) => RecordTypes::IssueComment(comment),
                    Ok(None) => RecordTypes::DeletedIssueComment { rowid },
                    Err(e) => {
                        tracing::error!("读取 issue_comment rowid={} 失败: {}", rowid, e);
                        return;
                    }
                }
            }
            HookTables::PipelineRuns => {
                match db::models::pipeline::PipelineRuns::find_by_rowid(&db.pool, rowid).await {
                    Ok(Some(run)) => RecordTypes::PipelineRun(run),
                    Ok(None) => RecordTypes::DeletedPipelineRun { rowid },
                    Err(e) => {
                        tracing::error!("读取 pipeline_run rowid={} 失败: {}", rowid, e);
                        return;
                    }
                }
            }
            HookTables::PipelineStageRuns => {
                match db::models::pipeline::PipelineStageRuns::find_by_rowid(&db.pool, rowid).await
                {
                    Ok(Some(stage_run)) => RecordTypes::PipelineStageRun(stage_run),
                    Ok(None) => RecordTypes::DeletedPipelineStageRun { rowid },
                    Err(e) => {
                        tracing::error!("读取 pipeline_stage_run rowid={} 失败: {}", rowid, e);
                        return;
                    }
                }
            }
        };

        let is_insert = change == RowChange::Insert;
        match &record_type {
            RecordTypes::Scratch(scratch) => {
                let patch = if is_insert {
                    scratch_patch::add(scratch)
                } else {
                    scratch_patch::replace(scratch)
                };
                msg_store.push_patch(patch);
            }
            RecordTypes::Workspace(workspace) => {
                // Emit workspace patch with status
                if let Ok(Some(workspace_with_status)) =
                    Workspace::find_by_id_with_status(&db.pool, workspace.id).await
                {
                    let patch = if is_insert {
                        workspace_patch::add(&workspace_with_status)
                    } else {
                        workspace_patch::replace(&workspace_with_status)
                    };
                    msg_store.push_patch(patch);
                }
            }
            RecordTypes::ExecutionProcess(process) => {
                let patch = if is_insert {
                    execution_process_patch::add(process)
                } else {
                    execution_process_patch::replace(process)
                };
                msg_store.push_patch(patch);

                if let Err(err) = EventService::push_workspace_update_for_session(
                    &db.pool,
                    msg_store.clone(),
                    process.session_id,
                )
                .await
                {
                    tracing::error!(
                        "Failed to push workspace update after execution process change: {:?}",
                        err
                    );
                }
            }
            RecordTypes::Issue(issue) => {
                let patch = if is_insert {
                    issue_patch::add(issue)
                } else {
                    issue_patch::replace(issue)
                };
                msg_store.push_patch(patch);
            }
            RecordTypes::ProjectStatus(status) => {
                let patch = if is_insert {
                    project_status_patch::add(status)
                } else {
                    project_status_patch::replace(status)
                };
                msg_store.push_patch(patch);
            }
            RecordTypes::IssueComment(comment) => {
                let patch = if is_insert {
                    issue_comment_patch::add(comment)
                } else {
                    issue_comment_patch::replace(comment)
                };
                msg_store.push_patch(patch);
            }
            RecordTypes::PipelineRun(run) => {
                let patch = if is_insert {
                    pipeline_run_patch::add(run)
                } else {
                    pipeline_run_patch::replace(run)
                };
                msg_store.push_patch(patch);
            }
            RecordTypes::PipelineStageRun(stage_run) => {
                let patch = if is_insert {
                    pipeline_stage_run_patch::add(stage_run)
                } else {
                    pipeline_stage_run_patch::replace(stage_run)
                };
                msg_store.push_patch(patch);
            }
            // 反查不到行（提交后又被别的事务删掉）：删除已由 preupdate 推过 remove，这里不推。
            RecordTypes::DeletedWorkspace { .. }
            | RecordTypes::DeletedExecutionProcess { .. }
            | RecordTypes::DeletedScratch { .. }
            | RecordTypes::DeletedIssue { .. }
            | RecordTypes::DeletedProjectStatus { .. }
            | RecordTypes::DeletedIssueComment { .. }
            | RecordTypes::DeletedPipelineRun { .. }
            | RecordTypes::DeletedPipelineStageRun { .. } => {}
        }
    }

    pub fn msg_store(&self) -> &Arc<MsgStore> {
        &self.msg_store
    }
}

/// F2：验证变更钩子（`set_preupdate_hook` / `set_update_hook`）在 WAL 下行为
/// 与 Delete 模式一致——真跑数据库，而不是只看代码推断。
///
/// 关键点：`EventService::create_hook` 需要一个独立的 `DBService` 用来在钩子里按
/// rowid 反查记录（见 :62-76），这个 DBService 必须和挂了钩子的那个池指向同一个
/// 数据库文件，否则查不到刚写入的行。这与生产装配（`crates/local-deployment/src/lib.rs`
/// 里先建一个临时 `DBService::new_with_wal()` 传给 `create_hook`，再用
/// `DBService::new_with_after_connect_and_wal` 建真正的池）是同一个模式，这里只是把两条
/// 路径都指向测试用的临时文件路径。
#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use db::{DBService, test_support::TestDb};
    use json_patch::PatchOperation;
    use sqlx::sqlite::SqliteJournalMode;
    use tokio::sync::RwLock;
    use utils::{log_msg::LogMsg, msg_store::MsgStore};
    use uuid::Uuid;

    use super::EventService;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: DBService,
        msg_store: Arc<MsgStore>,
        /// 过提交屏障的次数与推送延迟。
        probe: Arc<super::HookProbe>,
    }

    /// 建两个指向同一数据库文件的 `DBService`：一个专供钩子内部查询用，一个是
    /// 挂了钩子、真正对外提供 pool 的那个——和生产装配的接线方式一致。
    async fn setup(journal_mode: SqliteJournalMode) -> Fixture {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("events_hook_test.sqlite");

        let hook_query_db = DBService::new_at_path_with_journal(&path, journal_mode)
            .await
            .expect("初始化钩子查询用的 DBService 失败");

        let msg_store = Arc::new(MsgStore::new());
        let probe = Arc::new(super::HookProbe::default());
        let hook =
            EventService::create_hook_with_probe(msg_store.clone(), hook_query_db, probe.clone());

        let db = DBService::new_at_path_with_after_connect(&path, journal_mode, hook)
            .await
            .expect("装钩子初始化 DBService 失败");

        Fixture {
            _dir: dir,
            db,
            msg_store,
            probe,
        }
    }

    async fn insert_project_and_status(pool: &sqlx::SqlitePool) -> (Uuid, Uuid) {
        let project_id = Uuid::new_v4();
        let status_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO local_projects (id, organization_id, name) VALUES (?1, ?2, '钩子测试项目')",
        )
        .bind(project_id)
        .bind(Uuid::from_u128(1))
        .execute(pool)
        .await
        .expect("插入 local_projects 失败");
        sqlx::query(
            "INSERT INTO project_statuses (id, project_id, name) VALUES (?1, ?2, '待开发')",
        )
        .bind(status_id)
        .bind(project_id)
        .execute(pool)
        .await
        .expect("插入 project_statuses 失败");
        (project_id, status_id)
    }

    fn patch_path(op: &PatchOperation) -> &str {
        op.path().as_str()
    }

    async fn insert_issue(pool: &sqlx::SqlitePool, project_id: Uuid, status_id: Uuid) -> Uuid {
        let issue_id = Uuid::new_v4();
        let number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(issue_number), 0) + 1 FROM issues WHERE project_id = ?1",
        )
        .bind(project_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
             VALUES (?1, ?2, ?3, ?4, ?5, '流水线推送测试')",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(number)
        .bind(format!("PL-{number}"))
        .bind(status_id)
        .execute(pool)
        .await
        .expect("插入 issue 失败");
        issue_id
    }

    fn run_params(project_id: Uuid, issue_id: Uuid) -> db::models::pipeline::CreatePipelineRun {
        db::models::pipeline::CreatePipelineRun {
            issue_id,
            project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: db::models::pipeline::PipelineStageKey::Requirement,
        }
    }

    /// `update_hook` 里用 `runtime_handle.spawn` 异步反查再 push patch（:154），
    /// 不能假设 INSERT/UPDATE/DELETE 语句一返回 patch 就已经进了 msg_store，
    /// 所以这里轮询历史直到出现满足条件的 patch 或超时。
    async fn wait_for_patch(
        msg_store: &MsgStore,
        predicate: impl Fn(&PatchOperation) -> bool,
    ) -> PatchOperation {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for msg in msg_store.get_history() {
                    if let LogMsg::JsonPatch(patch) = msg
                        && let Some(op) = patch.0.iter().find(|op| predicate(op))
                    {
                        return op.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等待变更钩子产出 patch 超时")
    }

    /// insert/update/delete 一条 issues 行，断言 `preupdate_hook` / `update_hook`
    /// 都产出了正确的 add/replace/remove patch。同一段逻辑分别用 WAL 和 Delete
    /// 跑一遍（见下面两个 `#[tokio::test]`），用来证明「切 WAL 不改变钩子行为」。
    async fn insert_update_delete_issue_produces_correct_patches(journal_mode: SqliteJournalMode) {
        let fixture = setup(journal_mode).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;

        let issue_id = Uuid::new_v4();
        let issue_path = format!("/issues/{issue_id}");
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
             VALUES (?1, ?2, 1, 'HK-1', ?3, '钩子测试需求')",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(status_id)
        .execute(&pool)
        .await
        .expect("插入 issue 失败");

        let add_op = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
        assert!(
            matches!(add_op, PatchOperation::Add(_)),
            "insert 应该产出 add patch，实际: {add_op:?}"
        );

        sqlx::query("UPDATE issues SET title = '钩子测试需求-改' WHERE id = ?1")
            .bind(issue_id)
            .execute(&pool)
            .await
            .expect("更新 issue 失败");

        let replace_op = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Replace(_))
        })
        .await;
        assert!(
            matches!(replace_op, PatchOperation::Replace(_)),
            "update 应该产出 replace patch，实际: {replace_op:?}"
        );

        sqlx::query("DELETE FROM issues WHERE id = ?1")
            .bind(issue_id)
            .execute(&pool)
            .await
            .expect("删除 issue 失败");

        let remove_op = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Remove(_))
        })
        .await;
        assert!(
            matches!(remove_op, PatchOperation::Remove(_)),
            "delete 应该产出 remove patch（走 preupdate_hook 取第 0 列 id），实际: {remove_op:?}"
        );
    }

    #[tokio::test]
    async fn wal_下变更钩子产出正确的_json_patch() {
        insert_update_delete_issue_produces_correct_patches(SqliteJournalMode::Wal).await;
    }

    #[tokio::test]
    async fn delete_模式下变更钩子同样产出正确的_json_patch() {
        // 参数化对照：证明 WAL 切换没有改变钩子行为，而不是恰好在 WAL 下碰巧能跑通。
        insert_update_delete_issue_produces_correct_patches(SqliteJournalMode::Delete).await;
    }

    /// WAL 的核心卖点：写事务不提交时，另一个连接的读应立即返回，而不是等到
    /// 10s busy_timeout。用 2s 超时兜底，明显小于 BUSY_TIMEOUT，区分度足够。
    #[tokio::test]
    async fn wal_下并发读不被写事务阻塞() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, _status_id) = insert_project_and_status(&pool).await;

        let mut tx = pool.begin().await.expect("开启写事务失败");
        sqlx::query("UPDATE local_projects SET name = '占用写锁，未提交' WHERE id = ?1")
            .bind(project_id)
            .execute(&mut *tx)
            .await
            .expect("事务内写入失败");

        let read_pool = pool.clone();
        let read_result = tokio::time::timeout(Duration::from_secs(2), async move {
            sqlx::query_as::<_, (String,)>("SELECT name FROM local_projects WHERE id = ?1")
                .bind(project_id)
                .fetch_one(&read_pool)
                .await
        })
        .await;

        tx.rollback().await.expect("回滚事务失败");

        let row = read_result
            .expect("WAL 下并发读不应等到 busy_timeout 超时，说明读被写事务阻塞了")
            .expect("查询本身应成功");
        assert_eq!(
            row.0, "钩子测试项目",
            "读到的应是写事务提交前的快照（MVCC），而不是未提交的新值"
        );
    }

    // 注意：这里没有写「Delete 模式下并发读会被未提交的写事务阻塞」的对照测试。
    // 起初写了一个，实测发现断言是错的：SQLite 传统回滚日志（DELETE）模式下，写方在
    // BEGIN + 首次写入后只持有 RESERVED 锁，这个锁并不阻塞其它连接的 SHARED 读锁；
    // 真正会短暂阻塞读的是 COMMIT 那一刻升级到 EXCLUSIVE 锁的瞬间，窗口极短，不适合
    // 用 tokio::time::timeout 稳定复现。也就是说「未提交的写事务不阻塞并发读」在
    // DELETE 模式下同样成立，WAL 的真正优势在于连读方在写方 COMMIT 的那一刻也不会
    // 被阻塞（MVCC 快照隔离）。F2 的验收点只要求验证 WAL 下并发读不被阻塞（上面的
    // `wal_下并发读不被写事务阻塞`），这里不再补一条基于错误前提的对照测试。

    /// 极简探针钩子：任何写操作都往 msg_store 推一条固定的 add patch。
    /// 写成返回 `impl Fn(...) -> Pin<Box<dyn Future<...>>>` 的具名函数（而不是内联闭包变量），
    /// 是因为 `after_connect` 需要满足 `for<'a> Fn(&'a mut SqliteConnection) -> Pin<Box<dyn
    /// Future<...> + Send + 'a>>` 这个高阶生命周期签名，闭包字面量存进 `let` 变量后类型
    /// 推断拿不到这个签名，只有让编译器从函数的返回类型标注反推才行
    /// （`EventService::create_hook` 用的是同一个套路）。
    fn probe_hook(
        msg_store: Arc<MsgStore>,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        move |conn: &mut sqlx::sqlite::SqliteConnection| {
            let msg_store_for_hook = msg_store.clone();
            Box::pin(async move {
                let mut handle = conn.lock_handle().await?;
                handle.set_update_hook(move |_hook: sqlx::sqlite::UpdateHookResult<'_>| {
                    msg_store_for_hook.push_patch(json_patch::Patch(vec![PatchOperation::Add(
                        json_patch::AddOperation {
                            path: "/probe".try_into().unwrap(),
                            value: serde_json::json!(true),
                        },
                    )]));
                });
                Ok(())
            })
        }
    }

    /// 顺带核实 TestDb 的钩子便捷方法（`new_with_hook`）确实能装上钩子并产出 patch，
    /// 覆盖计划要求新增的 `TestDb::new_with_hook` API。这里不需要反查同一份数据，
    /// 用一次简单的 insert 触发即可。
    #[tokio::test]
    async fn test_db_new_with_hook_能装上变更钩子() {
        let msg_store = Arc::new(MsgStore::new());

        let test_db = TestDb::new_with_hook(probe_hook(msg_store.clone())).await;
        sqlx::query(
            "INSERT INTO local_projects (id, organization_id, name) VALUES (?1, ?2, '探针项目')",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::from_u128(1))
        .execute(test_db.pool())
        .await
        .expect("插入失败");

        let op = wait_for_patch(&msg_store, |op| patch_path(op) == "/probe").await;
        assert!(matches!(op, PatchOperation::Add(_)));
    }

    #[tokio::test]
    async fn 流水线运行与阶段的增改删都产出_json_patch() {
        use db::models::pipeline::{
            CreateStageRun, GateKind, PipelineRunStatus, PipelineRuns, PipelineStageKey,
            PipelineStageRuns, PipelineStageStatus,
        };

        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;

        let run = PipelineRuns::create(&pool, &run_params(project_id, issue_id))
            .await
            .unwrap();
        let run_path = format!("/pipeline_runs/{}", run.id);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        PipelineRuns::update_status(&pool, run.id, PipelineRunStatus::Paused, None)
            .await
            .unwrap();
        let replace = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path
                && matches!(op, PatchOperation::Replace(r) if r.value["status"] == "paused")
        })
        .await;
        let PatchOperation::Replace(replace) = replace else {
            unreachable!()
        };
        assert_eq!(replace.value["project_id"], project_id.to_string());

        let stage = PipelineStageRuns::create(
            &pool,
            &CreateStageRun {
                run_id: run.id,
                project_id,
                stage_key: PipelineStageKey::Requirement,
                gate_kind: GateKind::Human,
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await
        .unwrap();
        let stage_path = format!("/pipeline_stage_runs/{}", stage.id);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == stage_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        sqlx::query("DELETE FROM pipeline_stage_runs WHERE id = ?1")
            .bind(stage.id)
            .execute(&pool)
            .await
            .unwrap();
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == stage_path && matches!(op, PatchOperation::Remove(_))
        })
        .await;

        sqlx::query("DELETE FROM pipeline_runs WHERE id = ?1")
            .bind(run.id)
            .execute(&pool)
            .await
            .unwrap();
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Remove(_))
        })
        .await;
    }

    #[tokio::test]
    async fn 需求流首帧带流水线快照且只转发本项目的流水线增量() {
        use db::models::pipeline::PipelineRuns;
        use futures::StreamExt;

        use super::pipeline_run_patch;

        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_a, status_a) = insert_project_and_status(&pool).await;
        let (project_b, status_b) = insert_project_and_status(&pool).await;
        let issue_a = insert_issue(&pool, project_a, status_a).await;
        let issue_b = insert_issue(&pool, project_b, status_b).await;
        let run_a = PipelineRuns::create(&pool, &run_params(project_a, issue_a))
            .await
            .unwrap();
        let run_b = PipelineRuns::create(&pool, &run_params(project_b, issue_b))
            .await
            .unwrap();

        let events = EventService::new(
            fixture.db.clone(),
            fixture.msg_store.clone(),
            Arc::new(RwLock::new(0)),
        );
        let mut stream = events.stream_issues_raw(project_a).await.unwrap();

        let Some(Ok(LogMsg::JsonPatch(first))) = stream.next().await else {
            panic!("首帧应是 JSON Patch");
        };
        let snapshot = serde_json::to_value(&first).unwrap();
        let ops = snapshot.as_array().unwrap();
        let runs_op = ops
            .iter()
            .find(|op| op["path"] == "/pipeline_runs")
            .expect("首帧必须带 /pipeline_runs");
        assert!(runs_op["value"].get(run_a.id.to_string()).is_some());
        assert!(
            runs_op["value"].get(run_b.id.to_string()).is_none(),
            "首帧不得带别的项目的运行"
        );
        assert!(
            ops.iter().any(|op| op["path"] == "/pipeline_stage_runs"),
            "首帧必须带 /pipeline_stage_runs"
        );

        fixture
            .msg_store
            .push_patch(pipeline_run_patch::replace(&run_b));
        fixture
            .msg_store
            .push_patch(pipeline_run_patch::replace(&run_a));

        let path_a = format!("/pipeline_runs/{}", run_a.id);
        let path_b = format!("/pipeline_runs/{}", run_b.id);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let Some(msg) = stream.next().await else {
                    panic!("需求流意外结束");
                };
                let Ok(LogMsg::JsonPatch(patch)) = msg else {
                    continue;
                };
                let path = patch.0[0].path().to_string();
                assert_ne!(path, path_b, "不得转发别的项目的流水线");
                if path == path_a {
                    return;
                }
            }
        })
        .await
        .expect("应收到本项目流水线的增量");
    }

    /// 回归：钩子在语句执行时（提交之前）就触发。反查若不等提交，UPDATE 会推出旧值、
    /// INSERT 会查不到行而丢掉 add。这里让写事务故意晚 200ms 提交，稳定复现。
    async fn delayed_commit_update_pushes_committed_value(journal_mode: SqliteJournalMode) {
        use db::models::pipeline::PipelineRuns;

        let fixture = setup(journal_mode).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        let run = PipelineRuns::create(&pool, &run_params(project_id, issue_id))
            .await
            .unwrap();
        let run_path = format!("/pipeline_runs/{}", run.id);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        let mut tx = pool.begin().await.unwrap();
        sqlx::query("UPDATE pipeline_runs SET status = 'paused' WHERE id = ?1")
            .bind(run.id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.commit().await.unwrap();

        let replace = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Replace(_))
        })
        .await;
        let PatchOperation::Replace(replace) = replace else {
            unreachable!()
        };
        assert_eq!(
            replace.value["status"], "paused",
            "反查必须等写事务提交后再读，不能推出提交前的旧值"
        );
    }

    async fn delayed_commit_insert_pushes_add(journal_mode: SqliteJournalMode) {
        let fixture = setup(journal_mode).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;

        let run_id = Uuid::new_v4();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO pipeline_runs (id, issue_id, project_id, template_key, \
             template_version, template_json, executor_config, status, current_stage_key, \
             created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'standard', 1, '{}', '{}', 'running', 'requirement', ?4, ?4)",
        )
        .bind(run_id)
        .bind(issue_id)
        .bind(project_id)
        .bind(chrono::Utc::now())
        .execute(&mut *tx)
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.commit().await.unwrap();

        let run_path = format!("/pipeline_runs/{run_id}");
        let add = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
        let PatchOperation::Add(add) = add else {
            unreachable!()
        };
        assert_eq!(add.value["status"], "running");
    }

    #[tokio::test]
    async fn wal_下写事务延迟提交时_replace_补丁带的是提交后的值() {
        delayed_commit_update_pushes_committed_value(SqliteJournalMode::Wal).await;
    }

    #[tokio::test]
    async fn delete_模式下写事务延迟提交时_replace_补丁带的是提交后的值() {
        delayed_commit_update_pushes_committed_value(SqliteJournalMode::Delete).await;
    }

    #[tokio::test]
    async fn wal_下写事务延迟提交时_insert_仍产出_add_补丁() {
        delayed_commit_insert_pushes_add(SqliteJournalMode::Wal).await;
    }

    #[tokio::test]
    async fn delete_模式下写事务延迟提交时_insert_仍产出_add_补丁() {
        delayed_commit_insert_pushes_add(SqliteJournalMode::Delete).await;
    }

    /// 等异步推送落地：钩子任务是 spawn 出去的，断言「没有推」之前要给它时间。
    async fn let_hooks_settle() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    fn patches_since(msg_store: &MsgStore, from: usize) -> Vec<PatchOperation> {
        msg_store
            .get_history()
            .into_iter()
            .skip(from)
            .filter_map(|msg| match msg {
                LogMsg::JsonPatch(patch) => Some(patch.0),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[tokio::test]
    async fn 回滚的_insert_不推任何补丁() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let_hooks_settle().await;
        let from = fixture.msg_store.get_history().len();

        let issue_id = Uuid::new_v4();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
             VALUES (?1, ?2, 1, 'RB-1', ?3, '回滚的需求')",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(status_id)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        let_hooks_settle().await;

        let pushed = patches_since(&fixture.msg_store, from);
        assert!(
            pushed.is_empty(),
            "回滚的写入不应推任何补丁，实际: {pushed:?}"
        );
    }

    #[tokio::test]
    async fn 回滚的_delete_不推_remove() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
        let_hooks_settle().await;
        let from = fixture.msg_store.get_history().len();

        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM issues WHERE id = ?1")
            .bind(issue_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let_hooks_settle().await;

        let pushed = patches_since(&fixture.msg_store, from);
        assert!(
            !pushed
                .iter()
                .any(|op| matches!(op, PatchOperation::Remove(_))),
            "回滚的删除不应推 remove，实际: {pushed:?}"
        );
    }

    #[tokio::test]
    async fn 同一事务多次更新同一行只推一次最终值() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
        let_hooks_settle().await;
        let from = fixture.msg_store.get_history().len();

        let mut tx = pool.begin().await.unwrap();
        for title in ["第一次", "第二次", "最终"] {
            sqlx::query("UPDATE issues SET title = ?2 WHERE id = ?1")
                .bind(issue_id)
                .bind(title)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path
                && matches!(op, PatchOperation::Replace(r) if r.value["title"] == "最终")
        })
        .await;
        let_hooks_settle().await;

        let replaces: Vec<_> = patches_since(&fixture.msg_store, from)
            .into_iter()
            .filter(|op| patch_path(op) == issue_path)
            .collect();
        assert_eq!(replaces.len(), 1, "同一行只推一次，实际: {replaces:?}");
    }

    /// 屏障池不可用（连不上、超过 busy_timeout……）时不能丢弃变更：降级为直接反查推送。
    #[tokio::test]
    async fn 提交屏障失败时降级为照常反查推送() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("broken_barrier.sqlite");
        let hook_query_db = DBService::new_at_path_with_journal(&path, SqliteJournalMode::Wal)
            .await
            .expect("初始化钩子查询用的 DBService 失败");
        let msg_store = Arc::new(MsgStore::new());
        let hook = EventService::create_hook_with_broken_barrier(msg_store.clone(), hook_query_db);
        let db = DBService::new_at_path_with_after_connect(&path, SqliteJournalMode::Wal, hook)
            .await
            .expect("装钩子初始化 DBService 失败");

        let (project_id, status_id) = insert_project_and_status(&db.pool).await;
        let issue_id = insert_issue(&db.pool, project_id, status_id).await;
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        sqlx::query("UPDATE issues SET title = '改过的标题' WHERE id = ?1")
            .bind(issue_id)
            .execute(&db.pool)
            .await
            .unwrap();
        let replaced = wait_for_patch(&msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Replace(_))
        })
        .await;
        let PatchOperation::Replace(op) = replaced else {
            panic!("应是 replace 补丁");
        };
        assert_eq!(op.value["title"], "改过的标题");
    }

    #[tokio::test]
    async fn 一个事务多次写只过一次提交屏障() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let_hooks_settle().await;
        let before = fixture
            .probe
            .barrier_count
            .load(std::sync::atomic::Ordering::SeqCst);

        let mut tx = pool.begin().await.unwrap();
        let mut ids = Vec::new();
        for number in 1..=5 {
            let issue_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
                 VALUES (?1, ?2, ?3, ?4, ?5, '批量需求')",
            )
            .bind(issue_id)
            .bind(project_id)
            .bind(number)
            .bind(format!("BT-{number}"))
            .bind(status_id)
            .execute(&mut *tx)
            .await
            .unwrap();
            ids.push(issue_id);
        }
        tx.commit().await.unwrap();
        for issue_id in &ids {
            let path = format!("/issues/{issue_id}");
            wait_for_patch(&fixture.msg_store, |op| {
                patch_path(op) == path && matches!(op, PatchOperation::Add(_))
            })
            .await;
        }
        let_hooks_settle().await;

        let after = fixture
            .probe
            .barrier_count
            .load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after - before, 1, "一个事务只应过一次提交屏障");
    }

    /// 屏障只作「写者已提交」的同步点：推送（反查）进行中，别的写事务应能立即拿到写锁。
    #[tokio::test]
    async fn 推送进行中别的写事务不必等推送结束() {
        use std::sync::atomic::Ordering;

        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let_hooks_settle().await;

        const DELAY_MS: u64 = 2000;
        fixture
            .probe
            .publish_delay_ms
            .store(DELAY_MS, Ordering::SeqCst);
        let before = fixture.probe.barrier_count.load(Ordering::SeqCst);
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        // 等推送任务过了屏障、进入人为延迟。
        tokio::time::timeout(Duration::from_secs(5), async {
            while fixture.probe.barrier_count.load(Ordering::SeqCst) == before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("推送任务应过屏障");

        let started = std::time::Instant::now();
        sqlx::query("UPDATE local_projects SET name = '并发写' WHERE id = ?1")
            .bind(project_id)
            .execute(&pool)
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(DELAY_MS / 4),
            "推送期间写锁不应被占着，并发写耗时 {elapsed:?}"
        );

        fixture.probe.publish_delay_ms.store(0, Ordering::SeqCst);
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
    }

    #[tokio::test]
    async fn 跨事务删除后用同一_id_重新插入_最终存在() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;
        let_hooks_settle().await;
        let from = fixture.msg_store.get_history().len();

        sqlx::query("DELETE FROM issues WHERE id = ?1")
            .bind(issue_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
             VALUES (?1, ?2, 99, 'RE-99', ?3, '重新插入')",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(status_id)
        .execute(&pool)
        .await
        .unwrap();
        let_hooks_settle().await;

        let ops: Vec<_> = patches_since(&fixture.msg_store, from)
            .into_iter()
            .filter(|op| patch_path(op) == issue_path)
            .collect();
        assert!(
            matches!(ops.first(), Some(PatchOperation::Remove(_))),
            "先 remove，实际: {ops:?}"
        );
        assert!(
            matches!(ops.last(), Some(PatchOperation::Add(_))),
            "最后应是 add，行最终存在，实际: {ops:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn 多事务交替更新同一行_最后一个补丁是最终值() {
        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;
        let issue_path = format!("/issues/{issue_id}");
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        const ROUNDS: usize = 40;
        for round in 0..ROUNDS {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("UPDATE issues SET title = ?2 WHERE id = ?1")
                .bind(issue_id)
                .bind(format!("第{round}轮-中间"))
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query("UPDATE issues SET title = ?2 WHERE id = ?1")
                .bind(issue_id)
                .bind(format!("第{round}轮"))
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        let final_title = format!("第{}轮", ROUNDS - 1);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == issue_path
                && matches!(op, PatchOperation::Replace(r) if r.value["title"] == final_title.as_str())
        })
        .await;
        let_hooks_settle().await;

        let last = patches_since(&fixture.msg_store, 0)
            .into_iter()
            .rfind(|op| patch_path(op) == issue_path)
            .unwrap();
        let PatchOperation::Replace(last) = last else {
            panic!("最后一个补丁应是 replace，实际: {last:?}")
        };
        assert_eq!(last.value["title"], final_title.as_str());
    }
}
