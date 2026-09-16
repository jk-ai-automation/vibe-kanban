//! 个人版本地需求接口。路径与云端 /v1 同构，挂在 /api/local 下。
//! 快照统一返回 { "<前端表名>": [ ...行 ] }，写操作统一返回 { "txid": 0 }
//! （个人版不使用 ElectricSQL 事务对账，txid 只是占位）。

pub mod issues;
pub mod projections;
pub mod projects;
pub mod side;
pub mod statuses;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, OnceLock},
};

use axum::{Json, Router, routing::MethodRouter};
use db::models::issue::IssueError;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{DeploymentImpl, error::ApiError};

/// `router()` 实际挂载出来的端点表：完整路径（含 `/api` 前缀）→ 方法集合。
///
/// 本地路由一律经 [`LocalRoutes`] 注册，路由契约测试拿这张表跟前端调用点
/// （localEndpoints.ts / remoteApi.ts / localCollections.ts）对账，防止出现
/// 「前端有调用、后端没路由」这种只有真跑起来才会暴露的问题。
static ROUTE_REGISTRY: OnceLock<Mutex<BTreeMap<String, BTreeSet<&'static str>>>> = OnceLock::new();

fn route_registry() -> &'static Mutex<BTreeMap<String, BTreeSet<&'static str>>> {
    ROUTE_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 读取当前已注册的端点表。调用前需先构造一次 [`router()`]。
pub fn registered_endpoints() -> BTreeMap<String, BTreeSet<&'static str>> {
    route_registry().lock().expect("端点表被污染").clone()
}

/// 本地资源路由构建器：挂路由的同时把路径与方法登记进 [`ROUTE_REGISTRY`]。
///
/// `methods` 与 `handler` 里的方法需要人工保持一致（axum 不暴露 MethodRouter 的
/// 方法集合）；漏登记只会让契约测试失败，不会让路由被误判为存在。
pub struct LocalRoutes {
    prefix: String,
    router: Router<DeploymentImpl>,
}

impl LocalRoutes {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            router: Router::new(),
        }
    }

    pub fn route(
        mut self,
        path: &str,
        methods: &[&'static str],
        handler: MethodRouter<DeploymentImpl>,
    ) -> Self {
        let full = if path == "/" {
            format!("/api/local{}", self.prefix)
        } else {
            format!("/api/local{}{}", self.prefix, path)
        };
        let mut registry = route_registry().lock().expect("端点表被污染");
        let entry = registry.entry(full).or_default();
        for method in methods {
            entry.insert(*method);
        }
        drop(registry);

        self.router = self.router.route(path, handler);
        self
    }

    pub fn into_router(self) -> Router<DeploymentImpl> {
        Router::new().nest(&self.prefix, self.router)
    }
}

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

/// 把模型层的校验错误映射成 400，冲突映射成 409，行不存在映射成 404，
/// 其余映射成数据库错误。
pub fn map_issue_error(error: IssueError) -> ApiError {
    match error {
        IssueError::Validation(message) => ApiError::BadRequest(message),
        IssueError::Conflict(message) => ApiError::Conflict(message),
        IssueError::Database(err) => map_db_error(err),
    }
}

