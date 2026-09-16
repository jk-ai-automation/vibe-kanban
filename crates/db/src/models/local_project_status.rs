use api_types::project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::db_retry::retry_on_busy;

/// 本地状态列行。字段与 api_types::ProjectStatus 一一对应，
/// 额外多出 stage_type（个人版专有，用于状态自动流转；
/// 多出的 JSON 字段前端会原样忽略，不影响 shared/remote-types.ts）。
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct LocalProjectStatus {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub color: String,
    pub sort_order: i32,
    pub hidden: bool,
    pub stage_type: String,
    pub created_at: DateTime<Utc>,
}

/// 流程阶段。用户可以给状态列改名，自动流转靠 stage_type 识别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageType {
    Backlog,
    Todo,
    Dev,
    Review,
    Test,
    Done,
}

impl StageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StageType::Backlog => "backlog",
            StageType::Todo => "todo",
            StageType::Dev => "dev",
            StageType::Review => "review",
            StageType::Test => "test",
            StageType::Done => "done",
        }
    }
}

/// 状态列名称长度上限。
pub const MAX_STATUS_NAME_LEN: usize = 100;

pub struct ProjectStatuses;

impl ProjectStatuses {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE project_id = $1
               ORDER BY sort_order ASC, created_at ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_stage(
        pool: &SqlitePool,
        project_id: Uuid,
        stage: StageType,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        let stage_type = stage.as_str();
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE project_id = $1 AND stage_type = $2
               ORDER BY sort_order ASC
               LIMIT 1"#,
            project_id,
            stage_type
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name: String = data.name.chars().take(MAX_STATUS_NAME_LEN).collect();
        // 显式绑定 created_at：SQLite 的 DEFAULT 写的是空格分隔格式，
        // 和代码写入的 RFC3339 混在一起会让 ORDER BY sort_order, created_at
        // 在 sort_order 并列时排出错误顺序。
        let now = Utc::now();

        sqlx::query_as!(
            LocalProjectStatus,
            r#"INSERT INTO project_statuses
                   (id, project_id, name, color, sort_order, hidden, stage_type, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, 'todo', $7)
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         hidden AS "hidden!: bool",
                         stage_type AS "stage_type!",
                         created_at AS "created_at!: DateTime<Utc>""#,
            id,
            data.project_id,
            name,
            data.color,
            data.sort_order,
            data.hidden,
            now
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        retry_on_busy(|| async {
            let mut tx = pool.begin().await?;
            let updated = Self::update_in_tx(&mut tx, id, data).await?;
            tx.commit().await?;
            Ok(updated)
        })
        .await
    }

    /// 批量更新（拖拽排序）。单事务，任一条失败整体回滚。
    pub async fn bulk_update(
        pool: &SqlitePool,
        updates: &[(Uuid, UpdateProjectStatusRequest)],
    ) -> Result<Vec<LocalProjectStatus>, sqlx::Error> {
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
        data: &UpdateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| n.chars().take(MAX_STATUS_NAME_LEN).collect());
        let set_color = data.color.is_some();
        let set_sort_order = data.sort_order.is_some();
        let set_hidden = data.hidden.is_some();

        sqlx::query_as!(
            LocalProjectStatus,
            r#"UPDATE project_statuses SET
                   name       = CASE WHEN $2 THEN $3 ELSE name END,
                   color      = CASE WHEN $4 THEN $5 ELSE color END,
                   sort_order = CASE WHEN $6 THEN $7 ELSE sort_order END,
                   hidden     = CASE WHEN $8 THEN $9 ELSE hidden END
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         hidden AS "hidden!: bool",
                         stage_type AS "stage_type!",
                         created_at AS "created_at!: DateTime<Utc>""#,
            id,
            set_name,
            name,
            set_color,
            data.color,
            set_sort_order,
            data.sort_order,
            set_hidden,
            data.hidden
        )
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM project_statuses WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use api_types::{
        project::CreateProjectRequest,
        project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest},
    };
    use uuid::Uuid;

    use super::{ProjectStatuses, StageType};
    use crate::{
        models::local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
        test_support::TestDb,
    };

    async fn 建项目(test_db: &TestDb, name: &str) -> Uuid {
        LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: name.to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .expect("建项目失败")
        .id
    }

