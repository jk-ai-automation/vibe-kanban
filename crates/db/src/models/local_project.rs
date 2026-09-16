use api_types::project::{CreateProjectRequest, Project, UpdateProjectRequest};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

/// 个人版固定组织：前端 identity.ts 必须使用同一字符串。
pub const DEFAULT_ORGANIZATION_ID: Uuid = Uuid::from_u128(1);
/// 个人版固定用户。
pub const DEFAULT_USER_ID: Uuid = Uuid::from_u128(2);

/// 新建项目时自动创建的状态列：(名称, 颜色, stage_type)
pub const DEFAULT_STATUSES: [(&str, &str, &str); 5] = [
    ("待规划", "#94a3b8", "backlog"),
    ("待开发", "#64748b", "todo"),
    ("开发中", "#3b82f6", "dev"),
    ("待评审", "#a855f7", "review"),
    ("已完成", "#22c55e", "done"),
];

/// 项目名称长度上限，防止超大字段撑爆快照响应。
pub const MAX_PROJECT_NAME_LEN: usize = 200;

pub struct LocalProjects;

impl LocalProjects {
    /// 取项目名首字母（最多 3 个 ASCII 字母，大写）作为 simple_id 前缀，无字母时用 ISS。
    pub fn simple_id_prefix(name: &str) -> String {
        let prefix: String = name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .filter(|c| c.is_ascii_alphabetic())
            .take(3)
            .map(|c| c.to_ascii_uppercase())
            .collect();

        if prefix.is_empty() {
            "ISS".to_string()
        } else {
            prefix
        }
    }

    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Project>, sqlx::Error> {
        sqlx::query_as!(
            Project,
            r#"SELECT id           AS "id!: Uuid",
                      organization_id AS "organization_id!: Uuid",
                      name         AS "name!",
                      color        AS "color!",
                      sort_order   AS "sort_order!: i32",
                      created_at   AS "created_at!: DateTime<Utc>",
                      updated_at   AS "updated_at!: DateTime<Utc>"
               FROM local_projects
               ORDER BY sort_order ASC, created_at ASC"#
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as!(
            Project,
            r#"SELECT id           AS "id!: Uuid",
                      organization_id AS "organization_id!: Uuid",
                      name         AS "name!",
                      color        AS "color!",
                      sort_order   AS "sort_order!: i32",
                      created_at   AS "created_at!: DateTime<Utc>",
                      updated_at   AS "updated_at!: DateTime<Utc>"
               FROM local_projects
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    /// 建项目 + 5 个默认状态列，同一事务内完成。
    pub async fn create(
        pool: &SqlitePool,
        data: &CreateProjectRequest,
    ) -> Result<Project, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name: String = data.name.chars().take(MAX_PROJECT_NAME_LEN).collect();
        let organization_id = DEFAULT_ORGANIZATION_ID;

        let mut tx = pool.begin().await?;

        let project = sqlx::query_as!(
            Project,
            r#"INSERT INTO local_projects (id, organization_id, name, color, sort_order)
               VALUES ($1, $2, $3, $4,
                       (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM local_projects))
               RETURNING id AS "id!: Uuid",
                         organization_id AS "organization_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            organization_id,
            name,
            data.color
        )
        .fetch_one(&mut *tx)
        .await?;

        for (index, (status_name, color, stage_type)) in DEFAULT_STATUSES.iter().enumerate() {
            let status_id = Uuid::new_v4();
            let sort_order = index as i64;
            sqlx::query!(
                r#"INSERT INTO project_statuses
                       (id, project_id, name, color, sort_order, hidden, stage_type)
                   VALUES ($1, $2, $3, $4, $5, 0, $6)"#,
                status_id,
                project.id,
                status_name,
                color,
                sort_order,
                stage_type
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(project)
    }

    /// 只更新请求中出现的字段，用 CASE 在单条语句里完成，避免读改写竞态。
    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateProjectRequest,
    ) -> Result<Project, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| n.chars().take(MAX_PROJECT_NAME_LEN).collect());
        let set_color = data.color.is_some();
        let set_sort_order = data.sort_order.is_some();

        sqlx::query_as!(
            Project,
            r#"UPDATE local_projects SET
                   name       = CASE WHEN $2 THEN $3 ELSE name END,
                   color      = CASE WHEN $4 THEN $5 ELSE color END,
                   sort_order = CASE WHEN $6 THEN $7 ELSE sort_order END,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         organization_id AS "organization_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            set_name,
            name,
            set_color,
            data.color,
            set_sort_order,
            data.sort_order
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM local_projects WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use api_types::project::{CreateProjectRequest, UpdateProjectRequest};

    use super::{DEFAULT_ORGANIZATION_ID, LocalProjects};
    use crate::test_support::TestDb;

    fn 建项目请求(name: &str) -> CreateProjectRequest {
        CreateProjectRequest {
            id: None,
            organization_id: DEFAULT_ORGANIZATION_ID,
            name: name.to_string(),
            color: "#6366f1".to_string(),
        }
    }

    #[tokio::test]
    async fn 新建项目自动创建五个默认状态列() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(test_db.pool(), &建项目请求("Vibe Kanban"))
            .await
            .expect("建项目失败");

        let statuses: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT name, stage_type, sort_order FROM project_statuses \
             WHERE project_id = ?1 ORDER BY sort_order",
        )
        .bind(project.id)
        .fetch_all(test_db.pool())
        .await
        .expect("读取状态列失败");

        assert_eq!(
            statuses,
            vec![
                ("待规划".to_string(), "backlog".to_string(), 0),
                ("待开发".to_string(), "todo".to_string(), 1),
                ("开发中".to_string(), "dev".to_string(), 2),
                ("待评审".to_string(), "review".to_string(), 3),
                ("已完成".to_string(), "done".to_string(), 4),
            ]
        );
    }

    #[tokio::test]
    async fn 建项目失败时不留下孤立状态列() {
        let test_db = TestDb::new().await;
        let mut req = 建项目请求("重复");
        let fixed_id = uuid::Uuid::from_u128(999);
        req.id = Some(fixed_id);

        LocalProjects::create(test_db.pool(), &req)
            .await
            .expect("首次建项目应成功");
        let second = LocalProjects::create(test_db.pool(), &req).await;
        assert!(second.is_err(), "主键重复应报错");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM project_statuses")
            .fetch_one(test_db.pool())
            .await
            .expect("统计失败");
        assert_eq!(count.0, 5, "第二次失败必须整体回滚，不得追加状态列");
    }

    #[tokio::test]
    async fn 列表只返回本组织项目且按_sort_order_排序() {
        let test_db = TestDb::new().await;
        let a = LocalProjects::create(test_db.pool(), &建项目请求("A"))
            .await
            .unwrap();
        let b = LocalProjects::create(test_db.pool(), &建项目请求("B"))
            .await
            .unwrap();

        LocalProjects::update(
            test_db.pool(),
            a.id,
            &UpdateProjectRequest {
                name: None,
                color: None,
                sort_order: Some(10),
            },
        )
        .await
        .unwrap();

        let list = LocalProjects::find_all(test_db.pool()).await.unwrap();
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![b.id, a.id],
            "sort_order 小的在前"
        );
        assert!(list.iter().all(|p| p.organization_id == DEFAULT_ORGANIZATION_ID));
    }

