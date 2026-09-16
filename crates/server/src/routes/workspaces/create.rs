use std::collections::HashMap;

use axum::{Json, extract::State, response::Json as ResponseJson};
use db::models::{
    requests::{
        CreateAndStartWorkspaceRequest, CreateAndStartWorkspaceResponse, CreateWorkspaceApiRequest,
    },
    workspace::{CreateWorkspace, Workspace},
};
use deployment::Deployment;
use services::services::container::ContainerService;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    error::ApiError,
    routes::workspaces::attachments::{
        ImportedIssueAttachment, import_issue_attachments_from_remote,
    },
};

pub(crate) async fn create_workspace_record(
    deployment: &DeploymentImpl,
    name: Option<String>,
    created_by_user_id: Uuid,
) -> Result<Workspace, ApiError> {
    let workspace_id = Uuid::new_v4();
    let branch_label = name
        .as_deref()
        .filter(|branch_label| !branch_label.is_empty())
        .unwrap_or("workspace");
    let git_branch_name = deployment
        .container()
        .git_branch_from_workspace(&workspace_id, branch_label)
        .await;

    let workspace = Workspace::create(
        &deployment.db().pool,
        &CreateWorkspace {
            branch: git_branch_name,
            name: name.filter(|workspace_name| !workspace_name.is_empty()),
        },
        workspace_id,
        created_by_user_id,
    )
    .await?;

    Ok(workspace)
}

