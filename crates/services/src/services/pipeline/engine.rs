//! 流水线引擎：对外唯一入口 [`PipelineService`]（设计 §6，行为总表见计划 A §1.3）。
//!
//! 所有状态变更都在一把异步锁里串行执行：HTTP 请求（启动、关卡、暂停……）与
//! 执行进程退出回调可能并发到达，串行后状态机不会交错。锁内会调用启动器
//! （开会话、起进程），启动器不会回调本服务，所以不会死锁。

use std::{path::PathBuf, sync::Arc};

use api_types::issue::Issue;
use db::{
    DBService,
    models::{
        db_retry::is_unique_violation,
        execution_process::{ExecutionProcess, ExecutionProcessRunReason, ExecutionProcessStatus},
        issue::Issues,
        local_project_status::StageType,
        pipeline::{
            CreatePipelineRun, CreateStageRun, GateDecisionKind, GateDecisionRequest,
            IssueArtifacts, IssuePipelineView, MANUAL_STOP_ERROR, PipelineGateDecisions,
            PipelineRun, PipelineRunStatus, PipelineRuns, PipelineStageKey, PipelineStageRun,
            PipelineStageRuns, PipelineStageStatus,
        },
        workspace::Workspace,
    },
};
use executors::profile::ExecutorConfig;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;
use utils::log_msg::LogMsg;
use uuid::Uuid;

use super::{
    artifacts,
    gates::{self, StageVerdict},
    launcher::{self, StageLauncher},
    prompt::{self, StagePromptInput},
    template::{self, PipelineTemplate},
    transition::{self, FinishedProcess, PipelineExitEvent, Transition},
};

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("找不到{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("启动执行失败：{0}")]
    Launch(String),
}

pub struct StartPipelineInput {
    pub issue: Issue,
    pub workspace_id: Uuid,
    /// 读 `.vibe/pipeline.yaml` 的仓库根目录（第一个仓库的主检出目录）。
    pub repo_root: PathBuf,
    pub executor_config: ExecutorConfig,
    pub template_key: Option<String>,
}

const ACTIVE_RUN_CONFLICT: &str = "该需求已有未结束的流水线";

#[derive(Clone)]
pub struct PipelineService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DBService,
    launcher: Arc<dyn StageLauncher>,
    lock: Mutex<()>,
}

