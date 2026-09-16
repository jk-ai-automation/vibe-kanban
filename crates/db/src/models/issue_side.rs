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

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub struct ProjectTags;

impl ProjectTags {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<Tag>, sqlx::Error> {
        sqlx::query_as!(
            Tag,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!"
               FROM project_tags
               WHERE project_id = $1
               ORDER BY name ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateTagRequest) -> Result<Tag, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name = truncate(data.name.trim(), MAX_TAG_NAME_LEN);

        sqlx::query_as!(
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
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateTagRequest,
    ) -> Result<Tag, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| truncate(n.trim(), MAX_TAG_NAME_LEN));
        let set_color = data.color.is_some();

        sqlx::query_as!(
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
        .await
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
    ) -> Result<Vec<IssueTag>, sqlx::Error> {
        sqlx::query_as!(
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
            MAX_SIDE_ROWS
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueTagRequest,
    ) -> Result<IssueTag, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        sqlx::query_as!(
            IssueTag,
            r#"INSERT INTO issue_tags (id, issue_id, tag_id)
               VALUES ($1, $2, $3)
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         tag_id AS "tag_id!: Uuid""#,
            id,
            data.issue_id,
            data.tag_id
        )
        .fetch_one(pool)
        .await
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
    ) -> Result<Vec<IssueComment>, sqlx::Error> {
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
               WHERE issue_id = $1
               ORDER BY created_at ASC
               LIMIT $2"#,
            issue_id,
            MAX_SIDE_ROWS
        )
        .fetch_all(pool)
        .await
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

        let comment = sqlx::query_as!(
            IssueComment,
            r#"INSERT INTO issue_comments (id, issue_id, author_id, parent_id, message)
               VALUES ($1, $2, $3, $4, $5)
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
            message
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

        let comment = sqlx::query_as!(
            IssueComment,
            r#"UPDATE issue_comments SET
                   message    = CASE WHEN $2 THEN $3 ELSE message END,
                   parent_id  = CASE WHEN $4 THEN $5 ELSE parent_id END,
                   updated_at = datetime('now', 'subsec')
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
            parent_id
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
        assert_eq!(same.len(), 1);
        assert_eq!(same[0].id, tag.id);

        let other = ProjectTags::find_by_project(test_db.pool(), Uuid::from_u128(77))
            .await
            .unwrap();
        assert!(other.is_empty(), "不得返回其他项目的标签");
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
        IssueTags::create(test_db.pool(), &request).await.unwrap();
        assert!(
            IssueTags::create(test_db.pool(), &request).await.is_err(),
            "同一需求同一标签不得重复关联"
        );

        let list = IssueTags::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].issue_id, issue_id);
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
                .is_empty()
        );
        assert!(
            IssueComments::find_by_issue(test_db.pool(), issue_id)
                .await
                .unwrap()
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
        assert_eq!(list.len(), 1);

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
}
