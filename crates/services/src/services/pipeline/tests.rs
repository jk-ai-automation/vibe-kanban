//! 引擎集成测试：真 SQLite + 真引擎 + qa-mode 同款产出物，假启动器代替容器。

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use PipelineStageKey::{Deliver, Develop, Requirement, Review, Spec, Test, TestDesign};
use PipelineStageStatus::{Failed, Passed, Pending, Rejected, Running, Skipped, WaitingGate};
use api_types::issue::Issue;
use db::{
    models::{
        execution_process::{
            CreateExecutionProcess, ExecutionProcess, ExecutionProcessRunReason,
            ExecutionProcessStatus,
        },
        issue::Issues,
        local_project::DEFAULT_USER_ID,
        local_project_status::{ProjectStatuses, StageType},
        pipeline::{
            ArtifactKind, GateDecisionKind, GateDecisionRequest, IssuePipelineView,
            PipelineRunStatus, PipelineRuns, PipelineStageKey, PipelineStageRuns,
            PipelineStageStatus,
        },
        session::{CreateSession, Session},
    },
    test_support::TestDb,
};
use executors::{
    actions::{
        ExecutorAction, ExecutorActionType,
        coding_agent_initial::CodingAgentInitialRequest,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    },
    executors::BaseCodingAgent,
    profile::ExecutorConfig,
};
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    FinishedProcess, PipelineError, PipelineExitEvent, PipelineService, StartPipelineInput,
    artifacts,
    test_support::{FakeLaunch, FakeLaunchKind, FakeLauncher, 准备工作区, 准备需求},
    transition::stage_column,
};

struct 场景 {
    test_db: TestDb,
    dir: TempDir,
    launcher: Arc<FakeLauncher>,
    service: PipelineService,
    issue: Issue,
    workspace_id: Uuid,
}

impl 场景 {
    async fn 新建(title: &str) -> Self {
        let test_db = TestDb::new().await;
        let dir = tempfile::tempdir().unwrap();
        let issue = 准备需求(&test_db, title).await;
        let workspace_id = 准备工作区(&test_db).await;
        let launcher = Arc::new(FakeLauncher::new(
            test_db.pool().clone(),
            dir.path().join("workspace"),
        ));
        let service = PipelineService::new(test_db.db.clone(), launcher.clone());
        Self {
            test_db,
            dir,
            launcher,
            service,
            issue,
            workspace_id,
        }
    }

