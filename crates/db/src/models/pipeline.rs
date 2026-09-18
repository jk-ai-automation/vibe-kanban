//! 交付流水线（U2）的数据模型与读写。
//!
//! 类型名与字段名是前后端契约（`docs/superpowers/plans/2026-09-18-pipeline-contract.md` §1），
//! 改名先改契约。查询一律用运行时 `sqlx::query_as::<_, T>(..)`，不进 `.sqlx` 离线缓存。

use chrono::{DateTime, Utc};
use executors::profile::ExecutorConfig;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use ts_rs::TS;
use uuid::Uuid;

/// 产出物入库上限：超过部分只留在磁盘（设计 §5）。
pub const MAX_ARTIFACT_BYTES: usize = 256 * 1024;

/// 截断后追加在库内内容末尾的标记。
pub const TRUNCATION_MARKER: &str =
    "\n\n……（内容超过 256 KB，库里只保留前 256 KB；完整内容见工作区产出物目录）\n";

// ---------------------------------------------------------------------------
// 枚举
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_stage_key", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageKey {
    Requirement,
    Spec,
    TestDesign,
    Develop,
    Review,
    Test,
    Deliver,
}

impl PipelineStageKey {
    /// 标准顺序。模板里的阶段必须按这个顺序出现（可以省略某些阶段）。
    pub const ALL: [PipelineStageKey; 7] = [
        PipelineStageKey::Requirement,
        PipelineStageKey::Spec,
        PipelineStageKey::TestDesign,
        PipelineStageKey::Develop,
        PipelineStageKey::Review,
        PipelineStageKey::Test,
        PipelineStageKey::Deliver,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PipelineStageKey::Requirement => "requirement",
            PipelineStageKey::Spec => "spec",
            PipelineStageKey::TestDesign => "test_design",
            PipelineStageKey::Develop => "develop",
            PipelineStageKey::Review => "review",
            PipelineStageKey::Test => "test",
            PipelineStageKey::Deliver => "deliver",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == value)
    }

    /// 在标准顺序里的位置，从 0 开始。
    pub fn order(self) -> usize {
        Self::ALL
            .iter()
            .position(|key| *key == self)
            .expect("ALL 覆盖全部取值")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_run_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    Running,
    WaitingGate,
    Paused,
    Failed,
    Completed,
    Cancelled,
}

impl PipelineRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineRunStatus::Running => "running",
            PipelineRunStatus::WaitingGate => "waiting_gate",
            PipelineRunStatus::Paused => "paused",
            PipelineRunStatus::Failed => "failed",
            PipelineRunStatus::Completed => "completed",
            PipelineRunStatus::Cancelled => "cancelled",
        }
    }

    /// 已结束：不能再暂停、继续或决策。`failed` 可以继续，所以不算结束。
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            PipelineRunStatus::Completed | PipelineRunStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_stage_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageStatus {
    Pending,
    Running,
    WaitingGate,
    Passed,
    Rejected,
    Failed,
    Skipped,
}

impl PipelineStageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineStageStatus::Pending => "pending",
            PipelineStageStatus::Running => "running",
            PipelineStageStatus::WaitingGate => "waiting_gate",
            PipelineStageStatus::Passed => "passed",
            PipelineStageStatus::Rejected => "rejected",
            PipelineStageStatus::Failed => "failed",
            PipelineStageStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "gate_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Human,
    Auto,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "gate_decision_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GateDecisionKind {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "artifact_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Requirement,
    Spec,
    Plan,
    TestCases,
    TraceMatrix,
    Review,
    TestReport,
    DeliveryReport,
}

