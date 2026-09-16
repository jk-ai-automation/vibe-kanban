use api_types::issue::{
    CreateIssueRequest, Issue, IssuePriority, IssueSortField, ListIssuesResponse,
    SearchIssuesRequest, SortDirection, UpdateIssueRequest,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

use super::{
    local_project::LocalProjects,
    local_project_status::{ProjectStatuses, StageType},
};

/// 标题上限，超出部分截断（防止超大字段撑爆快照与 WS 推送）。
pub const MAX_TITLE_LEN: usize = 500;
/// 描述上限，超出部分截断。
pub const MAX_DESCRIPTION_LEN: usize = 100_000;
/// 单次搜索返回上限，防止前端一次拉取过多行。
pub const MAX_PAGE_SIZE: usize = 500;
/// 编号分配的重试次数（并发插入撞 UNIQUE，或 SQLite 在 rollback-journal 模式下
/// 多个连接同时争抢写锁返回 SQLITE_BUSY/LOCKED 时重试）。
const NUMBER_RETRY: u32 = 8;

/// 判断一个数据库错误是否值得重试：唯一约束冲突（并发拿到同一个编号），
/// 或 SQLite 在没有 WAL 的情况下多连接争抢锁时返回的 BUSY/LOCKED 系列错误码。
/// `sqlite3_busy_timeout` 只能缓解、不能完全消除这种情况，因此应用层仍需自行重试。
fn is_retryable_db_error(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };
    if db_err.is_unique_violation() {
        return true;
    }
    // SQLITE_BUSY = 5, SQLITE_LOCKED = 6, SQLITE_BUSY_RECOVERY = 261, SQLITE_BUSY_SNAPSHOT = 517
    matches!(
        db_err.code().as_deref(),
        Some("5") | Some("6") | Some("261") | Some("517")
    )
}

#[derive(Debug, Error)]
pub enum IssueError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

/// 用于 sqlx 解码的中间行：extension_metadata 在 SQLite 里是 TEXT。
struct IssueRow {
    id: Uuid,
    project_id: Uuid,
    issue_number: i32,
    simple_id: String,
    status_id: Uuid,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    start_date: Option<DateTime<Utc>>,
    target_date: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    sort_order: f64,
    parent_issue_id: Option<Uuid>,
    parent_issue_sort_order: Option<f64>,
    extension_metadata: String,
    creator_user_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<IssueRow> for Issue {
    fn from(row: IssueRow) -> Self {
        Issue {
            id: row.id,
            project_id: row.project_id,
            issue_number: row.issue_number,
            simple_id: row.simple_id,
            status_id: row.status_id,
            title: row.title,
            description: row.description,
            priority: row.priority.as_deref().and_then(parse_priority),
            start_date: row.start_date,
            target_date: row.target_date,
            completed_at: row.completed_at,
            sort_order: row.sort_order,
            parent_issue_id: row.parent_issue_id,
            parent_issue_sort_order: row.parent_issue_sort_order,
            extension_metadata: serde_json::from_str(&row.extension_metadata)
                .unwrap_or_else(|_| Value::Object(Default::default())),
            creator_user_id: row.creator_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn parse_priority(raw: &str) -> Option<IssuePriority> {
    match raw {
        "urgent" => Some(IssuePriority::Urgent),
        "high" => Some(IssuePriority::High),
        "medium" => Some(IssuePriority::Medium),
        "low" => Some(IssuePriority::Low),
        _ => None,
    }
}

fn priority_to_str(priority: IssuePriority) -> &'static str {
    match priority {
        IssuePriority::Urgent => "urgent",
        IssuePriority::High => "high",
        IssuePriority::Medium => "medium",
        IssuePriority::Low => "low",
    }
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 转义 LIKE 通配符，配合 SQL 中的 ESCAPE '\' 使用。
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub struct Issues;

impl Issues {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<Issue>, sqlx::Error> {
        let rows = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE project_id = $1
               ORDER BY sort_order ASC, created_at ASC
               LIMIT $2"#,
            project_id,
            MAX_SNAPSHOT_ROWS
        )
        .fetch_all(pool)
        .await?;

        Ok(rows.into_iter().map(Issue::from).collect())
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Issue>, sqlx::Error> {
        let row = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Issue::from))
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<Issue>, sqlx::Error> {
        let row = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Issue::from))
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueRequest,
    ) -> Result<Issue, IssueError> {
        let title = truncate(data.title.trim(), MAX_TITLE_LEN);
        if title.is_empty() {
            return Err(IssueError::Validation("需求标题不能为空".to_string()));
        }
        let description = data
            .description
            .as_ref()
            .map(|d| truncate(d, MAX_DESCRIPTION_LEN));

        let project = LocalProjects::find_by_id(pool, data.project_id)
            .await?
            .ok_or_else(|| IssueError::Validation("项目不存在".to_string()))?;
        let prefix = LocalProjects::simple_id_prefix(&project.name);

        let status = ProjectStatuses::find_by_id(pool, data.status_id).await?;
        match status {
            Some(status) if status.project_id == data.project_id => {}
            Some(_) => {
                return Err(IssueError::Validation(
                    "状态列不属于该项目".to_string(),
                ));
            }
            None => return Err(IssueError::Validation("状态列不存在".to_string())),
        }

        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let priority = data.priority.map(priority_to_str);
        let metadata = data.extension_metadata.to_string();

        for attempt in 0..NUMBER_RETRY {
            let mut tx = match pool.begin().await {
                Ok(tx) => tx,
                Err(err) if is_retryable_db_error(&err) && attempt + 1 < NUMBER_RETRY => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        5 * (attempt as u64 + 1),
                    ))
                    .await;
                    continue;
                }
                Err(err) => return Err(IssueError::Database(err)),
            };

