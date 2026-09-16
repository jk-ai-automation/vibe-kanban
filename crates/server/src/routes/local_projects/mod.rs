//! 个人版本地需求接口。路径与云端 /v1 同构，挂在 /api/local 下。
//! 快照统一返回 { "<前端表名>": [ ...行 ] }，写操作统一返回 { "txid": 0 }
//! （个人版不使用 ElectricSQL 事务对账，txid 只是占位）。

pub mod issues;
pub mod projections;
pub mod projects;
pub mod side;
pub mod statuses;

use axum::{Json, Router};
use db::models::issue::IssueError;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Serialize)]
pub struct TxidResponse {
    pub txid: i64,
}

/// 写操作的统一响应。
pub fn txid() -> Json<TxidResponse> {
    Json(TxidResponse { txid: 0 })
}

/// 快照响应：key 必须与前端 ShapeDefinition.table 完全一致。
pub fn snapshot<T: Serialize>(table: &str, rows: Vec<T>) -> Json<Value> {
    Json(json!({ table: rows }))
}

/// 带截断标记的快照。前端只读 payload[table]，多出来的 `truncated` 不会影响它，
/// 但让调用方（以及排查问题的人）能看到「这一页不是全量」。
pub fn snapshot_truncatable<T: Serialize>(
    table: &str,
    rows: Vec<T>,
    truncated: bool,
) -> Json<Value> {
    Json(json!({ table: rows, "truncated": truncated }))
}

/// 把模型层的校验错误映射成 400，行不存在映射成 404，其余映射成数据库错误。
pub fn map_issue_error(error: IssueError) -> ApiError {
    match error {
        IssueError::Validation(message) => ApiError::BadRequest(message),
        IssueError::Database(err) => map_db_error(err),
    }
}

/// `UPDATE ... RETURNING` 打不中行时 sqlx 返回 RowNotFound，这是 404 而不是 500。
pub fn map_db_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::RowNotFound => ApiError::NotFound,
        other => ApiError::Database(other),
    }
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/local",
        Router::new()
            .merge(projects::router())
            .merge(statuses::router())
            .merge(issues::router())
            .merge(side::router())
            .merge(projections::router()),
    )
}

use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ProjectScopedQuery {
    pub project_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IssueScopedQuery {
    pub issue_id: Uuid,
}

/// 批量更新请求体：{"updates": [{"id": "...", ...变更字段}]}
#[derive(Debug, Deserialize)]
pub struct BulkUpdateItem<T> {
    pub id: Uuid,
    #[serde(flatten)]
    pub changes: T,
}

#[derive(Debug, Deserialize)]
pub struct BulkUpdateRequest<T> {
    pub updates: Vec<BulkUpdateItem<T>>,
}

/// 一次批量更新的条数上限，防止前端误发超大请求打满事务。
pub const MAX_BULK_UPDATES: usize = 1000;
