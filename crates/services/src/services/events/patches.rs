use db::models::{
    execution_process::ExecutionProcess, scratch::Scratch, workspace::WorkspaceWithStatus,
};
use json_patch::{AddOperation, Patch, PatchOperation, RemoveOperation, ReplaceOperation};
use uuid::Uuid;

// Shared helper to escape JSON Pointer segments
fn escape_pointer_segment(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Helper functions for creating execution process-specific patches
pub mod execution_process_patch {
    use super::*;

    fn execution_process_path(process_id: Uuid) -> String {
        format!(
            "/execution_processes/{}",
            escape_pointer_segment(&process_id.to_string())
        )
    }

    /// Create patch for adding a new execution process
    pub fn add(process: &ExecutionProcess) -> Patch {
        Patch(vec![PatchOperation::Add(AddOperation {
            path: execution_process_path(process.id)
                .try_into()
                .expect("Execution process path should be valid"),
            value: serde_json::to_value(process)
                .expect("Execution process serialization should not fail"),
        })])
    }

    /// Create patch for updating an existing execution process
    pub fn replace(process: &ExecutionProcess) -> Patch {
        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: execution_process_path(process.id)
                .try_into()
                .expect("Execution process path should be valid"),
            value: serde_json::to_value(process)
                .expect("Execution process serialization should not fail"),
        })])
    }

    /// Create patch for removing an execution process
    pub fn remove(process_id: Uuid) -> Patch {
        Patch(vec![PatchOperation::Remove(RemoveOperation {
            path: execution_process_path(process_id)
                .try_into()
                .expect("Execution process path should be valid"),
        })])
    }
}

/// Helper functions for creating workspace-specific patches
pub mod workspace_patch {
    use super::*;

    fn workspace_path(workspace_id: Uuid) -> String {
        format!(
            "/workspaces/{}",
            escape_pointer_segment(&workspace_id.to_string())
        )
    }

    pub fn add(workspace: &WorkspaceWithStatus) -> Patch {
        Patch(vec![PatchOperation::Add(AddOperation {
            path: workspace_path(workspace.id)
                .try_into()
                .expect("Workspace path should be valid"),
            value: serde_json::to_value(workspace)
                .expect("Workspace serialization should not fail"),
        })])
    }

    pub fn replace(workspace: &WorkspaceWithStatus) -> Patch {
        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: workspace_path(workspace.id)
                .try_into()
                .expect("Workspace path should be valid"),
            value: serde_json::to_value(workspace)
                .expect("Workspace serialization should not fail"),
        })])
    }

    pub fn remove(workspace_id: Uuid) -> Patch {
        Patch(vec![PatchOperation::Remove(RemoveOperation {
            path: workspace_path(workspace_id)
                .try_into()
                .expect("Workspace path should be valid"),
        })])
    }
}

/// Helper functions for creating scratch-specific patches.
/// All patches use path "/scratch" - filtering is done by matching id and payload type in the value.
pub mod scratch_patch {
    use super::*;

    const SCRATCH_PATH: &str = "/scratch";

    /// Create patch for adding a new scratch
    pub fn add(scratch: &Scratch) -> Patch {
        Patch(vec![PatchOperation::Add(AddOperation {
            path: SCRATCH_PATH
                .try_into()
                .expect("Scratch path should be valid"),
            value: serde_json::to_value(scratch).expect("Scratch serialization should not fail"),
        })])
    }

    /// Create patch for updating an existing scratch
    pub fn replace(scratch: &Scratch) -> Patch {
        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: SCRATCH_PATH
                .try_into()
                .expect("Scratch path should be valid"),
            value: serde_json::to_value(scratch).expect("Scratch serialization should not fail"),
        })])
    }

    /// Create patch for removing a scratch.
    /// Uses Replace with deleted marker so clients can filter by id and payload type.
    pub fn remove(scratch_id: Uuid, scratch_type_str: &str) -> Patch {
        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: SCRATCH_PATH
                .try_into()
                .expect("Scratch path should be valid"),
            value: serde_json::json!({
                "id": scratch_id,
                "payload": { "type": scratch_type_str },
                "deleted": true
            }),
        })])
    }
}

/// 需求相关 patch。三张表都走 "/<表名>/<id>" 的路径，
/// 前端按 value.project_id 过滤，因此 value 必须包含 project_id。
macro_rules! id_keyed_patch_module {
    ($module:ident, $root:expr, $ty:ty) => {
        pub mod $module {
            use super::*;

            fn path_for(id: Uuid) -> String {
                format!("{}/{}", $root, escape_pointer_segment(&id.to_string()))
            }

            pub fn add(record: &$ty) -> Patch {
                Patch(vec![PatchOperation::Add(AddOperation {
                    path: path_for(record.id).try_into().expect("路径应合法"),
                    value: serde_json::to_value(record).expect("序列化不应失败"),
                })])
            }

            pub fn replace(record: &$ty) -> Patch {
                Patch(vec![PatchOperation::Replace(ReplaceOperation {
                    path: path_for(record.id).try_into().expect("路径应合法"),
                    value: serde_json::to_value(record).expect("序列化不应失败"),
                })])
            }

            pub fn remove(id: Uuid) -> Patch {
                Patch(vec![PatchOperation::Remove(RemoveOperation {
                    path: path_for(id).try_into().expect("路径应合法"),
                })])
            }
        }
    };
}

