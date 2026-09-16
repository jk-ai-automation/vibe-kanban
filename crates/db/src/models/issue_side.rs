use api_types::{
    issue_comment::{CreateIssueCommentRequest, IssueComment, UpdateIssueCommentRequest},
    issue_tag::{CreateIssueTagRequest, IssueTag},
    tag::{CreateTagRequest, Tag, UpdateTagRequest},
};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{issue::IssueError, local_project::DEFAULT_USER_ID};

/// 标签名长度上限。
pub const MAX_TAG_NAME_LEN: usize = 60;
/// 评论正文长度上限。
pub const MAX_COMMENT_LEN: usize = 20_000;
/// 单次快照返回的关联行上限。
pub const MAX_SIDE_ROWS: i64 = 5000;
/// 评论父链的最大层数，同时用作成环检测的兜底深度。
pub const MAX_COMMENT_DEPTH: i64 = 32;

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 关联行快照。`truncated` 为真表示行数触到 [`MAX_SIDE_ROWS`]，
/// 调用方（路由层）必须把这一事实透出去，不能静默截断。
/// 与 [`crate::models::issue::IssueSnapshot`] 同一套约定。
#[derive(Debug)]
pub struct SideSnapshot<T> {
    pub rows: Vec<T>,
    pub truncated: bool,
}

impl<T> SideSnapshot<T> {
    /// 多取一行用来判断是否被截断，返回前丢掉。
    fn from_probe(mut rows: Vec<T>) -> Self {
        let truncated = rows.len() as i64 > MAX_SIDE_ROWS;
        rows.truncate(MAX_SIDE_ROWS as usize);
        Self { rows, truncated }
    }
}

/// 探测用 LIMIT：比上限多取一行。
const PROBE_LIMIT: i64 = MAX_SIDE_ROWS + 1;

pub struct ProjectTags;

impl ProjectTags {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<SideSnapshot<Tag>, sqlx::Error> {
        let rows = sqlx::query_as!(
            Tag,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!"
               FROM project_tags
               WHERE project_id = $1
               ORDER BY name ASC
               LIMIT $2"#,
            project_id,
            PROBE_LIMIT
        )
        .fetch_all(pool)
        .await?;

        Ok(SideSnapshot::from_probe(rows))
    }

    pub async fn create(pool: &SqlitePool, data: &CreateTagRequest) -> Result<Tag, IssueError> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name = truncate(data.name.trim(), MAX_TAG_NAME_LEN);
        if name.is_empty() {
            return Err(IssueError::Validation("标签名不能为空".to_string()));
        }

        let tag = sqlx::query_as!(
            Tag,
            r#"INSERT INTO project_tags (id, project_id, name, color)
               VALUES ($1, $2, $3, $4)
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!""#,
            id,
            data.project_id,
            name,
            data.color
        )
        .fetch_one(pool)
        .await?;

