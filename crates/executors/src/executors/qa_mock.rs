//! QA Mode: Mock executor for testing
//!
//! This module provides a mock executor that:
//! 1. Performs random file operations (create, delete, modify); for pipeline prompts
//!    (contract §5 / C9) it only writes the declared artifacts instead
//! 2. Streams 10 mock log entries over 10 seconds (1 second in pipeline mode)
//! 3. Outputs logs in ClaudeJson format for compatibility with existing log normalization

use std::{path::Path, process::Stdio, sync::Arc};

use async_trait::async_trait;
use rand::seq::SliceRandom as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use ts_rs::TS;
use workspace_utils::{command_ext::GroupSpawnNoWindowExt, msg_store::MsgStore};

use crate::{
    env::ExecutionEnv,
    executors::{
        BaseCodingAgent, ExecutorError, SpawnedChild, StandardCodingAgentExecutor,
        claude::{
            ClaudeContentItem, ClaudeJson, ClaudeMessage, ClaudeMessageContent, ClaudeToolData,
        },
    },
    logs::utils::EntryIndexProvider,
    profile::ExecutorConfig,
};

/// Mock executor for QA testing
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS, JsonSchema)]
pub struct QaMockExecutor;

#[async_trait]
impl StandardCodingAgentExecutor for QaMockExecutor {
    fn apply_overrides(&mut self, _executor_config: &ExecutorConfig) {}

