use api_types::project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use db::models::{local_project::LocalProjects, local_project_status::ProjectStatuses};
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, MAX_BULK_UPDATES, ProjectScopedQuery, TxidResponse, snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let statuses = ProjectStatuses::find_by_project(pool, project_id).await?;
    Ok(snapshot("project_statuses", statuses))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateProjectStatusRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, payload.project_id)
        .await?
        .is_none()
    {
        return Err(ApiError::BadRequest("项目不存在".to_string()));
    }
    ProjectStatuses::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateProjectStatusRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if ProjectStatuses::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    ProjectStatuses::update(pool, id, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_bulk_update(
    pool: &SqlitePool,
    payload: BulkUpdateRequest<UpdateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.updates.len() > MAX_BULK_UPDATES {
        return Err(ApiError::BadRequest(format!(
            "单次最多更新 {MAX_BULK_UPDATES} 条状态列"
        )));
    }
    let updates: Vec<_> = payload
        .updates
        .into_iter()
        .map(|item| (item.id, item.changes))
        .collect();
    ProjectStatuses::bulk_update(pool, &updates).await?;
    Ok(txid())
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let in_use: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status_id = ?1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if in_use.0 > 0 {
        return Err(ApiError::Conflict(
            "该状态列下仍有需求，请先移走再删除".to_string(),
        ));
    }

    let affected = ProjectStatuses::delete(pool, id).await?;
    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

async fn list(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list(&deployment.db().pool, query.project_id).await
}

async fn create(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn bulk_update(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<BulkUpdateRequest<UpdateProjectStatusRequest>>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_bulk_update(&deployment.db().pool, payload).await
}

async fn delete(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/project_statuses",
        Router::new()
            .route("/", get(list).post(create))
            .route("/bulk", post(bulk_update))
            .route("/{id}", patch(update).delete(delete)),
    )
}

#[cfg(test)]
mod tests {
    use api_types::{
        project::CreateProjectRequest,
        project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest},
    };
    use db::{
        models::local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{handle_bulk_update, handle_create, handle_delete, handle_list};
    use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest};

    async fn 建项目(test_db: &TestDb) -> Uuid {
        LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn 列表用_project_statuses_作为_key_并只返回本项目() {
        let test_db = TestDb::new().await;
        let a = 建项目(&test_db).await;
        let b = 建项目(&test_db).await;

        let body = handle_list(test_db.pool(), a).await.unwrap().0;
        let rows = body["project_statuses"].as_array().unwrap();

        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r["project_id"] == a.to_string()));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn 快照包含_stage_type_字段() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db).await;
        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(body["project_statuses"][2]["stage_type"], "dev");
    }

    #[tokio::test]
    async fn 新建状态列必须属于已存在的项目() {
        let test_db = TestDb::new().await;
        let result = handle_create(
            test_db.pool(),
            CreateProjectStatusRequest {
                id: None,
                project_id: Uuid::from_u128(9090),
                name: "野状态".to_string(),
                color: "#000000".to_string(),
                sort_order: 0,
                hidden: false,
            },
        )
        .await;
        assert!(result.is_err(), "项目不存在时必须拒绝");
    }

    #[tokio::test]
    async fn 批量更新超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let updates = (0..super::super::MAX_BULK_UPDATES + 1)
            .map(|_| BulkUpdateItem {
                id: Uuid::new_v4(),
                changes: UpdateProjectStatusRequest::default(),
            })
            .collect();

        let result = handle_bulk_update(test_db.pool(), BulkUpdateRequest { updates }).await;
        assert!(result.is_err(), "超过上限必须返回错误而不是进事务");
    }

    #[tokio::test]
    async fn 删除仍有需求引用的状态列被拒绝() {
        use api_types::issue::CreateIssueRequest;
        use db::models::{
            issue::Issues,
            local_project_status::{ProjectStatuses, StageType},
        };

        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db).await;
        let todo = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();

        Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id,
                status_id: todo.id,
                title: "占用".to_string(),
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
        )
        .await
        .unwrap();

        let result = handle_delete(test_db.pool(), todo.id).await;
        assert!(result.is_err(), "有需求引用的状态列不得删除");
    }
}
