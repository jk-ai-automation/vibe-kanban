use api_types::issue::{
    CreateIssueRequest, ListIssuesResponse, SearchIssuesRequest, UpdateIssueRequest,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use db::models::issue::Issues;
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, LocalRoutes, MAX_BULK_UPDATES, ProjectScopedQuery, TxidResponse,
    map_issue_error, snapshot_truncatable, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let snapshot = Issues::find_by_project(pool, project_id).await?;
    if snapshot.truncated {
        tracing::warn!(
            %project_id,
            limit = db::models::issue::MAX_SNAPSHOT_ROWS,
            "需求快照已截断，前端应改用 /issues/search 分页"
        );
    }
    Ok(snapshot_truncatable(
        "issues",
        snapshot.issues,
        snapshot.truncated,
    ))
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
    current_user_id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    Issues::create(pool, &payload, current_user_id)
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
    // 个人版还没实现的筛选条件：宁可 400 也不能静默忽略，否则前端拿到的是
    // 「看起来筛过、其实没筛」的结果。
    let unsupported: Vec<&str> = [
        ("assignee_user_id", payload.assignee_user_id.is_some()),
        ("parent_issue_id", payload.parent_issue_id.is_some()),
    ]
    .into_iter()
    .filter(|(_, present)| *present)
    .map(|(name, _)| name)
    .collect();
    if !unsupported.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "未支持的筛选条件：{}",
            unsupported.join("、")
        )));
    }

    Issues::search(pool, &payload)
        .await
        .map(Json)
        .map_err(map_issue_error)
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
    current_user: crate::middleware::local_session::CurrentUser,
    Json(payload): Json<CreateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload, current_user.id).await
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
    LocalRoutes::new("/issues")
        .route("/", &["GET", "POST"], get(list).post(create))
        .route("/bulk", &["POST"], post(bulk_update))
        .route("/search", &["POST"], post(search))
        .route(
            "/{id}",
            &["GET", "PATCH", "DELETE"],
            get(get_one).patch(update).delete(delete),
        )
        // 流水线（契约 §2）。handler 在 pipeline.rs；与 /{id} 共用同一个 nest 与参数名（C4）。
        .route(
            "/{id}/pipeline",
            &["GET", "POST"],
            get(super::pipeline::get_issue_pipeline).post(super::pipeline::start_pipeline),
        )
        .into_router()
}