            let next_result: Result<(i64,), sqlx::Error> = sqlx::query_as(
                "SELECT COALESCE(MAX(issue_number), 0) + 1 FROM issues WHERE project_id = ?1",
            )
            .bind(data.project_id)
            .fetch_one(&mut *tx)
            .await;
            let next = match next_result {
                Ok(next) => next,
                Err(err) if is_retryable_db_error(&err) && attempt + 1 < NUMBER_RETRY => {
                    let _ = tx.rollback().await;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        5 * (attempt as u64 + 1),
                    ))
                    .await;
                    continue;
                }
                Err(err) => return Err(IssueError::Database(err)),
            };
            let issue_number = next.0 as i32;
            let simple_id = format!("{prefix}-{issue_number}");

            let inserted = sqlx::query_as!(
                IssueRow,
                r#"INSERT INTO issues
                       (id, project_id, issue_number, simple_id, status_id, title, description,
                        priority, start_date, target_date, completed_at, sort_order,
                        parent_issue_id, parent_issue_sort_order, extension_metadata,
                        creator_user_id)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
                   RETURNING id                      AS "id!: Uuid",
                             project_id              AS "project_id!: Uuid",
                             issue_number            AS "issue_number!: i32",
                             simple_id               AS "simple_id!",
                             status_id               AS "status_id!: Uuid",
                             title                   AS "title!",
                             description,
                             priority,
                             start_date              AS "start_date: DateTime<Utc>",
                             target_date             AS "target_date: DateTime<Utc>",
                             completed_at            AS "completed_at: DateTime<Utc>",
                             sort_order              AS "sort_order!: f64",
                             parent_issue_id         AS "parent_issue_id: Uuid",
                             parent_issue_sort_order AS "parent_issue_sort_order: f64",
                             extension_metadata      AS "extension_metadata!",
                             creator_user_id         AS "creator_user_id: Uuid",
                             created_at              AS "created_at!: DateTime<Utc>",
                             updated_at              AS "updated_at!: DateTime<Utc>""#,
                id,
                data.project_id,
                issue_number,
                simple_id,
                data.status_id,
                title,
                description,
                priority,
                data.start_date,
                data.target_date,
                data.completed_at,
                data.sort_order,
                data.parent_issue_id,
                data.parent_issue_sort_order,
                metadata,
                super::local_project::DEFAULT_USER_ID
            )
            .fetch_one(&mut *tx)
            .await;

            match inserted {
                Ok(row) => {
                    tx.commit().await?;
                    return Ok(Issue::from(row));
                }
                Err(err) if is_retryable_db_error(&err) && attempt + 1 < NUMBER_RETRY => {
                    // 并发下另一个事务先拿走了这个编号，或写锁被别的连接占用，回滚后重试
                    let _ = tx.rollback().await;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        5 * (attempt as u64 + 1),
                    ))
                    .await;
                    continue;
                }
                Err(err) => return Err(IssueError::Database(err)),
            }
        }

        Err(IssueError::Validation("分配需求编号失败，请重试".to_string()))
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> Result<Issue, IssueError> {
        let mut tx = pool.begin().await?;
        let updated = Self::update_in_tx(&mut tx, id, data).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// 拖拽排序等批量写入：单事务，任一条失败整体回滚。
    pub async fn bulk_update(
        pool: &SqlitePool,
        updates: &[(Uuid, UpdateIssueRequest)],
    ) -> Result<Vec<Issue>, IssueError> {
        let mut tx = pool.begin().await?;
        let mut rows = Vec::with_capacity(updates.len());
        for (id, data) in updates {
            rows.push(Self::update_in_tx(&mut tx, *id, data).await?);
        }
        tx.commit().await?;
        Ok(rows)
    }

    async fn update_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> Result<Issue, IssueError> {
        // 每个字段配一个「是否设置」布尔量，用 CASE 在一条 UPDATE 里完成，
        // 避免先读后写的竞态：并发更新不同字段时互不覆盖。
        let set_status = data.status_id.is_some();
        let set_title = data.title.is_some();
        let title: Option<String> = match data.title.as_ref() {
            Some(raw) => {
                let trimmed = truncate(raw.trim(), MAX_TITLE_LEN);
                if trimmed.is_empty() {
                    return Err(IssueError::Validation("需求标题不能为空".to_string()));
                }
                Some(trimmed)
            }
            None => None,
        };
        let set_description = data.description.is_some();
        let description: Option<String> = data
            .description
            .clone()
            .flatten()
            .map(|d| truncate(&d, MAX_DESCRIPTION_LEN));
        let set_priority = data.priority.is_some();
        let priority = data.priority.flatten().map(priority_to_str);
        let set_start = data.start_date.is_some();
        let start_date = data.start_date.flatten();
        let set_target = data.target_date.is_some();
        let target_date = data.target_date.flatten();
        let set_completed = data.completed_at.is_some();
        let completed_at = data.completed_at.flatten();
        let set_sort = data.sort_order.is_some();
        let set_parent = data.parent_issue_id.is_some();
        let parent_issue_id = data.parent_issue_id.flatten();
        let set_parent_sort = data.parent_issue_sort_order.is_some();
        let parent_issue_sort_order = data.parent_issue_sort_order.flatten();
        let set_metadata = data.extension_metadata.is_some();
        let metadata = data.extension_metadata.as_ref().map(|v| v.to_string());

        let row = sqlx::query_as!(
            IssueRow,
            r#"UPDATE issues SET
                   status_id               = CASE WHEN $2  THEN $3  ELSE status_id END,
                   title                   = CASE WHEN $4  THEN $5  ELSE title END,
                   description             = CASE WHEN $6  THEN $7  ELSE description END,
                   priority                = CASE WHEN $8  THEN $9  ELSE priority END,
                   start_date              = CASE WHEN $10 THEN $11 ELSE start_date END,
                   target_date             = CASE WHEN $12 THEN $13 ELSE target_date END,
                   completed_at            = CASE WHEN $14 THEN $15 ELSE completed_at END,
                   sort_order              = CASE WHEN $16 THEN $17 ELSE sort_order END,
                   parent_issue_id         = CASE WHEN $18 THEN $19 ELSE parent_issue_id END,
                   parent_issue_sort_order = CASE WHEN $20 THEN $21 ELSE parent_issue_sort_order END,
                   extension_metadata      = CASE WHEN $22 THEN $23 ELSE extension_metadata END,
                   updated_at              = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id                      AS "id!: Uuid",
                         project_id              AS "project_id!: Uuid",
                         issue_number            AS "issue_number!: i32",
                         simple_id               AS "simple_id!",
                         status_id               AS "status_id!: Uuid",
                         title                   AS "title!",
                         description,
                         priority,
                         start_date              AS "start_date: DateTime<Utc>",
                         target_date             AS "target_date: DateTime<Utc>",
                         completed_at            AS "completed_at: DateTime<Utc>",
                         sort_order              AS "sort_order!: f64",
                         parent_issue_id         AS "parent_issue_id: Uuid",
                         parent_issue_sort_order AS "parent_issue_sort_order: f64",
                         extension_metadata      AS "extension_metadata!",
                         creator_user_id         AS "creator_user_id: Uuid",
                         created_at              AS "created_at!: DateTime<Utc>",
                         updated_at              AS "updated_at!: DateTime<Utc>""#,
            id,
            set_status,
            data.status_id,
            set_title,
            title,
            set_description,
            description,
            set_priority,
            priority,
            set_start,
            start_date,
            set_target,
            target_date,
            set_completed,
            completed_at,
            set_sort,
            data.sort_order,
            set_parent,
            parent_issue_id,
            set_parent_sort,
            parent_issue_sort_order,
            set_metadata,
            metadata
        )
        .fetch_one(&mut **tx)
        .await?;

        Ok(Issue::from(row))
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issues WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// 把需求移到项目里指定阶段的状态列。需求或状态列不存在时返回 None。
    pub async fn move_to_stage(
        pool: &SqlitePool,
        issue_id: Uuid,
        stage: StageType,
    ) -> Result<Option<Issue>, IssueError> {
        let Some(issue) = Self::find_by_id(pool, issue_id).await? else {
            return Ok(None);
        };
        let Some(status) = ProjectStatuses::find_stage(pool, issue.project_id, stage).await? else {
            return Ok(None);
        };

        let completed_at = if stage == StageType::Done {
            Some(Some(Utc::now()))
        } else {
            Some(None)
        };

        let updated = Self::update(
            pool,
            issue_id,
            &UpdateIssueRequest {
                status_id: Some(status.id),
                completed_at,
                ..Default::default()
            },
        )
        .await?;

        Ok(Some(updated))
    }

    pub async fn search(
        pool: &SqlitePool,
        request: &SearchIssuesRequest,
    ) -> Result<ListIssuesResponse, sqlx::Error> {
        let limit = request
            .limit
            .map(|l| (l.max(1) as usize).min(MAX_PAGE_SIZE))
            .unwrap_or(MAX_PAGE_SIZE);
        let offset = request.offset.map(|o| o.max(0) as usize).unwrap_or(0);

        let search_pattern = request
            .search
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!("%{}%", escape_like(s)));
        let has_search = search_pattern.is_some();
        let has_status = request.status_id.is_some();
        let has_priority = request.priority.is_some();
        let priority = request.priority.map(priority_to_str);
        let has_simple_id = request.simple_id.is_some();

        // 排序字段来自枚举，不来自用户输入的字符串，因此不存在 SQL 注入面。
        let ascending = !matches!(request.sort_direction, Some(SortDirection::Desc));
        let sort_field = request.sort_field.unwrap_or(IssueSortField::SortOrder);

        let total: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM issues
               WHERE project_id = ?1
                 AND (?2 = 0 OR title LIKE ?3 ESCAPE '\' OR IFNULL(description, '') LIKE ?3 ESCAPE '\')
                 AND (?4 = 0 OR status_id = ?5)
                 AND (?6 = 0 OR priority = ?7)
                 AND (?8 = 0 OR simple_id = ?9)"#,
        )
        .bind(request.project_id)
        .bind(has_search)
        .bind(search_pattern.clone())
        .bind(has_status)
        .bind(request.status_id)
        .bind(has_priority)
        .bind(priority)
        .bind(has_simple_id)
        .bind(request.simple_id.clone())
        .fetch_one(pool)
        .await?;

        let order_sql = match (sort_field, ascending) {
            (IssueSortField::SortOrder, true) => "sort_order ASC, created_at ASC",
            (IssueSortField::SortOrder, false) => "sort_order DESC, created_at DESC",
            (IssueSortField::Priority, true) => "priority ASC, created_at ASC",
            (IssueSortField::Priority, false) => "priority DESC, created_at DESC",
            (IssueSortField::CreatedAt, true) => "created_at ASC",
            (IssueSortField::CreatedAt, false) => "created_at DESC",
            (IssueSortField::UpdatedAt, true) => "updated_at ASC",
            (IssueSortField::UpdatedAt, false) => "updated_at DESC",
            (IssueSortField::Title, true) => "title ASC",
            (IssueSortField::Title, false) => "title DESC",
        };

        let sql = format!(
            r#"SELECT id, project_id, issue_number, simple_id, status_id, title, description,
                      priority, start_date, target_date, completed_at, sort_order,
                      parent_issue_id, parent_issue_sort_order, extension_metadata,
                      creator_user_id, created_at, updated_at
               FROM issues
               WHERE project_id = ?1
                 AND (?2 = 0 OR title LIKE ?3 ESCAPE '\' OR IFNULL(description, '') LIKE ?3 ESCAPE '\')
                 AND (?4 = 0 OR status_id = ?5)
                 AND (?6 = 0 OR priority = ?7)
                 AND (?8 = 0 OR simple_id = ?9)
               ORDER BY {order_sql}
               LIMIT ?10 OFFSET ?11"#
        );

        let rows = sqlx::query_as::<_, IssueSqlRow>(&sql)
            .bind(request.project_id)
            .bind(has_search)
            .bind(search_pattern)
            .bind(has_status)
            .bind(request.status_id)
            .bind(has_priority)
            .bind(priority)
            .bind(has_simple_id)
            .bind(request.simple_id.clone())
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(pool)
            .await?;

        Ok(ListIssuesResponse {
            issues: rows.into_iter().map(|row| Issue::from(row.0)).collect(),
            total_count: total.0 as usize,
            limit,
            offset,
        })
    }
}

/// 运行时（非宏）查询用的包装行。
struct IssueSqlRow(IssueRow);

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for IssueSqlRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(IssueSqlRow(IssueRow {
            id: row.try_get("id")?,
            project_id: row.try_get("project_id")?,
            issue_number: row.try_get("issue_number")?,
            simple_id: row.try_get("simple_id")?,
            status_id: row.try_get("status_id")?,
            title: row.try_get("title")?,
            description: row.try_get("description")?,
            priority: row.try_get("priority")?,
            start_date: row.try_get("start_date")?,
            target_date: row.try_get("target_date")?,
            completed_at: row.try_get("completed_at")?,
            sort_order: row.try_get("sort_order")?,
            parent_issue_id: row.try_get("parent_issue_id")?,
            parent_issue_sort_order: row.try_get("parent_issue_sort_order")?,
            extension_metadata: row.try_get("extension_metadata")?,
            creator_user_id: row.try_get("creator_user_id")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        }))
    }
}