    async fn spawn(
        &self,
        current_dir: &Path,
        prompt: &str,
        _env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        info!("QA Mock Executor: spawning mock execution");

        // 1. Perform file operations (or write pipeline artifacts) before spawning the log output process
        let pipeline_mode = apply_side_effects(current_dir, prompt).await;

        // 2. Generate mock logs and write to temp file to avoid shell escaping issues
        let logs = generate_mock_logs(prompt);
        let temp_dir = std::env::temp_dir();
        let log_file = temp_dir.join(format!("qa_mock_logs_{}.jsonl", uuid::Uuid::new_v4()));

        // Write all logs to file, one per line
        let content = logs.join("\n") + "\n";
        tokio::fs::write(&log_file, &content)
            .await
            .map_err(|e| ExecutorError::Io(std::io::Error::other(e)))?;

        // 3. Create shell script that reads file and outputs with delays
        let script = mock_log_script(&log_file, pipeline_mode);

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(&script)
            .current_dir(current_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.group_spawn_no_window().map_err(ExecutorError::Io)?;
        Ok(SpawnedChild::from(child))
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &Path,
        prompt: &str,
        _session_id: &str,
        _reset_to_message_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        // QA mode doesn't support real sessions, just spawn fresh
        info!("QA Mock Executor: follow-up request treated as new spawn");
        self.spawn(current_dir, prompt, env).await
    }

    fn normalize_logs(
        &self,
        msg_store: Arc<MsgStore>,
        current_dir: &Path,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        // Reuse Claude's log processor since we output ClaudeJson format
        let entry_index_provider = EntryIndexProvider::start_from(&msg_store);
        let h1 = crate::executors::claude::ClaudeLogProcessor::process_logs(
            msg_store,
            current_dir,
            entry_index_provider,
            crate::executors::claude::HistoryStrategy::Default,
        );
        vec![h1]
    }

    fn default_mcp_config_path(&self) -> Option<std::path::PathBuf> {
        None // QA mock doesn't need MCP config
    }

    fn get_preset_options(&self) -> ExecutorConfig {
        ExecutorConfig {
            executor: BaseCodingAgent::QaMock,
            variant: None,
            model_id: Some("qa-mock".to_string()),
            agent_id: None,
            reasoning_id: None,
            permission_policy: Some(crate::model_selector::PermissionPolicy::Auto),
        }
    }
}

/// 流水线提示词（带「产出物目录」行，契约 §5 / C9）：只写约定产出物，不做随机删改
/// ——随机删改会波及工作区里的 `.vk/` 产出物。普通提示词：沿用原来的随机文件操作。
/// 返回是否是流水线模式。
async fn apply_side_effects(current_dir: &Path, prompt: &str) -> bool {
    match crate::pipeline_prompt::parse_pipeline_prompt(prompt) {
        Some(parsed) => {
            match crate::pipeline_prompt::write_mock_artifacts(&parsed) {
                Ok(paths) => info!("QA Mock: 写出流水线产出物 {:?}", paths),
                Err(e) => warn!("QA Mock: 写流水线产出物失败: {}", e),
            }
            true
        }
        None => {
            perform_file_operations(current_dir).await;
            false
        }
    }
}

/// 逐行输出日志的 shell 脚本。流水线模式每行 0.1 秒：七个阶段按原来的 1 秒/行
/// 要 70 秒以上，端到端测试太慢。
/// Using IFS= read -r to preserve exact content (no word splitting, no backslash interpretation)
fn mock_log_script(log_file: &Path, pipeline_mode: bool) -> String {
    let line_delay = if pipeline_mode { "0.1" } else { "1" };
    format!(
        r#"while IFS= read -r line; do echo "$line"; sleep {}; done < "{}"; rm -f "{}""#,
        line_delay,
        log_file.display(),
        log_file.display()
    )
}

/// Perform random file operations in the worktree
async fn perform_file_operations(dir: &Path) {
    info!("QA Mock: performing file operations in {:?}", dir);

    // Create: qa_created_{uuid}.txt
    let uuid = uuid::Uuid::new_v4();
    let new_file = dir.join(format!("qa_created_{}.txt", uuid));
    match tokio::fs::write(&new_file, "QA mode created this file\n").await {
        Ok(_) => info!("QA Mock: created file {:?}", new_file),
        Err(e) => warn!("QA Mock: failed to create file: {}", e),
    }

    // Find files (excluding .git and binary files)
    let files: Vec<_> = walkdir::WalkDir::new(dir)
        .max_depth(3) // Limit depth to avoid long walks
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !e.path().to_string_lossy().contains(".git"))
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ["rs", "ts", "js", "txt", "md", "json"].contains(&ext))
        })
        .collect();

    if files.len() >= 2 {
        // Pick random indices before any await points (thread_rng is not Send)
        let (remove_idx, modify_idx) = {
            let mut rng = rand::thread_rng();
            let mut indices: Vec<usize> = (0..files.len()).collect();
            indices.shuffle(&mut rng);
            (indices.first().copied(), indices.get(1).copied())
        };

        // Remove a random file (first shuffled index)
        if let Some(idx) = remove_idx {
            let file_to_remove = files[idx].path().to_path_buf();
            // Don't remove the file we just created
            if file_to_remove != new_file {
                match tokio::fs::remove_file(&file_to_remove).await {
                    Ok(_) => info!("QA Mock: removed file {:?}", file_to_remove),
                    Err(e) => warn!("QA Mock: failed to remove file: {}", e),
                }
            }
        }

        // Modify a different random file (second shuffled index)
        if let Some(idx) = modify_idx {
            let file_to_modify = files[idx].path().to_path_buf();
            // Don't modify the file we just created
            if file_to_modify != new_file {
                match tokio::fs::read_to_string(&file_to_modify).await {
                    Ok(content) => {
                        let modified = format!(
                            "{}\n// QA modification at {}\n",
                            content,
                            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
                        );
                        match tokio::fs::write(&file_to_modify, modified).await {
                            Ok(_) => info!("QA Mock: modified file {:?}", file_to_modify),
                            Err(e) => warn!("QA Mock: failed to write modified file: {}", e),
                        }
                    }
                    Err(e) => warn!("QA Mock: failed to read file for modification: {}", e),
                }
            }
        }
    } else {
        info!(
            "QA Mock: not enough files found for remove/modify operations (found {})",
            files.len()
        );
    }
}