    fn 仓库目录(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn 写仓库模板(&self, yaml: &str) {
        std::fs::create_dir_all(self.仓库目录().join(".vibe")).unwrap();
        std::fs::write(self.仓库目录().join(".vibe/pipeline.yaml"), yaml).unwrap();
    }

    async fn 启动(&self) -> Result<IssuePipelineView, PipelineError> {
        self.service
            .start(StartPipelineInput {
                issue: self.issue.clone(),
                workspace_id: self.workspace_id,
                repo_root: self.仓库目录(),
                executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
                template_key: None,
            })
            .await
    }

    async fn 视图(&self) -> IssuePipelineView {
        self.service
            .view_for_issue(self.issue.id)
            .await
            .unwrap()
            .expect("应有运行")
    }

    async fn 当前(&self) -> (PipelineStageKey, PipelineStageStatus) {
        let view = self.视图().await;
        let last = view.stages.last().expect("至少一条尝试");
        (last.stage_key, last.status)
    }

    async fn 结束(&self, launch: &FakeLaunch, ok: bool) {
        self.service
            .on_process_finished(FinishedProcess {
                execution_process_id: launch.execution_process_id,
                session_id: launch.session_id,
                run_reason: match launch.kind {
                    FakeLaunchKind::Agent => ExecutionProcessRunReason::CodingAgent,
                    FakeLaunchKind::Checks => ExecutionProcessRunReason::PipelineStep,
                },
                status: if ok {
                    ExecutionProcessStatus::Completed
                } else {
                    ExecutionProcessStatus::Failed
                },
                exit_code: Some(if ok { 0 } else { 1 }),
                has_next_action: false,
                chain_continues: false,
            })
            .await
            .unwrap();
    }

    async fn 结束为(
        &self,
        launch: &FakeLaunch,
        run_reason: ExecutionProcessRunReason,
        status: ExecutionProcessStatus,
        exit_code: Option<i64>,
    ) {
        self.service
            .on_process_finished(FinishedProcess {
                execution_process_id: launch.execution_process_id,
                session_id: launch.session_id,
                run_reason,
                status,
                exit_code,
                has_next_action: false,
                chain_continues: false,
            })
            .await
            .unwrap();
    }

    fn 产出物目录(&self) -> PathBuf {
        artifacts::artifacts_dir(&self.dir.path().join("workspace"), &self.issue.simple_id)
    }

    async fn 打回(&self, comment: &str) -> IssuePipelineView {
        let id = self.等待确认的阶段().await;
        self.service
            .decide_gate(
                id,
                GateDecisionRequest {
                    decision: GateDecisionKind::Reject,
                    comment: Some(comment.to_string()),
                },
                Some(DEFAULT_USER_ID),
            )
            .await
            .unwrap()
    }

    async fn 结束最近一次启动(&self, ok: bool) {
        let launch = self.launcher.last();
        self.结束(&launch, ok).await;
    }

    async fn 等待确认的阶段(&self) -> Uuid {
        self.视图()
            .await
            .stages
            .iter()
            .rev()
            .find(|stage| stage.status == WaitingGate)
            .expect("应有等待确认的阶段")
            .id
    }

    async fn 通过关卡(&self) {
        let id = self.等待确认的阶段().await;
        self.service
            .decide_gate(
                id,
                GateDecisionRequest {
                    decision: GateDecisionKind::Approve,
                    comment: None,
                },
                Some(DEFAULT_USER_ID),
            )
            .await
            .unwrap();
    }

    async fn 推进到(&self, target: PipelineStageKey) {
        for _ in 0..30 {
            match self.当前().await {
                (key, Running) if key == target => return,
                (_, Running) => self.结束最近一次启动(true).await,
                (_, WaitingGate) => self.通过关卡().await,
                other => panic!("推进途中遇到 {other:?}"),
            }
        }
        panic!("30 步内没有走到 {target:?}");
    }

    async fn 需求所在列(&self) -> StageType {
        let pool = self.test_db.pool();
        let issue = Issues::find_by_id(pool, self.issue.id)
            .await
            .unwrap()
            .unwrap();
        for column in [
            StageType::Backlog,
            StageType::Todo,
            StageType::Dev,
            StageType::Review,
            StageType::Test,
            StageType::Done,
        ] {
            let status = ProjectStatuses::find_stage(pool, issue.project_id, column)
                .await
                .unwrap()
                .unwrap();
            if status.id == issue.status_id {
                return column;
            }
        }
        panic!("需求不在任何阶段列里");
    }

    fn 尝试(view: &IssuePipelineView, key: PipelineStageKey) -> Vec<(i64, PipelineStageStatus)> {
        view.stages
            .iter()
            .filter(|stage| stage.stage_key == key)
            .map(|stage| (stage.attempt, stage.status))
            .collect()
    }
}

#[tokio::test]
async fn 黄金路径七个阶段依次跑完() {
    let s = 场景::新建("用户可以导出报表").await;
    let view = s.启动().await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(view.template.key, "standard");
    assert_eq!(view.template.stages.len(), 7);

    for key in PipelineStageKey::ALL {
        assert_eq!(s.当前().await, (key, Running), "应轮到 {key:?}");
        assert_eq!(
            s.需求所在列().await,
            stage_column(key),
            "{key:?} 阶段需求应在对应列"
        );
        s.结束最近一次启动(true).await;
        if matches!(key, Requirement | Spec | TestDesign) {
            assert_eq!(s.当前().await, (key, WaitingGate));
            assert_eq!(s.视图().await.run.status, PipelineRunStatus::WaitingGate);
            let pending = PipelineRuns::list_pending(s.test_db.pool(), None)
                .await
                .unwrap();
            assert_eq!(pending.len(), 1, "等人工时出现在待处理列表");
            assert_eq!(pending[0].stage_run.stage_key, key);
            s.通过关卡().await;
        }
    }

    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Completed);
    assert!(view.run.finished_at.is_some());
    assert_eq!(view.stages.len(), 7);
    assert!(
        view.stages
            .iter()
            .all(|stage| stage.status == Passed && stage.attempt == 1)
    );
    assert_eq!(view.decisions.len(), 3);
    assert!(
        view.decisions
            .iter()
            .all(|decision| decision.decision == GateDecisionKind::Approve)
    );
    for kind in [
        ArtifactKind::Requirement,
        ArtifactKind::Spec,
        ArtifactKind::Plan,
        ArtifactKind::TestCases,
        ArtifactKind::TraceMatrix,
        ArtifactKind::Review,
        ArtifactKind::TestReport,
        ArtifactKind::DeliveryReport,
    ] {
        assert!(
            view.artifacts.iter().any(|artifact| artifact.kind == kind),
            "缺产出物 {kind:?}"
        );
    }
    assert_eq!(s.需求所在列().await, StageType::Done);
    assert!(
        PipelineRuns::list_pending(s.test_db.pool(), None)
            .await
            .unwrap()
            .is_empty()
    );