pub async fn create_workspace(
    State(deployment): State<DeploymentImpl>,
    current_user: crate::middleware::local_session::CurrentUser,
    Json(payload): Json<CreateWorkspaceApiRequest>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    let workspace = create_workspace_record(&deployment, payload.name, current_user.id).await?;

    deployment
        .track_if_analytics_allowed(
            "workspace_created",
            serde_json::json!({
                "workspace_id": workspace.id.to_string(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(workspace)))
}

fn normalize_prompt(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn escape_markdown_label(label: &str) -> String {
    let mut escaped = String::with_capacity(label.len());
    for ch in label.chars() {
        if matches!(ch, '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn build_workspace_attachment_markdown(
    file: &ImportedIssueAttachment,
    label: &str,
    uses_image_markdown: bool,
) -> String {
    let path = format!(".vibe-attachments/{}", file.file.file_path);
    let normalized_label = if label.trim().is_empty() {
        file.file.original_name.as_str()
    } else {
        label
    };
    let escaped_label = escape_markdown_label(normalized_label);

    if uses_image_markdown {
        format!("![{}]({})", escaped_label, path)
    } else {
        format!("[{}]({})", escaped_label, path)
    }
}

struct ParsedAttachmentMarkdown<'a> {
    attachment_id: Uuid,
    label: &'a str,
    uses_image_markdown: bool,
    end: usize,
}

fn find_unescaped_char(haystack: &str, target: char) -> Option<usize> {
    let mut escaped = false;

    for (index, ch) in haystack.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == target {
            return Some(index);
        }
    }

    None
}

fn parse_attachment_markdown_at(
    prompt: &str,
    start: usize,
) -> Option<ParsedAttachmentMarkdown<'_>> {
    let rest = prompt.get(start..)?;
    let (uses_image_markdown, label_start_offset) = if rest.starts_with("![") {
        (true, 2)
    } else if rest.starts_with('[') {
        (false, 1)
    } else {
        return None;
    };

    let label_rest = rest.get(label_start_offset..)?;
    let label_end_offset = find_unescaped_char(label_rest, ']')?;
    let label = &label_rest[..label_end_offset];

    let after_label = label_rest.get(label_end_offset + 1..)?;
    let attachment_prefix = "(attachment://";
    if !after_label.starts_with(attachment_prefix) {
        return None;
    }

    let attachment_id_start =
        start + label_start_offset + label_end_offset + 1 + attachment_prefix.len();
    let attachment_id_rest = prompt.get(attachment_id_start..)?;
    let attachment_id_end_offset = attachment_id_rest.find(')')?;
    let attachment_id = Uuid::parse_str(&attachment_id_rest[..attachment_id_end_offset]).ok()?;

    Some(ParsedAttachmentMarkdown {
        attachment_id,
        label,
        uses_image_markdown,
        end: attachment_id_start + attachment_id_end_offset + 1,
    })
}

fn rewrite_imported_issue_attachments_markdown(
    prompt: &str,
    imported_attachments: &[ImportedIssueAttachment],
) -> String {
    if imported_attachments.is_empty() {
        return prompt.to_string();
    }

    let imported_by_attachment_id = imported_attachments
        .iter()
        .map(|attachment| (attachment.attachment_id, attachment))
        .collect::<HashMap<_, _>>();
    let mut rewritten = String::with_capacity(prompt.len());
    let mut index = 0;

    while index < prompt.len() {
        if let Some(parsed) = parse_attachment_markdown_at(prompt, index)
            && let Some(attachment) = imported_by_attachment_id.get(&parsed.attachment_id)
        {
            rewritten.push_str(&build_workspace_attachment_markdown(
                attachment,
                parsed.label,
                parsed.uses_image_markdown,
            ));
            index = parsed.end;
            continue;
        }

        let Some(ch) = prompt[index..].chars().next() else {
            break;
        };
        rewritten.push(ch);
        index += ch.len_utf8();
    }

    rewritten
}

pub async fn create_and_start_workspace(
    State(deployment): State<DeploymentImpl>,
    current_user: crate::middleware::local_session::CurrentUser,
    Json(payload): Json<CreateAndStartWorkspaceRequest>,
) -> Result<ResponseJson<ApiResponse<CreateAndStartWorkspaceResponse>>, ApiError> {
    let CreateAndStartWorkspaceRequest {
        name,
        repos,
        linked_issue,
        executor_config,
        prompt,
        attachment_ids,
    } = payload;

    let mut workspace_prompt = normalize_prompt(&prompt).ok_or_else(|| {
        ApiError::BadRequest(
            "A workspace prompt is required. Provide a non-empty `prompt`.".to_string(),
        )
    })?;

    if repos.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one repository is required".to_string(),
        ));
    }

    let mut managed_workspace = deployment
        .workspace_manager()
        .load_managed_workspace(create_workspace_record(&deployment, name, current_user.id).await?)
        .await?;

    for repo in &repos {
        managed_workspace
            .add_repository(repo, deployment.git())
            .await
            .map_err(ApiError::from)?;
    }

    if let Some(ids) = &attachment_ids {
        managed_workspace.associate_attachments(ids).await?;
    }

    // 个人版：linked_issue.issue_id 指向本地 issues 表时，绑定工作区并把需求推进到「开发中」，
    // 不再调用云端。团队版（本地查不到）保持原有的云端附件导入逻辑。
    let local_issue = match &linked_issue {
        Some(info) => db::models::issue::Issues::find_by_id(&deployment.db().pool, info.issue_id)
            .await
            .map_err(ApiError::Database)?,
        None => None,
    };

    if let Some(issue) = &local_issue {
        db::models::workspace::Workspace::set_issue_id(
            &deployment.db().pool,
            managed_workspace.workspace.id,
            Some(issue.id),
        )
        .await
        .map_err(ApiError::Database)?;

        if let Err(e) = db::models::issue::Issues::move_to_stage(
            &deployment.db().pool,
            issue.id,
            db::models::local_project_status::StageType::Dev,
        )
        .await
        {
            tracing::warn!("需求 {} 流转到开发中失败: {}", issue.id, e);
        }
    }

    if local_issue.is_none()
        && let Some(linked_issue) = &linked_issue
        && let Ok(client) = deployment.remote_client()
    {
        match import_issue_attachments_from_remote(
            &client,
            deployment.file(),
            linked_issue.issue_id,
        )
        .await
        {
            Ok(imported_attachments) if !imported_attachments.is_empty() => {
                let imported_ids = imported_attachments
                    .iter()
                    .map(|imported| imported.file.id)
                    .collect::<Vec<_>>();

                if let Err(e) = managed_workspace.associate_attachments(&imported_ids).await {
                    tracing::warn!("Failed to associate imported files with workspace: {}", e);
                }

                workspace_prompt = rewrite_imported_issue_attachments_markdown(
                    &workspace_prompt,
                    &imported_attachments,
                );

                tracing::info!(
                    "Imported {} files from issue {}",
                    imported_ids.len(),
                    linked_issue.issue_id
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to import issue attachments for issue {}: {}",
                    linked_issue.issue_id,
                    e
                );
            }
        }
    }

    let workspace = managed_workspace.workspace.clone();
    tracing::info!("Created workspace {}", workspace.id);

    let execution_process = deployment
        .container()
        .start_workspace(&workspace, executor_config.clone(), workspace_prompt)
        .await?;

    deployment
        .track_if_analytics_allowed(
            "workspace_created_and_started",
            serde_json::json!({
                "executor": &executor_config.executor,
                "variant": &executor_config.variant,
                "workspace_id": workspace.id.to_string(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(
        CreateAndStartWorkspaceResponse {
            workspace,
            execution_process,
        },
    )))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use db::models::file::File;
    use uuid::Uuid;

    use super::{ImportedIssueAttachment, rewrite_imported_issue_attachments_markdown};

    fn imported_file(
        attachment_id: Uuid,
        original_name: &str,
        file_path: &str,
        mime_type: Option<&str>,
    ) -> ImportedIssueAttachment {
        ImportedIssueAttachment {
            attachment_id,
            file: File {
                id: Uuid::new_v4(),
                file_path: file_path.to_string(),
                original_name: original_name.to_string(),
                mime_type: mime_type.map(str::to_string),
                size_bytes: 123,
                hash: "hash".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        }
    }

    #[test]
    fn rewrites_imported_non_image_attachment_links() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("[proposal.pdf](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "[proposal.pdf](.vibe-attachments/abc_proposal.pdf)"
        );
    }

    #[test]
    fn preserves_authored_image_markdown_for_imported_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("![diagram.png](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "diagram.png",
            "xyz_diagram.png",
            Some("image/png"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "![diagram.png](.vibe-attachments/xyz_diagram.png)"
        );
    }

    #[test]
    fn preserves_authored_link_markdown_for_imported_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("[diagram.png](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "diagram.png",
            "xyz_diagram.png",
            Some("image/png"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "[diagram.png](.vibe-attachments/xyz_diagram.png)"
        );
    }

    #[test]
    fn preserves_authored_image_markdown_for_imported_non_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("![proposal.pdf](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "![proposal.pdf](.vibe-attachments/abc_proposal.pdf)"
        );
    }

    #[test]
    fn leaves_unknown_attachment_references_unchanged() {
        let prompt = format!("[proposal.pdf](attachment://{})", Uuid::new_v4());
        let imported = vec![imported_file(
            Uuid::new_v4(),
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(rewritten, prompt);
    }

    #[test]
    fn rewrites_multiple_attachments_and_leaves_other_links_alone() {
        let image_attachment_id = Uuid::new_v4();
        let file_attachment_id = Uuid::new_v4();
        let prompt = format!(
            "See [doc.pdf](attachment://{}) and ![shot.png](attachment://{}). https://example.com",
            file_attachment_id, image_attachment_id
        );
        let imported = vec![
            imported_file(
                file_attachment_id,
                "doc.pdf",
                "doc_file.pdf",
                Some("application/pdf"),
            ),
            imported_file(
                image_attachment_id,
                "shot.png",
                "shot_file.png",
                Some("image/png"),
            ),
        ];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "See [doc.pdf](.vibe-attachments/doc_file.pdf) and ![shot.png](.vibe-attachments/shot_file.png). https://example.com"
        );
    }

    #[tokio::test]
    async fn 本地需求存在时绑定工作区并流转到开发中() {
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

        // 模拟路由中的本地分支
        Workspace::set_issue_id(test_db.pool(), workspace.id, Some(issue.id))
            .await
            .unwrap();
        Issues::move_to_stage(test_db.pool(), issue.id, StageType::Dev)
            .await
            .unwrap();

        let dev = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        let after = Issues::find_by_id(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status_id, dev.id);
        assert_eq!(
            Workspace::find_by_id(test_db.pool(), workspace.id)
                .await
                .unwrap()
                .unwrap()
                .issue_id,
            Some(issue.id)
        );
    }
}