/// Generate 10 mock log entries in ClaudeJson format using strongly-typed structs
fn generate_mock_logs(prompt: &str) -> Vec<String> {
    let session_id = uuid::Uuid::new_v4().to_string();

    let logs: Vec<ClaudeJson> = vec![
        // 1. System init
        ClaudeJson::System {
            subtype: Some("init".to_string()),
            session_id: Some(session_id.clone()),
            cwd: None,
            tools: None,
            model: Some("qa-mock-executor".to_string()),
            api_key_source: Some("unknown".to_string()),
            status: None,
            slash_commands: vec![],
            plugins: vec![],
            agents: vec![],
            task_id: None,
            tool_use_id: None,
            description: None,
            task_type: None,
            prompt: None,
            summary: None,
            last_tool_name: None,
        },
        // 2. Assistant thinking
        ClaudeJson::Assistant {
            message: ClaudeMessage {
                id: Some("msg-qa-1".to_string()),
                message_type: Some("message".to_string()),
                role: "assistant".to_string(),
                model: Some("qa-mock".to_string()),
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::Thinking {
                    thinking: "Analyzing the QA task and preparing mock execution...".to_string(),
                }]),
                stop_reason: None,
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-1".to_string()),
        },
        // 3. Read tool use
        ClaudeJson::Assistant {
            message: ClaudeMessage {
                id: Some("msg-qa-2".to_string()),
                message_type: Some("message".to_string()),
                role: "assistant".to_string(),
                model: Some("qa-mock".to_string()),
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolUse {
                    id: "qa-tool-1".to_string(),
                    tool_data: ClaudeToolData::Read {
                        file_path: "README.md".to_string(),
                    },
                }]),
                stop_reason: None,
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-2".to_string()),
        },
        // 4. Read tool result
        ClaudeJson::User {
            message: ClaudeMessage {
                id: Some("msg-qa-3".to_string()),
                message_type: Some("message".to_string()),
                role: "user".to_string(),
                model: None,
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolResult {
                    tool_use_id: "qa-tool-1".to_string(),
                    content: serde_json::json!(
                        "# Project README\\n\\nThis is a QA test repository."
                    ),
                    is_error: Some(false),
                }]),
                stop_reason: None,
            },
            is_synthetic: false,
            is_replay: false,
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-3".to_string()),
        },
        // 5. Write tool use
        ClaudeJson::Assistant {
            message: ClaudeMessage {
                id: Some("msg-qa-4".to_string()),
                message_type: Some("message".to_string()),
                role: "assistant".to_string(),
                model: Some("qa-mock".to_string()),
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolUse {
                    id: "qa-tool-2".to_string(),
                    tool_data: ClaudeToolData::Write {
                        file_path: "qa_output.txt".to_string(),
                        content: "QA generated content".to_string(),
                    },
                }]),
                stop_reason: None,
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-4".to_string()),
        },
        // 6. Write tool result
        ClaudeJson::User {
            message: ClaudeMessage {
                id: Some("msg-qa-5".to_string()),
                message_type: Some("message".to_string()),
                role: "user".to_string(),
                model: None,
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolResult {
                    tool_use_id: "qa-tool-2".to_string(),
                    content: serde_json::json!("File written successfully"),
                    is_error: Some(false),
                }]),
                stop_reason: None,
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-5".to_string()),
            is_synthetic: false,
            is_replay: false,
        },
        // 7. Bash tool use
        ClaudeJson::Assistant {
            message: ClaudeMessage {
                id: Some("msg-qa-6".to_string()),
                message_type: Some("message".to_string()),
                role: "assistant".to_string(),
                model: Some("qa-mock".to_string()),
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolUse {
                    id: "qa-tool-3".to_string(),
                    tool_data: ClaudeToolData::Bash {
                        command: "echo 'QA test complete'".to_string(),
                        description: Some("Run QA test command".to_string()),
                    },
                }]),
                stop_reason: None,
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-6".to_string()),
        },
        // 8. Bash tool result
        ClaudeJson::User {
            message: ClaudeMessage {
                id: Some("msg-qa-7".to_string()),
                message_type: Some("message".to_string()),
                role: "user".to_string(),
                model: None,
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::ToolResult {
                    tool_use_id: "qa-tool-3".to_string(),
                    content: serde_json::json!("QA test complete\\n"),
                    is_error: Some(false),
                }]),
                stop_reason: None,
            },
            is_synthetic: false,
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-7".to_string()),
            is_replay: false,
        },
        // 9. Assistant final message
        ClaudeJson::Assistant {
            message: ClaudeMessage {
                id: Some("msg-qa-8".to_string()),
                message_type: Some("message".to_string()),
                role: "assistant".to_string(),
                model: Some("qa-mock".to_string()),
                content: ClaudeMessageContent::Array(vec![ClaudeContentItem::Text {
                    text: format!(
                        "QA mode execution completed successfully.\\n\\nI performed the following operations:\\n1. Read README.md\\n2. Created qa_output.txt\\n3. Ran a test command\\nOriginal prompt: {}",
                        prompt
                    ),
                }]),
                stop_reason: Some("end_turn".to_string()),
            },
            session_id: Some(session_id.clone()),
            uuid: Some("uuid-qa-8".to_string()),
        },
        // 10. Result success
        ClaudeJson::Result {
            subtype: Some("success".to_string()),
            is_error: Some(false),
            duration_ms: Some(10000),
            result: None,
            error: None,
            num_turns: Some(3),
            session_id: Some(session_id),
            model_usage: None,
            usage: None,
        },
    ];

    // Serialize to JSON strings - this ensures proper escaping
    logs.into_iter()
        .map(|log| serde_json::to_string(&log).expect("ClaudeJson should serialize"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_mock_logs_count() {
        let logs = generate_mock_logs("test prompt");
        assert_eq!(logs.len(), 10, "Should generate exactly 10 log entries");
    }

    #[test]
    fn test_generate_mock_logs_valid_json() {
        let logs = generate_mock_logs("test prompt");
        for (i, log) in logs.iter().enumerate() {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(log);
            assert!(
                parsed.is_ok(),
                "Log entry {} should be valid JSON: {}",
                i,
                log
            );
        }
    }

    #[test]
    fn test_generate_mock_logs_deserializes_to_claudejson() {
        let logs = generate_mock_logs("test prompt");
        for (i, log) in logs.iter().enumerate() {
            let parsed: Result<ClaudeJson, _> = serde_json::from_str(log);
            assert!(
                parsed.is_ok(),
                "Log entry {} should deserialize to ClaudeJson: {} - error: {:?}",
                i,
                log,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_escape_special_characters() {
        let logs = generate_mock_logs("test with \"quotes\" and\nnewlines");
        // The final assistant message (index 8) should contain the prompt
        let final_log = &logs[8];
        let parsed: ClaudeJson = serde_json::from_str(final_log).unwrap();

        if let ClaudeJson::Assistant { message, .. } = parsed {
            if let ClaudeMessageContent::Array(items) = &message.content {
                if let Some(ClaudeContentItem::Text { text }) = items.first() {
                    assert!(text.contains("test with \"quotes\" and\nnewlines"));
                } else {
                    panic!("Expected Text content item");
                }
            } else {
                panic!("Expected Array content");
            }
        } else {
            panic!("Expected Assistant variant");
        }
    }

    #[tokio::test]
    async fn 流水线提示词只写产出物不做随机改动() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("repo");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("a.md"), "a").unwrap();
        std::fs::write(work.join("b.md"), "b").unwrap();
        let artifacts = dir.path().join(".vk/runs/VK-1");
        let prompt = format!(
            "使用技能 vk-requirement。\n需求：VK-1 示例\n产出物目录（绝对路径）：{}\n必须产出：requirement.md\n",
            artifacts.display()
        );

        assert!(apply_side_effects(&work, &prompt).await);
        assert!(artifacts.join("requirement.md").exists());
        assert_eq!(std::fs::read_to_string(work.join("a.md")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(work.join("b.md")).unwrap(), "b");
        assert_eq!(
            std::fs::read_dir(&work).unwrap().count(),
            2,
            "流水线模式不应新建 qa_created_*.txt"
        );
    }

    #[tokio::test]
    async fn 普通提示词仍走随机文件操作() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!apply_side_effects(dir.path(), "随便改点东西").await);
        let created = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("qa_created_"));
        assert!(created, "普通模式行为不变：会新建 qa_created_*.txt");
    }

    #[test]
    fn 流水线模式日志每行间隔零点一秒() {
        let script = mock_log_script(Path::new("/tmp/x.jsonl"), true);
        assert!(script.contains("sleep 0.1;"), "{script}");
        let script = mock_log_script(Path::new("/tmp/x.jsonl"), false);
        assert!(script.contains("sleep 1;"), "{script}");
    }
}