    let launches = s.launcher.launches();
    assert_eq!(launches.len(), 7, "每个阶段开一个新会话");
    assert!(launches[0].run_setup, "首个会话跑 setup 脚本");
    assert!(
        launches[1..].iter().all(|l| !l.run_setup),
        "后续会话不跑 setup"
    );
    assert!(
        launches[0]
            .prompt
            .contains(&format!("需求：{} 用户可以导出报表", s.issue.simple_id))
    );
    assert!(launches[0].prompt.contains("必须产出：requirement.md"));
    assert!(launches[1].prompt.contains("上一阶段产出：requirement.md"));
    assert!(
        launches[3].prompt.contains("必须产出：无"),
        "开发阶段不产出文件"
    );
}

#[tokio::test]
async fn 人工打回后同阶段重跑且意见进提示词() {
    let s = 场景::新建("打回").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    let view = s
        .service
        .decide_gate(
            waiting,
            GateDecisionRequest {
                decision: GateDecisionKind::Reject,
                comment: Some("  补充验收标准 ".to_string()),
            },
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Rejected), (2, Running)]
    );
    assert_eq!(view.stages[0].error.as_deref(), Some("补充验收标准"));
    assert_eq!(view.decisions[0].comment.as_deref(), Some("补充验收标准"));
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert!(
        s.launcher
            .last()
            .prompt
            .contains("上一次被打回的意见：补充验收标准")
    );
}

#[tokio::test]
async fn 关卡决策的错误映射() {
    let s = 场景::新建("决策错误").await;
    let view = s.启动().await.unwrap();
    let running = view.stages[0].id;
    let approve = GateDecisionRequest {
        decision: GateDecisionKind::Approve,
        comment: None,
    };
    assert!(matches!(
        s.service.decide_gate(running, approve.clone(), None).await,
        Err(PipelineError::Conflict(_))
    ));
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    assert!(matches!(
        s.service
            .decide_gate(
                waiting,
                GateDecisionRequest {
                    decision: GateDecisionKind::Reject,
                    comment: Some("   ".to_string()),
                },
                None,
            )
            .await,
        Err(PipelineError::BadRequest(_))
    ));
    assert!(matches!(
        s.service.decide_gate(Uuid::new_v4(), approve, None).await,
        Err(PipelineError::NotFound(_))
    ));
}

#[tokio::test]
async fn 评审阻断一次后重跑通过() {
    let s = 场景::新建("评审 [qa:review-blocker-once]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    s.结束最近一次启动(true).await;

    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Review), vec![(1, Failed), (2, Running)]);
    let first = view.stages.iter().find(|x| x.stage_key == Review).unwrap();
    assert!(
        first
            .error
            .as_deref()
            .unwrap()
            .contains("评审发现 1 个阻断项")
    );
    assert!(
        s.launcher
            .last()
            .prompt
            .contains("上一次被打回的意见：评审发现 1 个阻断项")
    );
    assert_eq!(s.需求所在列().await, StageType::Review);

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Test, Running));
}

#[tokio::test]
async fn 评审始终阻断时用尽轮次转人工() {
    let s = 场景::新建("评审 [qa:always-fail-review]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    for _ in 0..3 {
        s.结束最近一次启动(true).await;
    }
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(
        场景::尝试(&view, Review),
        vec![(1, Failed), (2, Failed), (3, Failed)]
    );
    assert!(
        view.stages
            .last()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("已累计失败 3 次")
    );
    let pending = PipelineRuns::list_pending(s.test_db.pool(), None)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stage_run.stage_key, Review);
    assert_eq!(pending[0].stage_run.status, Failed);
}