impl PipelineService {
    pub fn new(db: DBService, launcher: Arc<dyn StageLauncher>) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                launcher,
                lock: Mutex::new(()),
            }),
        }
    }

    fn pool(&self) -> &SqlitePool {
        &self.inner.db.pool
    }

    // ------------------------------------------------------------------
    // 查询（不加锁）
    // ------------------------------------------------------------------

    /// 需求最近一条运行的完整视图；没有运行时 None。
    pub async fn view_for_issue(
        &self,
        issue_id: Uuid,
    ) -> Result<Option<IssuePipelineView>, PipelineError> {
        match PipelineRuns::find_latest_for_issue(self.pool(), issue_id).await? {
            Some(run) => Ok(Some(self.view_for_run(run).await?)),
            None => Ok(None),
        }
    }

    pub async fn view_for_run_id(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let run = self.load_run(run_id).await?;
        self.view_for_run(run).await
    }

    async fn view_for_run(&self, run: PipelineRun) -> Result<IssuePipelineView, PipelineError> {
        let template = self.template_for(&run).await?;
        let pool = self.pool();
        Ok(IssuePipelineView {
            stages: PipelineStageRuns::list_by_run(pool, run.id).await?,
            decisions: PipelineGateDecisions::list_by_run(pool, run.id).await?,
            artifacts: IssueArtifacts::list_summaries_for_issue(pool, run.issue_id).await?,
            template: template.to_view(),
            run,
        })
    }

    // ------------------------------------------------------------------
    // 启动
    // ------------------------------------------------------------------

    /// 启动流水线。启动首阶段失败不返回错误：运行与阶段记为 failed，返回视图。
    pub async fn start(
        &self,
        input: StartPipelineInput,
    ) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();

        if PipelineRuns::has_active_for_issue(pool, input.issue.id).await? {
            return Err(PipelineError::Conflict(ACTIVE_RUN_CONFLICT.to_string()));
        }
        let (template, warning) = match input.template_key.as_deref() {
            None | Some(template::BUILTIN_TEMPLATE_KEY) => {
                template::load_template(&input.repo_root)
            }
            Some(other) => {
                return Err(PipelineError::BadRequest(format!("未知模板：{other}")));
            }
        };
        if let Some(warning) = &warning {
            tracing::warn!(issue_id = %input.issue.id, "{warning}");
        }

        // first_stage() 返回 Option（批 3 审查：杜绝空模板 panic）；load_template 的结果非空，这里只是兜底。
        let first = template
            .first_stage()
            .cloned()
            .ok_or_else(|| PipelineError::BadRequest("流水线模板没有任何阶段".to_string()))?;
        let run = PipelineRuns::create(
            pool,
            &CreatePipelineRun {
                issue_id: input.issue.id,
                project_id: input.issue.project_id,
                workspace_id: Some(input.workspace_id),
                template_key: template.key.clone(),
                template_version: template.version,
                template_json: serde_json::to_string(&template).expect("模板可以序列化"),
                template_warning: warning,
                executor_config_json: serde_json::to_string(&input.executor_config)
                    .expect("执行器配置可以序列化"),
                first_stage: first.key,
            },
        )
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                PipelineError::Conflict(ACTIVE_RUN_CONFLICT.to_string())
            } else {
                PipelineError::Database(e)
            }
        })?;

        let stage_run = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: first.key,
                gate_kind: first.gate.kind(),
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await?;
        self.move_issue(run.issue_id, transition::stage_column(first.key))
            .await;
        self.launch(&run, &stage_run, &template).await;

        self.view_for_run_id(run.id).await
    }

    // ------------------------------------------------------------------
    // 执行进程退出
    // ------------------------------------------------------------------

    /// 容器退出钩子发来的事件：读执行进程快照后交给 [`Self::on_process_finished`]。
    /// 错误只记日志——回调没有调用方可以上报。
    pub async fn handle_exit_event(&self, event: PipelineExitEvent) {
        let process =
            match ExecutionProcess::find_by_id(self.pool(), event.execution_process_id).await {
                Ok(Some(process)) => process,
                Ok(None) => return,
                Err(e) => {
                    tracing::warn!(
                        "流水线读取执行进程 {} 失败: {}",
                        event.execution_process_id,
                        e
                    );
                    return;
                }
            };
        let has_next_action = process
            .executor_action()
            .ok()
            .and_then(|action| action.next_action())
            .is_some();
        let finished = FinishedProcess {
            execution_process_id: process.id,
            session_id: process.session_id,
            run_reason: process.run_reason.clone(),
            status: process.status.clone(),
            exit_code: process.exit_code,
            has_next_action,
            chain_continues: event.chain_continues,
        };
        if let Err(e) = self.on_process_finished(finished).await {
            tracing::error!(
                "流水线处理执行进程 {} 退出失败: {}",
                event.execution_process_id,
                e
            );
        }
    }

    /// 阶段链终点的进程退出：判关卡并推进状态机。
    ///
    /// 模型层护栏拒绝写入（`RowNotFound`，例如取消与晚到的回调竞争）说明状态已被
    /// 别的路径改变：记日志后忽略本次回调。
    pub async fn on_process_finished(&self, process: FinishedProcess) -> Result<(), PipelineError> {
        if !transition::is_stage_terminal(&process) {
            return Ok(());
        }
        // 锁外先过滤：不是流水线会话（绝大多数普通工作区进程）直接返回，不去抢全局锁。
        // 锁内还会再查一次，以锁内结果为准。
        if PipelineStageRuns::find_running_by_session(self.pool(), process.session_id)
            .await?
            .is_none()
        {
            return Ok(());
        }
        let _guard = self.inner.lock.lock().await;
        match self.finish_stage_locked(&process).await {
            Err(e) if is_state_changed(&e) => {
                tracing::info!(
                    execution_process_id = %process.execution_process_id,
                    "流水线状态已被其他操作改变，忽略这次进程退出回调"
                );
                Ok(())
            }
            other => other,
        }
    }

    async fn finish_stage_locked(&self, process: &FinishedProcess) -> Result<(), PipelineError> {
        let pool = self.pool();

        let Some(stage_run) =
            PipelineStageRuns::find_running_by_session(pool, process.session_id).await?
        else {
            // 不是流水线的会话，或阶段已被取消/中断。
            return Ok(());
        };
        let run = self.load_run(stage_run.run_id).await?;
        if run.status.is_finished() {
            return Ok(());
        }
        let template = self.template_for(&run).await?;
        let Some(stage) = template.stage(stage_run.stage_key).cloned() else {
            let reason = format!("模板里没有阶段 {}", stage_run.stage_key.as_str());
            self.fail_run(&run, &stage_run, &reason).await?;
            return Ok(());
        };
        // 用户手动停止：不重跑、不计轮次，运行暂停等用户「继续」。
        if process.status == ExecutionProcessStatus::Killed {
            PipelineStageRuns::set_status(
                pool,
                stage_run.id,
                PipelineStageStatus::Failed,
                None,
                Some(MANUAL_STOP_ERROR),
            )
            .await?;
            PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Paused, None).await?;
            return Ok(());
        }
        // setup 脚本成为终点 = 顺序 setup 链断了，智能体没有启动，按失败处理。
        let setup_broken = process.run_reason == ExecutionProcessRunReason::SetupScript;
        let ok = !setup_broken && transition::process_succeeded(process);

        // 开发阶段：智能体成功后先在同会话跑检查脚本，检查脚本退出后再判关卡。
        if ok
            && !stage.checks.is_empty()
            && process.run_reason != ExecutionProcessRunReason::PipelineStep
        {
            let Some(workspace_id) = run.workspace_id else {
                self.fail_run(&run, &stage_run, "流水线没有关联工作区")
                    .await?;
                return Ok(());
            };
            match self
                .inner
                .launcher
                .start_checks(
                    workspace_id,
                    process.session_id,
                    launcher::checks_script(&stage.checks),
                )
                .await
            {
                Ok(step) => {
                    PipelineStageRuns::set_execution_process(
                        pool,
                        stage_run.id,
                        step.execution_process_id,
                    )
                    .await?;
                }
                Err(e) => {
                    let reason = format!("启动检查脚本失败：{e}");
                    self.fail_run(&run, &stage_run, &reason).await?;
                }
            }
            return Ok(());
        }

        if process.run_reason == ExecutionProcessRunReason::CodingAgent {
            PipelineStageRuns::set_execution_process(
                pool,
                stage_run.id,
                process.execution_process_id,
            )
            .await?;
        }

        let error = if setup_broken {
            Some(SETUP_BROKEN.to_string())
        } else if !ok {
            Some(self.describe_process_failure(process).await)
        } else {
            None
        };
        let issue = self.load_issue(run.issue_id).await?;
        // 进程失败时不读盘入库：失败轮次写了一半的文件不应成为新版本。
        let root = if ok {
            self.workspace_root(&run).await?
        } else {
            None
        };
        let collected = match root {
            Some(root) => {
                artifacts::ingest_stage_artifacts(
                    pool,
                    issue.id,
                    &issue.simple_id,
                    stage_run.id,
                    &artifacts::artifacts_dir(&root, &issue.simple_id),
                    &stage.artifacts,
                )
                .await?
            }
            None => Default::default(),
        };

        let verdict = gates::evaluate_stage(&stage, ok, error.as_deref(), &collected);
        let failed_before =
            PipelineStageRuns::count_failed(pool, run.id, stage_run.stage_key).await?;
        let failed_attempts = failed_before + i64::from(verdict.is_failure());
        let next =
            transition::next_transition(&template, stage_run.stage_key, &verdict, failed_attempts);
        let summary = summarize(&verdict);
        self.apply(
            &run,
            &stage_run,
            &template,
            next,
            PipelineStageStatus::Failed,
            Some(summary),
        )
        .await
    }

    // ------------------------------------------------------------------
    // 人工关卡
    // ------------------------------------------------------------------

    pub async fn decide_gate(
        &self,
        stage_run_id: Uuid,
        request: GateDecisionRequest,
        decided_by: Option<Uuid>,
    ) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        self.decide_gate_locked(stage_run_id, request, decided_by)
            .await
            .map_err(state_changed_as_conflict)
    }

    async fn decide_gate_locked(
        &self,
        stage_run_id: Uuid,
        request: GateDecisionRequest,
        decided_by: Option<Uuid>,
    ) -> Result<IssuePipelineView, PipelineError> {
        let pool = self.pool();

        let stage_run = PipelineStageRuns::find_by_id(pool, stage_run_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("阶段记录 {stage_run_id}")))?;
        if stage_run.status != PipelineStageStatus::WaitingGate {
            return Err(PipelineError::Conflict(format!(
                "该阶段当前状态是 {}，不在等待确认",
                stage_run.status.as_str()
            )));
        }
        let comment = request
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|comment| !comment.is_empty());
        if request.decision == GateDecisionKind::Reject && comment.is_none() {
            return Err(PipelineError::BadRequest("打回必须填写意见".to_string()));
        }
        let run = self.load_run(stage_run.run_id).await?;
        if run.status.is_finished() {
            return Err(PipelineError::Conflict("流水线已结束".to_string()));
        }

        // 先用比较并交换把阶段从 waiting_gate 关掉，成功后才记决策：重复决策第二次在这里
        // 得 RowNotFound（对外 409），不会重复记决策；中途出错也不会留下「有决策、阶段仍在等」。
        let closing = match request.decision {
            GateDecisionKind::Approve => PipelineStageStatus::Passed,
            GateDecisionKind::Reject => PipelineStageStatus::Rejected,
        };
        let error = match request.decision {
            GateDecisionKind::Approve => None,
            GateDecisionKind::Reject => comment,
        };
        PipelineStageRuns::set_status_from(
            pool,
            stage_run.id,
            PipelineStageStatus::WaitingGate,
            closing,
            error,
        )
        .await?;
        PipelineGateDecisions::create(pool, stage_run.id, request.decision, comment, decided_by)
            .await?;
        let template = self.template_for(&run).await?;
        let next = transition::gate_decision_transition(
            &template,
            stage_run.stage_key,
            request.decision,
            comment,
        );
        self.apply(
            &run,
            &stage_run,
            &template,
            next,
            PipelineStageStatus::Rejected,
            None,
        )
        .await?;
        self.view_for_run_id(run.id).await
    }

    // ------------------------------------------------------------------
    // 暂停 / 继续 / 取消
    // ------------------------------------------------------------------

    pub async fn pause(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        self.pause_locked(run_id)
            .await
            .map_err(state_changed_as_conflict)
    }

    async fn pause_locked(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let run = self.load_run(run_id).await?;
        match run.status {
            PipelineRunStatus::Running | PipelineRunStatus::WaitingGate => {
                PipelineRuns::update_status(self.pool(), run.id, PipelineRunStatus::Paused, None)
                    .await?;
            }
            other => {
                return Err(PipelineError::Conflict(format!(
                    "当前状态 {} 不能暂停",
                    other.as_str()
                )));
            }
        }
        self.view_for_run_id(run_id).await
    }

    pub async fn resume(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        self.resume_locked(run_id)
            .await
            .map_err(state_changed_as_conflict)
    }

    async fn resume_locked(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let pool = self.pool();
        let run = self.load_run(run_id).await?;
        if run.status.is_finished() {
            return Err(PipelineError::Conflict(format!(
                "当前状态 {} 不能继续",
                run.status.as_str()
            )));
        }
        let latest = PipelineStageRuns::find_latest_for_run(pool, run.id)
            .await?
            .ok_or_else(|| PipelineError::Conflict("流水线没有任何阶段记录".to_string()))?;
        // 正常情况只有 paused / failed 能继续；另外，最后一条尝试已结束（passed / rejected /
        // failed / skipped）却没有后续尝试，是多步写库中途出错或状态被别的路径改掉后留下的
        // 卡死态（没有活动尝试，也不会再有回调推进），任何未结束状态都允许继续。
        let stuck = matches!(
            latest.status,
            PipelineStageStatus::Passed
                | PipelineStageStatus::Rejected
                | PipelineStageStatus::Failed
                | PipelineStageStatus::Skipped
        );
        if !stuck
            && !matches!(
                run.status,
                PipelineRunStatus::Paused | PipelineRunStatus::Failed
            )
        {
            return Err(PipelineError::Conflict(format!(
                "当前状态 {} 不能继续",
                run.status.as_str()
            )));
        }
        let template = self.template_for(&run).await?;

        match latest.status {
            PipelineStageStatus::Pending => {
                let run = PipelineRuns::update_status(
                    pool,
                    run.id,
                    PipelineRunStatus::Running,
                    Some(latest.stage_key),
                )
                .await?;
                self.launch(&run, &latest, &template).await;
            }
            PipelineStageStatus::Running => {
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Running, None).await?;
            }
            PipelineStageStatus::WaitingGate => {
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::WaitingGate, None)
                    .await?;
            }
            PipelineStageStatus::Failed => {
                // 人工处理后再给一次机会：同阶段新尝试，上次失败原因带进提示词
                // （用户手动停止不算失败原因，不回喂）。
                let run = PipelineRuns::update_status(
                    pool,
                    run.id,
                    PipelineRunStatus::Running,
                    Some(latest.stage_key),
                )
                .await?;
                let feedback = latest
                    .error
                    .clone()
                    .filter(|error| error != MANUAL_STOP_ERROR);
                self.enter_stage(&run, &template, latest.stage_key, feedback)
                    .await?;
            }
            PipelineStageStatus::Skipped => {
                // 卡死态：尝试被跳过但运行未结束，同阶段重新开一次尝试。
                let run = PipelineRuns::update_status(
                    pool,
                    run.id,
                    PipelineRunStatus::Running,
                    Some(latest.stage_key),
                )
                .await?;
                self.enter_stage(&run, &template, latest.stage_key, None)
                    .await?;
            }
            PipelineStageStatus::Passed | PipelineStageStatus::Rejected => {
                // 卡死态：按转移规则重新推导——通过则进入下一阶段或完成，打回则同阶段新尝试。
                let run =
                    PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Running, None)
                        .await?;
                let next = if latest.status == PipelineStageStatus::Passed {
                    transition::gate_decision_transition(
                        &template,
                        latest.stage_key,
                        GateDecisionKind::Approve,
                        None,
                    )
                } else {
                    Transition::Rerun {
                        stage: latest.stage_key,
                        feedback: latest.error.clone().unwrap_or_default(),
                    }
                };
                self.apply(
                    &run,
                    &latest,
                    &template,
                    next,
                    PipelineStageStatus::Rejected,
                    None,
                )
                .await?;
            }
        }
        self.view_for_run_id(run_id).await
    }

    pub async fn cancel(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        self.cancel_locked(run_id)
            .await
            .map_err(state_changed_as_conflict)
    }

    async fn cancel_locked(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let pool = self.pool();
        let run = self.load_run(run_id).await?;
        if run.status.is_finished() {
            return Err(PipelineError::Conflict("流水线已结束".to_string()));
        }
        let mut was_running = false;
        if let Some(latest) = PipelineStageRuns::find_latest_for_run(pool, run.id).await?
            && matches!(
                latest.status,
                PipelineStageStatus::Pending
                    | PipelineStageStatus::Running
                    | PipelineStageStatus::WaitingGate
            )
        {
            was_running = latest.status == PipelineStageStatus::Running;
            PipelineStageRuns::set_status(
                pool,
                latest.id,
                PipelineStageStatus::Skipped,
                None,
                Some("流水线已取消"),
            )
            .await?;
        }
        PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Cancelled, None).await?;
        if was_running && let Some(workspace_id) = run.workspace_id {
            self.inner.launcher.stop_workspace(workspace_id).await;
        }
        self.view_for_run_id(run_id).await
    }

    /// 服务启动时调用：上次没跑完的阶段（进程已随服务退出）记失败，运行转人工。
    /// 返回处理的尝试数。
    pub async fn recover_interrupted(&self) -> Result<usize, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();
        let running = PipelineStageRuns::list_running(pool).await?;
        let count = running.len();
        for stage_run in running {
            let run = self.load_run(stage_run.run_id).await?;
            if run.status.is_finished() {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Skipped,
                    None,
                    None,
                )
                .await?;
                continue;
            }
            match self
                .fail_run(&run, &stage_run, "服务重启，阶段执行被中断")
                .await
            {
                Err(e) if is_state_changed(&e) => {
                    tracing::info!(stage_run_id = %stage_run.id, "阶段状态已被其他操作改变，跳过");
                }
                other => other?,
            }
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // 内部：应用转移、进入阶段、启动
    // ------------------------------------------------------------------

    async fn apply(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
        next: Transition,
        closing: PipelineStageStatus,
        summary: Option<String>,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        match next {
            Transition::WaitHuman => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::WaitingGate,
                    summary.as_deref(),
                    None,
                )
                .await?;
                // 必须用重新读出的运行状态：`run` 是处理开始时的快照，期间用户可能刚暂停。
                let current = self.load_run(run.id).await?.status;
                let next_status = transition::waiting_gate_run_status(current);
                if next_status != current {
                    PipelineRuns::update_status(pool, run.id, next_status, None).await?;
                }
            }
            Transition::Advance { to } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Passed,
                    summary.as_deref(),
                    None,
                )
                .await?;
                self.enter_stage(run, template, to, None).await?;
            }
            Transition::Complete => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Passed,
                    summary.as_deref(),
                    None,
                )
                .await?;
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Completed, None)
                    .await?;
                self.move_issue(run.issue_id, StageType::Done).await;
            }
            Transition::Rerun { stage, feedback } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    closing,
                    summary.as_deref(),
                    Some(&feedback),
                )
                .await?;
                self.enter_stage(run, template, stage, Some(feedback))
                    .await?;
            }
            Transition::FailRun { reason } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Failed,
                    summary.as_deref(),
                    Some(&reason),
                )
                .await?;
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
            }
        }
        Ok(())
    }

    /// 建阶段的新一次尝试并移动需求列；运行未暂停时立即启动，暂停中则留 pending。
    async fn enter_stage(
        &self,
        run: &PipelineRun,
        template: &PipelineTemplate,
        key: PipelineStageKey,
        feedback: Option<String>,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let Some(stage) = template.stage(key) else {
            PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
            return Ok(());
        };
        // 必须传重新读出的运行状态（C12：暂停中的关卡决策、退出处理期间用户刚暂停），
        // 不能用参数 `run` 这个旧快照。调用方保证运行未结束（entered_stage_statuses 的前提）。
        let current = self.load_run(run.id).await?.status;
        let (stage_status, run_status) = transition::entered_stage_statuses(current);
        let stage_run = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: key,
                gate_kind: stage.gate.kind(),
                status: stage_status,
                feedback,
            },
        )
        .await?;
        let run = PipelineRuns::update_status(pool, run.id, run_status, Some(key)).await?;
        self.move_issue(run.issue_id, transition::stage_column(key))
            .await;
        if stage_status == PipelineStageStatus::Running {
            self.launch(&run, &stage_run, template).await;
        }
        Ok(())
    }

    /// 启动一次尝试；失败时把尝试与运行记为 failed（不向上抛，调用方照常返回视图）。
    async fn launch(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
    ) {
        if let Err(e) = self.try_launch(run, stage_run, template).await {
            if is_state_changed(&e) {
                // 尝试已不在 pending / running（被别的路径改掉了），不能再记失败覆盖它。
                tracing::info!(
                    run_id = %run.id,
                    stage = stage_run.stage_key.as_str(),
                    "阶段状态已被其他操作改变，放弃本次启动记录"
                );
                return;
            }
            tracing::warn!(
                run_id = %run.id,
                stage = stage_run.stage_key.as_str(),
                "流水线阶段启动失败: {e}"
            );
            if let Err(db_error) = self.fail_run(run, stage_run, &e.to_string()).await {
                tracing::error!("记录阶段启动失败时出错: {db_error}");
            }
        }
    }

    async fn try_launch(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let workspace_id = run
            .workspace_id
            .ok_or_else(|| PipelineError::Conflict("流水线没有关联工作区".to_string()))?;
        let stage = template.stage(stage_run.stage_key).ok_or_else(|| {
            PipelineError::Conflict(format!("模板里没有阶段 {}", stage_run.stage_key.as_str()))
        })?;
        let issue = self.load_issue(run.issue_id).await?;
        let internals = PipelineRuns::internals(pool, run.id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("流水线 {}", run.id)))?;
        let executor_config: ExecutorConfig = serde_json::from_str(&internals.executor_config_json)
            .map_err(|e| PipelineError::Conflict(format!("执行器配置损坏：{e}")))?;

        let root = self.inner.launcher.prepare_workspace(workspace_id).await?;
        let dir = artifacts::artifacts_dir(&root, &issue.simple_id);
        tokio::fs::create_dir_all(&dir).await?;
        // 本阶段声明的旧产出物改名为 *.prev：本轮判定只认本轮新写的文件。
        artifacts::retire_declared(&dir, &stage.artifacts)?;
        let existing = artifacts::list_existing(&dir);
        let feedback = PipelineStageRuns::feedback(pool, stage_run.id).await?;
        let prompt = prompt::build_stage_prompt(&StagePromptInput {
            skill: &stage.skill,
            simple_id: &issue.simple_id,
            title: &issue.title,
            artifacts_dir: &dir,
            required: &stage.artifacts,
            existing: &existing,
            feedback: feedback.as_deref(),
        });
        let run_setup = !PipelineStageRuns::any_launched(pool, run.id).await?;
        let step = self
            .inner
            .launcher
            .start_agent_session(workspace_id, &executor_config, prompt, run_setup)
            .await?;
        PipelineStageRuns::mark_started(
            pool,
            stage_run.id,
            step.session_id,
            step.execution_process_id,
        )
        .await?;
        Ok(())
    }

    /// 进程失败原因。检查脚本附日志末尾（读不到日志就只写状态与退出码）。
    async fn describe_process_failure(&self, process: &FinishedProcess) -> String {
        let summary = describe_exit(process);
        if process.run_reason != ExecutionProcessRunReason::PipelineStep {
            return summary;
        }
        let tail = crate::services::execution_process::load_raw_log_messages(
            self.pool(),
            process.execution_process_id,
        )
        .await
        .and_then(|messages| log_tail(&messages, CHECK_LOG_TAIL_LINES));
        match tail {
            Some(tail) => {
                format!("{summary}\n检查脚本输出末尾（最多 {CHECK_LOG_TAIL_LINES} 行）：\n{tail}")
            }
            None => summary,
        }
    }

    async fn fail_run(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        reason: &str,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        PipelineStageRuns::set_status(
            pool,
            stage_run.id,
            PipelineStageStatus::Failed,
            None,
            Some(reason),
        )
        .await?;
        PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // 内部：读取
    // ------------------------------------------------------------------

    /// 运行启动时存下的模板快照；损坏或阶段为空时退回内置模板。
    async fn template_for(&self, run: &PipelineRun) -> Result<PipelineTemplate, PipelineError> {
        let internals = PipelineRuns::internals(self.pool(), run.id).await?;
        Ok(internals
            .and_then(|internals| {
                serde_json::from_str::<PipelineTemplate>(&internals.template_json).ok()
            })
            .filter(|template| !template.stages.is_empty())
            .unwrap_or_else(template::builtin_template))
    }

    async fn load_run(&self, run_id: Uuid) -> Result<PipelineRun, PipelineError> {
        PipelineRuns::find_by_id(self.pool(), run_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("流水线 {run_id}")))
    }

    async fn load_issue(&self, issue_id: Uuid) -> Result<Issue, PipelineError> {
        Issues::find_by_id(self.pool(), issue_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("需求 {issue_id}")))
    }

    async fn workspace_root(&self, run: &PipelineRun) -> Result<Option<PathBuf>, PipelineError> {
        let Some(workspace_id) = run.workspace_id else {
            return Ok(None);
        };
        Ok(Workspace::find_by_id(self.pool(), workspace_id)
            .await?
            .and_then(|workspace| workspace.container_ref)
            .map(PathBuf::from))
    }

    async fn move_issue(&self, issue_id: Uuid, column: StageType) {
        if let Err(e) = Issues::move_to_stage(self.pool(), issue_id, column).await {
            tracing::warn!(
                "流水线移动需求 {issue_id} 到 {} 列失败: {e}",
                column.as_str()
            );
        }
    }
}

