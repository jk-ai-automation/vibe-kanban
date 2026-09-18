use std::{str::FromStr, sync::Arc};

use db::{
    DBService,
    models::{
        execution_process::ExecutionProcess, scratch::Scratch, session::Session,
        workspace::Workspace,
    },
};
use serde_json::json;
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

/// 提交屏障池的连接数。每个变更事件只占一个连接很短的时间，2 个足够，
/// 而且与业务主池、钩子反查池都分开，排队的屏障不会占满业务连接。
const COMMIT_BARRIER_MAX_CONNECTIONS: u32 = 2;

/// 变更钩子专用的屏障池：同一数据库文件、同样的连接参数（含 busy_timeout），不装钩子。
/// 懒连接，建池时不开连接；整个钩子生命周期内复用。
fn commit_barrier_pool(query_pool: &SqlitePool) -> SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(COMMIT_BARRIER_MAX_CONNECTIONS)
        .connect_lazy_with((*query_pool.connect_options()).clone())
}

/// 在屏障连接上开 `BEGIN IMMEDIATE`。它要拿写锁，因此会一直等到触发钩子的那个写事务
/// 提交或回滚（最多等 busy_timeout），返回后数据库里只剩已提交数据；持有期间别的写者
/// 提交不了。只读用，结束时回滚。
async fn acquire_commit_barrier(
    barrier_pool: &SqlitePool,
) -> Result<sqlx::Transaction<'static, Sqlite>, SqlxError> {
    barrier_pool.begin_with("BEGIN IMMEDIATE").await
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
    pub fn create_hook(
        msg_store: Arc<MsgStore>,
        entry_count: Arc<RwLock<usize>>,
        db_service: DBService,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        let barrier_pool = commit_barrier_pool(&db_service.pool);
        move |conn: &mut sqlx::sqlite::SqliteConnection| {
            let msg_store_for_hook = msg_store.clone();
            let entry_count_for_hook = entry_count.clone();
            let db_for_hook = db_service.clone();
            let barrier_pool_for_hook = barrier_pool.clone();
            Box::pin(async move {
                let mut handle = conn.lock_handle().await?;
                let runtime_handle = tokio::runtime::Handle::current();
                handle.set_preupdate_hook({
                    let msg_store_for_preupdate = msg_store_for_hook.clone();
                    move |preupdate: sqlx::sqlite::PreupdateHookResult<'_>| {
                        if preupdate.operation != SqliteOperation::Delete {
                            return;
                        }

                        match preupdate.table {
                            "workspaces" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(workspace_id) =
                                        <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    let patch = workspace_patch::remove(workspace_id);
                                    msg_store_for_preupdate.push_patch(patch);
                                }
                            }
                            "execution_processes" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(process_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    let patch = execution_process_patch::remove(process_id);
                                    msg_store_for_preupdate.push_patch(patch);
                                }
                            }
                            "scratch" => {
                                // Composite key: need both id (column 0) and scratch_type (column 1)
                                if let Ok(id_val) = preupdate.get_old_column_value(0)
                                    && let Ok(scratch_id) = <Uuid as Decode<Sqlite>>::decode(id_val)
                                    && let Ok(type_val) = preupdate.get_old_column_value(1)
                                    && let Ok(type_str) =
                                        <String as Decode<Sqlite>>::decode(type_val)
                                {
                                    let patch = scratch_patch::remove(scratch_id, &type_str);
                                    msg_store_for_preupdate.push_patch(patch);
                                }
                            }
                            "issues" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(issue_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(issue_patch::remove(issue_id));
                                }
                            }
                            "project_statuses" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(status_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(project_status_patch::remove(status_id));
                                }
                            }
                            "issue_comments" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(comment_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(issue_comment_patch::remove(comment_id));
                                }
                            }
                            "pipeline_runs" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(run_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(pipeline_run_patch::remove(run_id));
                                }
                            }
                            "pipeline_stage_runs" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(stage_run_id) =
                                        <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(pipeline_stage_run_patch::remove(stage_run_id));
                                }
                            }
                            _ => {}
                        }
                    }
                });

                handle.set_update_hook(move |hook: sqlx::sqlite::UpdateHookResult<'_>| {
                    let runtime_handle = runtime_handle.clone();
                    let entry_count_for_hook = entry_count_for_hook.clone();
                    let msg_store_for_hook = msg_store_for_hook.clone();
                    let db = db_for_hook.clone();
                    let barrier_pool = barrier_pool_for_hook.clone();

                    if let Ok(table) = HookTables::from_str(hook.table) {
                        let rowid = hook.rowid;
                        runtime_handle.spawn(async move {
                            // 删除由 preupdate 钩子推 remove，这里什么都不用做，也不必抢写锁。
                            if matches!(hook.operation, SqliteOperation::Delete) {
                                return;
                            }
                            // 提交屏障：update_hook 在语句执行时（提交之前）就触发，必须等
                            // 触发它的写事务结束后再反查，否则会读到旧值或读不到新行。
                            let barrier = match acquire_commit_barrier(&barrier_pool).await {
                                Ok(barrier) => barrier,
                                Err(e) => {
                                    tracing::error!(
                                        "等待写事务提交失败，丢弃 {} rowid={} 的变更推送: {}",
                                        table,
                                        rowid,
                                        e
                                    );
                                    return;
                                }
                            };
                            // 持有屏障期间反查并推送：别的写者提交不了，下面在 db.pool 上的
                            // 所有读（含 find_by_id_with_status、push_workspace_update_for_session
                            // 这类二次查询）看到的都是同一份已提交快照，推送顺序也与提交顺序一致。
                            async {
                                let record_type: RecordTypes = match (table, hook.operation.clone()) {
                                    (HookTables::Workspaces, SqliteOperation::Delete)
                                    | (HookTables::ExecutionProcesses, SqliteOperation::Delete)
                                    | (HookTables::Scratch, SqliteOperation::Delete)
                                    | (HookTables::Issues, SqliteOperation::Delete)
                                    | (HookTables::ProjectStatuses, SqliteOperation::Delete)
                                    | (HookTables::IssueComments, SqliteOperation::Delete)
                                    | (HookTables::PipelineRuns, SqliteOperation::Delete)
                                    | (HookTables::PipelineStageRuns, SqliteOperation::Delete) => {
                                        return;
                                    }
                                    (HookTables::Workspaces, _) => {
                                        match Workspace::find_by_rowid(&db.pool, rowid).await {
                                            Ok(Some(workspace)) => RecordTypes::Workspace(workspace),
                                            Ok(None) => RecordTypes::DeletedWorkspace {
                                                rowid,
                                            },
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to fetch workspace: {:?}",
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::ExecutionProcesses, _) => {
                                        match ExecutionProcess::find_by_rowid(&db.pool, rowid).await {
                                            Ok(Some(process)) => RecordTypes::ExecutionProcess(process),
                                            Ok(None) => RecordTypes::DeletedExecutionProcess {
                                                rowid,
                                                session_id: None,
                                                process_id: None,
                                            },
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to fetch execution_process: {:?}",
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::Scratch, _) => {
                                        match Scratch::find_by_rowid(&db.pool, rowid).await {
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
                                        }
                                    }
                                    (HookTables::Issues, _) => {
                                        match db::models::issue::Issues::find_by_rowid(&db.pool, rowid)
                                            .await
                                        {
                                            Ok(Some(issue)) => RecordTypes::Issue(issue),
                                            Ok(None) => RecordTypes::DeletedIssue { rowid },
                                            Err(e) => {
                                                tracing::error!("读取 issue rowid={} 失败: {}", rowid, e);
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::ProjectStatuses, _) => {
                                        match db::models::local_project_status::ProjectStatuses::find_by_rowid(
                                            &db.pool, rowid,
                                        )
                                        .await
                                        {
                                            Ok(Some(status)) => RecordTypes::ProjectStatus(status),
                                            Ok(None) => RecordTypes::DeletedProjectStatus { rowid },
                                            Err(e) => {
                                                tracing::error!(
                                                    "读取 project_status rowid={} 失败: {}",
                                                    rowid,
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::IssueComments, _) => {
                                        match db::models::issue_side::IssueComments::find_by_rowid(
                                            &db.pool, rowid,
                                        )
                                        .await
                                        {
                                            Ok(Some(comment)) => RecordTypes::IssueComment(comment),
                                            Ok(None) => RecordTypes::DeletedIssueComment { rowid },
                                            Err(e) => {
                                                tracing::error!(
                                                    "读取 issue_comment rowid={} 失败: {}",
                                                    rowid,
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::PipelineRuns, _) => {
                                        match db::models::pipeline::PipelineRuns::find_by_rowid(
                                            &db.pool, rowid,
                                        )
                                        .await
                                        {
                                            Ok(Some(run)) => RecordTypes::PipelineRun(run),
                                            Ok(None) => RecordTypes::DeletedPipelineRun { rowid },
                                            Err(e) => {
                                                tracing::error!(
                                                    "读取 pipeline_run rowid={} 失败: {}",
                                                    rowid,
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    (HookTables::PipelineStageRuns, _) => {
                                        match db::models::pipeline::PipelineStageRuns::find_by_rowid(
                                            &db.pool, rowid,
                                        )
                                        .await
                                        {
                                            Ok(Some(stage_run)) => {
                                                RecordTypes::PipelineStageRun(stage_run)
                                            }
                                            Ok(None) => {
                                                RecordTypes::DeletedPipelineStageRun { rowid }
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "读取 pipeline_stage_run rowid={} 失败: {}",
                                                    rowid,
                                                    e
                                                );
                                                return;
                                            }
                                        }
                                    }
                                };

                                let db_op: &str = match hook.operation {
                                    SqliteOperation::Insert => "insert",
                                    SqliteOperation::Delete => "delete",
                                    SqliteOperation::Update => "update",
                                    SqliteOperation::Unknown(_) => "unknown",
                                };

                                // Handle operations with direct patches
                                match &record_type {
                                    RecordTypes::Scratch(scratch) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => scratch_patch::add(scratch),
                                            SqliteOperation::Update => scratch_patch::replace(scratch),
                                            _ => scratch_patch::replace(scratch),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::DeletedScratch {
                                        scratch_id: Some(scratch_id),
                                        scratch_type: Some(scratch_type_str),
                                        ..
                                    } => {
                                        let patch = scratch_patch::remove(*scratch_id, scratch_type_str);
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::Workspace(workspace) => {
                                        // Emit workspace patch with status
                                        if let Ok(Some(workspace_with_status)) =
                                            Workspace::find_by_id_with_status(&db.pool, workspace.id)
                                                .await
                                        {
                                            let patch = match hook.operation {
                                                SqliteOperation::Insert => {
                                                    workspace_patch::add(&workspace_with_status)
                                                }
                                                _ => workspace_patch::replace(&workspace_with_status),
                                            };
                                            msg_store_for_hook.push_patch(patch);
                                        }
                                        return;
                                    }
                                    RecordTypes::DeletedWorkspace { .. } => {
                                        return;
                                    }
                                    RecordTypes::ExecutionProcess(process) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => {
                                                execution_process_patch::add(process)
                                            }
                                            SqliteOperation::Update => {
                                                execution_process_patch::replace(process)
                                            }
                                            _ => execution_process_patch::replace(process), // fallback
                                        };
                                        msg_store_for_hook.push_patch(patch);

                                        if let Err(err) = EventService::push_workspace_update_for_session(
                                            &db.pool,
                                            msg_store_for_hook.clone(),
                                            process.session_id,
                                        )
                                        .await
                                        {
                                            tracing::error!(
                                                "Failed to push workspace update after execution process change: {:?}",
                                                err
                                            );
                                        }

                                        return;
                                    }
                                    RecordTypes::DeletedExecutionProcess {
                                        process_id: Some(process_id),
                                        session_id,
                                        ..
                                    } => {
                                        let patch = execution_process_patch::remove(*process_id);
                                        msg_store_for_hook.push_patch(patch);

                                        if let Some(session_id) = session_id
                                            && let Err(err) =
                                                EventService::push_workspace_update_for_session(
                                                    &db.pool,
                                                    msg_store_for_hook.clone(),
                                                    *session_id,
                                                )
                                                .await
                                            {
                                                tracing::error!(
                                                    "Failed to push workspace update after execution process removal: {:?}",
                                                    err
                                                );
                                        }

                                        return;
                                    }
                                    RecordTypes::Issue(issue) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => issue_patch::add(issue),
                                            _ => issue_patch::replace(issue),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::ProjectStatus(status) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => project_status_patch::add(status),
                                            _ => project_status_patch::replace(status),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::IssueComment(comment) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => issue_comment_patch::add(comment),
                                            _ => issue_comment_patch::replace(comment),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::PipelineRun(run) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => pipeline_run_patch::add(run),
                                            _ => pipeline_run_patch::replace(run),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    RecordTypes::PipelineStageRun(stage_run) => {
                                        let patch = match hook.operation {
                                            SqliteOperation::Insert => {
                                                pipeline_stage_run_patch::add(stage_run)
                                            }
                                            _ => pipeline_stage_run_patch::replace(stage_run),
                                        };
                                        msg_store_for_hook.push_patch(patch);
                                        return;
                                    }
                                    // 删除已由 preupdate 推过 remove，这里不再走旧的 entries 兜底格式。
                                    RecordTypes::DeletedPipelineRun { .. }
                                    | RecordTypes::DeletedPipelineStageRun { .. } => {
                                        return;
                                    }
                                    _ => {}
                                }

                                // Fallback: use the old entries format for other record types
                                let next_entry_count = {
                                    let mut entry_count = entry_count_for_hook.write().await;
                                    *entry_count += 1;
                                    *entry_count
                                };

                                let event_patch: EventPatch = EventPatch {
                                    op: "add".to_string(),
                                    path: format!("/entries/{next_entry_count}"),
                                    value: EventPatchInner {
                                        db_op: db_op.to_string(),
                                        record: record_type,
                                    },
                                };

                                let patch =
                                    serde_json::from_value(json!([
                                        serde_json::to_value(event_patch).unwrap()
                                    ]))
                                    .unwrap();

                                msg_store_for_hook.push_patch(patch);
                            }
                            .await;
                            // 只读，回滚即可；回滚失败时 sqlx 在连接归还前会再补一次回滚。
                            if let Err(e) = barrier.rollback().await {
                                tracing::error!("释放变更推送屏障失败: {}", e);
                            }
                        });
                    }
                });

                Ok(())
            })
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
        let entry_count = Arc::new(RwLock::new(0usize));
        let hook = EventService::create_hook(msg_store.clone(), entry_count, hook_query_db);

        let db = DBService::new_at_path_with_after_connect(&path, journal_mode, hook)
            .await
            .expect("装钩子初始化 DBService 失败");

        Fixture {
            _dir: dir,
            db,
            msg_store,
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
    #[tokio::test]
    async fn 写事务延迟提交时_replace_补丁带的是提交后的值() {
        use db::models::pipeline::PipelineRuns;

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

    #[tokio::test]
    async fn 写事务延迟提交时_insert_仍产出_add_补丁() {
        let fixture = setup(SqliteJournalMode::Wal).await;
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
}
