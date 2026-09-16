//! 个人版本地需求接口。路径与云端 /v1 同构，挂在 /api/local 下。
//! 快照统一返回 { "<前端表名>": [ ...行 ] }，写操作统一返回 { "txid": 0 }
//! （个人版不使用 ElectricSQL 事务对账，txid 只是占位）。

pub mod projects;

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

/// 把模型层的校验错误映射成 400，其余映射成数据库错误。
pub fn map_issue_error(error: IssueError) -> ApiError {
    match error {
        IssueError::Validation(message) => ApiError::BadRequest(message),
        IssueError::Database(err) => ApiError::Database(err),
    }
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest("/local", projects::router())
}
