use api_types::{
    issue_comment::{CreateIssueCommentRequest, UpdateIssueCommentRequest},
    issue_tag::CreateIssueTagRequest,
    tag::{CreateTagRequest, UpdateTagRequest},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use db::models::{
    issue::Issues,
    issue_side::{IssueComments, IssueTags, MAX_SIDE_ROWS, ProjectTags},
    local_project::LocalProjects,
};
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    IssueScopedQuery, LocalRoutes, ProjectScopedQuery, TxidResponse, map_db_error, map_issue_error,
    snapshot_truncatable, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let snapshot = ProjectTags::find_by_project(pool, project_id).await?;
    if snapshot.truncated {
        tracing::warn!(
            %project_id,
            limit = MAX_SIDE_ROWS,
            "标签快照已截断，前端看到的不是全量"
        );
    }
    Ok(snapshot_truncatable(
        "tags",
        snapshot.rows,
        snapshot.truncated,
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
    ProjectTags::create(pool, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_tag_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    ProjectTags::update(pool, id, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if ProjectTags::delete(pool, id).await.map_err(map_db_error)? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let snapshot = IssueTags::find_by_project(pool, project_id).await?;
    if snapshot.truncated {
        tracing::warn!(
            %project_id,
            limit = MAX_SIDE_ROWS,
            "需求标签关联快照已截断，前端看到的不是全量"
        );
    }
    Ok(snapshot_truncatable(
        "issue_tags",
        snapshot.rows,
        snapshot.truncated,
    ))
}

pub(crate) async fn handle_issue_tag_create(
    pool: &SqlitePool,
    payload: CreateIssueTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, payload.issue_id).await?.is_none() {
        return Err(ApiError::BadRequest("需求不存在".to_string()));
    }
    IssueTags::create(pool, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if IssueTags::delete(pool, id).await.map_err(map_db_error)? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_comment_list(
    pool: &SqlitePool,
    issue_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let snapshot = IssueComments::find_by_issue(pool, issue_id).await?;
    if snapshot.truncated {
        tracing::warn!(
            %issue_id,
            limit = MAX_SIDE_ROWS,
            "评论快照已截断，前端看到的不是全量"
        );
    }
    Ok(snapshot_truncatable(
        "issue_comments",
        snapshot.rows,
        snapshot.truncated,
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
    if IssueComments::delete(pool, id)
        .await
        .map_err(map_db_error)?
        == 0
    {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

/// 个人版不支持的多行写操作。
///
/// 前端的本地集合在一次事务里更新多行时会统一打 `<base>/bulk`
/// （localCollections.ts 的 onUpdate），标签、需求标签关联、评论目前没有任何
/// 需要多行更新的界面。这里仍然把路由挂上并返回一个说明清楚的 400，
/// 而不是留给 axum 回一个空响应体的 405：405 在前端只会变成
/// 「Failed to write tags」这种查不到原因的报错。
async fn 不支持的批量更新(资源: &'static str) -> ApiError {
    ApiError::BadRequest(format!("个人版不支持批量更新{资源}"))
}

pub fn router() -> Router<DeploymentImpl> {
    let tags = LocalRoutes::new("/tags")
        .route(
            "/",
            &["GET", "POST"],
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
            "/bulk",
            &["POST"],
            post(|| async { 不支持的批量更新("标签").await }),
        )
        .route(
            "/{id}",
            &["PATCH", "DELETE"],
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
        )
        .into_router();

    let issue_tags = LocalRoutes::new("/issue_tags")
        .route(
            "/",
            &["GET", "POST"],
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
            "/bulk",
            &["POST"],
            post(|| async { 不支持的批量更新("需求标签关联").await }),
        )
        .route(
            "/{id}",
            &["PATCH", "DELETE"],
            // 关联行只有 issue_id / tag_id 两列，改任何一列都等于换一条关联，
            // 因此只支持删除后重建；PATCH 明确回 400 而不是 405。
            patch(|| async {
                ApiError::BadRequest("需求标签关联不支持修改，请删除后重建".to_string())
            })
            .delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_issue_tag_delete(&d.db().pool, id).await
                },
            ),
        )
        .into_router();

    let comments = LocalRoutes::new("/issue_comments")
        .route(
            "/",
            &["GET", "POST"],
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
            "/bulk",
            &["POST"],
            post(|| async { 不支持的批量更新("评论").await }),
        )
        .route(
            "/{id}",
            &["PATCH", "DELETE"],
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
        )
        .into_router();

    Router::new().merge(tags).merge(issue_tags).merge(comments)
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
        handle_comment_create, handle_comment_list, handle_comment_update, handle_issue_tag_create,
        handle_issue_tag_list, handle_tag_create, handle_tag_list, handle_tag_update,
    };
    use crate::error::ApiError;

    fn 断言是_400(error: ApiError) {
        assert!(
            matches!(error, ApiError::BadRequest(_)),
            "应映射为 400，实际：{error:?}"
        );
    }

    fn 断言是_404(error: ApiError) {
        assert!(
            matches!(error, ApiError::NotFound),
            "应映射为 404，实际：{error:?}"
        );
    }

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid) {
        准备具名(test_db, "Vibe Kanban").await
    }

    async fn 准备具名(test_db: &TestDb, name: &str) -> (Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: name.to_string(),
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

    #[tokio::test]
    async fn 跨项目标签返回_400() {
        let test_db = TestDb::new().await;
        let (_, issue_a) = 准备具名(&test_db, "Alpha").await;
        let (project_b, _) = 准备具名(&test_db, "Beta").await;

        handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id: project_b,
                name: "别家".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        let tag_id: Uuid =
            handle_tag_list(test_db.pool(), project_b).await.unwrap().0["tags"][0]["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();

        let err = handle_issue_tag_create(
            test_db.pool(),
            CreateIssueTagRequest {
                id: None,
                issue_id: issue_a,
                tag_id,
            },
        )
        .await
        .expect_err("跨项目标签必须拒绝");
        断言是_400(err);

        assert!(
            handle_issue_tag_list(test_db.pool(), project_b)
                .await
                .unwrap()
                .0["issue_tags"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn 空名标签返回_400() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        let err = handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id,
                name: "   ".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .expect_err("空名标签必须拒绝");
        断言是_400(err);
    }

    #[tokio::test]
    async fn 更新不存在的标签返回_404_而不是_500() {
        use api_types::tag::UpdateTagRequest;

        let test_db = TestDb::new().await;
        let err = handle_tag_update(
            test_db.pool(),
            Uuid::from_u128(60001),
            UpdateTagRequest {
                name: Some("新名".to_string()),
                color: None,
            },
        )
        .await
        .expect_err("不存在的标签必须报错");
        断言是_404(err);
    }

    #[tokio::test]
    async fn 更新不存在的评论返回_404_而不是_500() {
        use api_types::issue_comment::UpdateIssueCommentRequest;

        let test_db = TestDb::new().await;
        let err = handle_comment_update(
            test_db.pool(),
            Uuid::from_u128(60002),
            UpdateIssueCommentRequest {
                message: Some("改后".to_string()),
                parent_id: None,
            },
        )
        .await
        .expect_err("不存在的评论必须报错");
        断言是_404(err);
    }

    #[tokio::test]
    async fn 三类关联快照都带出_truncated_字段() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;

        for body in [
            handle_tag_list(test_db.pool(), project_id).await.unwrap().0,
            handle_issue_tag_list(test_db.pool(), project_id)
                .await
                .unwrap()
                .0,
            handle_comment_list(test_db.pool(), issue_id)
                .await
                .unwrap()
                .0,
        ] {
            assert_eq!(
                body["truncated"],
                serde_json::json!(false),
                "关联快照必须透出截断标记：{body}"
            );
        }
    }

    #[tokio::test]
    async fn 重复挂同一个标签是幂等的() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
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
        let tag_id: Uuid = handle_tag_list(test_db.pool(), project_id).await.unwrap().0["tags"][0]
            ["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let request = CreateIssueTagRequest {
            id: None,
            issue_id,
            tag_id,
        };
        handle_issue_tag_create(test_db.pool(), request.clone())
            .await
            .unwrap();
        handle_issue_tag_create(test_db.pool(), request)
            .await
            .expect("重复挂同一标签必须幂等成功，不得 500");

        let body = handle_issue_tag_list(test_db.pool(), project_id)
            .await
            .unwrap()
            .0;
        assert_eq!(
            body["issue_tags"].as_array().unwrap().len(),
            1,
            "幂等不得产生重复行"
        );
    }

    #[tokio::test]
    async fn 跨需求的父评论返回_400() {
        let test_db = TestDb::new().await;
        let (project_id, issue_a) = 准备(&test_db).await;
        let todo = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue_b = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id,
                status_id: todo.id,
                title: "另一条".to_string(),
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

        handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id: issue_a,
                message: "甲".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        let parent_id: Uuid = handle_comment_list(test_db.pool(), issue_a)
            .await
            .unwrap()
            .0["issue_comments"][0]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let err = handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id: issue_b.id,
                message: "串台".to_string(),
                parent_id: Some(parent_id),
            },
        )
        .await
        .expect_err("跨需求的父评论必须拒绝");
        断言是_400(err);
    }
}
