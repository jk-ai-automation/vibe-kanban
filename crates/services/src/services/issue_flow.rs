//! 个人版需求状态自动流转。
//! 所有入口都以「工作区 → 绑定需求」为起点；工作区未绑定需求时静默跳过，
//! 因此团队版（需求在云端）不会被影响。

use db::models::{issue::Issues, local_project_status::StageType, workspace::Workspace};
use sqlx::SqlitePool;
use uuid::Uuid;

/// 可由开发流程触发的阶段。刻意不暴露 backlog/todo：那两个只由用户手动拖拽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueFlowStage {
    Dev,
    Review,
    Done,
}

impl From<IssueFlowStage> for StageType {
    fn from(stage: IssueFlowStage) -> Self {
        match stage {
            IssueFlowStage::Dev => StageType::Dev,
            IssueFlowStage::Review => StageType::Review,
            IssueFlowStage::Done => StageType::Done,
        }
    }
}

/// 把工作区绑定的本地需求推进到指定阶段。
/// 返回 true 表示确实改了需求；工作区不存在、未绑定需求、
/// 或项目里没有对应阶段的状态列时返回 false，不报错。
pub async fn advance_issue_for_workspace(
    pool: &SqlitePool,
    workspace_id: Uuid,
    stage: IssueFlowStage,
) -> Result<bool, sqlx::Error> {
    let Some(workspace) = Workspace::find_by_id(pool, workspace_id).await? else {
        return Ok(false);
    };
    let Some(issue_id) = workspace.issue_id else {
        return Ok(false);
    };

    match Issues::move_to_stage(pool, issue_id, stage.into()).await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(db::models::issue::IssueError::Database(err)) => Err(err),
        Err(err) => {
            tracing::warn!("需求 {} 自动流转失败: {}", issue_id, err);
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
            workspace::{CreateWorkspace, Workspace},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{IssueFlowStage, advance_issue_for_workspace};

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid, Uuid) {
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
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/demo".to_string(),
                name: None,
            },
            Uuid::new_v4(),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        Workspace::set_issue_id(test_db.pool(), workspace.id, Some(issue.id))
            .await
            .unwrap();

        (project.id, issue.id, workspace.id)
    }

    #[tokio::test]
    async fn 建_pr_把绑定需求推进到待评审() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Review)
            .await
            .unwrap();

        let review = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Review)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue.status_id, review.id);
        assert!(issue.completed_at.is_none(), "待评审不写完成时间");
    }

    #[tokio::test]
    async fn 合并把绑定需求推进到已完成并写完成时间() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();

        let done = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue.status_id, done.id);
        assert!(issue.completed_at.is_some());
    }

    #[tokio::test]
    async fn 未绑定需求的工作区不做任何事() {
        let test_db = TestDb::new().await;
        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/none".to_string(),
                name: None,
            },
            Uuid::new_v4(),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();

        let moved = advance_issue_for_workspace(test_db.pool(), workspace.id, IssueFlowStage::Done)
            .await
            .unwrap();
        assert!(!moved, "未绑定需求时返回 false，且不得报错");
    }

    #[tokio::test]
    async fn 不存在的工作区不报错() {
        let test_db = TestDb::new().await;
        let moved = advance_issue_for_workspace(
            test_db.pool(),
            Uuid::from_u128(88888),
            IssueFlowStage::Review,
        )
        .await
        .unwrap();
        assert!(!moved);
    }

    #[tokio::test]
    async fn 重复流转是幂等的() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();
        let first = Issues::find_by_id(test_db.pool(), issue_id)
            .await
            .unwrap()
            .unwrap();

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();
        let second = Issues::find_by_id(test_db.pool(), issue_id)
            .await
            .unwrap()
            .unwrap();

        let done = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.status_id, done.id);
        assert_eq!(second.status_id, done.id);
    }
}