/// 一次快照最多返回多少条需求。超过时前端应改用 /search 分页。
pub const MAX_SNAPSHOT_ROWS: i64 = 2000;

#[cfg(test)]
mod tests {
    use api_types::{
        issue::{
            CreateIssueRequest, IssuePriority, IssueSortField, SearchIssuesRequest, SortDirection,
            UpdateIssueRequest,
        },
        project::CreateProjectRequest,
    };
    use uuid::Uuid;

    use super::{Issues, MAX_DESCRIPTION_LEN, MAX_TITLE_LEN};
    use crate::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

    struct 场景 {
        project_id: Uuid,
        todo_status_id: Uuid,
    }

    async fn 准备(test_db: &TestDb, name: &str) -> 场景 {
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
        .expect("建项目失败");

        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .expect("应有 todo 状态列");

        场景 {
            project_id: project.id,
            todo_status_id: todo.id,
        }
    }

    fn 建需求请求(场景: &场景, title: &str) -> CreateIssueRequest {
        CreateIssueRequest {
            id: None,
            project_id: 场景.project_id,
            status_id: 场景.todo_status_id,
            title: title.to_string(),
            description: None,
            priority: Some(IssuePriority::Medium),
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
    async fn 需求编号按项目自增且_simple_id_带项目前缀() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let first = Issues::create(test_db.pool(), &建需求请求(&场景, "第一条"))
            .await
            .unwrap();
        let second = Issues::create(test_db.pool(), &建需求请求(&场景, "第二条"))
            .await
            .unwrap();

        assert_eq!(first.issue_number, 1);
        assert_eq!(second.issue_number, 2);
        assert_eq!(first.simple_id, "VK-1");
        assert_eq!(second.simple_id, "VK-2");
    }

    #[tokio::test]
    async fn 两个项目的编号互不干扰() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        let a1 = Issues::create(test_db.pool(), &建需求请求(&a, "A1")).await.unwrap();
        let b1 = Issues::create(test_db.pool(), &建需求请求(&b, "B1")).await.unwrap();

        assert_eq!(a1.issue_number, 1);
        assert_eq!(b1.issue_number, 1);
        assert_eq!(a1.simple_id, "A-1");
        assert_eq!(b1.simple_id, "B-1");
    }

