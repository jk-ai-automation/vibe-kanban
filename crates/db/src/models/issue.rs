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
    db_retry::{RetryableDbError, is_retryable_db_error, retry_on_busy},
    local_project::LocalProjects,
    local_project_status::{ProjectStatuses, StageType},
};

/// 标题上限，超出部分截断（防止超大字段撑爆快照与 WS 推送）。
pub const MAX_TITLE_LEN: usize = 500;
/// 描述上限，超出部分截断。
pub const MAX_DESCRIPTION_LEN: usize = 100_000;
/// 单次搜索返回上限，防止前端一次拉取过多行。
pub const MAX_PAGE_SIZE: usize = 500;
/// extension_metadata 序列化后的字节上限，防止单行撑爆快照与 WS 推送。
pub const MAX_METADATA_BYTES: usize = 32_768;
/// 搜索里 IN (...) 能接受的 id 个数上限。SQLite 默认最多 999 个绑定变量，
/// 不设上限时超长筛选数组会直接把查询打成 500。
pub const MAX_FILTER_IDS: usize = 200;

/// 父需求链的最大层数，同时用作成环检测的兜底深度。
pub const MAX_PARENT_DEPTH: i64 = 32;

/// extension_metadata 必须是 JSON 对象或 null，且序列化后不超过 [`MAX_METADATA_BYTES`]。
///
/// `null` 按「没有扩展字段」处理，落库成 `{}`：前端新建需求时发的就是
/// `extension_metadata: null`（见 KanbanIssuePanelContainer 的提交逻辑），
/// 若按非对象拒绝，个人版会完全无法新建需求。
fn validate_metadata(value: &Value) -> Result<String, IssueError> {
    let text = match value {
        Value::Null => "{}".to_string(),
        Value::Object(_) => value.to_string(),
        _ => {
            return Err(IssueError::Validation(
                "extension_metadata 必须是 JSON 对象".to_string(),
            ));
        }
    };
    if text.len() > MAX_METADATA_BYTES {
        return Err(IssueError::Validation(format!(
            "extension_metadata 超过 {MAX_METADATA_BYTES} 字节上限"
        )));
    }
    Ok(text)
}

#[derive(Debug, Error)]
pub enum IssueError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    Validation(String),
    /// 唯一约束/主键冲突这类「请求本身没错、但和已有数据撞了」的情况。
    #[error("Conflict: {0}")]
    Conflict(String),
}

impl RetryableDbError for IssueError {
    fn is_busy(&self) -> bool {
        match self {
            IssueError::Database(err) => is_retryable_db_error(err),
            IssueError::Validation(_) | IssueError::Conflict(_) => false,
        }
    }
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

/// 标题：去空白后校验长度。超长直接拒绝，不再静默截断
/// （截断会让「保存成功但内容被悄悄改掉」，客户端无从察觉）。
fn validate_title(raw: &str) -> Result<String, IssueError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(IssueError::Validation("需求标题不能为空".to_string()));
    }
    if trimmed.chars().count() > MAX_TITLE_LEN {
        return Err(IssueError::Validation(format!(
            "需求标题超过 {MAX_TITLE_LEN} 字上限"
        )));
    }
    Ok(trimmed.to_string())
}

/// 描述：超长同样拒绝而不是截断。
fn validate_description(raw: &str) -> Result<String, IssueError> {
    if raw.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(IssueError::Validation(format!(
            "需求描述超过 {MAX_DESCRIPTION_LEN} 字上限"
        )));
    }
    Ok(raw.to_string())
}

/// 优先级的业务权重：数据库里存的是字符串，直接 ORDER BY 会按字典序
/// （high < low < medium < urgent），必须先映射成序号。
const PRIORITY_RANK_SQL: &str = "CASE priority \
     WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END";

/// 搜索里按需拼接条件时用到的绑定值。拼进 SQL 的只有占位符编号，值一律走绑定。
enum Bind {
    Uuid(Uuid),
    Text(String),
    Int(i64),
}

impl Bind {
    fn apply<'q, O>(
        &'q self,
        query: sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'q>>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'q>> {
        match self {
            Bind::Uuid(value) => query.bind(*value),
            Bind::Text(value) => query.bind(value.as_str()),
            Bind::Int(value) => query.bind(*value),
        }
    }
}

/// 转义 LIKE 通配符，配合 SQL 中的 ESCAPE '\' 使用。
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 按项目取快照的结果。`truncated` 为真表示行数触到 [`MAX_SNAPSHOT_ROWS`]，
/// 调用方（路由层）必须把这一事实透出去，不能静默截断。
#[derive(Debug)]
pub struct IssueSnapshot {
    pub issues: Vec<Issue>,
    pub truncated: bool,
}

pub struct Issues;

impl Issues {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<IssueSnapshot, sqlx::Error> {
        // 多取一行用来判断是否被截断，返回前丢掉。
        let probe_limit = MAX_SNAPSHOT_ROWS + 1;
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
            probe_limit
        )
        .fetch_all(pool)
        .await?;

        let truncated = rows.len() as i64 > MAX_SNAPSHOT_ROWS;
        let issues: Vec<Issue> = rows
            .into_iter()
            .take(MAX_SNAPSHOT_ROWS as usize)
            .map(Issue::from)
            .collect();

