use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use db::models::local_project::DEFAULT_USER_ID;
use deployment::Deployment;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{LocalRoutes, ProjectScopedQuery, snapshot};
use crate::{DeploymentImpl, error::ApiError};

/// 个人版不实现的集合：返回空数组，保证前端订阅这些 shape 时不报错。
/// 数组内容必须与 shared/remote-types.ts 中各 ShapeDefinition 的 table 完全一致。
pub const EMPTY_TABLES: &[&str] = &[
    "issue_assignees",
    "issue_followers",
    "issue_relationships",
    "issue_comment_reactions",
    "pull_request_issues",
    "notifications",
    "users",
    "organization_member_metadata",
];

/// 云端 Workspace 行结构的本地投影。字段与 api_types::workspace::Workspace 一致。
#[derive(Debug, Serialize)]
struct ProjectedWorkspace {
    id: Uuid,
    project_id: Uuid,
    owner_user_id: Uuid,
    issue_id: Option<Uuid>,
    local_workspace_id: Option<Uuid>,
    name: Option<String>,
    archived: bool,
    files_changed: Option<i32>,
    lines_added: Option<i32>,
    lines_removed: Option<i32>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// 云端 PullRequest 行结构的本地投影。
#[derive(Debug, Serialize)]
struct ProjectedPullRequest {
    id: String,
    url: String,
    number: i32,
    status: String,
    merged_at: Option<DateTime<Utc>>,
    merge_commit_sha: Option<String>,
    target_branch_name: String,
    project_id: Uuid,
    issue_id: Option<Uuid>,
    workspace_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) fn handle_empty(table: &str) -> Json<Value> {
    Json(json!({ table: Vec::<Value>::new() }))
}

/// 本地 pull_requests 的 pr_status 多一个 `unknown`（尚未查到状态），
/// 而前端类型只有 open/merged/closed。投影时按 `open` 处理：
/// 未知状态的 PR 还没合并、也没关闭，当作「进行中」最接近事实，
/// 原样输出会让前端按未知分支渲染。
fn project_pr_status(raw: &str) -> &str {
    match raw {
        "merged" | "closed" => raw,
        _ => "open",
    }
}

pub(crate) async fn handle_workspaces(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    struct Row {
        id: Uuid,
        issue_id: Option<Uuid>,
        name: Option<String>,
        archived: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    let rows = sqlx::query_as!(
        Row,
        r#"SELECT w.id         AS "id!: Uuid",
                  w.issue_id   AS "issue_id: Uuid",
                  w.name,
                  w.archived   AS "archived!: bool",
                  w.created_at AS "created_at!: DateTime<Utc>",
                  w.updated_at AS "updated_at!: DateTime<Utc>"
           FROM workspaces w
           JOIN issues i ON i.id = w.issue_id
           WHERE i.project_id = $1
           ORDER BY w.created_at DESC"#,
        project_id
    )
    .fetch_all(pool)
    .await?;

    // 只投影云端 Workspace 行结构中的字段，container_ref 等本地路径一律不输出。
    let projected: Vec<ProjectedWorkspace> = rows
        .into_iter()
        .map(|row| ProjectedWorkspace {
            id: row.id,
            project_id,
            owner_user_id: DEFAULT_USER_ID,
            issue_id: row.issue_id,
            local_workspace_id: Some(row.id),
            name: row.name,
            archived: row.archived,
            files_changed: None,
            lines_added: None,
            lines_removed: None,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(snapshot("workspaces", projected))
}

pub(crate) async fn handle_pull_requests(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    struct Row {
        id: String,
        pr_url: String,
        pr_number: i64,
        pr_status: String,
        merged_at: Option<DateTime<Utc>>,
        merge_commit_sha: Option<String>,
        target_branch_name: String,
        workspace_id: Option<Uuid>,
        issue_id: Option<Uuid>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    let rows = sqlx::query_as!(
        Row,
        r#"SELECT pr.id                 AS "id!",
                  pr.pr_url             AS "pr_url!",
                  pr.pr_number          AS "pr_number!: i64",
                  pr.pr_status          AS "pr_status!",
                  pr.merged_at          AS "merged_at: DateTime<Utc>",
                  pr.merge_commit_sha,
                  pr.target_branch_name AS "target_branch_name!",
                  pr.workspace_id       AS "workspace_id: Uuid",
                  w.issue_id            AS "issue_id: Uuid",
                  pr.created_at         AS "created_at!: DateTime<Utc>",
                  pr.updated_at         AS "updated_at!: DateTime<Utc>"
           FROM pull_requests pr
           JOIN workspaces w ON w.id = pr.workspace_id
           JOIN issues i ON i.id = w.issue_id
           WHERE i.project_id = $1
           ORDER BY pr.created_at DESC"#,
        project_id
    )
    .fetch_all(pool)
    .await?;

    let projected: Vec<ProjectedPullRequest> = rows
        .into_iter()
        .map(|row| ProjectedPullRequest {
            id: row.id,
            url: row.pr_url,
            number: row.pr_number as i32,
            status: project_pr_status(&row.pr_status).to_string(),
            merged_at: row.merged_at,
            merge_commit_sha: row.merge_commit_sha,
            target_branch_name: row.target_branch_name,
            project_id,
            issue_id: row.issue_id,
            workspace_id: row.workspace_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(snapshot("pull_requests", projected))
}

pub fn router() -> Router<DeploymentImpl> {
    let mut router = LocalRoutes::new("/workspaces")
        .route(
            "/",
            &["GET"],
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_workspaces(&d.db().pool, q.project_id).await
                },
            ),
        )
        .into_router()
        .merge(
            LocalRoutes::new("/pull_requests")
                .route(
                    "/",
                    &["GET"],
                    get(|State(d): State<DeploymentImpl>,
                         Query(q): Query<ProjectScopedQuery>| async move {
                        handle_pull_requests(&d.db().pool, q.project_id).await
                    }),
                )
                .into_router(),
        );

    for table in EMPTY_TABLES {
        let table = *table;
        router = router.merge(
            LocalRoutes::new(format!("/{table}"))
                .route(
                    "/",
                    &["GET"],
                    get(move || async move { handle_empty(table) }),
                )
                .into_router(),
        );
    }

    router
}

#[cfg(test)]
mod tests {
    use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{EMPTY_TABLES, handle_empty, handle_pull_requests, handle_workspaces};

    #[tokio::test]
    async fn 空集合按表名返回空数组() {
        let test_db = TestDb::new().await;
        for table in EMPTY_TABLES {
            let body = handle_empty(table).0;
            assert_eq!(
                body[*table],
                serde_json::json!([]),
                "{table} 必须返回空数组"
            );
        }
        // 确保没有误用数据库
        assert!(!test_db.pool().is_closed());
    }

    #[tokio::test]
    async fn 工作区投影只返回绑定本项目需求的工作区且字段与云端同构() {
        let test_db = TestDb::new().await;
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

        let linked = Uuid::new_v4();
        let orphan = Uuid::new_v4();
        for (id, issue_id) in [(linked, Some(issue.id)), (orphan, None)] {
            sqlx::query(
                "INSERT INTO workspaces (id, branch, name, issue_id) VALUES (?1, 'vk/x', 'W', ?2)",
            )
            .bind(id)
            .bind(issue_id)
            .execute(test_db.pool())
            .await
            .unwrap();
        }

        let body = handle_workspaces(test_db.pool(), project.id)
            .await
            .unwrap()
            .0;
        let rows = body["workspaces"].as_array().unwrap();

        assert_eq!(rows.len(), 1, "未绑定需求的工作区不出现在项目投影里");
        assert_eq!(rows[0]["id"], linked.to_string());
        assert_eq!(rows[0]["project_id"], project.id.to_string());
        assert_eq!(rows[0]["issue_id"], issue.id.to_string());
        assert_eq!(rows[0]["owner_user_id"], DEFAULT_USER_ID.to_string());
        assert_eq!(rows[0]["local_workspace_id"], linked.to_string());
        assert_ne!(linked, orphan);
    }

    #[tokio::test]
    async fn 工作区投影不泄露本地路径() {
        let test_db = TestDb::new().await;
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

        sqlx::query(
            "INSERT INTO workspaces (id, branch, container_ref, issue_id) \
             VALUES (?1, 'vk/x', '/Users/secret/worktrees/abc', ?2)",
        )
        .bind(Uuid::new_v4())
        .bind(issue.id)
        .execute(test_db.pool())
        .await
        .unwrap();

        let body = handle_workspaces(test_db.pool(), project.id)
            .await
            .unwrap()
            .0;
        let text = body.to_string();
        assert!(
            !text.contains("/Users/secret"),
            "投影不得包含 container_ref"
        );
        assert!(!text.contains("container_ref"));
    }

    #[tokio::test]
    async fn 拉取请求投影按需求反查() {
        let test_db = TestDb::new().await;
        let body = handle_pull_requests(test_db.pool(), Uuid::from_u128(1))
            .await
            .unwrap()
            .0;
        assert_eq!(body["pull_requests"], serde_json::json!([]));
    }

    /// 本地的 pr_status 多一个 unknown，前端类型里没有，必须映射成 open。
    #[test]
    fn 未知的拉取请求状态映射成_open() {
        assert_eq!(super::project_pr_status("unknown"), "open");
        assert_eq!(super::project_pr_status("open"), "open");
        assert_eq!(super::project_pr_status("merged"), "merged");
        assert_eq!(super::project_pr_status("closed"), "closed");
        assert_eq!(
            super::project_pr_status("以后新增的状态"),
            "open",
            "没见过的状态一律按进行中处理"
        );
    }
}
