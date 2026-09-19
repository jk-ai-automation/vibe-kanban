//! 流水线测试夹具（只在测试里编译）。

use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use api_types::{
    issue::{CreateIssueRequest, Issue},
    project::CreateProjectRequest,
};
use async_trait::async_trait;
use db::{
    models::{
        issue::Issues,
        local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
        local_project_status::{ProjectStatuses, StageType},
        pipeline::{
            CreatePipelineRun, CreateStageRun, GateKind, PipelineRun, PipelineRuns,
            PipelineStageKey, PipelineStageRun, PipelineStageRuns, PipelineStageStatus,
        },
        workspace::{CreateWorkspace, Workspace},
    },
    test_support::TestDb,
};
use executors::{
    pipeline_prompt::{parse_pipeline_prompt, write_mock_artifacts},
    profile::ExecutorConfig,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    engine::PipelineError,
    launcher::{LaunchedStep, StageLauncher},
};

pub(crate) async fn 准备需求(test_db: &TestDb, title: &str) -> Issue {
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

pub(crate) async fn 准备工作区(test_db: &TestDb) -> Uuid {
    Workspace::create(
        test_db.pool(),
        &CreateWorkspace {
            branch: "vk/pipeline".to_string(),
            name: None,
        },
        Uuid::new_v4(),
        DEFAULT_USER_ID,
    )
    .await
    .unwrap()
    .id
}

/// 直接落一条运行与一条尝试（不经引擎）。
pub(crate) async fn 准备运行与阶段(
    test_db: &TestDb,
    issue: &Issue,
    stage_key: PipelineStageKey,
) -> (PipelineRun, PipelineStageRun) {
    let run = PipelineRuns::create(
        test_db.pool(),
        &CreatePipelineRun {
            issue_id: issue.id,
            project_id: issue.project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: stage_key,
        },
    )
    .await
    .unwrap();
    let stage = PipelineStageRuns::create(
        test_db.pool(),
        &CreateStageRun {
            run_id: run.id,
            project_id: run.project_id,
            stage_key,
            gate_kind: GateKind::Human,
            status: PipelineStageStatus::Running,
            feedback: None,
        },
    )
    .await
    .unwrap();
    (run, stage)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FakeLaunchKind {
    Agent,
    Checks,
}

#[derive(Debug, Clone)]
pub(crate) struct FakeLaunch {
    pub kind: FakeLaunchKind,
    pub session_id: Uuid,
    pub execution_process_id: Uuid,
    /// 智能体是提示词，检查是脚本。
    pub prompt: String,
    pub run_setup: bool,
}

/// 假启动器：不起进程，只记账；智能体启动时按提示词写 qa-mode 同款占位产出物。
pub(crate) struct FakeLauncher {
    pool: SqlitePool,
    root: PathBuf,
    launches: Mutex<Vec<FakeLaunch>>,
    pub(crate) stopped: Mutex<Vec<Uuid>>,
    /// 关掉后智能体「什么都不产出」，用来测缺产出物。
    pub(crate) write_artifacts: AtomicBool,
    /// 打开后所有启动都失败。
    pub(crate) fail_start: AtomicBool,
    /// 打开后智能体启动期间把 running 的尝试改成 skipped，模拟「启动途中状态被别的路径改掉」。
    pub(crate) skip_running_on_start: AtomicBool,
}

impl FakeLauncher {
    pub(crate) fn new(pool: SqlitePool, root: PathBuf) -> Self {
        Self {
            pool,
            root,
            launches: Mutex::new(Vec::new()),
            stopped: Mutex::new(Vec::new()),
            write_artifacts: AtomicBool::new(true),
            fail_start: AtomicBool::new(false),
            skip_running_on_start: AtomicBool::new(false),
        }
    }

    pub(crate) fn launches(&self) -> Vec<FakeLaunch> {
        self.launches.lock().unwrap().clone()
    }

    pub(crate) fn last(&self) -> FakeLaunch {
        self.launches().last().cloned().expect("还没有任何启动")
    }

    fn record(&self, launch: FakeLaunch) -> LaunchedStep {
        let step = LaunchedStep {
            session_id: launch.session_id,
            execution_process_id: launch.execution_process_id,
        };
        self.launches.lock().unwrap().push(launch);
        step
    }
}

#[async_trait]
impl StageLauncher for FakeLauncher {
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        if self.fail_start.load(Ordering::SeqCst) {
            return Err(PipelineError::Launch("模拟启动失败".to_string()));
        }
        std::fs::create_dir_all(&self.root)?;
        Workspace::update_container_ref(&self.pool, workspace_id, &self.root.to_string_lossy())
            .await?;
        Ok(self.root.clone())
    }

    async fn start_agent_session(
        &self,
        _workspace_id: Uuid,
        _executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
    ) -> Result<LaunchedStep, PipelineError> {
        if self.skip_running_on_start.load(Ordering::SeqCst) {
            sqlx::query(
                "UPDATE pipeline_stage_runs SET status = 'skipped' WHERE status = 'running'",
            )
            .execute(&self.pool)
            .await?;
        }
        if self.write_artifacts.load(Ordering::SeqCst)
            && let Some(parsed) = parse_pipeline_prompt(&prompt)
        {
            write_mock_artifacts(&parsed)?;
        }
        Ok(self.record(FakeLaunch {
            kind: FakeLaunchKind::Agent,
            session_id: Uuid::new_v4(),
            execution_process_id: Uuid::new_v4(),
            prompt,
            run_setup,
        }))
    }

    async fn start_checks(
        &self,
        _workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        Ok(self.record(FakeLaunch {
            kind: FakeLaunchKind::Checks,
            session_id,
            execution_process_id: Uuid::new_v4(),
            prompt: script,
            run_setup: false,
        }))
    }

    async fn stop_workspace(&self, workspace_id: Uuid) {
        self.stopped.lock().unwrap().push(workspace_id);
    }
}