        Ok(IssueSnapshot { issues, truncated })
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
        creator_user_id: Uuid,
    ) -> Result<Issue, IssueError> {
        let title = validate_title(&data.title)?;
        let description = data
            .description
            .as_ref()
            .map(|d| validate_description(d))
            .transpose()?;

        if LocalProjects::find_by_id(pool, data.project_id)
            .await?
            .is_none()
        {
            return Err(IssueError::Validation("项目不存在".to_string()));
        }

        let status = ProjectStatuses::find_by_id(pool, data.status_id).await?;
        match status {
            Some(status) if status.project_id == data.project_id => {}
            Some(_) => {
                return Err(IssueError::Validation("状态列不属于该项目".to_string()));
            }
            None => return Err(IssueError::Validation("状态列不存在".to_string())),
        }

        // 父需求必须存在且同项目，否则会形成跨项目引用。
        if let Some(parent_id) = data.parent_issue_id {
            match Self::find_by_id(pool, parent_id).await? {
                Some(parent) if parent.project_id == data.project_id => {}
                Some(_) => {
                    return Err(IssueError::Validation("父需求不属于该项目".to_string()));
                }
                None => return Err(IssueError::Validation("父需求不存在".to_string())),
            }
        }

        let id = data.id.unwrap_or_else(Uuid::new_v4);
        if data.parent_issue_id == Some(id) {
            return Err(IssueError::Validation(
                "需求不能把自己当作父需求".to_string(),
            ));
        }
        let priority = data.priority.map(priority_to_str);
        let metadata = validate_metadata(&data.extension_metadata)?;
        let now = Utc::now();

        retry_on_busy(|| async {
            let mut tx = pool.begin().await?;

            // 编号从项目行上的游标取，取完即自增：删除末条需求后编号不会被复用。
            // 前缀也从项目行读，建项目时就定死，之后改名不影响任何需求。
            let (prefix, next): (String, i64) = sqlx::query_as(
                "UPDATE local_projects SET next_issue_number = next_issue_number + 1 \
                 WHERE id = ?1 RETURNING simple_id_prefix, next_issue_number - 1",
            )
            .bind(data.project_id)
            .fetch_one(&mut *tx)
            .await?;
            let issue_number = next as i32;
            let simple_id = format!("{prefix}-{issue_number}");

            // 新需求还没有子需求，成不了环，但父链本身可能已经太深。
            if let Some(parent_id) = data.parent_issue_id {
                Self::ensure_no_parent_cycle(&mut tx, id, parent_id).await?;
            }

            let inserted = sqlx::query_as!(
                IssueRow,
                r#"INSERT INTO issues
                       (id, project_id, issue_number, simple_id, status_id, title, description,
                        priority, start_date, target_date, completed_at, sort_order,
                        parent_issue_id, parent_issue_sort_order, extension_metadata,
                        creator_user_id, created_at, updated_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                           $17, $17)
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
                creator_user_id,
                now
            )
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(Issue::from(inserted))
        })
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> Result<Issue, IssueError> {
        retry_on_busy(|| async {
            let mut tx = pool.begin().await?;
            let updated = Self::update_in_tx(&mut tx, id, data).await?;
            tx.commit().await?;
            Ok(updated)
        })
        .await
    }

    /// 拖拽排序等批量写入：单事务，任一条失败整体回滚。
    pub async fn bulk_update(
        pool: &SqlitePool,
        updates: &[(Uuid, UpdateIssueRequest)],
    ) -> Result<Vec<Issue>, IssueError> {
        retry_on_busy(|| async {
            let mut tx = pool.begin().await?;
            let mut rows = Vec::with_capacity(updates.len());
            for (id, data) in updates {
                rows.push(Self::update_in_tx(&mut tx, *id, data).await?);
            }
            tx.commit().await?;
            Ok(rows)
        })
        .await
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
            Some(raw) => Some(validate_title(raw)?),
            None => None,
        };
        let set_description = data.description.is_some();
        let description: Option<String> = match data.description.clone().flatten() {
            Some(raw) => Some(validate_description(&raw)?),
            None => None,
        };
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
        let metadata = match data.extension_metadata.as_ref() {
            Some(value) => Some(validate_metadata(value)?),
            None => None,
        };

        // 成环检查：把 P 设成 I 的父需求前，确认 I 不在 P 的祖先链上，
        // 否则前端渲染子需求树时会无限递归。
        if let Some(parent_id) = parent_issue_id {
            Self::ensure_no_parent_cycle(tx, id, parent_id).await?;
        }

        let now = Utc::now();

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
                   updated_at              = $24
               WHERE id = $1
                 -- 越权校验直接写进 WHERE：不需要先读后写，避免并发更新时
                 -- 两个事务各持读锁、再同时申请写锁而互相锁死。
                 AND ($2 = 0 OR EXISTS (
                         SELECT 1 FROM project_statuses s
                         WHERE s.id = $3 AND s.project_id = issues.project_id))
                 AND ($18 = 0 OR $19 IS NULL OR ($19 <> $1 AND EXISTS (
                         SELECT 1 FROM issues p
                         WHERE p.id = $19 AND p.project_id = issues.project_id)))
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
            metadata,
            now
        )
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(row) => Ok(Issue::from(row)),
            // 打不中行：可能是需求不存在，也可能是越权。这条路径只在出错时走，
            // 多几次查询不影响正常写入的并发。
            None => Err(Self::diagnose_update_failure(tx, id, data).await),
        }
    }

    /// 沿 `parent_id` 往上走祖先链，确认 `id` 不在链上，且链长不超过
    /// [`MAX_PARENT_DEPTH`]。递归 CTE 自带深度上限，即使库里已经有环也会停下来。
    async fn ensure_no_parent_cycle(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: Uuid,
        parent_id: Uuid,
    ) -> Result<(), IssueError> {
        if parent_id == id {
            return Err(IssueError::Validation(
                "需求不能把自己当作父需求".to_string(),
            ));
        }

        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "WITH RECURSIVE 祖先(id, depth) AS ( \
                 SELECT ?1, 0 \
                 UNION ALL \
                 SELECT i.parent_issue_id, 祖先.depth + 1 \
                 FROM issues i JOIN 祖先 ON i.id = 祖先.id \
                 WHERE i.parent_issue_id IS NOT NULL AND 祖先.depth < ?3 \
             ) SELECT id, depth FROM 祖先 WHERE id = ?2 OR depth >= ?3 LIMIT 1",
        )
        .bind(parent_id)
        .bind(id)
        .bind(MAX_PARENT_DEPTH)
        .fetch_all(&mut **tx)
        .await?;

        match rows.first() {
            Some((hit, _)) if *hit == id => Err(IssueError::Validation(
                "父需求会形成环，请换一个父需求".to_string(),
            )),
            Some(_) => Err(IssueError::Validation(format!(
                "父需求层级超过 {MAX_PARENT_DEPTH} 层上限"
            ))),
            None => Ok(()),
        }
    }

    /// UPDATE 打不中行时，查清到底是「需求不存在」还是「越权」，给出具体原因。
    async fn diagnose_update_failure(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> IssueError {
        let owner: Result<Option<(Uuid,)>, sqlx::Error> =
            sqlx::query_as("SELECT project_id FROM issues WHERE id = ?1")
                .bind(id)
                .fetch_optional(&mut **tx)
                .await;
        let project_id = match owner {
            Ok(Some((project_id,))) => project_id,
            Ok(None) => return IssueError::Database(sqlx::Error::RowNotFound),
            Err(err) => return IssueError::Database(err),
        };

        if let Some(status_id) = data.status_id {
            let owns: Result<Option<(i64,)>, sqlx::Error> =
                sqlx::query_as("SELECT 1 FROM project_statuses WHERE id = ?1 AND project_id = ?2")
                    .bind(status_id)
                    .bind(project_id)
                    .fetch_optional(&mut **tx)
                    .await;
            match owns {
                Ok(None) => {
                    return IssueError::Validation("状态列不存在或不属于该项目".to_string());
                }
                Err(err) => return IssueError::Database(err),
                Ok(Some(_)) => {}
            }
        }

        if let Some(Some(parent_id)) = data.parent_issue_id {
            if parent_id == id {
                return IssueError::Validation("需求不能把自己当作父需求".to_string());
            }
            let owns: Result<Option<(i64,)>, sqlx::Error> =
                sqlx::query_as("SELECT 1 FROM issues WHERE id = ?1 AND project_id = ?2")
                    .bind(parent_id)
                    .bind(project_id)
                    .fetch_optional(&mut **tx)
                    .await;
            match owns {
                Ok(None) => {
                    return IssueError::Validation("父需求不存在或不属于该项目".to_string());
                }
                Err(err) => return IssueError::Database(err),
                Ok(Some(_)) => {}
            }
        }

        IssueError::Database(sqlx::Error::RowNotFound)
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
    ) -> Result<ListIssuesResponse, IssueError> {
        // 筛选数组直接展开成 IN (...) 的绑定变量，不设上限会撞上 SQLite 的
        // 变量个数上限（默认 999），那时报的是 500 而不是「参数不合法」。
        for (name, len) in [
            (
                "status_ids",
                request.status_ids.as_ref().map_or(0, Vec::len),
            ),
            ("tag_ids", request.tag_ids.as_ref().map_or(0, Vec::len)),
        ] {
            if len > MAX_FILTER_IDS {
                return Err(IssueError::Validation(format!(
                    "{name} 最多 {MAX_FILTER_IDS} 项，实际 {len} 项"
                )));
            }
        }

        let limit = request
            .limit
            .map(|l| (l.max(1) as usize).min(MAX_PAGE_SIZE))
            .unwrap_or(MAX_PAGE_SIZE);
        let offset = request.offset.map(|o| o.max(0) as usize).unwrap_or(0);

        // 条件按需拼接：拼进去的只有占位符编号，值一律走绑定，没有 SQL 注入面。
        // 之所以不用 json_each 传数组：uuid 在 SQLite 里是 BLOB，json_each 只能给出
        // 文本，比对需要额外的 hex/unhex 转换且用不上索引。
        let mut conditions: Vec<String> = Vec::new();
        let mut binds: Vec<Bind> = Vec::new();

        binds.push(Bind::Uuid(request.project_id));
        conditions.push(format!("project_id = ?{}", binds.len()));

        if let Some(pattern) = request
            .search
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!("%{}%", escape_like(s)))
        {
            binds.push(Bind::Text(pattern));
            let n = binds.len();
            conditions.push(format!(
                "(title LIKE ?{n} ESCAPE '\\' OR IFNULL(description, '') LIKE ?{n} ESCAPE '\\')"
            ));
        }

        if let Some(status_id) = request.status_id {
            binds.push(Bind::Uuid(status_id));
            conditions.push(format!("status_id = ?{}", binds.len()));
        }

        // 空数组视为「不筛选」，避免前端清空筛选器时误把结果清空。
        if let Some(status_ids) = request.status_ids.as_ref().filter(|ids| !ids.is_empty()) {
            let mut slots = Vec::with_capacity(status_ids.len());
            for status_id in status_ids {
                binds.push(Bind::Uuid(*status_id));
                slots.push(format!("?{}", binds.len()));
            }
            conditions.push(format!("status_id IN ({})", slots.join(", ")));
        }

        if let Some(priority) = request.priority.map(priority_to_str) {
            binds.push(Bind::Text(priority.to_string()));
            conditions.push(format!("priority = ?{}", binds.len()));
        }

        if let Some(simple_id) = request.simple_id.clone() {
            binds.push(Bind::Text(simple_id));
            conditions.push(format!("simple_id = ?{}", binds.len()));
        }

        // tag_id（单数）等价于 tag_ids: [id]，两者同时出现时取并集。
        let tag_ids: Vec<Uuid> = request
            .tag_id
            .into_iter()
            .chain(request.tag_ids.iter().flatten().copied())
            .collect();
        if !tag_ids.is_empty() {
            let mut slots = Vec::with_capacity(tag_ids.len());
            for tag_id in &tag_ids {
                binds.push(Bind::Uuid(*tag_id));
                slots.push(format!("?{}", binds.len()));
            }
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM issue_tags it \
                 WHERE it.issue_id = issues.id AND it.tag_id IN ({}))",
                slots.join(", ")
            ));
        }

        let where_sql = conditions.join(" AND ");

        let count_sql = format!("SELECT COUNT(*) FROM issues WHERE {where_sql}");
        let mut count_query = sqlx::query_as::<_, (i64,)>(&count_sql);
        for bind in &binds {
            count_query = bind.apply(count_query);
        }
        let total = count_query.fetch_one(pool).await?;

        // 排序字段来自枚举，不来自用户输入的字符串，因此不存在 SQL 注入面。
        let ascending = !matches!(request.sort_direction, Some(SortDirection::Desc));
        let sort_field = request.sort_field.unwrap_or(IssueSortField::SortOrder);
        let order_sql = match (sort_field, ascending) {
            (IssueSortField::SortOrder, true) => "sort_order ASC, created_at ASC".to_string(),
            (IssueSortField::SortOrder, false) => "sort_order DESC, created_at DESC".to_string(),
            // priority 存的是字符串，字典序会把 urgent 排到最后，必须按业务权重排。
            (IssueSortField::Priority, true) => {
                format!("{PRIORITY_RANK_SQL} ASC, created_at ASC")
            }
            (IssueSortField::Priority, false) => {
                format!("{PRIORITY_RANK_SQL} DESC, created_at DESC")
            }
            (IssueSortField::CreatedAt, true) => "created_at ASC".to_string(),
            (IssueSortField::CreatedAt, false) => "created_at DESC".to_string(),
            (IssueSortField::UpdatedAt, true) => "updated_at ASC, created_at ASC".to_string(),
            (IssueSortField::UpdatedAt, false) => "updated_at DESC, created_at DESC".to_string(),
            (IssueSortField::Title, true) => "title ASC, created_at ASC".to_string(),
            (IssueSortField::Title, false) => "title DESC, created_at DESC".to_string(),
        };

        binds.push(Bind::Int(limit as i64));
        let limit_slot = binds.len();
        binds.push(Bind::Int(offset as i64));
        let offset_slot = binds.len();

        let page_sql = format!(
            r#"SELECT id, project_id, issue_number, simple_id, status_id, title, description,
                      priority, start_date, target_date, completed_at, sort_order,
                      parent_issue_id, parent_issue_sort_order, extension_metadata,
                      creator_user_id, created_at, updated_at
               FROM issues
               WHERE {where_sql}
               ORDER BY {order_sql}
               LIMIT ?{limit_slot} OFFSET ?{offset_slot}"#
        );

        let mut page_query = sqlx::query_as::<_, IssueSqlRow>(&page_sql);
        for bind in &binds {
            page_query = bind.apply(page_query);
        }
        let rows = page_query.fetch_all(pool).await?;

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

    use super::{Issues, MAX_DESCRIPTION_LEN, MAX_METADATA_BYTES, MAX_TITLE_LEN};
    use crate::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
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
    async fn 创建人取自参数而不是固定用户() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let creator = Uuid::from_u128(999);
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "指定创建人"), creator)
            .await
            .unwrap();

        assert_eq!(issue.creator_user_id, Some(creator));
    }

    #[tokio::test]
    async fn 创建人为默认用户时与旧行为一致() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let issue = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "默认创建人"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        assert_eq!(issue.creator_user_id, Some(DEFAULT_USER_ID));
    }

    #[tokio::test]
    async fn 需求编号按项目自增且_simple_id_带项目前缀() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let first = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "第一条"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let second = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "第二条"),
            DEFAULT_USER_ID,
        )
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

        let a1 = Issues::create(test_db.pool(), &建需求请求(&a, "A1"), DEFAULT_USER_ID)
            .await
            .unwrap();
        let b1 = Issues::create(test_db.pool(), &建需求请求(&b, "B1"), DEFAULT_USER_ID)
            .await
            .unwrap();

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
                Issues::create(&pool, &request, DEFAULT_USER_ID).await
            }));
        }

        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(
                handle
                    .await
                    .unwrap()
                    .expect("并发建需求不应失败")
                    .issue_number,
            );
        }
        numbers.sort_unstable();

        assert_eq!(numbers, (1..=8).collect::<Vec<i32>>(), "编号必须连续且唯一");
    }

    /// 超长字段一律拒绝：截断会让「保存成功但内容被悄悄改掉」，
    /// 客户端拿不到任何信号，下一次编辑还会把截断后的内容当成原文。
    #[tokio::test]
    async fn 标题与描述超长时被拒绝而不是静默截断() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let mut 超长标题 = 建需求请求(&场景, &"标".repeat(MAX_TITLE_LEN + 1));
        assert!(matches!(
            Issues::create(test_db.pool(), &超长标题, DEFAULT_USER_ID).await,
            Err(super::IssueError::Validation(_))
        ));

        超长标题.title = "正常标题".to_string();
        超长标题.description = Some("描".repeat(MAX_DESCRIPTION_LEN + 1));
        assert!(matches!(
            Issues::create(test_db.pool(), &超长标题, DEFAULT_USER_ID).await,
            Err(super::IssueError::Validation(_))
        ));

        // 恰好卡在上限可以通过，且内容一字不改
        let mut 刚好 = 建需求请求(&场景, &"标".repeat(MAX_TITLE_LEN));
        刚好.description = Some("描".repeat(MAX_DESCRIPTION_LEN));
        let issue = Issues::create(test_db.pool(), &刚好, DEFAULT_USER_ID)
            .await
            .unwrap();
        assert_eq!(issue.title.chars().count(), MAX_TITLE_LEN);
        assert_eq!(
            issue.description.as_ref().unwrap().chars().count(),
            MAX_DESCRIPTION_LEN
        );

        // 更新路径同样拒绝
        assert!(matches!(
            Issues::update(
                test_db.pool(),
                issue.id,
                &UpdateIssueRequest {
                    title: Some("标".repeat(MAX_TITLE_LEN + 1)),
                    ..Default::default()
                },
            )
            .await,
            Err(super::IssueError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn 空标题被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let result =
            Issues::create(test_db.pool(), &建需求请求(&场景, "   "), DEFAULT_USER_ID).await;
        assert!(result.is_err(), "空白标题必须拒绝");
    }

    #[tokio::test]
    async fn 并发更新不同字段互不覆盖() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "原标题"),
            DEFAULT_USER_ID,
        )
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

        let after = Issues::find_by_id(test_db.pool(), id)
            .await
            .unwrap()
            .unwrap();
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
        let issue = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .unwrap();

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
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "拖拽"), DEFAULT_USER_ID)
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

        let after = Issues::find_by_id(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.sort_order, 0.0, "失败必须整体回滚");
    }

    #[tokio::test]
    async fn 搜索按标题模糊匹配且转义通配符() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "登录页面"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "100%完成度"),
            DEFAULT_USER_ID,
        )
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
            Issues::create(
                test_db.pool(),
                &建需求请求(&场景, &format!("需求 {index}")),
                DEFAULT_USER_ID,
            )
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

        Issues::create(test_db.pool(), &建需求请求(&a, "A1"), DEFAULT_USER_ID)
            .await
            .unwrap();
        Issues::create(test_db.pool(), &建需求请求(&b, "B1"), DEFAULT_USER_ID)
            .await
            .unwrap();

        let list = Issues::find_by_project(test_db.pool(), a.project_id)
            .await
            .unwrap();
        assert_eq!(list.issues.len(), 1);
        assert_eq!(list.issues[0].title, "A1");
        assert!(!list.truncated);
    }

    #[tokio::test]
    async fn 移动到已完成阶段会写入完成时间() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "待办"), DEFAULT_USER_ID)
            .await
            .unwrap();
        assert!(issue.completed_at.is_none());

        let done = Issues::move_to_stage(test_db.pool(), issue.id, StageType::Done)
            .await
            .unwrap()
            .expect("应完成流转");
        assert!(done.completed_at.is_some());

        let done_status =
            ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Done)
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
        let issue = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "要删的"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let workspace_id = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces (id, branch, issue_id) VALUES (?1, 'vk/test', ?2)")
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

    // ---- 跨项目越权 ----

    #[tokio::test]
    async fn 建需求时跨项目的父需求被拒绝() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        let parent = Issues::create(
            test_db.pool(),
            &建需求请求(&b, "别的项目的父需求"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let mut request = 建需求请求(&a, "子需求");
        request.parent_issue_id = Some(parent.id);

        let err = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .expect_err("跨项目父需求必须拒绝");
        assert!(
            matches!(err, super::IssueError::Validation(_)),
            "应是校验错误：{err:?}"
        );
    }

    #[tokio::test]
    async fn 建需求时不存在的父需求被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let mut request = 建需求请求(&场景, "子需求");
        request.parent_issue_id = Some(Uuid::from_u128(424242));

        let err = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .expect_err("父需求不存在必须拒绝");
        assert!(matches!(err, super::IssueError::Validation(_)));
    }

    #[tokio::test]
    async fn 建需求时把自己当父需求被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let id = Uuid::new_v4();
        let mut request = 建需求请求(&场景, "自己当爹");
        request.id = Some(id);
        request.parent_issue_id = Some(id);

        let err = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .expect_err("自引用必须拒绝");
        assert!(matches!(err, super::IssueError::Validation(_)));
    }

    #[tokio::test]
    async fn 更新时跨项目的状态列被拒绝() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        let issue = Issues::create(test_db.pool(), &建需求请求(&a, "A1"), DEFAULT_USER_ID)
            .await
            .unwrap();

        let err = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                status_id: Some(b.todo_status_id),
                ..Default::default()
            },
        )
        .await
        .expect_err("跨项目状态列必须拒绝");
        assert!(
            matches!(err, super::IssueError::Validation(_)),
            "应是校验错误：{err:?}"
        );

        let after = Issues::find_by_id(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status_id, a.todo_status_id, "越权更新不得落库");
    }

    #[tokio::test]
    async fn 更新时跨项目的父需求被拒绝() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        let issue = Issues::create(test_db.pool(), &建需求请求(&a, "A1"), DEFAULT_USER_ID)
            .await
            .unwrap();
        let parent = Issues::create(test_db.pool(), &建需求请求(&b, "B1"), DEFAULT_USER_ID)
            .await
            .unwrap();

        let err = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                parent_issue_id: Some(Some(parent.id)),
                ..Default::default()
            },
        )
        .await
        .expect_err("跨项目父需求必须拒绝");
        assert!(matches!(err, super::IssueError::Validation(_)));

        let after = Issues::find_by_id(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.parent_issue_id, None);
    }

    #[tokio::test]
    async fn 更新时把自己当父需求被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "A1"), DEFAULT_USER_ID)
            .await
            .unwrap();

        let err = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                parent_issue_id: Some(Some(issue.id)),
                ..Default::default()
            },
        )
        .await
        .expect_err("自引用必须拒绝");
        assert!(matches!(err, super::IssueError::Validation(_)));
    }

    #[tokio::test]
    async fn 更新同项目的状态列与父需求可以通过() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let parent = Issues::create(test_db.pool(), &建需求请求(&场景, "父"), DEFAULT_USER_ID)
            .await
            .unwrap();
        let child = Issues::create(test_db.pool(), &建需求请求(&场景, "子"), DEFAULT_USER_ID)
            .await
            .unwrap();
        let dev = ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();

        let updated = Issues::update(
            test_db.pool(),
            child.id,
            &UpdateIssueRequest {
                status_id: Some(dev.id),
                parent_issue_id: Some(Some(parent.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.status_id, dev.id);
        assert_eq!(updated.parent_issue_id, Some(parent.id));
    }

    #[tokio::test]
    async fn 更新不存在的需求返回_row_not_found() {
        let test_db = TestDb::new().await;
        let err = Issues::update(
            test_db.pool(),
            Uuid::from_u128(987123),
            &UpdateIssueRequest {
                title: Some("新".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("不存在的需求必须报错");
        assert!(
            matches!(err, super::IssueError::Database(sqlx::Error::RowNotFound)),
            "应是 RowNotFound：{err:?}"
        );
    }

    // ---- extension_metadata 上限 ----

    #[tokio::test]
    async fn 超大的_metadata_被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let huge = "x".repeat(MAX_METADATA_BYTES + 10);
        let mut request = 建需求请求(&场景, "超大");
        request.extension_metadata = serde_json::json!({ "blob": huge });

        assert!(matches!(
            Issues::create(test_db.pool(), &request, DEFAULT_USER_ID).await,
            Err(super::IssueError::Validation(_))
        ));

        let ok = Issues::create(test_db.pool(), &建需求请求(&场景, "正常"), DEFAULT_USER_ID)
            .await
            .unwrap();
        assert!(matches!(
            Issues::update(
                test_db.pool(),
                ok.id,
                &UpdateIssueRequest {
                    extension_metadata: Some(
                        serde_json::json!({ "blob": "x".repeat(MAX_METADATA_BYTES + 10) })
                    ),
                    ..Default::default()
                },
            )
            .await,
            Err(super::IssueError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn 非对象的_metadata_被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        for bad in [
            serde_json::json!("字符串"),
            serde_json::json!([1, 2, 3]),
            serde_json::json!(42),
        ] {
            let mut request = 建需求请求(&场景, "非对象");
            request.extension_metadata = bad.clone();
            assert!(
                matches!(
                    Issues::create(test_db.pool(), &request, DEFAULT_USER_ID).await,
                    Err(super::IssueError::Validation(_))
                ),
                "metadata {bad} 必须被拒绝"
            );
        }
    }

    #[tokio::test]
    async fn null_的_metadata_按空对象处理() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let mut request = 建需求请求(&场景, "前端默认值");
        request.extension_metadata = serde_json::Value::Null;
        let issue = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .expect("null 必须按空对象接受");
        assert_eq!(issue.extension_metadata, serde_json::json!({}));

        let updated = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                extension_metadata: Some(serde_json::Value::Null),
                ..Default::default()
            },
        )
        .await
        .expect("更新时的 null 也必须接受");
        assert_eq!(updated.extension_metadata, serde_json::json!({}));
    }

    #[tokio::test]
    async fn 恰好卡在上限的_metadata_可以通过() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        // {"blob":"<padding>"} 的固定开销是 11 字节
        let padding = "x".repeat(MAX_METADATA_BYTES - 11);
        let mut request = 建需求请求(&场景, "刚好");
        request.extension_metadata = serde_json::json!({ "blob": padding });
        assert_eq!(
            request.extension_metadata.to_string().len(),
            MAX_METADATA_BYTES
        );

        let issue = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .unwrap();
        assert_eq!(
            issue.extension_metadata["blob"].as_str().unwrap().len(),
            MAX_METADATA_BYTES - 11
        );
    }

    // ---- 优先级排序 ----

    async fn 按优先级排序(
        test_db: &TestDb,
        场景: &场景,
        direction: SortDirection,
    ) -> Vec<String> {
        Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                sort_field: Some(IssueSortField::Priority),
                sort_direction: Some(direction),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .issues
        .into_iter()
        .map(|issue| issue.title)
        .collect()
    }

    #[tokio::test]
    async fn 优先级排序按业务权重而不是字典序() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        for (title, priority) in [
            ("低", Some(IssuePriority::Low)),
            ("紧急", Some(IssuePriority::Urgent)),
            ("中", Some(IssuePriority::Medium)),
            ("无", None),
            ("高", Some(IssuePriority::High)),
        ] {
            let mut request = 建需求请求(&场景, title);
            request.priority = priority;
            Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
                .await
                .unwrap();
        }

        assert_eq!(
            按优先级排序(&test_db, &场景, SortDirection::Asc).await,
            vec!["紧急", "高", "中", "低", "无"],
            "升序必须是 urgent > high > medium > low > 无"
        );
        assert_eq!(
            按优先级排序(&test_db, &场景, SortDirection::Desc).await,
            vec!["无", "低", "中", "高", "紧急"],
            "降序必须完全反过来"
        );
    }

    // ---- 编号发放 ----

    #[tokio::test]
    async fn 删除末条需求后编号不复用() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let first = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "第一条"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let second = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "第二条"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(second.issue_number, 2);

        Issues::delete(test_db.pool(), second.id).await.unwrap();

        let third = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "第三条"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(third.issue_number, 3, "编号不得复用已删除的 2");
        assert_eq!(third.simple_id, "VK-3");
        assert_eq!(first.issue_number, 1);
    }

    #[tokio::test]
    async fn 项目改名后前缀保持不变() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let before = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "改名前"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(before.simple_id, "VK-1");

        LocalProjects::update(
            test_db.pool(),
            场景.project_id,
            &api_types::project::UpdateProjectRequest {
                name: Some("Zebra Quartz".to_string()),
                color: None,
                sort_order: None,
            },
        )
        .await
        .unwrap();

        let after = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "改名后"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(after.simple_id, "VK-2", "改名不得让新需求的前缀漂移");

        let stored = Issues::find_by_id(test_db.pool(), before.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.simple_id, "VK-1",
            "已有需求的 simple_id 不受改名影响"
        );
    }

    #[tokio::test]
    async fn 三轮三十二并发建需求编号唯一且连续() {
        for round in 0..3 {
            let test_db = TestDb::new().await;
            let 场景 = 准备(&test_db, "Vibe Kanban").await;
            let pool = test_db.pool().clone();

            let mut handles = Vec::new();
            for index in 0..32 {
                let pool = pool.clone();
                let request = 建需求请求(&场景, &format!("并发 {round}-{index}"));
                handles.push(tokio::spawn(async move {
                    Issues::create(&pool, &request, DEFAULT_USER_ID).await
                }));
            }

            let mut numbers = Vec::new();
            for handle in handles {
                numbers.push(
                    handle
                        .await
                        .unwrap()
                        .expect("并发建需求不应失败")
                        .issue_number,
                );
            }
            numbers.sort_unstable();

            assert_eq!(
                numbers,
                (1..=32).collect::<Vec<i32>>(),
                "第 {round} 轮：32 并发的编号必须连续且唯一"
            );
        }
    }

    // ---- 快照截断 ----

    #[tokio::test]
    async fn 快照触到上限时返回截断标记() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let total = super::MAX_SNAPSHOT_ROWS + 5;
        let mut tx = test_db.pool().begin().await.unwrap();
        for index in 0..total {
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title, \
                 sort_order, extension_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            )
            .bind(Uuid::new_v4())
            .bind(场景.project_id)
            .bind(index + 1)
            .bind(format!("VK-{}", index + 1))
            .bind(场景.todo_status_id)
            .bind(format!("批量 {index}"))
            .bind(index as f64)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let snapshot = Issues::find_by_project(test_db.pool(), 场景.project_id)
            .await
            .unwrap();
        assert_eq!(snapshot.issues.len(), super::MAX_SNAPSHOT_ROWS as usize);
        assert!(snapshot.truncated, "触到上限必须让调用方感知");

        // 刚好等于上限时不算截断
        sqlx::query("DELETE FROM issues WHERE issue_number > ?1")
            .bind(super::MAX_SNAPSHOT_ROWS)
            .execute(test_db.pool())
            .await
            .unwrap();
        let exact = Issues::find_by_project(test_db.pool(), 场景.project_id)
            .await
            .unwrap();
        assert_eq!(exact.issues.len(), super::MAX_SNAPSHOT_ROWS as usize);
        assert!(!exact.truncated, "恰好等于上限不算截断");
    }

    // ---- 搜索筛选 ----

    #[tokio::test]
    async fn 搜索按_status_ids_过滤() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let dev = ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        let done = ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();

        Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "待开发"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let mut in_dev = 建需求请求(&场景, "开发中");
        in_dev.status_id = dev.id;
        Issues::create(test_db.pool(), &in_dev, DEFAULT_USER_ID)
            .await
            .unwrap();
        let mut finished = 建需求请求(&场景, "已完成");
        finished.status_id = done.id;
        Issues::create(test_db.pool(), &finished, DEFAULT_USER_ID)
            .await
            .unwrap();

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                status_ids: Some(vec![dev.id, done.id]),
                sort_field: Some(IssueSortField::CreatedAt),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut titles: Vec<String> = hit.issues.into_iter().map(|i| i.title).collect();
        titles.sort();
        assert_eq!(titles, vec!["已完成".to_string(), "开发中".to_string()]);
        assert_eq!(hit.total_count, 2, "总数必须也按筛选条件算");

        // 空数组视为不筛选
        let all = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                status_ids: Some(vec![]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(all.total_count, 3);
    }

    #[tokio::test]
    async fn 搜索按_tag_ids_过滤() {
        use api_types::{issue_tag::CreateIssueTagRequest, tag::CreateTagRequest};

        use crate::models::issue_side::{IssueTags, ProjectTags};

        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let 前端 = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id: 场景.project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        let 后端 = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id: 场景.project_id,
                name: "后端".to_string(),
                color: "#3b82f6".to_string(),
            },
        )
        .await
        .unwrap();

        let a = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "带前端标签"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let b = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "带后端标签"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "没有标签"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        for (issue_id, tag_id) in [(a.id, 前端.id), (b.id, 后端.id)] {
            IssueTags::create(
                test_db.pool(),
                &CreateIssueTagRequest {
                    id: None,
                    issue_id,
                    tag_id,
                },
            )
            .await
            .unwrap();
        }

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                tag_ids: Some(vec![前端.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hit.total_count, 1);
        assert_eq!(hit.issues[0].title, "带前端标签");

        // 多个标签是「任一命中」
        let both = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                tag_ids: Some(vec![前端.id, 后端.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(both.total_count, 2);
    }

    #[tokio::test]
    async fn 搜索条件可以叠加() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let mut urgent = 建需求请求(&场景, "登录页面崩溃");
        urgent.priority = Some(IssuePriority::Urgent);
        Issues::create(test_db.pool(), &urgent, DEFAULT_USER_ID)
            .await
            .unwrap();

        let mut low = 建需求请求(&场景, "登录页面配色");
        low.priority = Some(IssuePriority::Low);
        Issues::create(test_db.pool(), &low, DEFAULT_USER_ID)
            .await
            .unwrap();

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                search: Some("登录".to_string()),
                priority: Some(IssuePriority::Urgent),
                status_ids: Some(vec![场景.todo_status_id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hit.total_count, 1);
        assert_eq!(hit.issues[0].title, "登录页面崩溃");
    }

    #[tokio::test]
    async fn 搜索的_tag_id_单数等价于_tag_ids_单元素() {
        use api_types::{issue_tag::CreateIssueTagRequest, tag::CreateTagRequest};

        use crate::models::issue_side::{IssueTags, ProjectTags};

        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let 标签 = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id: 场景.project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let 命中 = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "带标签"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "没标签"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: None,
                issue_id: 命中.id,
                tag_id: 标签.id,
            },
        )
        .await
        .unwrap();

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                tag_id: Some(标签.id),
                ..Default::default()
            },
        )
        .await
        .expect("tag_id 单数必须被支持");
        assert_eq!(hit.total_count, 1);
        assert_eq!(hit.issues[0].title, "带标签");
    }

    #[tokio::test]
    async fn 筛选数组超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let 太多: Vec<Uuid> = (0..super::MAX_FILTER_IDS + 1)
            .map(|i| Uuid::from_u128(i as u128 + 1))
            .collect();

        for request in [
            SearchIssuesRequest {
                project_id: 场景.project_id,
                status_ids: Some(太多.clone()),
                ..Default::default()
            },
            SearchIssuesRequest {
                project_id: 场景.project_id,
                tag_ids: Some(太多.clone()),
                ..Default::default()
            },
        ] {
            assert!(
                matches!(
                    Issues::search(test_db.pool(), &request).await,
                    Err(super::IssueError::Validation(_))
                ),
                "超过上限的筛选数组必须被拒绝，而不是撞 SQLite 变量上限"
            );
        }

        // 恰好卡在上限仍然可用
        let 刚好: Vec<Uuid> = 太多.into_iter().take(super::MAX_FILTER_IDS).collect();
        Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                status_ids: Some(刚好),
                ..Default::default()
            },
        )
        .await
        .expect("恰好等于上限必须通过");
    }

    // ---- 父需求成环 ----

    #[tokio::test]
    async fn 父需求成环被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let 父 = Issues::create(test_db.pool(), &建需求请求(&场景, "父"), DEFAULT_USER_ID)
            .await
            .unwrap();
        let 子 = Issues::create(test_db.pool(), &建需求请求(&场景, "子"), DEFAULT_USER_ID)
            .await
            .unwrap();
        Issues::update(
            test_db.pool(),
            子.id,
            &UpdateIssueRequest {
                parent_issue_id: Some(Some(父.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // 把子设为父的父需求 → 成环
        let err = Issues::update(
            test_db.pool(),
            父.id,
            &UpdateIssueRequest {
                parent_issue_id: Some(Some(子.id)),
                ..Default::default()
            },
        )
        .await
        .expect_err("成环必须拒绝");
        assert!(
            matches!(err, super::IssueError::Validation(_)),
            "应是校验错误：{err:?}"
        );

        let after = Issues::find_by_id(test_db.pool(), 父.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.parent_issue_id, None, "成环的更新不得落库");
    }

    #[tokio::test]
    async fn 父需求层级超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let mut previous: Option<Uuid> = None;
        for index in 0..super::MAX_PARENT_DEPTH + 5 {
            let mut request = 建需求请求(&场景, &format!("层 {index}"));
            request.parent_issue_id = previous;
            let created = match Issues::create(test_db.pool(), &request, DEFAULT_USER_ID).await {
                Ok(issue) => issue,
                Err(err) => {
                    assert!(
                        matches!(err, super::IssueError::Validation(_)),
                        "超深父链应是校验错误：{err:?}"
                    );
                    return;
                }
            };
            previous = Some(created.id);
        }

        panic!("父链深度必须有上限");
    }

    // ---- 时间格式 ----

    #[tokio::test]
    async fn 同一行的时间字段可以按字符串比较() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;

        let mut request = 建需求请求(&场景, "带日期");
        // start_date 取一个明显早于「现在」的时刻：字符串比较必须也得出同样结论。
        request.start_date = Some(
            chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );
        let issue = Issues::create(test_db.pool(), &request, DEFAULT_USER_ID)
            .await
            .unwrap();

        let row: (String, String, String) =
            sqlx::query_as("SELECT created_at, updated_at, start_date FROM issues WHERE id = ?1")
                .bind(issue.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        let (created_at, updated_at, start_date) = row;

        assert!(
            start_date < created_at,
            "2020 年的 start_date 必须小于 created_at，实际 start_date={start_date} created_at={created_at}"
        );
        assert_eq!(created_at, updated_at, "新建行的两个时间戳应完全一致");
        assert!(
            created_at.contains('T'),
            "created_at 必须是 RFC3339：{created_at}"
        );

        // SQL 侧的比较也必须一致
        let cmp: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM issues WHERE id = ?1 AND start_date < created_at")
                .bind(issue.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert_eq!(cmp.0, 1, "SQL 里按字符串比较也必须成立");
    }

    #[tokio::test]
    async fn 更新会推进_updated_at_且格式一致() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Alpha").await;
        let issue = Issues::create(
            test_db.pool(),
            &建需求请求(&场景, "原标题"),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                title: Some("新标题".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let row: (String, String) =
            sqlx::query_as("SELECT created_at, updated_at FROM issues WHERE id = ?1")
                .bind(issue.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert!(row.1.contains('T'), "updated_at 必须是 RFC3339：{}", row.1);
        assert!(row.1 > row.0, "updated_at 必须推进：{} > {}", row.1, row.0);
    }
}