#[tokio::test]
async fn 失败的运行可以继续再给一次机会() {
    let s = 场景::新建("评审 [qa:always-fail-review]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    for _ in 0..3 {
        s.结束最近一次启动(true).await;
    }
    let launches_before = s.launcher.launches().len();
    let view = s.service.resume(s.视图().await.run.id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(场景::尝试(&view, Review).last(), Some(&(4, Running)));
    assert_eq!(s.launcher.launches().len(), launches_before + 1);
    assert!(s.launcher.last().prompt.contains("已累计失败 3 次"));
}

#[tokio::test]
async fn 测试归因代码缺陷回到开发() {
    let s = 场景::新建("测试 [qa:test-fail-code-once]").await;
    s.启动().await.unwrap();
    s.推进到(Test).await;
    s.结束最近一次启动(true).await;

    assert_eq!(s.当前().await, (Develop, Running));
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Develop), vec![(1, Passed), (2, Running)]);
    assert_eq!(s.需求所在列().await, StageType::Dev);
    assert!(s.launcher.last().prompt.contains("测试失败 1 条"));

    s.推进到(Deliver).await;
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Test), vec![(1, Failed), (2, Passed)]);
    assert_eq!(场景::尝试(&view, Review), vec![(1, Passed), (2, Passed)]);
}

#[tokio::test]
async fn 测试归因用例问题回到用例设计并重新走人工关卡() {
    let s = 场景::新建("测试 [qa:test-fail-case-once]").await;
    s.启动().await.unwrap();
    s.推进到(Test).await;
    s.结束最近一次启动(true).await;

    assert_eq!(s.当前().await, (TestDesign, Running));
    assert_eq!(s.需求所在列().await, StageType::Todo);
    s.结束最近一次启动(true).await;
    assert_eq!(
        s.当前().await,
        (TestDesign, WaitingGate),
        "改过的用例要人重新确认"
    );
    assert_eq!(s.视图().await.run.status, PipelineRunStatus::WaitingGate);
}

#[tokio::test]
async fn 智能体进程失败按轮次重跑并回喂原因() {
    let s = 场景::新建("进程失败").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(false).await;
    let view = s.视图().await;
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Failed), (2, Running)]
    );
    assert!(
        view.stages[0]
            .error
            .as_deref()
            .unwrap()
            .contains("编码智能体 进程未成功结束")
    );
}

#[tokio::test]
async fn 缺产出物按阶段失败处理() {
    let s = 场景::新建("缺产出物").await;
    s.launcher.write_artifacts.store(false, Ordering::SeqCst);
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let view = s.视图().await;
    assert_eq!(view.stages[0].status, Failed);
    assert_eq!(
        view.stages[0].error.as_deref(),
        Some("技能没有产出 requirement.md")
    );
    assert_eq!(s.当前().await, (Requirement, Running));
}

#[tokio::test]
async fn 暂停后阶段结束不再调度_继续后启动() {
    let s = 场景::新建("暂停").await;
    let run_id = s.启动().await.unwrap().run.id;
    let view = s.service.pause(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    assert!(matches!(
        s.service.pause(run_id).await,
        Err(PipelineError::Conflict(_))
    ));

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Requirement, WaitingGate));
    assert_eq!(
        s.视图().await.run.status,
        PipelineRunStatus::Paused,
        "暂停中保持 paused"
    );

    s.通过关卡().await;
    let view = s.视图().await;
    assert_eq!(s.当前().await, (Spec, Pending));
    assert_eq!(view.run.current_stage_key, Spec);
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    assert_eq!(s.launcher.launches().len(), 1, "暂停中不启动新阶段");

    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(s.当前().await, (Spec, Running));
    assert_eq!(s.launcher.launches().len(), 2);
}

#[tokio::test]
async fn 取消后回调被忽略且可以重新启动() {
    let s = 场景::新建("取消").await;
    let run_id = s.启动().await.unwrap().run.id;
    let view = s.service.cancel(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Cancelled);
    assert_eq!(view.stages[0].status, Skipped);
    assert_eq!(*s.launcher.stopped.lock().unwrap(), vec![s.workspace_id]);

    s.结束最近一次启动(true).await;
    let view = s.视图().await;
    assert_eq!(view.stages.len(), 1, "取消后的回调不再推进");
    assert!(matches!(
        s.service.cancel(run_id).await,
        Err(PipelineError::Conflict(_))
    ));
    assert!(matches!(
        s.service.resume(run_id).await,
        Err(PipelineError::Conflict(_))
    ));

    let again = s.启动().await.unwrap();
    assert_ne!(again.run.id, run_id, "取消后可以重新开一条");
}