        Ok(tag)
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateTagRequest,
    ) -> Result<Tag, IssueError> {
        let set_name = data.name.is_some();
        let name: Option<String> = match data.name.as_ref() {
            Some(raw) => {
                let trimmed = truncate(raw.trim(), MAX_TAG_NAME_LEN);
                if trimmed.is_empty() {
                    return Err(IssueError::Validation("标签名不能为空".to_string()));
                }
                Some(trimmed)
            }
            None => None,
        };
        let set_color = data.color.is_some();

        let tag = sqlx::query_as!(
            Tag,
            r#"UPDATE project_tags SET
                   name  = CASE WHEN $2 THEN $3 ELSE name END,
                   color = CASE WHEN $4 THEN $5 ELSE color END
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!""#,
            id,
            set_name,
            name,
            set_color,
            data.color
        )
        .fetch_one(pool)
        .await?;

        Ok(tag)
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM project_tags WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

pub struct IssueTags;

impl IssueTags {
    /// 按项目查：只返回该项目下需求的标签关联，跨项目不可见。
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<SideSnapshot<IssueTag>, sqlx::Error> {
        let rows = sqlx::query_as!(
            IssueTag,
            r#"SELECT it.id       AS "id!: Uuid",
                      it.issue_id AS "issue_id!: Uuid",
                      it.tag_id   AS "tag_id!: Uuid"
               FROM issue_tags it
               JOIN issues i ON i.id = it.issue_id
               WHERE i.project_id = $1
               ORDER BY it.rowid ASC
               LIMIT $2"#,
            project_id,
            PROBE_LIMIT
        )
        .fetch_all(pool)
        .await?;

        Ok(SideSnapshot::from_probe(rows))
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueTagRequest,
    ) -> Result<IssueTag, IssueError> {
        // 标签与需求必须属于同一项目，否则会把别的项目的标签挂到本项目需求上。
        let same_project: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM issues i JOIN project_tags t ON t.project_id = i.project_id \
             WHERE i.id = ?1 AND t.id = ?2",
        )
        .bind(data.issue_id)
        .bind(data.tag_id)
        .fetch_optional(pool)
        .await?;
        if same_project.is_none() {
            return Err(IssueError::Validation(
                "标签与需求不属于同一项目".to_string(),
            ));
        }

        let id = data.id.unwrap_or_else(Uuid::new_v4);

        // 重复挂同一个标签按幂等处理：ON CONFLICT DO NOTHING 之后把已有行读回来。
        // 前端的乐观更新会重放同一次操作（断线重连、重复点击），这在业务上不是错误，
        // 返回 500 只会让界面卡在「保存失败」。
        let inserted = sqlx::query_as!(
            IssueTag,
            r#"INSERT INTO issue_tags (id, issue_id, tag_id)
               VALUES ($1, $2, $3)
               ON CONFLICT DO NOTHING
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         tag_id AS "tag_id!: Uuid""#,
            id,
            data.issue_id,
            data.tag_id
        )
        .fetch_optional(pool)
        .await?;

        if let Some(issue_tag) = inserted {
            return Ok(issue_tag);
        }

        let existing = sqlx::query_as!(
            IssueTag,
            r#"SELECT id AS "id!: Uuid",
                      issue_id AS "issue_id!: Uuid",
                      tag_id AS "tag_id!: Uuid"
               FROM issue_tags
               WHERE issue_id = $1 AND tag_id = $2"#,
            data.issue_id,
            data.tag_id
        )
        .fetch_optional(pool)
        .await?;

        // 冲突不是「同一需求同一标签」，那就是客户端自带的 id 撞上了别的关联，
        // 这属于真正的冲突，必须报出来而不是悄悄返回一行别的数据。
        existing.ok_or_else(|| IssueError::Conflict("关联 id 已被占用，请换一个 id".to_string()))
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issue_tags WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

pub struct IssueComments;

impl IssueComments {
    pub async fn find_by_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<SideSnapshot<IssueComment>, sqlx::Error> {
        let rows = sqlx::query_as!(
            IssueComment,
            r#"SELECT id         AS "id!: Uuid",
                      issue_id   AS "issue_id!: Uuid",
                      author_id  AS "author_id: Uuid",
                      parent_id  AS "parent_id: Uuid",
                      message    AS "message!",
                      created_at AS "created_at!: DateTime<Utc>",
                      updated_at AS "updated_at!: DateTime<Utc>"
               FROM issue_comments
               WHERE issue_id = $1
               ORDER BY created_at ASC
               LIMIT $2"#,
            issue_id,
            PROBE_LIMIT
        )
        .fetch_all(pool)
        .await?;

        Ok(SideSnapshot::from_probe(rows))
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<IssueComment>, sqlx::Error> {
        sqlx::query_as!(
            IssueComment,
            r#"SELECT id         AS "id!: Uuid",
                      issue_id   AS "issue_id!: Uuid",
                      author_id  AS "author_id: Uuid",
                      parent_id  AS "parent_id: Uuid",
                      message    AS "message!",
                      created_at AS "created_at!: DateTime<Utc>",
                      updated_at AS "updated_at!: DateTime<Utc>"
               FROM issue_comments
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueCommentRequest,
    ) -> Result<IssueComment, IssueError> {
        let message = truncate(data.message.trim(), MAX_COMMENT_LEN);
        if message.is_empty() {
            return Err(IssueError::Validation("评论内容不能为空".to_string()));
        }
        let id = data.id.unwrap_or_else(Uuid::new_v4);

        // 父评论必须挂在同一条需求下，否则会串到别的需求的评论树里。
        if let Some(parent_id) = data.parent_id {
            let same_issue: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM issue_comments WHERE id = ?1 AND issue_id = ?2")
                    .bind(parent_id)
                    .bind(data.issue_id)
                    .fetch_optional(pool)
                    .await?;
            if same_issue.is_none() {
                return Err(IssueError::Validation(
                    "父评论不存在或不属于该需求".to_string(),
                ));
            }
        }

        let now = Utc::now();
        let comment = sqlx::query_as!(
            IssueComment,
            r#"INSERT INTO issue_comments
                   (id, issue_id, author_id, parent_id, message, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $6)
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         author_id AS "author_id: Uuid",
                         parent_id AS "parent_id: Uuid",
                         message AS "message!",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            data.issue_id,
            DEFAULT_USER_ID,
            data.parent_id,
            message,
            now
        )
        .fetch_one(pool)
        .await?;

        Ok(comment)
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateIssueCommentRequest,
    ) -> Result<IssueComment, IssueError> {
        let set_message = data.message.is_some();
        let message: Option<String> = match data.message.as_ref() {
            Some(raw) => {
                let trimmed = truncate(raw.trim(), MAX_COMMENT_LEN);
                if trimmed.is_empty() {
                    return Err(IssueError::Validation("评论内容不能为空".to_string()));
                }
                Some(trimmed)
            }
            None => None,
        };
        let set_parent = data.parent_id.is_some();
        let parent_id = data.parent_id.flatten();

        if let Some(parent_id) = parent_id {
            if parent_id == id {
                return Err(IssueError::Validation(
                    "评论不能把自己当作父评论".to_string(),
                ));
            }
            let same_issue: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM issue_comments parent \
                 JOIN issue_comments child ON child.issue_id = parent.issue_id \
                 WHERE parent.id = ?1 AND child.id = ?2",
            )
            .bind(parent_id)
            .bind(id)
            .fetch_optional(pool)
            .await?;
            if same_issue.is_none() {
                return Err(IssueError::Validation(
                    "父评论不存在或不属于该需求".to_string(),
                ));
            }

            // 成环检查：沿父链往上走，若走回自己就会让前端渲染评论树时无限递归。
            let cycle: Option<(Uuid, i64)> = sqlx::query_as(
                "WITH RECURSIVE 祖先(id, depth) AS ( \
                     SELECT ?1, 0 \
                     UNION ALL \
                     SELECT c.parent_id, 祖先.depth + 1 \
                     FROM issue_comments c JOIN 祖先 ON c.id = 祖先.id \
                     WHERE c.parent_id IS NOT NULL AND 祖先.depth < ?3 \
                 ) SELECT id, depth FROM 祖先 WHERE id = ?2 OR depth >= ?3 LIMIT 1",
            )
            .bind(parent_id)
            .bind(id)
            .bind(MAX_COMMENT_DEPTH)
            .fetch_optional(pool)
            .await?;

            match cycle {
                Some((hit, _)) if hit == id => {
                    return Err(IssueError::Validation(
                        "父评论会形成环，请换一个父评论".to_string(),
                    ));
                }
                Some(_) => {
                    return Err(IssueError::Validation(format!(
                        "评论层级超过 {MAX_COMMENT_DEPTH} 层上限"
                    )));
                }
                None => {}
            }
        }

        let now = Utc::now();
        let comment = sqlx::query_as!(
            IssueComment,
            r#"UPDATE issue_comments SET
                   message    = CASE WHEN $2 THEN $3 ELSE message END,
                   parent_id  = CASE WHEN $4 THEN $5 ELSE parent_id END,
                   updated_at = $6
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         author_id AS "author_id: Uuid",
                         parent_id AS "parent_id: Uuid",
                         message AS "message!",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            set_message,
            message,
            set_parent,
            parent_id,
            now
        )
        .fetch_one(pool)
        .await?;

        Ok(comment)
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issue_comments WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use api_types::{
        issue::CreateIssueRequest,
        issue_comment::{CreateIssueCommentRequest, UpdateIssueCommentRequest},
        issue_tag::CreateIssueTagRequest,
        project::CreateProjectRequest,
        tag::{CreateTagRequest, UpdateTagRequest},
    };
    use uuid::Uuid;

    use super::{IssueComments, IssueTags, MAX_COMMENT_LEN, ProjectTags};
    use crate::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

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
                title: "示例需求".to_string(),
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
    async fn 标签按项目隔离() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let same = ProjectTags::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(same.rows.len(), 1);
        assert_eq!(same.rows[0].id, tag.id);
        assert!(!same.truncated);

        let other = ProjectTags::find_by_project(test_db.pool(), Uuid::from_u128(77))
            .await
            .unwrap();
        assert!(other.rows.is_empty(), "不得返回其他项目的标签");
    }

    #[tokio::test]
    async fn 标签更新与删除() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let updated = ProjectTags::update(
            test_db.pool(),
            tag.id,
            &UpdateTagRequest {
                name: Some("后端".to_string()),
                color: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "后端");
        assert_eq!(updated.color, "#22c55e", "未传的字段保持原值");

        assert_eq!(
            ProjectTags::delete(test_db.pool(), tag.id).await.unwrap(),
            1
        );
        assert_eq!(
            ProjectTags::delete(test_db.pool(), tag.id).await.unwrap(),
            0,
            "重复删除不得报错"
        );
    }

    #[tokio::test]
    async fn 需求标签关联按项目查询且去重() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let request = CreateIssueTagRequest {
            id: None,
            issue_id,
            tag_id: tag.id,
        };
        let first = IssueTags::create(test_db.pool(), &request).await.unwrap();
        // 重复挂同一标签按幂等处理：返回已有关联，不新增行、也不报 500。
        let again = IssueTags::create(test_db.pool(), &request)
            .await
            .expect("重复关联必须幂等成功");
        assert_eq!(again.id, first.id, "幂等返回的必须是已有关联");

        let list = IssueTags::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(list.rows.len(), 1);
        assert_eq!(list.rows[0].issue_id, issue_id);
    }

    #[tokio::test]
    async fn 删除需求级联删除其标签关联与评论() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: None,
                issue_id,
                tag_id: tag.id,
            },
        )
        .await
        .unwrap();
        IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "评论".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        Issues::delete(test_db.pool(), issue_id).await.unwrap();

        assert!(
            IssueTags::find_by_project(test_db.pool(), project_id)
                .await
                .unwrap()
                .rows
                .is_empty()
        );
        assert!(
            IssueComments::find_by_issue(test_db.pool(), issue_id)
                .await
                .unwrap()
                .rows
                .is_empty()
        );
    }

    #[tokio::test]
    async fn 评论增改查删与超长截断() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let comment = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "评".repeat(MAX_COMMENT_LEN + 50),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(comment.message.chars().count(), MAX_COMMENT_LEN);
        assert_eq!(
            comment.author_id,
            Some(crate::models::local_project::DEFAULT_USER_ID)
        );

        let updated = IssueComments::update(
            test_db.pool(),
            comment.id,
            &UpdateIssueCommentRequest {
                message: Some("改后".to_string()),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.message, "改后");
        assert!(updated.updated_at >= comment.updated_at);

        let list = IssueComments::find_by_issue(test_db.pool(), issue_id)
            .await
            .unwrap();
        assert_eq!(list.rows.len(), 1);

        assert_eq!(
            IssueComments::delete(test_db.pool(), comment.id)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn 空评论被拒绝() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let result = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "  \n ".to_string(),
                parent_id: None,
            },
        )
        .await;
        assert!(result.is_err(), "空白评论必须拒绝");
    }

    #[tokio::test]
    async fn 跨项目的标签不能挂到需求上() {
        let test_db = TestDb::new().await;
        let (_, issue_a) = 准备具名(&test_db, "Alpha").await;
        let (project_b, _) = 准备具名(&test_db, "Beta").await;

        let 别家标签 = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id: project_b,
                name: "别家".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let err = IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: None,
                issue_id: issue_a,
                tag_id: 别家标签.id,
            },
        )
        .await
        .expect_err("跨项目标签必须拒绝");
        assert!(
            matches!(err, crate::models::issue::IssueError::Validation(_)),
            "应是校验错误：{err:?}"
        );

        assert!(
            IssueTags::find_by_project(test_db.pool(), project_b)
                .await
                .unwrap()
                .rows
                .is_empty(),
            "被拒绝的关联不得落库"
        );
    }

    #[tokio::test]
    async fn 不存在的标签不能挂到需求上() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let err = IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: None,
                issue_id,
                tag_id: Uuid::from_u128(9911),
            },
        )
        .await
        .expect_err("标签不存在必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn 空名标签被拒绝() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        for name in ["", "   ", "\n\t "] {
            let err = ProjectTags::create(
                test_db.pool(),
                &CreateTagRequest {
                    id: None,
                    project_id,
                    name: name.to_string(),
                    color: "#22c55e".to_string(),
                },
            )
            .await
            .expect_err("空名标签必须拒绝");
            assert!(matches!(
                err,
                crate::models::issue::IssueError::Validation(_)
            ));
        }

        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "有效".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        let err = ProjectTags::update(
            test_db.pool(),
            tag.id,
            &UpdateTagRequest {
                name: Some("  ".to_string()),
                color: None,
            },
        )
        .await
        .expect_err("改成空名也必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn 评论的父评论必须属于同一需求() {
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
                title: "另一条需求".to_string(),
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

        let 甲的评论 = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id: issue_a,
                message: "甲".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        // 新建：父评论挂在别的需求上
        let err = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id: issue_b.id,
                message: "串台".to_string(),
                parent_id: Some(甲的评论.id),
            },
        )
        .await
        .expect_err("跨需求的父评论必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));

        // 更新：把父评论改成别的需求下的评论
        let 乙的评论 = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id: issue_b.id,
                message: "乙".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        let err = IssueComments::update(
            test_db.pool(),
            乙的评论.id,
            &UpdateIssueCommentRequest {
                message: None,
                parent_id: Some(Some(甲的评论.id)),
            },
        )
        .await
        .expect_err("跨需求的父评论必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));

        // 同需求的父评论可以通过
        let 同需求 = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id: issue_a,
                message: "回复甲".to_string(),
                parent_id: Some(甲的评论.id),
            },
        )
        .await
        .unwrap();
        assert_eq!(同需求.parent_id, Some(甲的评论.id));
    }

    #[tokio::test]
    async fn 评论不能把自己当父评论() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;
        let comment = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "自己".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        let err = IssueComments::update(
            test_db.pool(),
            comment.id,
            &UpdateIssueCommentRequest {
                message: None,
                parent_id: Some(Some(comment.id)),
            },
        )
        .await
        .expect_err("自引用必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn 自带_id_撞上别的关联时报冲突而不是_500() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;

        let mut 标签 = Vec::new();
        for name in ["前端", "后端"] {
            标签.push(
                ProjectTags::create(
                    test_db.pool(),
                    &CreateTagRequest {
                        id: None,
                        project_id,
                        name: name.to_string(),
                        color: "#22c55e".to_string(),
                    },
                )
                .await
                .unwrap(),
            );
        }

        let 固定 = Uuid::new_v4();
        IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: Some(固定),
                issue_id,
                tag_id: 标签[0].id,
            },
        )
        .await
        .unwrap();

        let err = IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: Some(固定),
                issue_id,
                tag_id: 标签[1].id,
            },
        )
        .await
        .expect_err("id 撞上别的关联必须报错");
        assert!(
            matches!(err, crate::models::issue::IssueError::Conflict(_)),
            "应是冲突错误：{err:?}"
        );
    }

    #[tokio::test]
    async fn 标签与评论的快照触到上限时带出截断标记() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;

        let mut tx = test_db.pool().begin().await.unwrap();
        for index in 0..super::MAX_SIDE_ROWS + 2 {
            sqlx::query(
                "INSERT INTO project_tags (id, project_id, name, color) VALUES (?1, ?2, ?3, '#fff')",
            )
            .bind(Uuid::new_v4())
            .bind(project_id)
            .bind(format!("标签 {index:06}"))
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO issue_comments (id, issue_id, message, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?4)",
            )
            .bind(Uuid::new_v4())
            .bind(issue_id)
            .bind(format!("评论 {index}"))
            .bind(chrono::Utc::now())
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let tags = ProjectTags::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(tags.rows.len(), super::MAX_SIDE_ROWS as usize);
        assert!(tags.truncated, "标签触到上限必须让调用方感知");

        let comments = IssueComments::find_by_issue(test_db.pool(), issue_id)
            .await
            .unwrap();
        assert_eq!(comments.rows.len(), super::MAX_SIDE_ROWS as usize);
        assert!(comments.truncated, "评论触到上限必须让调用方感知");
    }

    #[tokio::test]
    async fn 评论父链成环被拒绝() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let 甲 = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "甲".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        let 乙 = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "乙".to_string(),
                parent_id: Some(甲.id),
            },
        )
        .await
        .unwrap();

        // 甲 -> 乙 会形成环（乙的父已经是甲）
        let err = IssueComments::update(
            test_db.pool(),
            甲.id,
            &UpdateIssueCommentRequest {
                message: None,
                parent_id: Some(Some(乙.id)),
            },
        )
        .await
        .expect_err("成环必须拒绝");
        assert!(matches!(
            err,
            crate::models::issue::IssueError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn 补齐的索引确实建出来了() {
        let test_db = TestDb::new().await;
        let names: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'index'")
                .fetch_all(test_db.pool())
                .await
                .unwrap();
        let names: Vec<String> = names.into_iter().map(|r| r.0).collect();
        for expected in [
            "idx_issues_project_sort",
            "idx_issue_tags_tag",
            "idx_issue_comments_parent",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "缺少索引 {expected}，实际：{names:?}"
            );
        }
    }
}
