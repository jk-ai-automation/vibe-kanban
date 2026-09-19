//! 交付流水线接口（契约 §2、§3），挂在 /api/local 下。
//!
//! `/issues/{id}/pipeline` 的两个 handler 在这里，但登记在 `issues.rs` 的路由表里
//! （与 `/issues/{id}` 共用一个 nest，见计划 A 任务 18 说明）。
//! §2 的接口统一包 `ApiResponse`；§3 的两个快照接口按本地集合约定返回 `{ "<表名>": [...] }`。

use api_types::issue::Issue;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use db::models::{
    issue::Issues,
    pipeline::{
        GateDecisionRequest, IssueArtifact, IssueArtifacts, IssuePipelineView, PendingPipelineItem,
        PipelineRuns, PipelineStageRuns, StartPipelineRequest,
    },
    repo::Repo,
    requests::WorkspaceRepoInput,
    workspace::Workspace,
};
use deployment::Deployment;
use serde::Deserialize;
use serde_json::Value;
use services::services::pipeline::{PipelineError, PipelineService, StartPipelineInput};
use sqlx::SqlitePool;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{LocalRoutes, ProjectScopedQuery, snapshot};
use crate::{
    DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser,
    routes::workspaces::create::create_workspace_with_repos,
};

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    pub project_id: Option<Uuid>,
}

pub(crate) fn map_pipeline_error(error: PipelineError) -> ApiError {
    match error {
        PipelineError::NotFound(_) => ApiError::NotFound,
        PipelineError::Conflict(message) => ApiError::Conflict(message),
        PipelineError::BadRequest(message) => ApiError::BadRequest(message),
        PipelineError::Database(error) => ApiError::Database(error),
        PipelineError::Io(error) => ApiError::Io(error),
        PipelineError::Launch(message) => ApiError::Conflict(message),
    }
}

// ---------------------------------------------------------------------------
// 可直接单测的 handler 主体
// ---------------------------------------------------------------------------