#[tokio::test]
async fn 同一需求重复启动返回冲突() {
    let s = 场景::新建("重复").await;
    s.启动().await.unwrap();
    assert!(matches!(s.启动().await, Err(PipelineError::Conflict(_))));
}

#[tokio::test]
async fn 未知模板返回请求无效() {
    let s = 场景::新建("模板").await;
    let result = s
        .service
        .start(StartPipelineInput {
            issue: s.issue.clone(),
            workspace_id: s.workspace_id,
            repo_root: s.仓库目录(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            template_key: Some("fancy".to_string()),
        })
        .await;
    assert!(matches!(result, Err(PipelineError::BadRequest(_))));
}

#[tokio::test]
async fn 启动失败记为阶段失败且修好后可以继续() {
    let s = 场景::新建("启动失败").await;
    s.launcher.fail_start.store(true, Ordering::SeqCst);
    let view = s.启动().await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(view.stages[0].status, Failed);
    assert!(
        view.stages[0]
            .error
            .as_deref()
            .unwrap()
            .contains("启动执行失败")
    );

    s.launcher.fail_start.store(false, Ordering::SeqCst);
    let view = s.service.resume(view.run.id).await.unwrap();
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Failed), (2, Running)]
    );
}

#[tokio::test]
async fn 服务重启后中断的阶段记失败且旧回调被忽略() {
    let s = 场景::新建("重启").await;
    s.启动().await.unwrap();
    assert_eq!(s.service.recover_interrupted().await.unwrap(), 1);
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(
        view.stages[0].error.as_deref(),
        Some("服务重启，阶段执行被中断")
    );

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Requirement, Failed));
}

/// 预期行为（批 3 审查确认）：服务重启被标 failed 的中断尝试**计入**本阶段失败次数。
/// `count_failed` 只按尝试状态计数，不区分失败原因；人工打回是 rejected，不计入。
#[tokio::test]
async fn 服务重启被标_failed_的中断尝试计入失败次数() {
    let s = 场景::新建("重启计数").await;
    s.启动().await.unwrap();
    s.service.recover_interrupted().await.unwrap();
    let run_id = s.视图().await.run.id;
    s.service.resume(run_id).await.unwrap();

    // 第 2 次尝试失败：累计 2 次（含中断那次）< 3，同阶段重跑。
    s.结束最近一次启动(false).await;
    let view = s.视图().await;
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Failed), (2, Failed), (3, Running)]
    );

    // 第 3 次尝试失败：累计 3 次 = 上限，运行失败转人工。
    s.结束最近一次启动(false).await;
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Failed), (2, Failed), (3, Failed)]
    );
}

#[tokio::test]
async fn 非终点事件与无关会话被忽略() {
    let s = 场景::新建("忽略").await;
    s.启动().await.unwrap();
    let launch = s.launcher.last();
    s.service
        .on_process_finished(FinishedProcess {
            execution_process_id: launch.execution_process_id,
            session_id: launch.session_id,
            run_reason: ExecutionProcessRunReason::CodingAgent,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action: true,
            chain_continues: true,
        })
        .await
        .unwrap();
    s.service
        .on_process_finished(FinishedProcess {
            execution_process_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_reason: ExecutionProcessRunReason::CodingAgent,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action: false,
            chain_continues: false,
        })
        .await
        .unwrap();
    assert_eq!(s.当前().await, (Requirement, Running));
}

#[tokio::test]
async fn 开发阶段有检查命令时先跑检查再判关卡() {
    let s = 场景::新建("检查").await;
    s.写仓库模板(
        "version: 1\nstages:\n  - key: develop\n    skill: vk-develop\n    checks: [\"cargo test\"]\n    gate: { auto: checks_passed }\n  - key: review\n    skill: vk-review\n    artifacts: [review.json]\n    gate: { auto: no_blocking_findings }\n",
    );
    let view = s.启动().await.unwrap();
    assert_eq!(view.template.key, "repo");
    assert_eq!(s.需求所在列().await, StageType::Dev);

    let agent = s.launcher.last();
    s.结束(&agent, true).await;
    let checks = s.launcher.last();
    assert_eq!(checks.kind, FakeLaunchKind::Checks);
    assert_eq!(
        checks.session_id, agent.session_id,
        "检查脚本在同一会话里跑"
    );
    assert_eq!(checks.prompt, "set -e\ncargo test\n");
    assert_eq!(s.当前().await, (Develop, Running));
    assert_eq!(
        s.视图().await.stages[0].execution_process_id,
        Some(checks.execution_process_id)
    );

    s.结束(&checks, false).await;
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Develop), vec![(1, Failed), (2, Running)]);
    assert!(
        view.stages[0]
            .error
            .as_deref()
            .unwrap()
            .contains("检查脚本 进程未成功结束")
    );

    s.结束最近一次启动(true).await;
    let checks = s.launcher.last();
    assert_eq!(checks.kind, FakeLaunchKind::Checks);
    s.结束(&checks, true).await;
    assert_eq!(s.当前().await, (Review, Running));
}