/// `UPDATE ... RETURNING` 打不中行时 sqlx 返回 RowNotFound，这是 404 而不是 500；
/// 客户端自带 id 重复提交会撞唯一约束/主键，那是 409 而不是 500。
pub fn map_db_error(error: sqlx::Error) -> ApiError {
    if db::models::db_retry::is_unique_violation(&error) {
        return ApiError::Conflict("该 id 已存在，请勿重复提交".to_string());
    }
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

#[cfg(test)]
mod tests {
    use api_types::issue::UpdateIssueRequest;

    use super::{BulkUpdateRequest, registered_endpoints, router};

    /// 前端把 `{id, ...changes}` 平铺成一项（remoteApi.ts / localCollections.ts），
    /// 后端靠 `#[serde(flatten)]` 拆开；`"description": null` 必须落成
    /// `Some(None)`（显式置空），而不是 `None`（没传）。
    #[test]
    fn 批量更新请求体能正确拆出_id_与变更字段() {
        let payload = serde_json::json!({
            "updates": [
                { "id": "00000000-0000-0000-0000-000000000001", "sort_order": 3.5 },
                { "id": "00000000-0000-0000-0000-000000000002", "description": null },
                { "id": "00000000-0000-0000-0000-000000000003" }
            ]
        });

        let request: BulkUpdateRequest<UpdateIssueRequest> =
            serde_json::from_value(payload).expect("批量更新报文必须能反序列化");
        assert_eq!(request.updates.len(), 3);

        assert_eq!(request.updates[0].id.as_u128(), 1);
        assert_eq!(request.updates[0].changes.sort_order, Some(3.5));
        assert!(
            request.updates[0].changes.description.is_none(),
            "没传的字段必须是 None"
        );

        assert_eq!(
            request.updates[1].changes.description,
            Some(None),
            "显式 null 必须是 Some(None)，即「置空」而不是「没传」"
        );

        assert!(request.updates[2].changes.sort_order.is_none());
        assert!(request.updates[2].changes.title.is_none());
    }

    /// 前端的本地端点解析表。
    const LOCAL_ENDPOINTS_TS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/web-core/src/shared/lib/local/localEndpoints.ts"
    ));
    /// 前端直接写死本地路径的地方（批量更新）。
    const REMOTE_API_TS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/web-core/src/shared/lib/remoteApi.ts"
    ));

    /// 取 `marker` 之后到第一个 `\n};` 之间的代码块。
    fn 取代码块(source: &str, marker: &str) -> String {
        let start = source
            .find(marker)
            .unwrap_or_else(|| panic!("前端文件里找不到 {marker}，请同步更新契约测试"));
        let rest = &source[start..];
        let end = rest.find("\n};").expect("代码块没有结束符");
        rest[..end].to_string()
    }

    /// 取代码块里所有单引号字符串。
    fn 取单引号值(block: &str) -> Vec<String> {
        let mut values = Vec::new();
        let mut rest = block;
        while let Some(open) = rest.find('\'') {
            let tail = &rest[open + 1..];
            let Some(close) = tail.find('\'') else { break };
            values.push(tail[..close].to_string());
            rest = &tail[close + 1..];
        }
        values
    }

    /// 取源码里出现的 `/api/local/...` 字面路径（模板前缀先展开）。
    fn 取字面路径(source: &str) -> Vec<String> {
        let expanded = source.replace("${LOCAL_API_PREFIX}", "/api/local");
        let mut paths = Vec::new();
        let mut rest = expanded.as_str();
        while let Some(at) = rest.find("/api/local/") {
            let tail = &rest[at..];
            let end = tail
                .find(|c: char| c == '\'' || c == '"' || c == '`' || c == '?' || c.is_whitespace())
                .unwrap_or(tail.len());
            paths.push(tail[..end].to_string());
            rest = &tail[end..];
        }
        paths
    }

    /// 前端调用点推导出的本地路由契约：(路径, 方法)。
    ///
    /// - 只读集合：localEndpoints.ts 的 REST_RESOURCE 决定 `GET /api/local/<表>`；
    /// - 可写集合：MUTATION_URL 里非 null 的项，按 localCollections.ts 的写法会打
    ///   `POST <base>`、`PATCH <base>/<id>`、`DELETE <base>/<id>`，多行更新合并成
    ///   `POST <base>/bulk`；
    /// - remoteApi.ts 里写死的 `/api/local/...` 字面路径。
    fn 前端要求的端点() -> Vec<(String, &'static str)> {
        let mut expected: Vec<(String, &'static str)> = Vec::new();

        let rest_resource = 取代码块(LOCAL_ENDPOINTS_TS, "const REST_RESOURCE");
        for resource in 取单引号值(&rest_resource) {
            expected.push((format!("/api/local/{resource}"), "GET"));
        }

        let mutation_url = 取代码块(LOCAL_ENDPOINTS_TS, "const MUTATION_URL");
        for resource in 取字面路径(&mutation_url) {
            expected.push((resource.clone(), "POST"));
            expected.push((format!("{resource}/bulk"), "POST"));
            expected.push((format!("{resource}/{{id}}"), "PATCH"));
            expected.push((format!("{resource}/{{id}}"), "DELETE"));
        }

        for path in 取字面路径(REMOTE_API_TS) {
            expected.push((path, "POST"));
        }

        expected.sort();
        expected.dedup();
        expected
    }

    #[test]
    fn 前端用到的本地端点都挂上了路由() {
        // 构造一次 router()，让所有路由完成登记。
        let _ = router();
        let registered = registered_endpoints();

        let mut missing: Vec<String> = Vec::new();
        for (path, method) in 前端要求的端点() {
            match registered.get(&path) {
                Some(methods) if methods.contains(method) => {}
                Some(methods) => missing.push(format!("{method} {path}（已有方法：{methods:?}）")),
                None => missing.push(format!("{method} {path}（路径未挂载）")),
            }
        }

        assert!(
            missing.is_empty(),
            "前端会调用但后端没挂的端点：{missing:#?}\n已注册：{registered:#?}"
        );
    }

    #[test]
    fn 契约测试确实解析到了前端的调用点() {
        // 防止解析器静默失效导致上面的测试变成空断言。
        let expected = 前端要求的端点();
        assert!(
            expected.len() > 20,
            "解析到的端点太少，解析器可能已失效：{expected:#?}"
        );
        assert!(expected.contains(&("/api/local/projects/bulk".to_string(), "POST")));
        assert!(expected.contains(&("/api/local/issues".to_string(), "GET")));
        assert!(expected.contains(&("/api/local/issue_comments/{id}".to_string(), "PATCH")));
    }
}
