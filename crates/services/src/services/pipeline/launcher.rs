//! 引擎与执行层之间的接缝。引擎只依赖 [`StageLauncher`]：生产环境用
//! [`ContainerStageLauncher`] 接到容器服务；测试用假实现或 [`NoopStageLauncher`]。

use std::path::PathBuf;

use async_trait::async_trait;
use db::models::{
    execution_process::ExecutionProcessRunReason, session::Session, workspace::Workspace,
};
use executors::{
    actions::{
        ExecutorAction, ExecutorActionType,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    },
    profile::ExecutorConfig,
};
use uuid::Uuid;

use super::engine::PipelineError;
use crate::services::container::ContainerService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedStep {
    pub session_id: Uuid,
    /// 会话里第一个进程（有 setup 脚本时是 setup 进程）。
    pub execution_process_id: Uuid,
}

#[async_trait]
pub trait StageLauncher: Send + Sync {
    /// 确保工作区目录与各仓库 worktree 存在，返回工作区根目录（container_ref）。
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError>;

    /// 在工作区里开新会话跑编码智能体；`run_setup` 为 true 时先跑 setup 脚本。
    /// `plugin_dirs` 是会话级插件目录（技能包），只有 Claude Code 会用。
    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 在已有会话里跑检查脚本（run_reason = PipelineStep）。
    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 停掉工作区里正在跑的进程（取消运行时用）。
    async fn stop_workspace(&self, workspace_id: Uuid);
}

/// 把模板的 checks 拼成一个脚本：`set -e`，任一命令失败即以非零退出。
pub fn checks_script(checks: &[String]) -> String {
    let mut script = String::from("set -e\n");
    for check in checks {
        script.push_str(check.trim());
        script.push('\n');
    }
    script
}

pub struct ContainerStageLauncher<C> {
    container: C,
}

impl<C> ContainerStageLauncher<C> {
    pub fn new(container: C) -> Self {
        Self { container }
    }
}

impl<C> ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync,
{
    async fn load_workspace(&self, workspace_id: Uuid) -> Result<Workspace, PipelineError> {
        Workspace::find_by_id(&self.container.db().pool, workspace_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("工作区 {workspace_id}")))
    }
}

fn launch_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Launch(error.to_string())
}

#[async_trait]
impl<C> StageLauncher for ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync + 'static,
{
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let root = self
            .container
            .ensure_container_exists(&workspace)
            .await
            .map_err(launch_error)?;
        Ok(PathBuf::from(root))
    }

    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let process = self
            .container
            .start_new_session(
                &workspace,
                executor_config.clone(),
                prompt,
                run_setup,
                plugin_dirs,
            )
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id: process.session_id,
            execution_process_id: process.id,
        })
    }

    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let session = Session::find_by_id(&self.container.db().pool, session_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("会话 {session_id}")))?;
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script,
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::PipelineCheck,
                working_dir: session.agent_working_dir.clone(),
            }),
            None,
        );
        let process = self
            .container
            .start_execution(
                &workspace,
                &session,
                &action,
                &ExecutionProcessRunReason::PipelineStep,
            )
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id,
            execution_process_id: process.id,
        })
    }

    async fn stop_workspace(&self, workspace_id: Uuid) {
        if let Ok(workspace) = self.load_workspace(workspace_id).await {
            self.container.try_stop(&workspace, false).await;
        }
    }
}

/// 不接执行层的启动器：所有启动都报错。给只测状态与错误映射的路由单测用。
pub struct NoopStageLauncher;

#[async_trait]
impl StageLauncher for NoopStageLauncher {
    async fn prepare_workspace(&self, _workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_agent_session(
        &self,
        _workspace_id: Uuid,
        _executor_config: &ExecutorConfig,
        _prompt: String,
        _run_setup: bool,
        _plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_checks(
        &self,
        _workspace_id: Uuid,
        _session_id: Uuid,
        _script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn stop_workspace(&self, _workspace_id: Uuid) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 检查脚本逐行执行且任一失败即退出() {
        let script = checks_script(&["pnpm run lint".to_string(), "  cargo test  ".to_string()]);
        assert_eq!(script, "set -e\npnpm run lint\ncargo test\n");
    }

    #[tokio::test]
    async fn 空启动器所有启动都报错() {
        let launcher = NoopStageLauncher;
        let id = Uuid::new_v4();
        assert!(launcher.prepare_workspace(id).await.is_err());
        assert!(
            launcher
                .start_agent_session(
                    id,
                    &ExecutorConfig::new(executors::executors::BaseCodingAgent::ClaudeCode),
                    "p".to_string(),
                    true,
                    Vec::new()
                )
                .await
                .is_err()
        );
        assert!(
            launcher
                .start_checks(id, id, "true".to_string())
                .await
                .is_err()
        );
        launcher.stop_workspace(id).await;
    }
}