#[tokio::test]
async fn 仓库模板损坏时回落内置模板并记警告() {
    let s = 场景::新建("坏模板").await;
    s.写仓库模板("version: 9\nstages: []\n");
    let view = s.启动().await.unwrap();
    assert_eq!(view.template.key, "standard");
    let internals = PipelineRuns::internals(s.test_db.pool(), view.run.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        internals
            .template_warning
            .as_deref()
            .unwrap()
            .contains(".vibe/pipeline.yaml")
    );
}

/// 只钉护栏语义：模型层护栏拒绝（`mark_started` 返回 RowNotFound）说明状态已被别的路径
/// 改变，启动流程不能再把尝试记成 failed 覆盖掉它，也不能报错。这样留下的「运行未结束、
/// 没有活动尝试」的卡死态由「继续」恢复（同阶段新尝试）。
#[tokio::test]
async fn 启动途中尝试被改成终态时不覆盖为失败() {
    let s = 场景::新建("启动竞争").await;
    s.launcher
        .skip_running_on_start
        .store(true, Ordering::SeqCst);
    let view = s.启动().await.unwrap();
    assert_eq!(view.stages.len(), 1);
    assert_eq!(view.stages[0].status, Skipped);
    assert_eq!(view.stages[0].error, None, "不写启动失败原因");
    assert_eq!(view.stages[0].session_id, None);
    assert_eq!(
        view.run.status,
        PipelineRunStatus::Running,
        "运行状态不被改成 failed"
    );

    // 晚到的回调找不到 running 的尝试，被忽略。
    s.结束最近一次启动(true).await;
    assert_eq!(s.视图().await.stages.len(), 1);

    s.launcher
        .skip_running_on_start
        .store(false, Ordering::SeqCst);
    let view = s.service.resume(view.run.id).await.unwrap();
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Skipped), (2, Running)]
    );
}

// ---------------------------------------------------------------------------
// 第 5 批审查补充
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 重跑时智能体没写新文件判缺产出物而不是沿用旧文件() {
    let s = 场景::新建("旧产出物").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let old = std::fs::read_to_string(s.产出物目录().join("requirement.md")).unwrap();

    s.launcher.write_artifacts.store(false, Ordering::SeqCst);
    s.打回("补充").await;
    assert!(
        !s.产出物目录().join("requirement.md").exists(),
        "重跑前旧文件被改名"
    );
    assert_eq!(
        std::fs::read_to_string(s.产出物目录().join("requirement.md.prev")).unwrap(),
        old
    );
    s.结束最近一次启动(true).await;
    let view = s.视图().await;
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Rejected), (2, Failed), (3, Running)]
    );
    assert_eq!(
        view.stages[1].error.as_deref(),
        Some("技能没有产出 requirement.md")
    );
}

