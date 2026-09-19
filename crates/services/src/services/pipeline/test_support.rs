//! 流水线测试夹具（只在测试里编译）。

use api_types::{
    issue::{CreateIssueRequest, Issue},
    project::CreateProjectRequest,
};
use db::{
    models::{
        issue::Issues,
        local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
        local_project_status::{ProjectStatuses, StageType},
        pipeline::{
            CreatePipelineRun, CreateStageRun, GateKind, PipelineRun, PipelineRuns,
            PipelineStageKey, PipelineStageRun, PipelineStageRuns, PipelineStageStatus,
        },
    },
    test_support::TestDb,
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