    #[tokio::test]
    async fn 并发建需求不会产生重复编号() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let pool = test_db.pool().clone();

        let mut handles = Vec::new();
        for index in 0..8 {
            let pool = pool.clone();
            let request = 建需求请求(&场景, &format!("并发 {index}"));
            handles.push(tokio::spawn(async move {
                Issues::create(&pool, &request).await
            }));
        }

        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap().expect("并发建需求不应失败").issue_number);
        }
        numbers.sort_unstable();

        assert_eq!(numbers, (1..=8).collect::<Vec<i32>>(), "编号必须连续且唯一");
    }

    #[tokio::test]
    async fn 标题与描述超长时被截断而不是报错() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let mut request = 建需求请求(&场景, &"标".repeat(MAX_TITLE_LEN + 100));
        request.description = Some("描".repeat(MAX_DESCRIPTION_LEN + 100));

        let issue = Issues::create(test_db.pool(), &request).await.unwrap();

        assert_eq!(issue.title.chars().count(), MAX_TITLE_LEN);
        assert_eq!(
            issue.description.as_ref().unwrap().chars().count(),
            MAX_DESCRIPTION_LEN
        );
    }

    #[tokio::test]
    async fn 空标题被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let result = Issues::create(test_db.pool(), &建需求请求(&场景, "   ")).await;
        assert!(result.is_err(), "空白标题必须拒绝");
    }

    #[tokio::test]
    async fn 并发更新不同字段互不覆盖() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "原标题"))
            .await
            .unwrap();

        let pool_a = test_db.pool().clone();
        let pool_b = test_db.pool().clone();
        let id = issue.id;

        let a = tokio::spawn(async move {
            Issues::update(
                &pool_a,
                id,
                &UpdateIssueRequest {
                    title: Some("新标题".to_string()),
                    ..Default::default()
                },
            )
            .await
        });
        let b = tokio::spawn(async move {
            Issues::update(
                &pool_b,
                id,
                &UpdateIssueRequest {
                    priority: Some(Some(IssuePriority::Urgent)),
                    ..Default::default()
                },
            )
            .await
        });

        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let after = Issues::find_by_id(test_db.pool(), id).await.unwrap().unwrap();
        assert_eq!(after.title, "新标题", "并发更新不得丢失标题");
        assert!(
            matches!(after.priority, Some(IssuePriority::Urgent)),
            "并发更新不得丢失优先级"
        );
    }

    #[tokio::test]
    async fn 更新可以把描述显式置空() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let mut request = 建需求请求(&场景, "带描述");
        request.description = Some("正文".to_string());
        let issue = Issues::create(test_db.pool(), &request).await.unwrap();

        let updated = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                description: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.description, None);
    }

    #[tokio::test]
    async fn 批量更新排序在单事务内全成或全败() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "拖拽"))
            .await
            .unwrap();

        let updates = vec![
            (
                issue.id,
                UpdateIssueRequest {
                    sort_order: Some(5.0),
                    ..Default::default()
                },
            ),
            (
                Uuid::from_u128(987654),
                UpdateIssueRequest {
                    sort_order: Some(6.0),
                    ..Default::default()
                },
            ),
        ];

        assert!(Issues::bulk_update(test_db.pool(), &updates).await.is_err());

        let after = Issues::find_by_id(test_db.pool(), issue.id).await.unwrap().unwrap();
        assert_eq!(after.sort_order, 0.0, "失败必须整体回滚");
    }

    #[tokio::test]
    async fn 搜索按标题模糊匹配且转义通配符() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        Issues::create(test_db.pool(), &建需求请求(&场景, "登录页面"))
            .await
            .unwrap();
        Issues::create(test_db.pool(), &建需求请求(&场景, "100%完成度"))
            .await
            .unwrap();

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                search: Some("登录".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hit.issues.len(), 1);
        assert_eq!(hit.total_count, 1);

        // "%" 是 LIKE 通配符，必须被转义，否则会匹配到全部行
        let escaped = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                search: Some("%".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(escaped.issues.len(), 1, "% 必须按字面量匹配");
        assert_eq!(escaped.issues[0].title, "100%完成度");
    }

    #[tokio::test]
    async fn 搜索限制单页上限并返回总数() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        for index in 0..5 {
            Issues::create(test_db.pool(), &建需求请求(&场景, &format!("需求 {index}")))
                .await
                .unwrap();
        }

        let page = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                limit: Some(2),
                offset: Some(1),
                sort_field: Some(IssueSortField::CreatedAt),
                sort_direction: Some(SortDirection::Asc),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(page.issues.len(), 2);
        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, 2);
        assert_eq!(page.offset, 1);
        assert_eq!(page.issues[0].title, "需求 1");

        // 超出上限的 limit 被夹到 MAX_PAGE_SIZE
        let huge = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                limit: Some(100_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(huge.limit, super::MAX_PAGE_SIZE);
    }

    #[tokio::test]
    async fn 按项目列出时不返回其他项目的需求() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        Issues::create(test_db.pool(), &建需求请求(&a, "A1")).await.unwrap();
        Issues::create(test_db.pool(), &建需求请求(&b, "B1")).await.unwrap();

        let list = Issues::find_by_project(test_db.pool(), a.project_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "A1");
    }

    #[tokio::test]
    async fn 移动到已完成阶段会写入完成时间() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "待办"))
            .await
            .unwrap();
        assert!(issue.completed_at.is_none());

        let done = Issues::move_to_stage(test_db.pool(), issue.id, StageType::Done)
            .await
            .unwrap()
            .expect("应完成流转");
        assert!(done.completed_at.is_some());

        let done_status = ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status_id, done_status.id);

        // 再流转回开发中，完成时间必须清空
        let dev = Issues::move_to_stage(test_db.pool(), issue.id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        assert!(dev.completed_at.is_none());
    }

    #[tokio::test]
    async fn 流转不存在的需求返回_none_而不是报错() {
        let test_db = TestDb::new().await;
        let result = Issues::move_to_stage(test_db.pool(), Uuid::from_u128(5555), StageType::Dev)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn 删除需求把工作区的_issue_id_置空() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "要删的"))
            .await
            .unwrap();

        let workspace_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workspaces (id, branch, issue_id) VALUES (?1, 'vk/test', ?2)",
        )
        .bind(workspace_id)
        .bind(issue.id)
        .execute(test_db.pool())
        .await
        .expect("插入工作区失败");

        Issues::delete(test_db.pool(), issue.id).await.unwrap();

        let linked: (Option<Uuid>,) =
            sqlx::query_as("SELECT issue_id FROM workspaces WHERE id = ?1")
                .bind(workspace_id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert!(linked.0.is_none(), "删除需求后工作区必须解绑而不是被删除");
    }
}