#[tokio::test]
async fn 退出事件读真实进程行_顺序_setup_链断按失败_并行_setup_忽略() {
    let s = 场景::新建("setup 链断").await;
    s.启动().await.unwrap();
    let launch = s.launcher.last();
    let pool = s.test_db.pool();
    Session::create(
        pool,
        &CreateSession {
            executor: None,
            name: None,
        },
        launch.session_id,
        s.workspace_id,
    )
    .await
    .unwrap();
    let setup = |next: Option<Box<ExecutorAction>>| {
        ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script: "true".to_string(),
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::SetupScript,
                working_dir: None,
            }),
            next,
        )
    };
    let agent = ExecutorAction::new(
        ExecutorActionType::CodingAgentInitialRequest(CodingAgentInitialRequest {
            prompt: "p".to_string(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            working_dir: None,
        }),
        None,
    );
    let mut ids = Vec::new();
    for action in [setup(None), setup(Some(Box::new(agent)))] {
        let process = ExecutionProcess::create(
            pool,
            &CreateExecutionProcess {
                session_id: launch.session_id,
                executor_action: action,
                run_reason: ExecutionProcessRunReason::SetupScript,
            },
            Uuid::new_v4(),
            &[],
        )
        .await
        .unwrap();
        ExecutionProcess::update_completion(
            pool,
            process.id,
            ExecutionProcessStatus::Completed,
            Some(0),
        )
        .await
        .unwrap();
        ids.push(process.id);
    }

    // 并行模式的 setup（不带 next_action）不是终点。
    s.service
        .handle_exit_event(PipelineExitEvent {
            execution_process_id: ids[0],
            chain_continues: false,
        })
        .await;
    assert_eq!(s.当前().await, (Requirement, Running));

    // 顺序模式的 setup 带 next_action 却没接上：链断，按失败处理。
    s.service
        .handle_exit_event(PipelineExitEvent {
            execution_process_id: ids[1],
            chain_continues: false,
        })
        .await;
    let view = s.视图().await;
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Failed), (2, Running)]
    );
    assert_eq!(
        view.stages[0].error.as_deref(),
        Some("setup 脚本后智能体未启动")
    );
}

#[tokio::test]
async fn 用户手动停止时运行暂停_不计轮次_继续后新开尝试() {
    let s = 场景::新建("手动停止").await;
    let run_id = s.启动().await.unwrap().run.id;
    for attempt in 1..=4 {
        let launch = s.launcher.last();
        s.结束为(
            &launch,
            ExecutionProcessRunReason::CodingAgent,
            ExecutionProcessStatus::Killed,
            None,
        )
        .await;
        let view = s.视图().await;
        assert_eq!(
            view.run.status,
            PipelineRunStatus::Paused,
            "第 {attempt} 次"
        );
        assert_eq!(view.stages.last().unwrap().status, Failed);
        assert_eq!(
            view.stages.last().unwrap().error.as_deref(),
            Some("用户手动停止")
        );
        assert_eq!(s.launcher.launches().len(), attempt, "停止后不自动重跑");

        let view = s.service.resume(run_id).await.unwrap();
        assert_eq!(view.run.status, PipelineRunStatus::Running);
        assert_eq!(
            场景::尝试(&view, Requirement).last(),
            Some(&(attempt as i64 + 1, Running)),
            "手动停止不计轮次，超过 max_rounds 也照样新开尝试"
        );
        assert!(!s.launcher.last().prompt.contains("用户手动停止"));
    }
}

#[tokio::test]
async fn 重复关卡决策第二次返回冲突且只记一条决策() {
    let s = 场景::新建("重复决策").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    let approve = GateDecisionRequest {
        decision: GateDecisionKind::Approve,
        comment: None,
    };
    s.service
        .decide_gate(waiting, approve.clone(), None)
        .await
        .unwrap();
    assert!(matches!(
        s.service.decide_gate(waiting, approve, None).await,
        Err(PipelineError::Conflict(_))
    ));
    let view = s.视图().await;
    assert_eq!(view.decisions.len(), 1);
    assert_eq!(s.当前().await, (Spec, Running));
}

#[tokio::test]
async fn 半写留下的卡死态可以继续_通过则进入下一阶段() {
    let s = 场景::新建("卡死通过").await;
    let run_id = s.启动().await.unwrap().run.id;
    assert!(
        matches!(
            s.service.resume(run_id).await,
            Err(PipelineError::Conflict(_))
        ),
        "有活动尝试的 running 不能继续"
    );
    s.结束最近一次启动(true).await;
    // 模拟「阶段已关、下一阶段没建」：直接把等待确认的尝试改成 passed。
    let waiting = s.等待确认的阶段().await;
    PipelineStageRuns::set_status(s.test_db.pool(), waiting, Passed, None, None)
        .await
        .unwrap();
    assert_eq!(s.视图().await.run.status, PipelineRunStatus::WaitingGate);

    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(s.当前().await, (Spec, Running));
}

#[tokio::test]
async fn 半写留下的卡死态可以继续_打回则同阶段新尝试() {
    let s = 场景::新建("卡死打回").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    PipelineStageRuns::set_status(s.test_db.pool(), waiting, Rejected, None, Some("补充范围"))
        .await
        .unwrap();
    PipelineRuns::update_status(s.test_db.pool(), run_id, PipelineRunStatus::Running, None)
        .await
        .unwrap();

    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Rejected), (2, Running)]
    );
    assert!(
        s.launcher
            .last()
            .prompt
            .contains("上一次被打回的意见：补充范围")
    );
}