impl ArtifactKind {
    /// 契约 §4：产出物文件名 → kind。不在表里的文件名不入库。
    pub fn from_file_name(name: &str) -> Option<Self> {
        match name {
            "requirement.md" => Some(ArtifactKind::Requirement),
            "spec.md" => Some(ArtifactKind::Spec),
            "plan.md" => Some(ArtifactKind::Plan),
            "test-cases.csv" => Some(ArtifactKind::TestCases),
            "trace-matrix.md" => Some(ArtifactKind::TraceMatrix),
            "review.json" => Some(ArtifactKind::Review),
            "test-report.json" => Some(ArtifactKind::TestReport),
            "delivery-report.md" => Some(ArtifactKind::DeliveryReport),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 行类型（契约 §1）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineRun {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub template_key: String,
    #[ts(type = "number")]
    pub template_version: i64,
    pub status: PipelineRunStatus,
    pub current_stage_key: PipelineStageKey,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineStageRun {
    pub id: Uuid,
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub stage_key: PipelineStageKey,
    #[ts(type = "number")]
    pub attempt: i64,
    pub status: PipelineStageStatus,
    pub gate_kind: GateKind,
    pub session_id: Option<Uuid>,
    pub execution_process_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineGateDecision {
    pub id: Uuid,
    pub stage_run_id: Uuid,
    pub decision: GateDecisionKind,
    pub comment: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct IssueArtifactSummary {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub stage_run_id: Uuid,
    pub kind: ArtifactKind,
    pub rel_path: String,
    #[ts(type = "number")]
    pub version: i64,
    pub truncated: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct IssueArtifact {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub summary: IssueArtifactSummary,
    pub content: String,
}

// ---------------------------------------------------------------------------
// 视图与请求类型（契约 §1）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineTemplateStageView {
    pub key: PipelineStageKey,
    pub skill: String,
    pub gate_kind: GateKind,
    /// 人工关卡中文名，如「需求确认」。
    pub gate_label: Option<String>,
    /// 自动关卡的判定名（`checks_passed` / `no_blocking_findings` / `all_cases_passed` /
    /// `artifacts_present`）；人工关卡与无关卡为 None。契约修订 C10。
    pub gate_condition: Option<String>,
    #[ts(type = "number")]
    pub max_rounds: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineTemplateView {
    pub key: String,
    #[ts(type = "number")]
    pub version: i64,
    pub stages: Vec<PipelineTemplateStageView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct IssuePipelineView {
    pub run: PipelineRun,
    /// 全部尝试，按创建顺序（已启动者即 started_at 升序，pending 排最后）。
    pub stages: Vec<PipelineStageRun>,
    /// 全部关卡决策，按 decided_at 升序。
    pub decisions: Vec<PipelineGateDecision>,
    /// 该需求每个 kind 的全部版本。
    pub artifacts: Vec<IssueArtifactSummary>,
    pub template: PipelineTemplateView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct StartPipelineRepo {
    pub repo_id: Uuid,
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StartPipelineRequest {
    pub repos: Vec<StartPipelineRepo>,
    /// 与 `create_and_start_workspace` 相同的类型。
    pub executor_config: ExecutorConfig,
    /// 缺省 "standard"。
    pub template_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GateDecisionRequest {
    pub decision: GateDecisionKind,
    /// reject 时必填，后端校验非空。
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PendingPipelineItem {
    pub run: PipelineRun,
    /// 当前等待或失败的那一条。
    pub stage_run: PipelineStageRun,
    pub issue_simple_id: String,
    pub issue_title: String,
}

// ---------------------------------------------------------------------------
// 写入参数与内部列
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CreatePipelineRun {
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub template_key: String,
    pub template_version: i64,
    pub template_json: String,
    pub template_warning: Option<String>,
    pub executor_config_json: String,
    pub first_stage: PipelineStageKey,
}

/// `pipeline_runs` 里不对外暴露的列。
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineRunInternals {
    pub template_json: String,
    pub template_warning: Option<String>,
    pub executor_config_json: String,
}

#[derive(Debug, Clone)]
pub struct CreateStageRun {
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub stage_key: PipelineStageKey,
    pub gate_kind: GateKind,
    /// 只允许 `Running`（立即启动）或 `Pending`（运行已暂停）。
    pub status: PipelineStageStatus,
    /// 本次尝试要带进提示词的打回意见或失败原因。
    pub feedback: Option<String>,
}

const RUN_COLUMNS: &str = "id, issue_id, project_id, workspace_id, template_key, template_version, \
     status, current_stage_key, created_at, updated_at, finished_at";

const STAGE_COLUMNS: &str = "id, run_id, project_id, stage_key, attempt, status, gate_kind, \
     session_id, execution_process_id, started_at, finished_at, summary, error";

const DECISION_COLUMNS: &str = "id, stage_run_id, decision, comment, decided_by, decided_at";

const ARTIFACT_SUMMARY_COLUMNS: &str =
    "id, issue_id, stage_run_id, kind, rel_path, version, truncated, created_at";

// ---------------------------------------------------------------------------
// pipeline_runs
// ---------------------------------------------------------------------------

pub struct PipelineRuns;

impl PipelineRuns {
    pub async fn create(
        pool: &SqlitePool,
        data: &CreatePipelineRun,
    ) -> Result<PipelineRun, sqlx::Error> {
        let sql = format!(
            "INSERT INTO pipeline_runs (id, issue_id, project_id, workspace_id, template_key, \
             template_version, template_json, template_warning, executor_config, status, \
             current_stage_key, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'running', ?10, ?11, ?11) \
             RETURNING {RUN_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(Uuid::new_v4())
            .bind(data.issue_id)
            .bind(data.project_id)
            .bind(data.workspace_id)
            .bind(&data.template_key)
            .bind(data.template_version)
            .bind(&data.template_json)
            .bind(&data.template_warning)
            .bind(&data.executor_config_json)
            .bind(data.first_stage)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE id = ?1");
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE rowid = ?1");
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(rowid)
            .fetch_optional(pool)
            .await
    }

    /// 该需求最近一条运行（不论状态）。
    pub async fn find_latest_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE issue_id = ?1 \
             ORDER BY created_at DESC, rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(issue_id)
            .fetch_optional(pool)
            .await
    }

    /// 该需求未结束（非 completed / cancelled）的运行。
    pub async fn find_active_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE issue_id = ?1 \
             AND status NOT IN ('completed', 'cancelled') ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(issue_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn has_active_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        Ok(Self::find_active_for_issue(pool, issue_id).await?.is_some())
    }

    pub async fn list_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE project_id = ?1 \
             ORDER BY created_at, rowid"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(project_id)
            .fetch_all(pool)
            .await
    }

    pub async fn internals(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineRunInternals>, sqlx::Error> {
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT template_json, template_warning, executor_config FROM pipeline_runs WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        Ok(
            row.map(|(template_json, template_warning, executor_config_json)| {
                PipelineRunInternals {
                    template_json,
                    template_warning,
                    executor_config_json,
                }
            }),
        )
    }

    /// 改运行状态；`current_stage` 为 None 时保持不变。completed / cancelled 写 finished_at。
    pub async fn update_status(
        pool: &SqlitePool,
        id: Uuid,
        status: PipelineRunStatus,
        current_stage: Option<PipelineStageKey>,
    ) -> Result<PipelineRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_runs SET status = ?2, \
             current_stage_key = COALESCE(?3, current_stage_key), updated_at = ?4, \
             finished_at = CASE WHEN ?2 IN ('completed', 'cancelled') THEN ?4 ELSE NULL END \
             WHERE id = ?1 RETURNING {RUN_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(id)
            .bind(status)
            .bind(current_stage)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }

    /// 工作台「需要你确认 / 需要你处理」：waiting_gate 与 failed 的运行，等得最久的在前。
    pub async fn list_pending(
        pool: &SqlitePool,
        project_id: Option<Uuid>,
    ) -> Result<Vec<PendingPipelineItem>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs \
             WHERE status IN ('waiting_gate', 'failed') AND (?1 IS NULL OR project_id = ?1) \
             ORDER BY updated_at ASC, rowid ASC"
        );
        let runs = sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(project_id)
            .fetch_all(pool)
            .await?;

        let mut items = Vec::with_capacity(runs.len());
        for run in runs {
            let Some(stage_run) = PipelineStageRuns::find_latest_for_run(pool, run.id).await?
            else {
                continue;
            };
            let issue: Option<(String, String)> =
                sqlx::query_as("SELECT simple_id, title FROM issues WHERE id = ?1")
                    .bind(run.issue_id)
                    .fetch_optional(pool)
                    .await?;
            let Some((issue_simple_id, issue_title)) = issue else {
                continue;
            };
            items.push(PendingPipelineItem {
                run,
                stage_run,
                issue_simple_id,
                issue_title,
            });
        }
        Ok(items)
    }
}

// ---------------------------------------------------------------------------
// pipeline_stage_runs
// ---------------------------------------------------------------------------

pub struct PipelineStageRuns;

impl PipelineStageRuns {
    /// 新建一次尝试；attempt = 同运行同阶段已有尝试数 + 1。status 为 Running 时写 started_at。
    pub async fn create(
        pool: &SqlitePool,
        data: &CreateStageRun,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let started_at = (data.status == PipelineStageStatus::Running).then(Utc::now);
        let sql = format!(
            "INSERT INTO pipeline_stage_runs (id, run_id, project_id, stage_key, attempt, status, \
             gate_kind, feedback, started_at) \
             SELECT ?1, ?2, ?3, ?4, COALESCE(MAX(attempt), 0) + 1, ?5, ?6, ?7, ?8 \
             FROM pipeline_stage_runs WHERE run_id = ?2 AND stage_key = ?4 \
             RETURNING {STAGE_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(Uuid::new_v4())
            .bind(data.run_id)
            .bind(data.project_id)
            .bind(data.stage_key)
            .bind(data.status)
            .bind(data.gate_kind)
            .bind(&data.feedback)
            .bind(started_at)
            .fetch_one(pool)
            .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!("SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE id = ?1");
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!("SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE rowid = ?1");
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(rowid)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE run_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(run_id)
            .fetch_all(pool)
            .await
    }

    pub async fn list_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE project_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(project_id)
            .fetch_all(pool)
            .await
    }

    pub async fn find_latest_for_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE run_id = ?1 \
             ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(run_id)
            .fetch_optional(pool)
            .await
    }

    /// 会话里正在跑的那条尝试。执行进程退出回调靠它认领。
    pub async fn find_running_by_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs \
             WHERE session_id = ?1 AND status = 'running' ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(session_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_running(pool: &SqlitePool) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE status = 'running' ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .fetch_all(pool)
            .await
    }

    /// 同运行同阶段 status = failed 的尝试数（人工打回是 rejected，不计入）。
    pub async fn count_failed(
        pool: &SqlitePool,
        run_id: Uuid,
        stage_key: PipelineStageKey,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM pipeline_stage_runs \
             WHERE run_id = ?1 AND stage_key = ?2 AND status = 'failed'",
        )
        .bind(run_id)
        .bind(stage_key)
        .fetch_one(pool)
        .await
    }

    /// 该运行是否已经启动过任何会话（决定首个会话是否跑 setup 脚本）。
    pub async fn any_launched(pool: &SqlitePool, run_id: Uuid) -> Result<bool, sqlx::Error> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pipeline_stage_runs WHERE run_id = ?1 AND session_id IS NOT NULL",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn feedback(pool: &SqlitePool, id: Uuid) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT feedback FROM pipeline_stage_runs WHERE id = ?1")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        Ok(row.and_then(|(feedback,)| feedback))
    }

    /// 会话已开：记会话与首个进程，状态置 running，started_at 只写一次。
    pub async fn mark_started(
        pool: &SqlitePool,
        id: Uuid,
        session_id: Uuid,
        execution_process_id: Uuid,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_stage_runs SET status = 'running', session_id = ?2, \
             execution_process_id = ?3, started_at = COALESCE(started_at, ?4) \
             WHERE id = ?1 RETURNING {STAGE_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(id)
            .bind(session_id)
            .bind(execution_process_id)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }

    pub async fn set_execution_process(
        pool: &SqlitePool,
        id: Uuid,
        execution_process_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE pipeline_stage_runs SET execution_process_id = ?2 WHERE id = ?1")
            .bind(id)
            .bind(execution_process_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// 改尝试状态。summary / error 为 None 时保持原值；进入终态时写 finished_at。
    pub async fn set_status(
        pool: &SqlitePool,
        id: Uuid,
        status: PipelineStageStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_stage_runs SET status = ?2, summary = COALESCE(?3, summary), \
             error = COALESCE(?4, error), \
             finished_at = CASE WHEN ?2 IN ('passed', 'rejected', 'failed', 'skipped') \
                                THEN ?5 ELSE finished_at END \
             WHERE id = ?1 RETURNING {STAGE_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(id)
            .bind(status)
            .bind(summary)
            .bind(error)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }
}

// ---------------------------------------------------------------------------
// pipeline_gate_decisions
// ---------------------------------------------------------------------------

pub struct PipelineGateDecisions;

impl PipelineGateDecisions {
    pub async fn create(
        pool: &SqlitePool,
        stage_run_id: Uuid,
        decision: GateDecisionKind,
        comment: Option<&str>,
        decided_by: Option<Uuid>,
    ) -> Result<PipelineGateDecision, sqlx::Error> {
        let sql = format!(
            "INSERT INTO pipeline_gate_decisions (id, stage_run_id, decision, comment, decided_by, decided_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING {DECISION_COLUMNS}"
        );
        sqlx::query_as::<_, PipelineGateDecision>(&sql)
            .bind(Uuid::new_v4())
            .bind(stage_run_id)
            .bind(decision)
            .bind(comment)
            .bind(decided_by)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }

    pub async fn list_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<PipelineGateDecision>, sqlx::Error> {
        sqlx::query_as::<_, PipelineGateDecision>(
            "SELECT d.id, d.stage_run_id, d.decision, d.comment, d.decided_by, d.decided_at \
             FROM pipeline_gate_decisions d \
             JOIN pipeline_stage_runs s ON s.id = d.stage_run_id \
             WHERE s.run_id = ?1 ORDER BY d.decided_at, d.rowid",
        )
        .bind(run_id)
        .fetch_all(pool)
        .await
    }
}

// ---------------------------------------------------------------------------
// issue_artifacts
// ---------------------------------------------------------------------------

/// 入库前截断：不超过 256 KB 原样返回；超过则在字符边界截断并追加标记。
pub fn truncate_for_storage(content: &str) -> (String, bool) {
    if content.len() <= MAX_ARTIFACT_BYTES {
        return (content.to_string(), false);
    }
    let mut end = MAX_ARTIFACT_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut stored = content[..end].to_string();
    stored.push_str(TRUNCATION_MARKER);
    (stored, true)
}

pub struct IssueArtifacts;

impl IssueArtifacts {
    /// 写一个新版本；version = 同需求同 kind 的最大版本 + 1。content 超限会被截断。
    pub async fn insert_version(
        pool: &SqlitePool,
        issue_id: Uuid,
        stage_run_id: Uuid,
        kind: ArtifactKind,
        rel_path: &str,
        content: &str,
    ) -> Result<IssueArtifactSummary, sqlx::Error> {
        let (stored, truncated) = truncate_for_storage(content);
        let sql = format!(
            "INSERT INTO issue_artifacts (id, issue_id, stage_run_id, kind, rel_path, content, \
             truncated, version, created_at) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(MAX(version), 0) + 1, ?8 \
             FROM issue_artifacts WHERE issue_id = ?2 AND kind = ?4 \
             RETURNING {ARTIFACT_SUMMARY_COLUMNS}"
        );
        sqlx::query_as::<_, IssueArtifactSummary>(&sql)
            .bind(Uuid::new_v4())
            .bind(issue_id)
            .bind(stage_run_id)
            .bind(kind)
            .bind(rel_path)
            .bind(stored)
            .bind(truncated)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<IssueArtifact>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS}, content FROM issue_artifacts WHERE id = ?1"
        );
        sqlx::query_as::<_, IssueArtifact>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn latest_for_kind(
        pool: &SqlitePool,
        issue_id: Uuid,
        kind: ArtifactKind,
    ) -> Result<Option<IssueArtifact>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS}, content FROM issue_artifacts \
             WHERE issue_id = ?1 AND kind = ?2 ORDER BY version DESC LIMIT 1"
        );
        sqlx::query_as::<_, IssueArtifact>(&sql)
            .bind(issue_id)
            .bind(kind)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_summaries_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Vec<IssueArtifactSummary>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS} FROM issue_artifacts WHERE issue_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, IssueArtifactSummary>(&sql)
            .bind(issue_id)
            .fetch_all(pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use api_types::{
        issue::{CreateIssueRequest, Issue},
        project::CreateProjectRequest,
    };
    use uuid::Uuid;

    use super::*;
    use crate::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

    async fn 准备需求(test_db: &TestDb, title: &str) -> Issue {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "流水线项目".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let backlog = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Backlog)
            .await
            .unwrap()
            .unwrap();
        Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: backlog.id,
                title: title.to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
            DEFAULT_USER_ID,
        )
        .await
        .unwrap()
    }

    fn 建运行参数(issue: &Issue) -> CreatePipelineRun {
        CreatePipelineRun {
            issue_id: issue.id,
            project_id: issue.project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: PipelineStageKey::Requirement,
        }
    }

    fn 建阶段参数(
        run: &PipelineRun,
        stage_key: PipelineStageKey,
        status: PipelineStageStatus,
    ) -> CreateStageRun {
        CreateStageRun {
            run_id: run.id,
            project_id: run.project_id,
            stage_key,
            gate_kind: GateKind::Human,
            status,
            feedback: None,
        }
    }

    #[tokio::test]
    async fn 创建运行后可按需求查到最近一条且内部列不外露() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "导出报表").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();

        assert_eq!(run.status, PipelineRunStatus::Running);
        assert_eq!(run.current_stage_key, PipelineStageKey::Requirement);
        assert!(run.finished_at.is_none());

        let latest = PipelineRuns::find_latest_for_issue(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, run);

        let internals = PipelineRuns::internals(test_db.pool(), run.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(internals.executor_config_json, "{}");

        let json = serde_json::to_value(&run).unwrap();
        assert!(
            json.get("template_json").is_none(),
            "内部列不能出现在对外类型里"
        );
        assert_eq!(json["status"], "running");
        assert_eq!(json["current_stage_key"], "requirement");
    }

    #[tokio::test]
    async fn 同一需求不能同时有两条未结束的运行() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "唯一").await;
        PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let second = PipelineRuns::create(test_db.pool(), &建运行参数(&issue)).await;
        let error = second.expect_err("第二条未结束的运行必须被唯一索引拒绝");
        assert!(crate::models::db_retry::is_unique_violation(&error));
        assert!(
            PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn 运行取消后可以再开一条且写结束时间() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "重开").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let cancelled =
            PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Cancelled, None)
                .await
                .unwrap();
        assert!(cancelled.finished_at.is_some());
        assert_eq!(cancelled.current_stage_key, PipelineStageKey::Requirement);
        assert!(
            !PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
        PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .expect("旧运行结束后应能新开");
    }

    #[tokio::test]
    async fn 失败的运行算未结束() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "失败").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let failed =
            PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Failed, None)
                .await
                .unwrap();
        assert!(failed.finished_at.is_none(), "failed 可继续，不写结束时间");
        assert!(
            PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn 阶段尝试号按阶段递增() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "尝试号").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let pool = test_db.pool();
        let a1 = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        let a2 = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Pending,
            ),
        )
        .await
        .unwrap();
        let s1 = PipelineStageRuns::create(
            pool,
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        assert_eq!((a1.attempt, a2.attempt, s1.attempt), (1, 2, 1));
        assert!(a1.started_at.is_some(), "running 创建即写开始时间");
        assert!(a2.started_at.is_none(), "pending 不写开始时间");

        let all = PipelineStageRuns::list_by_run(pool, run.id).await.unwrap();
        assert_eq!(
            all.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![a1.id, a2.id, s1.id],
            "按创建顺序返回"
        );
        assert_eq!(
            PipelineStageRuns::find_latest_for_run(pool, run.id)
                .await
                .unwrap()
                .unwrap()
                .id,
            s1.id
        );
    }

    #[tokio::test]
    async fn 终态写结束时间而等待确认不写() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "终态").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();

        let waiting = PipelineStageRuns::set_status(
            test_db.pool(),
            stage.id,
            PipelineStageStatus::WaitingGate,
            Some("等待人工确认"),
            None,
        )
        .await
        .unwrap();
        assert!(waiting.finished_at.is_none());
        assert_eq!(waiting.summary.as_deref(), Some("等待人工确认"));

        let rejected = PipelineStageRuns::set_status(
            test_db.pool(),
            stage.id,
            PipelineStageStatus::Rejected,
            None,
            Some("补充验收标准"),
        )
        .await
        .unwrap();
        assert!(rejected.finished_at.is_some());
        assert_eq!(
            rejected.summary.as_deref(),
            Some("等待人工确认"),
            "None 不覆盖原值"
        );
        assert_eq!(rejected.error.as_deref(), Some("补充验收标准"));
    }

    #[tokio::test]
    async fn 失败次数只数_failed_且按会话认领正在跑的尝试() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "计数").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let pool = test_db.pool();
        for status in [PipelineStageStatus::Failed, PipelineStageStatus::Rejected] {
            let stage = PipelineStageRuns::create(
                pool,
                &建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Running),
            )
            .await
            .unwrap();
            PipelineStageRuns::set_status(pool, stage.id, status, None, None)
                .await
                .unwrap();
        }
        assert_eq!(
            PipelineStageRuns::count_failed(pool, run.id, PipelineStageKey::Review)
                .await
                .unwrap(),
            1
        );
        assert!(!PipelineStageRuns::any_launched(pool, run.id).await.unwrap());

        let running = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                feedback: Some("上次失败原因".to_string()),
                ..建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Pending)
            },
        )
        .await
        .unwrap();
        let session_id = Uuid::new_v4();
        let process_id = Uuid::new_v4();
        let started = PipelineStageRuns::mark_started(pool, running.id, session_id, process_id)
            .await
            .unwrap();
        assert_eq!(started.status, PipelineStageStatus::Running);
        assert!(started.started_at.is_some());
        assert_eq!(started.execution_process_id, Some(process_id));
        assert!(PipelineStageRuns::any_launched(pool, run.id).await.unwrap());
        assert_eq!(
            PipelineStageRuns::feedback(pool, running.id)
                .await
                .unwrap()
                .as_deref(),
            Some("上次失败原因")
        );
        assert_eq!(
            PipelineStageRuns::find_running_by_session(pool, session_id)
                .await
                .unwrap()
                .unwrap()
                .id,
            running.id
        );
        assert!(
            PipelineStageRuns::find_running_by_session(pool, Uuid::new_v4())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 产出物版本号按需求和种类递增() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "产出物").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let pool = test_db.pool();
        let v1 = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Spec,
            ".vk/runs/X/spec.md",
            "第一版",
        )
        .await
        .unwrap();
        let v2 = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Spec,
            ".vk/runs/X/spec.md",
            "第二版",
        )
        .await
        .unwrap();
        let plan = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Plan,
            ".vk/runs/X/plan.md",
            "计划",
        )
        .await
        .unwrap();
        assert_eq!((v1.version, v2.version, plan.version), (1, 2, 1));
        assert!(!v1.truncated);

        let latest = IssueArtifacts::latest_for_kind(pool, issue.id, ArtifactKind::Spec)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.content, "第二版");
        assert_eq!(latest.summary.id, v2.id);

        let fetched = IssueArtifacts::find_by_id(pool, plan.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.content, "计划");
        let json = serde_json::to_value(&fetched).unwrap();
        assert_eq!(json["kind"], "plan", "summary 字段平铺在顶层");
        assert_eq!(json["content"], "计划");

        let all = IssueArtifacts::list_summaries_for_issue(pool, issue.id)
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn 超过_256kb_的产出物截断入库并打标() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "大文件").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let big = "测".repeat(MAX_ARTIFACT_BYTES / 3 + 10);
        let summary = IssueArtifacts::insert_version(
            test_db.pool(),
            issue.id,
            stage.id,
            ArtifactKind::Review,
            ".vk/runs/X/review.json",
            &big,
        )
        .await
        .unwrap();
        assert!(summary.truncated);
        let stored = IssueArtifacts::find_by_id(test_db.pool(), summary.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.content.ends_with(TRUNCATION_MARKER));
        assert!(stored.content.len() <= MAX_ARTIFACT_BYTES + TRUNCATION_MARKER.len());
    }

    #[test]
    fn 截断落在字符边界上() {
        // 「测」是 3 字节，256 KB 不是 3 的倍数，截断点必须回退到字符边界。
        let text = "测".repeat(MAX_ARTIFACT_BYTES / 3 + 1);
        let (stored, truncated) = truncate_for_storage(&text);
        assert!(truncated);
        let body = stored.strip_suffix(TRUNCATION_MARKER).unwrap();
        assert!(body.chars().all(|c| c == '测'));
        assert!(body.len() <= MAX_ARTIFACT_BYTES);

        let (short, short_truncated) = truncate_for_storage("短文本");
        assert_eq!((short.as_str(), short_truncated), ("短文本", false));
    }

    #[tokio::test]
    async fn 待处理列表只含等待确认与失败且等得久的在前() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let a = 准备需求(&test_db, "甲").await;
        let b = 准备需求(&test_db, "乙").await;
        let c = 准备需求(&test_db, "丙").await;

        let mut runs = Vec::new();
        for issue in [&a, &b, &c] {
            let run = PipelineRuns::create(pool, &建运行参数(issue))
                .await
                .unwrap();
            PipelineStageRuns::create(
                pool,
                &建阶段参数(
                    &run,
                    PipelineStageKey::Requirement,
                    PipelineStageStatus::Running,
                ),
            )
            .await
            .unwrap();
            runs.push(run);
        }
        // 乙先进入等待，甲后失败，丙仍在跑。
        PipelineRuns::update_status(pool, runs[1].id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        PipelineRuns::update_status(pool, runs[0].id, PipelineRunStatus::Failed, None)
            .await
            .unwrap();

        let pending = PipelineRuns::list_pending(pool, None).await.unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|p| p.issue_title.as_str())
                .collect::<Vec<_>>(),
            vec!["乙", "甲"]
        );
        assert_eq!(
            pending[0].stage_run.stage_key,
            PipelineStageKey::Requirement
        );
        assert_eq!(pending[0].issue_simple_id, b.simple_id);

        let only_a = PipelineRuns::list_pending(pool, Some(a.project_id))
            .await
            .unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].run.issue_id, a.id);
    }

    #[tokio::test]
    async fn 关卡决策按运行列出() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "决策").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        PipelineGateDecisions::create(
            test_db.pool(),
            stage.id,
            GateDecisionKind::Reject,
            Some("补充边界"),
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
        PipelineGateDecisions::create(
            test_db.pool(),
            stage.id,
            GateDecisionKind::Approve,
            None,
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
        let decisions = PipelineGateDecisions::list_by_run(test_db.pool(), run.id)
            .await
            .unwrap();
        assert_eq!(
            decisions.iter().map(|d| d.decision).collect::<Vec<_>>(),
            vec![GateDecisionKind::Reject, GateDecisionKind::Approve]
        );
        assert_eq!(decisions[0].comment.as_deref(), Some("补充边界"));
    }

    #[test]
    fn 枚举序列化与库内取值一致() {
        assert_eq!(
            serde_json::to_value(PipelineStageKey::TestDesign).unwrap(),
            "test_design"
        );
        assert_eq!(
            serde_json::to_value(PipelineRunStatus::WaitingGate).unwrap(),
            "waiting_gate"
        );
        assert_eq!(
            serde_json::to_value(ArtifactKind::DeliveryReport).unwrap(),
            "delivery_report"
        );
        for key in PipelineStageKey::ALL {
            assert_eq!(PipelineStageKey::parse(key.as_str()), Some(key));
        }
        assert_eq!(
            ArtifactKind::from_file_name("test-cases.csv"),
            Some(ArtifactKind::TestCases)
        );
        assert_eq!(ArtifactKind::from_file_name("notes.md"), None);
    }

    #[test]
    fn 模板阶段视图序列化带_gate_condition() {
        let auto = PipelineTemplateStageView {
            key: PipelineStageKey::Develop,
            skill: "develop".to_string(),
            gate_kind: GateKind::Auto,
            gate_label: None,
            gate_condition: Some("checks_passed".to_string()),
            max_rounds: 3,
        };
        let json = serde_json::to_value(&auto).unwrap();
        assert_eq!(json["gate_condition"], "checks_passed");
        assert_eq!(json["gate_kind"], "auto");
        assert_eq!(json["max_rounds"], 3);

        let human = PipelineTemplateStageView {
            key: PipelineStageKey::Requirement,
            skill: "requirement".to_string(),
            gate_kind: GateKind::Human,
            gate_label: Some("需求确认".to_string()),
            gate_condition: None,
            max_rounds: 3,
        };
        let json = serde_json::to_value(&human).unwrap();
        assert!(
            json["gate_condition"].is_null(),
            "人工关卡的 gate_condition 为 null"
        );
        let back: PipelineTemplateStageView = serde_json::from_value(json).unwrap();
        assert_eq!(back, human);
    }
}
