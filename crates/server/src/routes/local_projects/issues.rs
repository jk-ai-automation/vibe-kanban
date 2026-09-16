use api_types::issue::{
    CreateIssueRequest, ListIssuesResponse, SearchIssuesRequest, UpdateIssueRequest,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use db::models::issue::Issues;
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, MAX_BULK_UPDATES, ProjectScopedQuery, TxidResponse, map_issue_error,
    snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let issues = Issues::find_by_project(pool, project_id).await?;
    Ok(snapshot("issues", issues))
}

pub(crate) async fn handle_get(pool: &SqlitePool, id: Uuid) -> Result<Json<Value>, ApiError> {
    let issue = Issues::find_by_id(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(serde_json::to_value(issue).unwrap_or(Value::Null)))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateIssueRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    Issues::create(pool, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateIssueRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    Issues::update(pool, id, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_bulk_update(
    pool: &SqlitePool,
    payload: BulkUpdateRequest<UpdateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.updates.len() > MAX_BULK_UPDATES {
        return Err(ApiError::BadRequest(format!(
            "单次最多更新 {MAX_BULK_UPDATES} 条需求"
        )));
    }
    let updates: Vec<_> = payload
        .updates
        .into_iter()
        .map(|item| (item.id, item.changes))
        .collect();
    Issues::bulk_update(pool, &updates)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_search(
    pool: &SqlitePool,
    payload: SearchIssuesRequest,
) -> Result<Json<ListIssuesResponse>, ApiError> {
    Ok(Json(Issues::search(pool, &payload).await?))
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let affected = Issues::delete(pool, id).await?;
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

async fn get_one(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    handle_get(&deployment.db().pool, id).await
}

async fn create(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn bulk_update(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<BulkUpdateRequest<UpdateIssueRequest>>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_bulk_update(&deployment.db().pool, payload).await
}

async fn search(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<SearchIssuesRequest>,
) -> Result<Json<ListIssuesResponse>, ApiError> {
    handle_search(&deployment.db().pool, payload).await
}

async fn delete(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/issues",
        Router::new()
            .route("/", get(list).post(create))
            .route("/bulk", post(bulk_update))
            .route("/search", post(search))
            .route("/{id}", get(get_one).patch(update).delete(delete)),
    )
}

#[cfg(test)]
mod tests {
    use api_types::{
        issue::{CreateIssueRequest, SearchIssuesRequest, UpdateIssueRequest},
        project::CreateProjectRequest,
    };
    use db::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{handle_bulk_update, handle_create, handle_delete, handle_list, handle_search};
    use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest};

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        (project.id, todo.id)
    }

    fn 建需求请求(project_id: Uuid, status_id: Uuid, title: &str) -> CreateIssueRequest {
        CreateIssueRequest {
            id: None,
            project_id,
            status_id,
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
        }
    }

    #[tokio::test]
    async fn 列表用_issues_作为_key() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "第一条"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["simple_id"], "VK-1");
        assert_eq!(rows[0]["title"], "第一条");
    }

    #[tokio::test]
    async fn 快照不包含任何本地文件路径字段() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "第一条"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let text = body.to_string();
        for leak in [
            "container_ref",
            "worktree",
            "/Users/",
            "/home/",
            "asset_dir",
        ] {
            assert!(!text.contains(leak), "需求快照不得泄露 {leak}");
        }
    }

    #[tokio::test]
    async fn 跨项目状态列被拒绝() {
        let test_db = TestDb::new().await;
        let (project_a, _) = 准备(&test_db).await;
        let (_, status_b) = 准备(&test_db).await;

        let result = handle_create(test_db.pool(), 建需求请求(project_a, status_b, "越权")).await;
        assert!(result.is_err(), "状态列不属于该项目时必须拒绝");
    }

    #[tokio::test]
    async fn 批量更新排序成功后按新顺序返回() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "A"))
            .await
            .unwrap();
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "B"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap().clone();
        let a_id: Uuid = rows.iter().find(|r| r["title"] == "A").unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        handle_bulk_update(
            test_db.pool(),
            BulkUpdateRequest {
                updates: vec![BulkUpdateItem {
                    id: a_id,
                    changes: UpdateIssueRequest {
                        sort_order: Some(99.0),
                        ..Default::default()
                    },
                }],
            },
        )
        .await
        .unwrap();

        let after = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(after["issues"][1]["title"], "A", "sort_order 大的排后面");
    }

    #[tokio::test]
    async fn 批量更新超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let updates = (0..super::super::MAX_BULK_UPDATES + 1)
            .map(|_| BulkUpdateItem {
                id: Uuid::new_v4(),
                changes: UpdateIssueRequest::default(),
            })
            .collect();

        let result = handle_bulk_update(test_db.pool(), BulkUpdateRequest { updates }).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn 搜索接口返回列表与总数() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        for index in 0..3 {
            handle_create(
                test_db.pool(),
                建需求请求(project_id, status_id, &format!("需求 {index}")),
            )
            .await
            .unwrap();
        }

        let body = handle_search(
            test_db.pool(),
            SearchIssuesRequest {
                project_id,
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;

        assert_eq!(body.issues.len(), 2);
        assert_eq!(body.total_count, 3);
    }

    #[tokio::test]
    async fn 删除需求只影响目标行() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "留"))
            .await
            .unwrap();
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "删"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let victim: Uuid = body["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["title"] == "删")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        handle_delete(test_db.pool(), victim).await.unwrap();

        let after = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(after["issues"].as_array().unwrap().len(), 1);
        assert_eq!(after["issues"][0]["title"], "留");
    }

    #[tokio::test]
    async fn 一千条需求的快照能在合理时间内返回() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;

        for index in 0..1000 {
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title, \
                 sort_order, extension_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            )
            .bind(Uuid::new_v4())
            .bind(project_id)
            .bind(index + 1)
            .bind(format!("VK-{}", index + 1))
            .bind(status_id)
            .bind(format!("批量需求 {index}"))
            .bind(index as f64)
            .execute(test_db.pool())
            .await
            .unwrap();
        }

        let start = std::time::Instant::now();
        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let elapsed = start.elapsed();

        assert_eq!(body["issues"].as_array().unwrap().len(), 1000);
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "1000 条快照耗时 {elapsed:?}，超过 2 秒上限"
        );
    }
}