id_keyed_patch_module!(issue_patch, "/issues", api_types::issue::Issue);
id_keyed_patch_module!(
    project_status_patch,
    "/project_statuses",
    db::models::local_project_status::LocalProjectStatus
);
id_keyed_patch_module!(
    issue_comment_patch,
    "/issue_comments",
    api_types::issue_comment::IssueComment
);

// 流水线两张表（契约 §3）：路径 /pipeline_runs/{id}、/pipeline_stage_runs/{id}，
// 值带 project_id，需求流按它过滤。
id_keyed_patch_module!(
    pipeline_run_patch,
    "/pipeline_runs",
    db::models::pipeline::PipelineRun
);
id_keyed_patch_module!(
    pipeline_stage_run_patch,
    "/pipeline_stage_runs",
    db::models::pipeline::PipelineStageRun
);

/// Helper functions for creating approval-specific patches.
pub mod approvals_patch {
    use super::*;

    const PENDING_PATH: &str = "/pending";

    fn pending_path(approval_id: &str) -> String {
        format!("{}/{}", PENDING_PATH, escape_pointer_segment(approval_id))
    }

    pub fn snapshot(pending: &[crate::services::approvals::ApprovalInfo]) -> Patch {
        let pending: serde_json::Map<String, serde_json::Value> = pending
            .iter()
            .map(|info| {
                (
                    info.approval_id.clone(),
                    serde_json::to_value(info).unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();

        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: PENDING_PATH
                .try_into()
                .expect("Pending approvals path should be valid"),
            value: serde_json::Value::Object(pending),
        })])
    }

    pub fn created(info: &crate::services::approvals::ApprovalInfo) -> Patch {
        let value = serde_json::to_value(info).unwrap_or(serde_json::Value::Null);
        Patch(vec![PatchOperation::Replace(ReplaceOperation {
            path: pending_path(&info.approval_id)
                .try_into()
                .expect("Approval path should be valid"),
            value,
        })])
    }

    pub fn resolved(approval_id: &str) -> Patch {
        Patch(vec![PatchOperation::Remove(RemoveOperation {
            path: pending_path(approval_id)
                .try_into()
                .expect("Approval path should be valid"),
        })])
    }
}

#[cfg(test)]
mod tests {
    use api_types::issue::Issue;
    use chrono::Utc;
    use uuid::Uuid;

    use super::{issue_comment_patch, issue_patch, project_status_patch};

    fn 示例需求(project_id: Uuid) -> Issue {
        Issue {
            id: Uuid::from_u128(7),
            project_id,
            issue_number: 1,
            simple_id: "VK-1".to_string(),
            status_id: Uuid::from_u128(8),
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
            creator_user_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn 新增需求的_patch_路径为_issues_加_id() {
        let issue = 示例需求(Uuid::from_u128(1));
        let patch = issue_patch::add(&issue);
        let op = patch.0.first().expect("必须有一个操作");
        assert_eq!(
            op.path().to_string(),
            format!("/issues/{}", Uuid::from_u128(7))
        );
    }

    #[test]
    fn 需求_patch_的值包含_project_id_供前端按项目过滤() {
        let project_id = Uuid::from_u128(1);
        let patch = issue_patch::replace(&示例需求(project_id));
        let json_patch::PatchOperation::Replace(op) = patch.0.first().unwrap() else {
            panic!("应为 Replace 操作");
        };
        assert_eq!(op.value["project_id"], project_id.to_string());
    }

    #[test]
    fn 需求_patch_不包含任何本地路径信息() {
        let patch = issue_patch::replace(&示例需求(Uuid::from_u128(1)));
        let text = serde_json::to_string(&patch).unwrap();
        for leak in ["container_ref", "worktree", "/Users/", "/home/"] {
            assert!(!text.contains(leak), "需求 patch 不得泄露 {leak}");
        }
    }

    #[test]
    fn 删除需求生成_remove_操作() {
        let patch = issue_patch::remove(Uuid::from_u128(7));
        assert!(matches!(
            patch.0.first().unwrap(),
            json_patch::PatchOperation::Remove(_)
        ));
    }

    #[test]
    fn 状态列与评论的_patch_路径前缀正确() {
        let status = db::models::local_project_status::LocalProjectStatus {
            id: Uuid::from_u128(9),
            project_id: Uuid::from_u128(1),
            name: "开发中".to_string(),
            color: "#3b82f6".to_string(),
            sort_order: 2,
            hidden: false,
            stage_type: "dev".to_string(),
            created_at: Utc::now(),
        };
        assert_eq!(
            project_status_patch::add(&status).0[0].path().to_string(),
            format!("/project_statuses/{}", Uuid::from_u128(9))
        );

        let comment = api_types::issue_comment::IssueComment {
            id: Uuid::from_u128(10),
            issue_id: Uuid::from_u128(7),
            author_id: None,
            parent_id: None,
            message: "评论".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(
            issue_comment_patch::add(&comment).0[0].path().to_string(),
            format!("/issue_comments/{}", Uuid::from_u128(10))
        );
    }
}