const STATE_CHANGED: &str = "流水线状态已被其他操作改变，请刷新后重试";

/// 模型层护栏（`mark_started` / `set_status`）拒绝写入时返回 `RowNotFound`：
/// 状态已被别的路径改变（例如取消与晚到的进程回调竞争）。
fn is_state_changed(error: &PipelineError) -> bool {
    matches!(error, PipelineError::Database(sqlx::Error::RowNotFound))
}

/// HTTP 调用遇到护栏拒绝时报 409 而不是 500。
fn state_changed_as_conflict(error: PipelineError) -> PipelineError {
    if is_state_changed(&error) {
        tracing::info!("流水线状态已被其他操作改变: {error}");
        PipelineError::Conflict(STATE_CHANGED.to_string())
    } else {
        error
    }
}

fn run_reason_label(run_reason: &ExecutionProcessRunReason) -> &'static str {
    match run_reason {
        ExecutionProcessRunReason::SetupScript => "setup 脚本",
        ExecutionProcessRunReason::CleanupScript => "cleanup 脚本",
        ExecutionProcessRunReason::ArchiveScript => "归档脚本",
        ExecutionProcessRunReason::CodingAgent => "编码智能体",
        ExecutionProcessRunReason::DevServer => "开发服务器",
        ExecutionProcessRunReason::PipelineStep => "检查脚本",
    }
}