#[cfg(test)]
mod tests {
    use api_types::{
        issue::{CreateIssueRequest, SearchIssuesRequest, UpdateIssueRequest},
        project::CreateProjectRequest,
    };
    use db::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{
        handle_bulk_update, handle_create, handle_delete, handle_list, handle_search, handle_update,
    };
    use crate::{
        error::ApiError,
        routes::local_projects::{BulkUpdateItem, BulkUpdateRequest},
    };

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
    async fn 建需求时落的是传入的当前用户() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let current_user_id = Uuid::from_u128(7);

        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "指定当前用户"),
            current_user_id,
        )
        .await
        .unwrap();

        let snapshot = db::models::issue::Issues::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(snapshot.issues[0].creator_user_id, Some(current_user_id));
    }

    /// 团队版场景：同一项目下，甲、乙各自建一条需求；两人共用同一份项目快照
    /// （本地列表接口不按用户过滤），但各自需求的 creator_user_id 必须互不串号。
    #[tokio::test]
    async fn 两个用户各自建需求且都能看到对方的但_creator_user_id_各自正确() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let 甲 = Uuid::from_u128(101);
        let 乙 = Uuid::from_u128(102);

        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "甲的需求"),
            甲,
        )
        .await
        .unwrap();
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "乙的需求"),
            乙,
        )
        .await
        .unwrap();

        // 列表接口不按请求者过滤：甲、乙都能在同一份快照里看到对方的需求。
        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "甲、乙的需求都应出现在同一份项目快照里");

        let creator_of = |title: &str| -> String {
            rows.iter()
                .find(|row| row["title"] == title)
                .unwrap_or_else(|| panic!("没找到标题为 {title} 的需求"))["creator_user_id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(creator_of("甲的需求"), 甲.to_string());
        assert_eq!(creator_of("乙的需求"), 乙.to_string());
        assert_ne!(creator_of("甲的需求"), creator_of("乙的需求"));
    }

    #[tokio::test]
    async fn 列表用_issues_作为_key() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "第一条"),
            DEFAULT_USER_ID,
        )
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
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "第一条"),
            DEFAULT_USER_ID,
        )
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

        let result = handle_create(
            test_db.pool(),
            建需求请求(project_a, status_b, "越权"),
            DEFAULT_USER_ID,
        )
        .await;
        assert!(result.is_err(), "状态列不属于该项目时必须拒绝");
    }

    #[tokio::test]
    async fn 批量更新排序成功后按新顺序返回() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "A"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "B"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap().clone();
        let a_id: Uuid = rows.iter().find(|r| r["title"] == "A").unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let _ = handle_bulk_update(
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
            let _ = handle_create(
                test_db.pool(),
                建需求请求(project_id, status_id, &format!("需求 {index}")),
                DEFAULT_USER_ID,
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
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "留"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "删"),
            DEFAULT_USER_ID,
        )
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

        let _ = handle_delete(test_db.pool(), victim).await.unwrap();

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

    #[tokio::test]
    async fn 跨项目状态列返回_400_而不是_500() {
        let test_db = TestDb::new().await;
        let (project_a, _) = 准备(&test_db).await;
        let (_, status_b) = 准备(&test_db).await;

        let err = handle_create(
            test_db.pool(),
            建需求请求(project_a, status_b, "越权"),
            DEFAULT_USER_ID,
        )
        .await
        .expect_err("状态列不属于该项目时必须拒绝");
        断言是_400(err);
    }

    #[tokio::test]
    async fn 跨项目父需求返回_400() {
        let test_db = TestDb::new().await;
        let (project_a, status_a) = 准备(&test_db).await;
        let (project_b, status_b) = 准备(&test_db).await;

        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_b, status_b, "别家的"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let parent_id: Uuid = handle_list(test_db.pool(), project_b).await.unwrap().0["issues"][0]
            ["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let mut request = 建需求请求(project_a, status_a, "子需求");
        request.parent_issue_id = Some(parent_id);
        断言是_400(
            handle_create(test_db.pool(), request, DEFAULT_USER_ID)
                .await
                .unwrap_err(),
        );
    }

    #[tokio::test]
    async fn 超大_metadata_返回_400() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;

        let mut request = 建需求请求(project_id, status_id, "超大");
        request.extension_metadata = serde_json::json!({ "blob": "x".repeat(40_000) });
        断言是_400(
            handle_create(test_db.pool(), request, DEFAULT_USER_ID)
                .await
                .unwrap_err(),
        );

        let mut not_object = 建需求请求(project_id, status_id, "非对象");
        not_object.extension_metadata = serde_json::json!([1, 2, 3]);
        断言是_400(
            handle_create(test_db.pool(), not_object, DEFAULT_USER_ID)
                .await
                .unwrap_err(),
        );
    }

    /// 前端 KanbanIssuePanelContainer 新建需求时发出的真实报文：
    /// 未填的字段一律是 null，`extension_metadata` 也是 null（不是 {}）。
    /// 这条链路一旦 400，个人版就完全无法新建需求。
    #[tokio::test]
    async fn 前端真实新建报文可以成功落库() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;

        let body = serde_json::json!({
            "id": Uuid::new_v4(),
            "project_id": project_id,
            "status_id": status_id,
            "title": "来自前端的需求",
            "description": null,
            "priority": null,
            "sort_order": -1.0,
            "start_date": null,
            "target_date": null,
            "completed_at": null,
            "parent_issue_id": null,
            "parent_issue_sort_order": null,
            "extension_metadata": null
        });
        let payload: CreateIssueRequest =
            serde_json::from_value(body).expect("前端报文必须能反序列化");

        let _ = handle_create(test_db.pool(), payload, DEFAULT_USER_ID)
            .await
            .expect("前端真实报文必须创建成功");

        let list = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(list["issues"].as_array().unwrap().len(), 1);
        assert_eq!(list["issues"][0]["title"], "来自前端的需求");
        assert_eq!(
            list["issues"][0]["extension_metadata"],
            serde_json::json!({}),
            "null 应按空对象落库"
        );
    }

    #[tokio::test]
    async fn 更新时把_extension_metadata_置为_null_等价于清空() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let mut request = 建需求请求(project_id, status_id, "带扩展");
        request.extension_metadata = serde_json::json!({ "a": 1 });
        let _ = handle_create(test_db.pool(), request, DEFAULT_USER_ID)
            .await
            .unwrap();

        let id: Uuid = handle_list(test_db.pool(), project_id).await.unwrap().0["issues"][0]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let payload: UpdateIssueRequest =
            serde_json::from_value(serde_json::json!({ "extension_metadata": null }))
                .expect("更新报文必须能反序列化");
        let _ = handle_update(test_db.pool(), id, payload)
            .await
            .expect("null 的 extension_metadata 必须被接受");

        let after = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(
            after["issues"][0]["extension_metadata"],
            serde_json::json!({})
        );
    }

    #[tokio::test]
    async fn 更新不存在的需求返回_404() {
        let test_db = TestDb::new().await;
        let err = handle_update(
            test_db.pool(),
            Uuid::from_u128(778899),
            UpdateIssueRequest {
                title: Some("新".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不存在的需求必须 404");
        断言是_404(err);
    }

    #[tokio::test]
    async fn 批量更新里有不存在的需求返回_404() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "A"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let err = handle_bulk_update(
            test_db.pool(),
            BulkUpdateRequest {
                updates: vec![BulkUpdateItem {
                    id: Uuid::from_u128(5150),
                    changes: UpdateIssueRequest {
                        sort_order: Some(1.0),
                        ..Default::default()
                    },
                }],
            },
        )
        .await
        .expect_err("目标不存在必须 404");
        断言是_404(err);
    }

    #[tokio::test]
    async fn 搜索遇到未支持的筛选条件返回_400() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        for (名称, request) in [
            (
                "assignee_user_id",
                SearchIssuesRequest {
                    project_id,
                    assignee_user_id: Some(Uuid::from_u128(3)),
                    ..Default::default()
                },
            ),
            (
                "parent_issue_id",
                SearchIssuesRequest {
                    project_id,
                    parent_issue_id: Some(Uuid::from_u128(4)),
                    ..Default::default()
                },
            ),
        ] {
            let err = handle_search(test_db.pool(), request)
                .await
                .expect_err("未支持的筛选条件必须报错");
            match err {
                ApiError::BadRequest(message) => {
                    assert!(message.contains(名称), "错误信息应点名 {名称}：{message}");
                }
                other => panic!("应是 400，实际：{other:?}"),
            }
        }
    }

    /// tag_id（单数）以前直接 400，现在等价于 tag_ids: [id]。
    #[tokio::test]
    async fn 搜索支持_tag_id_单数() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        let body = handle_search(
            test_db.pool(),
            SearchIssuesRequest {
                project_id,
                tag_id: Some(Uuid::from_u128(5)),
                ..Default::default()
            },
        )
        .await
        .expect("tag_id 不应再被拒绝")
        .0;
        assert_eq!(body.total_count, 0, "不存在的标签筛不到需求");
    }

    #[tokio::test]
    async fn 搜索筛选数组超过上限返回_400() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        let 太多: Vec<Uuid> = (0..db::models::issue::MAX_FILTER_IDS + 1)
            .map(|i| Uuid::from_u128(i as u128 + 1))
            .collect();
        let err = handle_search(
            test_db.pool(),
            SearchIssuesRequest {
                project_id,
                status_ids: Some(太多),
                ..Default::default()
            },
        )
        .await
        .expect_err("超长筛选数组必须 400 而不是 500");
        断言是_400(err);
    }

    #[tokio::test]
    async fn 搜索支持_status_ids_不再静默忽略() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let dev = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();

        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "待开发"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let mut in_dev = 建需求请求(project_id, status_id, "开发中");
        in_dev.status_id = dev.id;
        let _ = handle_create(test_db.pool(), in_dev, DEFAULT_USER_ID)
            .await
            .unwrap();

        let body = handle_search(
            test_db.pool(),
            SearchIssuesRequest {
                project_id,
                status_ids: Some(vec![dev.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
        assert_eq!(body.total_count, 1, "status_ids 必须真的生效");
        assert_eq!(body.issues[0].title, "开发中");
    }

    #[tokio::test]
    async fn 快照在未截断时也带出_truncated_字段() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        let _ = handle_create(
            test_db.pool(),
            建需求请求(project_id, status_id, "A"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(body["truncated"], serde_json::json!(false));
        assert_eq!(body["issues"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn 快照超过上限时_truncated_为真() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;

        let total = db::models::issue::MAX_SNAPSHOT_ROWS + 3;
        let mut tx = test_db.pool().begin().await.unwrap();
        for index in 0..total {
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title, \
                 sort_order, extension_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            )
            .bind(Uuid::new_v4())
            .bind(project_id)
            .bind(index + 1)
            .bind(format!("VK-{}", index + 1))
            .bind(status_id)
            .bind(format!("批量 {index}"))
            .bind(index as f64)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(body["truncated"], serde_json::json!(true), "截断必须被透出");
        assert_eq!(
            body["issues"].as_array().unwrap().len(),
            db::models::issue::MAX_SNAPSHOT_ROWS as usize
        );
    }
}