    #[tokio::test]
    async fn 按项目列出状态列并带出_stage_type() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        assert_eq!(statuses.len(), 6);
        assert_eq!(statuses[0].stage_type, "backlog");
        assert_eq!(statuses[4].stage_type, "test");
        assert_eq!(statuses[5].stage_type, "done");
        assert!(statuses.iter().all(|s| s.project_id == project_id));
    }

    #[tokio::test]
    async fn 列出状态列不会串到别的项目() {
        let test_db = TestDb::new().await;
        let a = 建项目(&test_db, "A").await;
        let b = 建项目(&test_db, "B").await;

        let statuses = ProjectStatuses::find_by_project(test_db.pool(), a)
            .await
            .unwrap();
        assert!(
            statuses.iter().all(|s| s.project_id == a),
            "不得返回项目 {b} 的状态列"
        );
    }

    #[tokio::test]
    async fn 按阶段查找状态列() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let dev = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .expect("应存在 dev 阶段状态列");
        assert_eq!(dev.name, "开发中");

        // 用户改名后仍能按 stage_type 找到
        ProjectStatuses::update(
            test_db.pool(),
            dev.id,
            &UpdateProjectStatusRequest {
                name: Some("Coding".to_string()),
                color: None,
                sort_order: None,
                hidden: None,
            },
        )
        .await
        .unwrap();

        let dev_again = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .expect("改名后仍应按 stage_type 命中");
        assert_eq!(dev_again.name, "Coding");
        assert_eq!(dev_again.id, dev.id);
    }

    #[tokio::test]
    async fn 新建的状态列_stage_type_为_todo() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let created = ProjectStatuses::create(
            test_db.pool(),
            &CreateProjectStatusRequest {
                id: None,
                project_id,
                name: "联调中".to_string(),
                color: "#f59e0b".to_string(),
                sort_order: 9,
                hidden: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(created.stage_type, "todo");
        assert_eq!(created.sort_order, 9);
        assert!(!created.hidden);
    }

    /// 用户新建的状态列必须和默认状态列用同一种时间戳格式，
    /// 否则 `ORDER BY sort_order, created_at` 在 sort_order 并列时会排错。
    #[tokio::test]
    async fn 用户新建状态列的时间戳与默认状态列格式一致() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let created = ProjectStatuses::create(
            test_db.pool(),
            &CreateProjectStatusRequest {
                id: None,
                project_id,
                name: "联调中".to_string(),
                // 与默认「待规划」同一个 sort_order，逼出并列排序
                sort_order: 0,
                color: "#f59e0b".to_string(),
                hidden: false,
            },
        )
        .await
        .unwrap();

        let 新建时间: (String,) =
            sqlx::query_as("SELECT created_at FROM project_statuses WHERE id = ?1")
                .bind(created.id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert!(
            新建时间.0.contains('T'),
            "新建状态列的 created_at 必须是 RFC3339：{}",
            新建时间.0
        );

        // 并列 sort_order 时，先建的默认状态列必须排在后建的前面
        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        let 并列: Vec<&str> = statuses
            .iter()
            .filter(|s| s.sort_order == 0)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(
            并列,
            vec!["待规划", "联调中"],
            "同 sort_order 时必须按创建时间排序"
        );
    }

    #[tokio::test]
    async fn 批量更新在单事务内全成或全败() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;
        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        let updates = vec![
            (
                statuses[0].id,
                UpdateProjectStatusRequest {
                    name: None,
                    color: None,
                    sort_order: Some(100),
                    hidden: None,
                },
            ),
            (
                Uuid::from_u128(424242),
                UpdateProjectStatusRequest {
                    name: None,
                    color: None,
                    sort_order: Some(200),
                    hidden: None,
                },
            ),
        ];

        let result = ProjectStatuses::bulk_update(test_db.pool(), &updates).await;
        assert!(result.is_err(), "含不存在 id 的批量更新必须失败");

        let after = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(
            after[0].sort_order, 0,
            "批量更新失败必须回滚第一条，不得留下半截状态"
        );
    }

    #[tokio::test]
    async fn 批量更新成功时按顺序返回所有行() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;
        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        let updates: Vec<_> = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                (
                    status.id,
                    UpdateProjectStatusRequest {
                        name: None,
                        color: None,
                        sort_order: Some(10 - index as i32),
                        hidden: None,
                    },
                )
            })
            .collect();

        let updated = ProjectStatuses::bulk_update(test_db.pool(), &updates)
            .await
            .unwrap();

        assert_eq!(updated.len(), 6);
        assert_eq!(updated[0].sort_order, 10);
        assert_eq!(updated[5].sort_order, 5);
    }

    #[test]
    fn 阶段枚举与字符串互转() {
        assert_eq!(StageType::Backlog.as_str(), "backlog");
        assert_eq!(StageType::Todo.as_str(), "todo");
        assert_eq!(StageType::Dev.as_str(), "dev");
        assert_eq!(StageType::Review.as_str(), "review");
        assert_eq!(StageType::Test.as_str(), "test");
        assert_eq!(StageType::Done.as_str(), "done");
    }

    #[tokio::test]
    async fn 按阶段可以查到测试中状态列() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let 测试 = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Test)
            .await
            .unwrap()
            .expect("应存在 test 阶段状态列");
        assert_eq!(测试.name, "测试中");
        assert_eq!(测试.stage_type, "test");
    }

    // ---- 迁移 20260917100000_add_test_stage_status.sql 的回填行为 ----

    const 回填迁移: &str =
        include_str!("../../migrations/20260917100000_add_test_stage_status.sql");

    /// 把项目退回「迁移前」的样子：删掉测试中列，并把它后面的列 sort_order 补回来。
    async fn 退回迁移前(pool: &sqlx::SqlitePool, project_id: Uuid) {
        let target: (i64,) = sqlx::query_as(
            "SELECT sort_order FROM project_statuses WHERE project_id = ?1 AND stage_type = 'test'",
        )
        .bind(project_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM project_statuses WHERE project_id = ?1 AND stage_type = 'test'")
            .bind(project_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE project_statuses SET sort_order = sort_order - 1 \
             WHERE project_id = ?1 AND sort_order > ?2",
        )
        .bind(project_id)
        .bind(target.0)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn 阶段顺序(pool: &sqlx::SqlitePool, project_id: Uuid) -> Vec<String> {
        ProjectStatuses::find_by_project(pool, project_id)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.stage_type)
            .collect()
    }

    #[tokio::test]
    async fn 迁移给已有项目补上测试中列且排在评审与完成之间() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;
        assert_eq!(
            阶段顺序(test_db.pool(), project_id).await,
            vec!["backlog", "todo", "dev", "review", "done"]
        );

        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        assert_eq!(
            阶段顺序(test_db.pool(), project_id).await,
            vec!["backlog", "todo", "dev", "review", "test", "done"]
        );
        let 测试列 = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Test)
            .await
            .unwrap()
            .expect("回填后应能按 stage_type 查到");
        assert_eq!(测试列.name, "测试中");
        assert_eq!(测试列.color, "#06b6d4");
        assert!(!测试列.hidden);
    }

    /// randomblob(16) 生成的主键必须能被 sqlx 解码成 Uuid，
    /// created_at 也必须能被解码成 DateTime<Utc>（上面的 find_by_project 已覆盖解码）。
    #[tokio::test]
    async fn 迁移回填的行能被解码成_uuid_与时间戳() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;
        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        let 测试列 = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Test)
            .await
            .unwrap()
            .expect("应存在");
        assert_ne!(测试列.id, Uuid::nil());
        // 按 id 再查一次，确认写进去的 BLOB 和读出来的 Uuid 能对上
        let 再查 = ProjectStatuses::find_by_id(test_db.pool(), 测试列.id)
            .await
            .unwrap()
            .expect("按 id 应能查到回填的行");
        assert_eq!(再查.id, 测试列.id);
    }

    #[tokio::test]
    async fn 迁移可重复执行且不会重复插入() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;

        for _ in 0..3 {
            sqlx::raw_sql(回填迁移)
                .execute(test_db.pool())
                .await
                .unwrap();
        }

        assert_eq!(
            阶段顺序(test_db.pool(), project_id).await,
            vec!["backlog", "todo", "dev", "review", "test", "done"]
        );
    }

    /// 已经有测试中列的项目（例如新建项目）整条跳过，sort_order 一格都不能动。
    #[tokio::test]
    async fn 迁移不碰已有测试中列的项目() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "新项目").await;
        let 之前 = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        let 之后 = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(之后.len(), 之前.len());
        for (a, b) in 之前.iter().zip(之后.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.sort_order, b.sort_order);
            assert_eq!(a.stage_type, b.stage_type);
        }
    }

    /// 用户删掉了「待评审」列：回填要退而求其次，插在第一条 done 之前。
    #[tokio::test]
    async fn 没有评审列时测试中插在已完成之前() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;
        sqlx::query("DELETE FROM project_statuses WHERE project_id = ?1 AND stage_type = 'review'")
            .bind(project_id)
            .execute(test_db.pool())
            .await
            .unwrap();

        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        assert_eq!(
            阶段顺序(test_db.pool(), project_id).await,
            vec!["backlog", "todo", "dev", "test", "done"]
        );
    }

    /// 既没有评审列也没有完成列：追加到末尾，不动任何既有列。
    #[tokio::test]
    async fn 既无评审也无完成时测试中追加到末尾() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;
        sqlx::query(
            "DELETE FROM project_statuses WHERE project_id = ?1 \
             AND stage_type IN ('review', 'done')",
        )
        .bind(project_id)
        .execute(test_db.pool())
        .await
        .unwrap();

        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        assert_eq!(
            阶段顺序(test_db.pool(), project_id).await,
            vec!["backlog", "todo", "dev", "test"]
        );
    }

    /// 回填不得改动任何需求的状态归属。
    #[tokio::test]
    async fn 迁移不改动需求的状态归属() {
        use api_types::issue::CreateIssueRequest;

        use crate::models::{issue::Issues, local_project::DEFAULT_USER_ID};

        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "老项目").await;
        退回迁移前(test_db.pool(), project_id).await;

        let done = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id,
                status_id: done.id,
                title: "已完成的需求".to_string(),
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
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        sqlx::raw_sql(回填迁移)
            .execute(test_db.pool())
            .await
            .unwrap();

        let 之后 = Issues::find_by_id(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(之后.status_id, done.id, "回填不得改动需求的状态列归属");
    }
}