#[tokio::test]
async fn cleanup_脚本也是阶段终点() {
    let s = 场景::新建("cleanup 终点").await;
    s.启动().await.unwrap();
    let launch = s.launcher.last();
    s.结束为(
        &launch,
        ExecutionProcessRunReason::CleanupScript,
        ExecutionProcessStatus::Completed,
        Some(0),
    )
    .await;
    assert_eq!(s.当前().await, (Requirement, WaitingGate));
}

#[tokio::test]
async fn 暂停时阶段仍在跑_继续后回到_running_不重复启动() {
    let s = 场景::新建("暂停在跑").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.service.pause(run_id).await.unwrap();
    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(场景::尝试(&view, Requirement), vec![(1, Running)]);
    assert_eq!(s.launcher.launches().len(), 1);
}

#[tokio::test]
async fn 等待确认时暂停再继续回到等待确认() {
    let s = 场景::新建("等确认暂停").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.结束最近一次启动(true).await;
    let view = s.service.pause(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::WaitingGate);
    assert_eq!(s.当前().await, (Requirement, WaitingGate));
}

#[tokio::test]
async fn 暂停中打回建同阶段_pending_不启动() {
    let s = 场景::新建("暂停打回").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.结束最近一次启动(true).await;
    s.service.pause(run_id).await.unwrap();
    let view = s.打回("再想想").await;
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    assert_eq!(
        场景::尝试(&view, Requirement),
        vec![(1, Rejected), (2, Pending)]
    );
    assert_eq!(s.launcher.launches().len(), 1);
}

#[tokio::test]
async fn 最后阶段为人工关卡时通过即完成() {
    let s = 场景::新建("单阶段").await;
    s.写仓库模板(
        "version: 1\nstages:\n  - key: requirement\n    skill: vk-requirement\n    artifacts: [requirement.md]\n    gate: { human: \"确认需求\" }\n",
    );
    let view = s.启动().await.unwrap();
    assert_eq!(view.template.key, "repo");
    s.结束最近一次启动(true).await;
    s.通过关卡().await;
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Completed);
    assert_eq!(场景::尝试(&view, Requirement), vec![(1, Passed)]);
    assert_eq!(s.需求所在列().await, StageType::Done);
}

#[tokio::test]
async fn 最后一条等待确认或_pending_时取消不停进程() {
    let s = 场景::新建("取消等确认").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.结束最近一次启动(true).await;
    let view = s.service.cancel(run_id).await.unwrap();
    assert_eq!(view.stages[0].status, Skipped);
    assert!(s.launcher.stopped.lock().unwrap().is_empty());

    let s = 场景::新建("取消 pending").await;
    let run_id = s.启动().await.unwrap().run.id;
    s.结束最近一次启动(true).await;
    s.service.pause(run_id).await.unwrap();
    s.通过关卡().await;
    assert_eq!(s.当前().await, (Spec, Pending));
    let view = s.service.cancel(run_id).await.unwrap();
    assert_eq!(view.stages.last().unwrap().status, Skipped);
    assert!(s.launcher.stopped.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 重启恢复时已结束运行里在跑的尝试改为_skipped() {
    let s = 场景::新建("恢复已结束").await;
    let run_id = s.启动().await.unwrap().run.id;
    PipelineRuns::update_status(s.test_db.pool(), run_id, PipelineRunStatus::Cancelled, None)
        .await
        .unwrap();
    assert_eq!(s.service.recover_interrupted().await.unwrap(), 1);
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Cancelled);
    assert_eq!(view.stages[0].status, Skipped);
    assert_eq!(view.stages[0].error, None);
}

#[tokio::test]
async fn 检查脚本失败后下一轮提示词带失败原因() {
    let s = 场景::新建("检查回喂").await;
    s.写仓库模板(
        "version: 1\nstages:\n  - key: develop\n    skill: vk-develop\n    checks: [\"cargo test\"]\n    gate: { auto: checks_passed }\n",
    );
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    s.结束最近一次启动(false).await;
    let prompt = s.launcher.last().prompt;
    assert!(
        prompt.contains("上一次被打回的意见：检查脚本 进程未成功结束（状态：失败，退出码 1）"),
        "{prompt}"
    );
}