/// 启动前校验：404 需求不存在；409 已有未结束的运行；400 没选仓库。
pub(crate) async fn validate_start(
    pool: &SqlitePool,
    issue_id: Uuid,
    payload: &StartPipelineRequest,
) -> Result<Issue, ApiError> {
    let issue = Issues::find_by_id(pool, issue_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if PipelineRuns::has_active_for_issue(pool, issue_id).await? {
        return Err(ApiError::Conflict("该需求已有未结束的流水线".to_string()));
    }
    if payload.repos.is_empty() {
        return Err(ApiError::BadRequest("至少选择一个仓库".to_string()));
    }
    Ok(issue)
}

pub(crate) async fn handle_get_issue_pipeline(
    pool: &SqlitePool,
    pipeline: &PipelineService,
    issue_id: Uuid,
) -> Result<Option<IssuePipelineView>, ApiError> {
    if Issues::find_by_id(pool, issue_id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    pipeline
        .view_for_issue(issue_id)
        .await
        .map_err(map_pipeline_error)
}

pub(crate) async fn handle_get_artifact(
    pool: &SqlitePool,
    artifact_id: Uuid,
) -> Result<IssueArtifact, ApiError> {
    IssueArtifacts::find_by_id(pool, artifact_id)
        .await?
        .ok_or(ApiError::NotFound)
}

pub(crate) async fn handle_gate(
    pipeline: &PipelineService,
    stage_run_id: Uuid,
    payload: GateDecisionRequest,
    decided_by: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline
        .decide_gate(stage_run_id, payload, Some(decided_by))
        .await
        .map_err(map_pipeline_error)
}

pub(crate) async fn handle_pause(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.pause(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_resume(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.resume(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_cancel(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.cancel(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_pending(
    pool: &SqlitePool,
    project_id: Option<Uuid>,
) -> Result<Vec<PendingPipelineItem>, ApiError> {
    Ok(PipelineRuns::list_pending(pool, project_id).await?)
}

pub(crate) async fn handle_list_runs(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "pipeline_runs",
        PipelineRuns::list_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_list_stage_runs(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "pipeline_stage_runs",
        PipelineStageRuns::list_by_project(pool, project_id).await?,
    ))
}

// ---------------------------------------------------------------------------
// axum handler
// ---------------------------------------------------------------------------

pub(super) async fn get_issue_pipeline(
    State(deployment): State<DeploymentImpl>,
    Path(issue_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Option<IssuePipelineView>>>, ApiError> {
    let view =
        handle_get_issue_pipeline(&deployment.db().pool, deployment.pipeline(), issue_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

pub(super) async fn start_pipeline(
    State(deployment): State<DeploymentImpl>,
    current_user: CurrentUser,
    Path(issue_id): Path<Uuid>,
    Json(payload): Json<StartPipelineRequest>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let pool = &deployment.db().pool;
    let issue = validate_start(pool, issue_id, &payload).await?;
    let repos: Vec<WorkspaceRepoInput> = payload
        .repos
        .iter()
        .map(|repo| WorkspaceRepoInput {
            repo_id: repo.repo_id,
            target_branch: repo.target_branch.clone(),
        })
        .collect();
    let first_repo = Repo::find_by_id(pool, repos[0].repo_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let managed = create_workspace_with_repos(
        &deployment,
        Some(format!("{} {}", issue.simple_id, issue.title)),
        &repos,
        current_user.id,
    )
    .await?;
    Workspace::set_issue_id(pool, managed.workspace.id, Some(issue.id)).await?;

    let view = deployment
        .pipeline()
        .start(StartPipelineInput {
            issue,
            workspace_id: managed.workspace.id,
            repo_root: first_repo.path.clone(),
            executor_config: payload.executor_config,
            template_key: payload.template_key,
        })
        .await
        .map_err(map_pipeline_error)?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn get_artifact(
    State(deployment): State<DeploymentImpl>,
    Path(artifact_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssueArtifact>>, ApiError> {
    let artifact = handle_get_artifact(&deployment.db().pool, artifact_id).await?;
    Ok(ResponseJson(ApiResponse::success(artifact)))
}

async fn decide_gate(
    State(deployment): State<DeploymentImpl>,
    current_user: CurrentUser,
    Path(stage_run_id): Path<Uuid>,
    Json(payload): Json<GateDecisionRequest>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_gate(
        deployment.pipeline(),
        stage_run_id,
        payload,
        current_user.id,
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn pause(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_pause(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn resume(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_resume(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn cancel(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_cancel(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn pending(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<PendingQuery>,
) -> Result<ResponseJson<ApiResponse<Vec<PendingPipelineItem>>>, ApiError> {
    let items = handle_pending(&deployment.db().pool, query.project_id).await?;
    Ok(ResponseJson(ApiResponse::success(items)))
}

async fn list_runs(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list_runs(&deployment.db().pool, query.project_id).await
}

async fn list_stage_runs(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list_stage_runs(&deployment.db().pool, query.project_id).await
}

pub fn router() -> Router<DeploymentImpl> {
    let pipeline = LocalRoutes::new("/pipeline")
        .route("/artifacts/{id}", &["GET"], get(get_artifact))
        .route("/stage-runs/{id}/gate", &["POST"], post(decide_gate))
        .route("/runs/{id}/pause", &["POST"], post(pause))
        .route("/runs/{id}/resume", &["POST"], post(resume))
        .route("/runs/{id}/cancel", &["POST"], post(cancel))
        .route("/pending", &["GET"], get(pending))
        .into_router();
    let runs = LocalRoutes::new("/pipeline_runs")
        .route("/", &["GET"], get(list_runs))
        .into_router();
    let stage_runs = LocalRoutes::new("/pipeline_stage_runs")
        .route("/", &["GET"], get(list_stage_runs))
        .into_router();
    Router::new().merge(pipeline).merge(runs).merge(stage_runs)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
                ArtifactKind, CreatePipelineRun, CreateStageRun, GateDecisionKind,
                GateDecisionRequest, GateKind, IssueArtifacts, PipelineRun, PipelineRunStatus,
                PipelineRuns, PipelineStageKey, PipelineStageRun, PipelineStageRuns,
                PipelineStageStatus, StartPipelineRepo, StartPipelineRequest,
            },
        },
        test_support::TestDb,
    };
    use executors::{executors::BaseCodingAgent, profile::ExecutorConfig};
    use services::services::pipeline::{NoopStageLauncher, PipelineService};
    use uuid::Uuid;

    use super::*;

    async fn 准备需求(test_db: &TestDb) -> Issue {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "流水线接口".to_string(),
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
                title: "接口测试".to_string(),
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

    /// 直接落一条运行 + 一条尝试（不经引擎，模板快照故意写 "{}"，引擎会退回内置模板）。
    async fn 准备运行(
        test_db: &TestDb,
        issue: &Issue,
        stage_status: PipelineStageStatus,
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
                first_stage: PipelineStageKey::Requirement,
            },
        )
        .await
        .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: PipelineStageKey::Requirement,
                gate_kind: GateKind::Human,
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await
        .unwrap();
        let stage = if stage_status == PipelineStageStatus::Running {
            stage
        } else {
            PipelineStageRuns::set_status(test_db.pool(), stage.id, stage_status, None, None)
                .await
                .unwrap()
        };
        (run, stage)
    }

    fn 引擎(test_db: &TestDb) -> PipelineService {
        PipelineService::new(test_db.db.clone(), Arc::new(NoopStageLauncher))
    }

    fn 启动请求(repos: Vec<StartPipelineRepo>) -> StartPipelineRequest {
        StartPipelineRequest {
            repos,
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            template_key: None,
        }
    }

    fn 一个仓库() -> Vec<StartPipelineRepo> {
        vec![StartPipelineRepo {
            repo_id: Uuid::new_v4(),
            target_branch: "main".to_string(),
        }]
    }

    #[tokio::test]
    async fn 启动前校验的三种错误() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;

        let missing = validate_start(test_db.pool(), Uuid::new_v4(), &启动请求(一个仓库())).await;
        assert!(matches!(missing, Err(ApiError::NotFound)));

        let no_repo = validate_start(test_db.pool(), issue.id, &启动请求(vec![])).await;
        assert!(matches!(no_repo, Err(ApiError::BadRequest(_))));

        assert_eq!(
            validate_start(test_db.pool(), issue.id, &启动请求(一个仓库()))
                .await
                .unwrap()
                .id,
            issue.id
        );

        准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let active = validate_start(test_db.pool(), issue.id, &启动请求(一个仓库())).await;
        assert!(matches!(active, Err(ApiError::Conflict(_))));
    }

    #[tokio::test]
    async fn 查询需求流水线() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;

        assert!(matches!(
            handle_get_issue_pipeline(test_db.pool(), &pipeline, Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
        assert!(
            handle_get_issue_pipeline(test_db.pool(), &pipeline, issue.id)
                .await
                .unwrap()
                .is_none()
        );

        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let view = handle_get_issue_pipeline(test_db.pool(), &pipeline, issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view.run.id, run.id);
        assert_eq!(view.stages.len(), 1);
        assert_eq!(view.template.stages.len(), 7, "模板快照损坏时退回内置模板");
    }

    #[tokio::test]
    async fn 关卡决策的错误码() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (_, running) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let approve = GateDecisionRequest {
            decision: GateDecisionKind::Approve,
            comment: None,
        };

        assert!(matches!(
            handle_gate(&pipeline, Uuid::new_v4(), approve.clone(), DEFAULT_USER_ID).await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            handle_gate(&pipeline, running.id, approve, DEFAULT_USER_ID).await,
            Err(ApiError::Conflict(_))
        ));

        PipelineStageRuns::set_status(
            test_db.pool(),
            running.id,
            PipelineStageStatus::WaitingGate,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            handle_gate(
                &pipeline,
                running.id,
                GateDecisionRequest {
                    decision: GateDecisionKind::Reject,
                    comment: None,
                },
                DEFAULT_USER_ID
            )
            .await,
            Err(ApiError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn 暂停中通过关卡只建待启动阶段() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (run, waiting) = 准备运行(&test_db, &issue, PipelineStageStatus::WaitingGate).await;
        PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Paused, None)
            .await
            .unwrap();

        let view = handle_gate(
            &pipeline,
            waiting.id,
            GateDecisionRequest {
                decision: GateDecisionKind::Approve,
                comment: None,
            },
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(view.run.status, PipelineRunStatus::Paused);
        assert_eq!(view.run.current_stage_key, PipelineStageKey::Spec);
        assert_eq!(
            view.stages
                .iter()
                .map(|s| (s.stage_key, s.status))
                .collect::<Vec<_>>(),
            vec![
                (PipelineStageKey::Requirement, PipelineStageStatus::Passed),
                (PipelineStageKey::Spec, PipelineStageStatus::Pending),
            ]
        );
        assert_eq!(view.decisions[0].decided_by, Some(DEFAULT_USER_ID));
    }

    #[tokio::test]
    async fn 暂停继续取消的状态约束() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;

        assert!(matches!(
            handle_resume(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert_eq!(
            handle_pause(&pipeline, run.id).await.unwrap().run.status,
            PipelineRunStatus::Paused
        );
        assert!(matches!(
            handle_pause(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert_eq!(
            handle_resume(&pipeline, run.id).await.unwrap().run.status,
            PipelineRunStatus::Running
        );
        let cancelled = handle_cancel(&pipeline, run.id).await.unwrap();
        assert_eq!(cancelled.run.status, PipelineRunStatus::Cancelled);
        assert_eq!(cancelled.stages[0].status, PipelineStageStatus::Skipped);
        assert!(matches!(
            handle_cancel(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            handle_pause(&pipeline, Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
    }

    #[tokio::test]
    async fn 待处理列表与两个快照接口() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;
        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::WaitingGate).await;
        PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();

        let all = handle_pending(test_db.pool(), None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].issue_title, "接口测试");
        assert!(
            handle_pending(test_db.pool(), Some(Uuid::new_v4()))
                .await
                .unwrap()
                .is_empty()
        );

        let runs = handle_list_runs(test_db.pool(), issue.project_id)
            .await
            .unwrap();
        assert_eq!(runs.0["pipeline_runs"].as_array().unwrap().len(), 1);
        let stages = handle_list_stage_runs(test_db.pool(), issue.project_id)
            .await
            .unwrap();
        assert_eq!(stages.0["pipeline_stage_runs"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn 取单个产出物() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;
        let (_, stage) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let summary = IssueArtifacts::insert_version(
            test_db.pool(),
            issue.id,
            stage.id,
            ArtifactKind::Requirement,
            ".vk/runs/X/requirement.md",
            "# 需求",
        )
        .await
        .unwrap();

        let artifact = handle_get_artifact(test_db.pool(), summary.id)
            .await
            .unwrap();
        assert_eq!(artifact.content, "# 需求");
        assert!(matches!(
            handle_get_artifact(test_db.pool(), Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
    }

    #[test]
    fn 流水线端点都已登记且路由不冲突() {
        // 构造 router() 同时验证 axum 路由表没有冲突（冲突会直接 panic）。
        let _ = crate::routes::local_projects::router();
        let registered = crate::routes::local_projects::registered_endpoints();
        for (path, method) in [
            ("/api/local/issues/{id}/pipeline", "GET"),
            ("/api/local/issues/{id}/pipeline", "POST"),
            ("/api/local/pipeline/artifacts/{id}", "GET"),
            ("/api/local/pipeline/stage-runs/{id}/gate", "POST"),
            ("/api/local/pipeline/runs/{id}/pause", "POST"),
            ("/api/local/pipeline/runs/{id}/resume", "POST"),
            ("/api/local/pipeline/runs/{id}/cancel", "POST"),
            ("/api/local/pipeline/pending", "GET"),
            ("/api/local/pipeline_runs", "GET"),
            ("/api/local/pipeline_stage_runs", "GET"),
        ] {
            assert!(
                registered
                    .get(path)
                    .is_some_and(|methods| methods.contains(method)),
                "{method} {path} 未登记，已登记：{registered:#?}"
            );
        }
    }
}
