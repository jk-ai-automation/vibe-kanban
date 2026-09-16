use api_types::project::{CreateProjectRequest, UpdateProjectRequest};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use db::models::local_project::LocalProjects;
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, LocalRoutes, MAX_BULK_UPDATES, TxidResponse, map_db_error, snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(pool: &SqlitePool) -> Result<Json<Value>, ApiError> {
    let projects = LocalProjects::find_all(pool).await?;
    Ok(snapshot("projects", projects))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateProjectRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.name.trim().is_empty() {
        return Err(ApiError::BadRequest("项目名称不能为空".to_string()));
    }
    LocalProjects::create(pool, &payload)
        .await
        .map_err(map_db_error)?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateProjectRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    LocalProjects::update(pool, id, &payload)
        .await
        .map_err(map_db_error)?;
    Ok(txid())
}

/// 项目排序等多行更新：前端把一次拖拽合并成一个 `/bulk` 请求
/// （localCollections.ts 的 onUpdate、remoteApi.ts 的 bulkUpdateProjects）。
/// 单事务，任一条失败整体回滚。
pub(crate) async fn handle_bulk_update(
    pool: &SqlitePool,
    payload: BulkUpdateRequest<UpdateProjectRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.updates.len() > MAX_BULK_UPDATES {
        return Err(ApiError::BadRequest(format!(
            "单次最多更新 {MAX_BULK_UPDATES} 个项目"
        )));
    }
    let updates: Vec<_> = payload
        .updates
        .into_iter()
        .map(|item| (item.id, item.changes))
        .collect();
    LocalProjects::bulk_update(pool, &updates)
        .await
        .map_err(map_db_error)?;
    Ok(txid())
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let affected = LocalProjects::delete(pool, id).await?;
    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

async fn list_projects(State(deployment): State<DeploymentImpl>) -> Result<Json<Value>, ApiError> {
    handle_list(&deployment.db().pool).await
}

async fn create_project(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update_project(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProjectRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn bulk_update_projects(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<BulkUpdateRequest<UpdateProjectRequest>>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_bulk_update(&deployment.db().pool, payload).await
}

async fn delete_project(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    LocalRoutes::new("/projects")
        .route(
            "/",
            &["GET", "POST"],
            get(list_projects).post(create_project),
        )
        .route("/bulk", &["POST"], post(bulk_update_projects))
        .route(
            "/{id}",
            &["PATCH", "DELETE"],
            axum::routing::patch(update_project).delete(delete_project),
        )
        .into_router()
}

#[cfg(test)]
mod tests {
    use api_types::project::{CreateProjectRequest, UpdateProjectRequest};
    use db::{models::local_project::DEFAULT_ORGANIZATION_ID, test_support::TestDb};
    use uuid::Uuid;

    use super::{handle_bulk_update, handle_create, handle_delete, handle_list, handle_update};

    fn 建项目请求(name: &str) -> CreateProjectRequest {
        CreateProjectRequest {
            id: None,
            organization_id: DEFAULT_ORGANIZATION_ID,
            name: name.to_string(),
            color: "#6366f1".to_string(),
        }
    }

    #[tokio::test]
    async fn 列表响应使用_projects_作为_key() {
        let test_db = TestDb::new().await;
        let _ = handle_create(test_db.pool(), 建项目请求("A"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool()).await.unwrap().0;
        let rows = body
            .get("projects")
            .and_then(|v| v.as_array())
            .expect("响应必须有 projects 数组");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "A");
        assert_eq!(
            rows[0]["organization_id"],
            DEFAULT_ORGANIZATION_ID.to_string()
        );
    }

    #[tokio::test]
    async fn 空库返回空数组而不是_null() {
        let test_db = TestDb::new().await;
        let body = handle_list(test_db.pool()).await.unwrap().0;
        assert_eq!(body["projects"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn 写操作返回_txid_零() {
        let test_db = TestDb::new().await;
        let response = handle_create(test_db.pool(), 建项目请求("A"))
            .await
            .unwrap();
        assert_eq!(response.0.txid, 0);
    }

    #[tokio::test]
    async fn 更新不存在的项目返回_404() {
        let test_db = TestDb::new().await;
        let result = handle_update(
            test_db.pool(),
            Uuid::from_u128(4242),
            UpdateProjectRequest {
                name: Some("X".to_string()),
                color: None,
                sort_order: None,
            },
        )
        .await;

        assert!(matches!(result, Err(crate::error::ApiError::NotFound)));
    }

    #[tokio::test]
    async fn 删除不存在的项目返回_404_而不是静默成功() {
        let test_db = TestDb::new().await;
        let result = handle_delete(test_db.pool(), Uuid::from_u128(4242)).await;
        assert!(matches!(result, Err(crate::error::ApiError::NotFound)));
    }

    #[tokio::test]
    async fn 删除只影响目标项目() {
        let test_db = TestDb::new().await;
        let _ = handle_create(test_db.pool(), 建项目请求("留下"))
            .await
            .unwrap();
        let body = handle_list(test_db.pool()).await.unwrap().0;
        let keep_id: Uuid = body["projects"][0]["id"].as_str().unwrap().parse().unwrap();

        let _ = handle_create(test_db.pool(), 建项目请求("删掉"))
            .await
            .unwrap();
        let body = handle_list(test_db.pool()).await.unwrap().0;
        let victim_id: Uuid = body["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "删掉")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let _ = handle_delete(test_db.pool(), victim_id).await.unwrap();

        let after = handle_list(test_db.pool()).await.unwrap().0;
        let names: Vec<_> = after["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["留下".to_string()]);
        assert_ne!(keep_id, victim_id);
    }

    #[tokio::test]
    async fn 批量更新项目排序成功并全成全败() {
        use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest};

        let test_db = TestDb::new().await;
        for name in ["A", "B"] {
            let _ = handle_create(test_db.pool(), 建项目请求(name))
                .await
                .unwrap();
        }
        let body = handle_list(test_db.pool()).await.unwrap().0;
        let id = |name: &str| -> Uuid {
            body["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        };

        let _ = handle_bulk_update(
            test_db.pool(),
            BulkUpdateRequest {
                updates: vec![
                    BulkUpdateItem {
                        id: id("A"),
                        changes: UpdateProjectRequest {
                            name: None,
                            color: None,
                            sort_order: Some(5),
                        },
                    },
                    BulkUpdateItem {
                        id: id("B"),
                        changes: UpdateProjectRequest {
                            name: None,
                            color: None,
                            sort_order: Some(1),
                        },
                    },
                ],
            },
        )
        .await
        .expect("批量更新必须成功");

        let after = handle_list(test_db.pool()).await.unwrap().0;
        assert_eq!(after["projects"][0]["name"], "B", "sort_order 小的排前面");

        // 含不存在的 id：整体回滚，返回 404
        let err = handle_bulk_update(
            test_db.pool(),
            BulkUpdateRequest {
                updates: vec![
                    BulkUpdateItem {
                        id: id("A"),
                        changes: UpdateProjectRequest {
                            name: None,
                            color: None,
                            sort_order: Some(99),
                        },
                    },
                    BulkUpdateItem {
                        id: Uuid::from_u128(4242),
                        changes: UpdateProjectRequest {
                            name: None,
                            color: None,
                            sort_order: Some(98),
                        },
                    },
                ],
            },
        )
        .await
        .expect_err("目标不存在必须报错");
        assert!(matches!(err, crate::error::ApiError::NotFound));

        let rolled_back = handle_list(test_db.pool()).await.unwrap().0;
        let a_sort = rolled_back["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "A")
            .unwrap()["sort_order"]
            .as_i64()
            .unwrap();
        assert_eq!(a_sort, 5, "失败必须整体回滚");
    }

    #[tokio::test]
    async fn 批量更新项目超过上限被拒绝() {
        use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest, MAX_BULK_UPDATES};

        let test_db = TestDb::new().await;
        let updates = (0..MAX_BULK_UPDATES + 1)
            .map(|_| BulkUpdateItem {
                id: Uuid::new_v4(),
                changes: UpdateProjectRequest {
                    name: None,
                    color: None,
                    sort_order: Some(1),
                },
            })
            .collect();

        let err = handle_bulk_update(test_db.pool(), BulkUpdateRequest { updates })
            .await
            .expect_err("超过上限必须拒绝");
        assert!(matches!(err, crate::error::ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn 重复提交同一个客户端_id_返回_409_而不是_500() {
        let test_db = TestDb::new().await;
        let mut request = 建项目请求("固定 id");
        request.id = Some(Uuid::from_u128(777));

        let _ = handle_create(test_db.pool(), request.clone())
            .await
            .unwrap();
        let err = handle_create(test_db.pool(), request)
            .await
            .expect_err("主键重复必须报错");
        assert!(
            matches!(err, crate::error::ApiError::Conflict(_)),
            "应映射为 409，实际：{err:?}"
        );
    }

    #[tokio::test]
    async fn 名称里的单引号不会破坏_sql() {
        let test_db = TestDb::new().await;
        let _ = handle_create(
            test_db.pool(),
            建项目请求("O'Brien'); DROP TABLE issues;--"),
        )
        .await
        .unwrap();

        let body = handle_list(test_db.pool()).await.unwrap().0;
        assert_eq!(
            body["projects"][0]["name"],
            "O'Brien'); DROP TABLE issues;--"
        );

        // issues 表必须还在
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
            .fetch_one(test_db.pool())
            .await
            .expect("issues 表必须还存在");
        assert_eq!(count.0, 0);
    }
}