/// 检查脚本失败时回喂的日志行数上限。
const CHECK_LOG_TAIL_LINES: usize = 50;

const SETUP_BROKEN: &str = "setup 脚本后智能体未启动";

fn status_label(status: &ExecutionProcessStatus) -> &'static str {
    match status {
        ExecutionProcessStatus::Running => "仍在运行",
        ExecutionProcessStatus::Completed => "已结束",
        ExecutionProcessStatus::Failed => "失败",
        ExecutionProcessStatus::Killed => "被终止",
    }
}

fn describe_exit(process: &FinishedProcess) -> String {
    format!(
        "{} 进程未成功结束（状态：{}，退出码 {}）",
        run_reason_label(&process.run_reason),
        status_label(&process.status),
        process
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "无".to_string())
    )
}

/// 日志末尾最多 `max_lines` 行（stdout 与 stderr 按到达顺序拼接）。没有输出时 None。
fn log_tail(messages: &[LogMsg], max_lines: usize) -> Option<String> {
    let text: String = messages
        .iter()
        .filter_map(|message| match message {
            LogMsg::Stdout(chunk) | LogMsg::Stderr(chunk) => Some(chunk.as_str()),
            _ => None,
        })
        .collect();
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(max_lines)..];
    let joined = tail.join("\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn summarize(verdict: &StageVerdict) -> String {
    match verdict {
        StageVerdict::WaitHuman => "产出齐全，等待人工确认".to_string(),
        StageVerdict::Pass => "自动判定通过".to_string(),
        StageVerdict::Fail { reason } => reason.clone(),
        StageVerdict::FailBackTo { stage, reason } => {
            format!("{reason}（退回「{}」）", stage.display_name())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日志末尾只取最后若干行且跳过非文本消息() {
        let messages = vec![
            LogMsg::Stdout("a\nb\n".to_string()),
            LogMsg::Ready,
            LogMsg::Stderr("c\nd".to_string()),
            LogMsg::Finished,
        ];
        assert_eq!(log_tail(&messages, 2).as_deref(), Some("c\nd"));
        assert_eq!(log_tail(&messages, 50).as_deref(), Some("a\nb\nc\nd"));
        assert_eq!(log_tail(&[LogMsg::Ready], 50), None);
    }

    #[test]
    fn 失败原因用中文状态() {
        let process = FinishedProcess {
            execution_process_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_reason: ExecutionProcessRunReason::PipelineStep,
            status: ExecutionProcessStatus::Failed,
            exit_code: Some(2),
            has_next_action: false,
            chain_continues: false,
        };
        assert_eq!(
            describe_exit(&process),
            "检查脚本 进程未成功结束（状态：失败，退出码 2）"
        );
    }

    #[test]
    fn 回流摘要用阶段中文名() {
        let text = summarize(&StageVerdict::FailBackTo {
            stage: PipelineStageKey::Develop,
            reason: "测试失败 1 条".to_string(),
        });
        assert!(
            text.contains(PipelineStageKey::Develop.display_name()),
            "{text}"
        );
        assert!(!text.contains("develop"), "{text}");
    }
}