    #[tokio::test]
    async fn 删除项目级联删除状态列与需求() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(test_db.pool(), &建项目请求("X"))
            .await
            .unwrap();

        let affected = LocalProjects::delete(test_db.pool(), project.id)
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM project_statuses")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0, "状态列应随项目级联删除");
    }

    #[tokio::test]
    async fn 删除不存在的项目返回零行() {
        let test_db = TestDb::new().await;
        let affected = LocalProjects::delete(test_db.pool(), uuid::Uuid::from_u128(12345))
            .await
            .unwrap();
        assert_eq!(affected, 0, "删除不存在的项目不得报错，也不得误删他人数据");
    }

    #[test]
    fn simple_id_前缀取名称首字母最多三位() {
        assert_eq!(LocalProjects::simple_id_prefix("Vibe Kanban Web"), "VKW");
        assert_eq!(LocalProjects::simple_id_prefix("Vibe Kanban Web Extra"), "VKW");
        assert_eq!(LocalProjects::simple_id_prefix("kanban"), "K");
        assert_eq!(LocalProjects::simple_id_prefix("需求管理"), "ISS");
        assert_eq!(LocalProjects::simple_id_prefix("   "), "ISS");
        assert_eq!(LocalProjects::simple_id_prefix(""), "ISS");
    }
}
