use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[cfg(not(feature = "qa-mode"))]
use crate::profile::ExecutorConfigs;
use crate::{
    actions::Executable,
    approvals::ExecutorApprovalService,
    env::ExecutionEnv,
    executors::{BaseCodingAgent, ExecutorError, SpawnedChild, StandardCodingAgentExecutor},
    profile::ExecutorConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct CodingAgentInitialRequest {
    pub prompt: String,
    /// Unified executor identity + overrides
    #[serde(alias = "executor_profile_id", alias = "profile_variant_label")]
    pub executor_config: ExecutorConfig,
    /// Optional relative path to execute the agent in (relative to container_ref).
    /// If None, uses the container_ref directory directly.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// 会话级插件目录（Claude Code `--plugin-dir`）。流水线用它注入技能包。
    ///
    /// **必须 `#[serde(default)]`**：库里 `execution_processes.executor_action` 的老 JSON
    /// 没有这个字段，反序列化不能失败。
    #[serde(default)]
    pub plugin_dirs: Vec<PathBuf>,
}

impl CodingAgentInitialRequest {
    pub fn base_executor(&self) -> BaseCodingAgent {
        self.executor_config.executor
    }

    pub fn effective_dir(&self, current_dir: &Path) -> std::path::PathBuf {
        match &self.working_dir {
            Some(rel_path) => current_dir.join(rel_path),
            None => current_dir.to_path_buf(),
        }
    }
}

#[async_trait]
impl Executable for CodingAgentInitialRequest {
    #[cfg_attr(feature = "qa-mode", allow(unused_variables))]
    async fn spawn(
        &self,
        current_dir: &Path,
        approvals: Arc<dyn ExecutorApprovalService>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let effective_dir = self.effective_dir(current_dir);

        #[cfg(feature = "qa-mode")]
        {
            tracing::info!("QA mode: using mock executor instead of real agent");
            let executor = crate::executors::qa_mock::QaMockExecutor;
            return executor.spawn(&effective_dir, &self.prompt, env).await;
        }

        #[cfg(not(feature = "qa-mode"))]
        {
            let profile_id = self.executor_config.profile_id();
            let mut agent = ExecutorConfigs::get_cached()
                .get_coding_agent(&profile_id)
                .ok_or(ExecutorError::UnknownExecutorType(profile_id.to_string()))?;

            if self.executor_config.has_overrides() {
                agent.apply_overrides(&self.executor_config);
            }
            agent.use_approvals(approvals.clone());
            agent.set_plugin_dirs(&self.plugin_dirs);

            agent.spawn(&effective_dir, &self.prompt, env).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ExecutorConfig;

    /// 库里 `execution_processes.executor_action` 的老 JSON 没有 `plugin_dirs`，
    /// 必须照常反序列化（否则老工作区的续聊会整条挂掉）。
    #[test]
    fn 旧数据没有_plugin_dirs_也能反序列化() {
        let old = r#"{"prompt":"做点事","executor_config":{"executor":"CLAUDE_CODE"},"working_dir":null}"#;
        let request: CodingAgentInitialRequest = serde_json::from_str(old).unwrap();
        assert!(request.plugin_dirs.is_empty());
        assert_eq!(request.prompt, "做点事");
    }

    #[test]
    fn 带_plugin_dirs_能往返() {
        let request = CodingAgentInitialRequest {
            prompt: "做点事".to_string(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            working_dir: None,
            plugin_dirs: vec![std::path::PathBuf::from("/tmp/vk/plugin")],
        };
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains("/tmp/vk/plugin"), "{text}");
        assert_eq!(
            serde_json::from_str::<CodingAgentInitialRequest>(&text).unwrap(),
            request
        );
    }
}
