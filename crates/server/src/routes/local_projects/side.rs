use api_types::{
    issue_comment::{CreateIssueCommentRequest, UpdateIssueCommentRequest},
    issue_tag::CreateIssueTagRequest,
    tag::{CreateTagRequest, UpdateTagRequest},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch},
};
use db::models::{
    issue::Issues,
    issue_side::{IssueComments, IssueTags, ProjectTags},
    local_project::LocalProjects,
};
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{IssueScopedQuery, ProjectScopedQuery, TxidResponse, map_issue_error, snapshot, txid};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "tags",
        ProjectTags::find_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_tag_create(
    pool: &SqlitePool,
    payload: CreateTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, payload.project_id)
        .await?
        .is_none()
    {
        return Err(ApiError::BadRequest("项目不存在".to_string()));
    }
    ProjectTags::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_tag_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    ProjectTags::update(pool, id, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if ProjectTags::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "issue_tags",
        IssueTags::find_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_issue_tag_create(
    pool: &SqlitePool,
    payload: CreateIssueTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, payload.issue_id).await?.is_none() {
        return Err(ApiError::BadRequest("需求不存在".to_string()));
    }
    IssueTags::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if IssueTags::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_comment_list(
    pool: &SqlitePool,
    issue_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "issue_comments",
        IssueComments::find_by_issue(pool, issue_id).await?,
    ))
}

pub(crate) async fn handle_comment_create(
    pool: &SqlitePool,
    payload: CreateIssueCommentRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, payload.issue_id).await?.is_none() {
        return Err(ApiError::BadRequest("需求不存在".to_string()));
    }
    IssueComments::create(pool, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_comment_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateIssueCommentRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    IssueComments::update(pool, id, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_comment_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if IssueComments::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub fn router() -> Router<DeploymentImpl> {
    let tags = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_tag_list(&d.db().pool, q.project_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>, Json(p): Json<CreateTagRequest>| async move {
                    handle_tag_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            patch(
                |State(d): State<DeploymentImpl>,
                 Path(id): Path<Uuid>,
                 Json(p): Json<UpdateTagRequest>| async move {
                    handle_tag_update(&d.db().pool, id, p).await
                },
            )
            .delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_tag_delete(&d.db().pool, id).await
                },
            ),
        );

    let issue_tags = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_issue_tag_list(&d.db().pool, q.project_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>, Json(p): Json<CreateIssueTagRequest>| async move {
                    handle_issue_tag_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            axum::routing::delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_issue_tag_delete(&d.db().pool, id).await
                },
            ),
        );

    let comments = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<IssueScopedQuery>| async move {
                    handle_comment_list(&d.db().pool, q.issue_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>,
                 Json(p): Json<CreateIssueCommentRequest>| async move {
                    handle_comment_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            patch(
                |State(d): State<DeploymentImpl>,
                 Path(id): Path<Uuid>,
                 Json(p): Json<UpdateIssueCommentRequest>| async move {
                    handle_comment_update(&d.db().pool, id, p).await
                },
            )
            .delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_comment_delete(&d.db().pool, id).await
                },
            ),
        );

    Router::new()
        .nest("/tags", tags)
        .nest("/issue_tags", issue_tags)
        .nest("/issue_comments", comments)
}

#[cfg(test)]
mod tests {
    use api_types::{
        issue::CreateIssueRequest, issue_comment::CreateIssueCommentRequest,
        issue_tag::CreateIssueTagRequest, project::CreateProjectRequest, tag::CreateTagRequest,
    };
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{
        handle_comment_create, handle_comment_list, handle_issue_tag_create, handle_issue_tag_list,
        handle_tag_create, handle_tag_list,
    };

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
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
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
        (project.id, issue.id)
    }

    #[tokio::test]
    async fn 标签列表的_key_是_tags_而不是数据库表名() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let body = handle_tag_list(test_db.pool(), project_id).await.unwrap().0;
        assert!(body.get("tags").is_some(), "响应 key 必须是 tags");
        assert!(body.get("project_tags").is_none());
        assert_eq!(body["tags"][0]["name"], "前端");
    }

    #[tokio::test]
    async fn 需求标签列表的_key_是_issue_tags() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag_body = {
            handle_tag_create(
                test_db.pool(),
                CreateTagRequest {
                    id: None,
                    project_id,
                    name: "前端".to_string(),
                    color: "#22c55e".to_string(),
                },
            )
            .await
            .unwrap();
            handle_tag_list(test_db.pool(), project_id).await.unwrap().0
        };
        let tag_id: Uuid = tag_body["tags"][0]["id"].as_str().unwrap().parse().unwrap();

        handle_issue_tag_create(
            test_db.pool(),
            CreateIssueTagRequest {
                id: None,
                issue_id,
                tag_id,
            },
        )
        .await
        .unwrap();

        let body = handle_issue_tag_list(test_db.pool(), project_id)
            .await
            .unwrap()
            .0;
        assert_eq!(body["issue_tags"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn 给不存在的需求加标签被拒绝() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        let body = handle_tag_list(test_db.pool(), project_id).await.unwrap().0;
        let tag_id: Uuid = body["tags"][0]["id"].as_str().unwrap().parse().unwrap();

        let result = handle_issue_tag_create(
            test_db.pool(),
            CreateIssueTagRequest {
                id: None,
                issue_id: Uuid::from_u128(31337),
                tag_id,
            },
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn 评论列表按需求过滤() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;
        handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "第一条评论".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        let body = handle_comment_list(test_db.pool(), issue_id)
            .await
            .unwrap()
            .0;
        assert_eq!(body["issue_comments"].as_array().unwrap().len(), 1);

        let other = handle_comment_list(test_db.pool(), Uuid::from_u128(4242))
            .await
            .unwrap()
            .0;
        assert_eq!(other["issue_comments"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn 空评论被接口拒绝() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;
        let result = handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "   ".to_string(),
                parent_id: None,
            },
        )
        .await;
        assert!(result.is_err());
    }
}
