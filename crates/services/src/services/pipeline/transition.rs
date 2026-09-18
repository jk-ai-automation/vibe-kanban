//! 流水线状态转移（纯函数）。读写数据库的部分在 engine.rs。

use db::models::{
    execution_process::{ExecutionProcessRunReason, ExecutionProcessStatus},
    local_project_status::StageType,
    pipeline::{GateDecisionKind, PipelineRunStatus, PipelineStageKey, PipelineStageStatus},
};
use uuid::Uuid;

use super::{
    gates::StageVerdict,
    template::{DEFAULT_MAX_ROUNDS, PipelineTemplate},
};

/// 容器退出钩子发给引擎的消息（任务 17 接线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineExitEvent {
    pub execution_process_id: Uuid,
    /// 这个进程收尾时是否接着启动了同会话的下一个进程（next_action 或排队的续聊）。
    pub chain_continues: bool,
}

/// 引擎判定用的执行进程快照。
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedProcess {
    pub execution_process_id: Uuid,
    pub session_id: Uuid,
    pub run_reason: ExecutionProcessRunReason,
    pub status: ExecutionProcessStatus,
    pub exit_code: Option<i64>,
    /// 这个进程的 action 是否带 next_action。
    pub has_next_action: bool,
    pub chain_continues: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// 阶段等人工确认。
    WaitHuman,
    /// 阶段通过，进入下一阶段。
    Advance { to: PipelineStageKey },
    /// 最后一个阶段通过，运行完成。
    Complete,
    /// 本阶段不通过（或被打回），进入 `stage` 的新一次尝试，`feedback` 带进提示词。
    Rerun {
        stage: PipelineStageKey,
        feedback: String,
    },
    /// 轮次用尽，运行失败转人工。
    FailRun { reason: String },
}

/// 自动判定后的转移。`failed_attempts` = 本阶段累计失败次数（**含本次**）。
pub fn next_transition(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    verdict: &StageVerdict,
    failed_attempts: i64,
) -> Transition {
    match verdict {
        StageVerdict::WaitHuman => Transition::WaitHuman,
        StageVerdict::Pass => advance_or_complete(template, current),
        StageVerdict::Fail { reason } => {
            rerun_or_fail(template, current, current, reason, failed_attempts)
        }
        StageVerdict::FailBackTo { stage, reason } => {
            let target = if template.stage(*stage).is_some() {
                *stage
            } else {
                current
            };
            rerun_or_fail(template, current, target, reason, failed_attempts)
        }
    }
}

/// 人工关卡决策后的转移。打回不计入失败次数，不受 max_rounds 限制。
pub fn gate_decision_transition(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    decision: GateDecisionKind,
    comment: Option<&str>,
) -> Transition {
    match decision {
        GateDecisionKind::Approve => advance_or_complete(template, current),
        GateDecisionKind::Reject => Transition::Rerun {
            stage: current,
            feedback: comment.unwrap_or_default().trim().to_string(),
        },
    }
}

fn advance_or_complete(template: &PipelineTemplate, current: PipelineStageKey) -> Transition {
    match template.next_stage(current) {
        Some(next) => Transition::Advance { to: next.key },
        None => Transition::Complete,
    }
}

fn rerun_or_fail(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    target: PipelineStageKey,
    reason: &str,
    failed_attempts: i64,
) -> Transition {
    let max_rounds = template
        .stage(current)
        .map(|stage| stage.max_rounds)
        .unwrap_or(DEFAULT_MAX_ROUNDS);
    if failed_attempts >= max_rounds {
        Transition::FailRun {
            reason: format!(
                "阶段 {} 已累计失败 {} 次（上限 {}），转人工处理。最后一次原因：{}",
                current.as_str(),
                failed_attempts,
                max_rounds,
                reason
            ),
        }
    } else {
        Transition::Rerun {
            stage: target,
            feedback: reason.to_string(),
        }
    }
}

/// 这个进程结束是否意味着「阶段的执行链走完了」。
///
/// - 进程收尾时接着起了下一个进程（`chain_continues`）→ 不是终点。
/// - setup 脚本：带 next_action（顺序模式）却没接上 → 链断了，是终点；
///   不带 next_action（并行模式）→ 智能体另行启动，与阶段结果无关。
/// - 开发服务器、归档脚本与阶段无关。
/// - 编码智能体、cleanup 脚本、检查脚本 → 是终点。编码智能体成功但没提交时
///   容器不跑 cleanup（`local-deployment/src/container.rs:590-621`），所以这里不能等 cleanup。
pub fn is_stage_terminal(process: &FinishedProcess) -> bool {
    if process.chain_continues {
        return false;
    }
    match process.run_reason {
        ExecutionProcessRunReason::SetupScript => process.has_next_action,
        ExecutionProcessRunReason::DevServer | ExecutionProcessRunReason::ArchiveScript => false,
        ExecutionProcessRunReason::CodingAgent
        | ExecutionProcessRunReason::CleanupScript
        | ExecutionProcessRunReason::PipelineStep => true,
    }
}

pub fn process_succeeded(process: &FinishedProcess) -> bool {
    process.status == ExecutionProcessStatus::Completed && process.exit_code == Some(0)
}

/// 阶段 → 看板列（设计 §6.4）。
pub fn stage_column(key: PipelineStageKey) -> StageType {
    match key {
        PipelineStageKey::Requirement => StageType::Backlog,
        PipelineStageKey::Spec | PipelineStageKey::TestDesign => StageType::Todo,
        PipelineStageKey::Develop => StageType::Dev,
        PipelineStageKey::Review => StageType::Review,
        PipelineStageKey::Test => StageType::Test,
        PipelineStageKey::Deliver => StageType::Done,
    }
}

/// 进入阶段时新尝试与运行的状态（行为总表「进入阶段 | 运行 paused」、契约修订 C12）：
/// 运行暂停中 → 新尝试建成 pending、运行保持 paused、不启动；否则新尝试 running、运行 running。
pub fn entered_stage_statuses(
    run_status: PipelineRunStatus,
) -> (PipelineStageStatus, PipelineRunStatus) {
    if run_status == PipelineRunStatus::Paused {
        (PipelineStageStatus::Pending, PipelineRunStatus::Paused)
    } else {
        (PipelineStageStatus::Running, PipelineRunStatus::Running)
    }
}

/// 判定 WaitHuman 时运行的新状态：运行已暂停则保持 paused，否则 waiting_gate。
pub fn waiting_gate_run_status(run_status: PipelineRunStatus) -> PipelineRunStatus {
    if run_status == PipelineRunStatus::Paused {
        PipelineRunStatus::Paused
    } else {
        PipelineRunStatus::WaitingGate
    }
}

#[cfg(test)]
mod tests {
    use db::models::{
        execution_process::{ExecutionProcessRunReason, ExecutionProcessStatus},
        local_project_status::StageType,
        pipeline::{GateDecisionKind, PipelineRunStatus, PipelineStageKey, PipelineStageStatus},
    };
    use uuid::Uuid;

    use super::*;
    use crate::services::pipeline::{gates::StageVerdict, template::builtin_template};

    fn 进程(
        run_reason: ExecutionProcessRunReason,
        has_next_action: bool,
        chain_continues: bool,
    ) -> FinishedProcess {
        FinishedProcess {
            execution_process_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_reason,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action,
            chain_continues,
        }
    }

    #[test]
    fn 通过后进入下一阶段_最后一阶段则完成() {
        let t = builtin_template();
        assert_eq!(
            next_transition(&t, PipelineStageKey::Develop, &StageVerdict::Pass, 0),
            Transition::Advance {
                to: PipelineStageKey::Review
            }
        );
        assert_eq!(
            next_transition(&t, PipelineStageKey::Deliver, &StageVerdict::Pass, 0),
            Transition::Complete
        );
    }

    #[test]
    fn 等人工() {
        let t = builtin_template();
        assert_eq!(
            next_transition(&t, PipelineStageKey::Spec, &StageVerdict::WaitHuman, 0),
            Transition::WaitHuman
        );
    }

    #[test]
    fn 失败未到上限时同阶段重跑并回喂原因() {
        let t = builtin_template();
        let verdict = StageVerdict::Fail {
            reason: "评审发现 1 个阻断项".to_string(),
        };
        assert_eq!(
            next_transition(&t, PipelineStageKey::Review, &verdict, 2),
            Transition::Rerun {
                stage: PipelineStageKey::Review,
                feedback: "评审发现 1 个阻断项".to_string()
            }
        );
    }

    #[test]
    fn 失败达到上限时运行失败() {
        let t = builtin_template();
        let verdict = StageVerdict::Fail {
            reason: "评审发现 1 个阻断项".to_string(),
        };
        let Transition::FailRun { reason } =
            next_transition(&t, PipelineStageKey::Review, &verdict, 3)
        else {
            panic!("第 3 次失败应转人工");
        };
        assert!(
            reason.contains("review") && reason.contains("3"),
            "{reason}"
        );
    }

    #[test]
    fn 测试失败按归因回退() {
        let t = builtin_template();
        for target in [PipelineStageKey::Develop, PipelineStageKey::TestDesign] {
            let verdict = StageVerdict::FailBackTo {
                stage: target,
                reason: "测试失败".to_string(),
            };
            assert_eq!(
                next_transition(&t, PipelineStageKey::Test, &verdict, 1),
                Transition::Rerun {
                    stage: target,
                    feedback: "测试失败".to_string()
                }
            );
        }
    }

    #[test]
    fn 回退目标不在模板里时重跑本阶段() {
        let mut t = builtin_template();
        t.stages.retain(|s| s.key != PipelineStageKey::TestDesign);
        let verdict = StageVerdict::FailBackTo {
            stage: PipelineStageKey::TestDesign,
            reason: "用例问题".to_string(),
        };
        assert_eq!(
            next_transition(&t, PipelineStageKey::Test, &verdict, 1),
            Transition::Rerun {
                stage: PipelineStageKey::Test,
                feedback: "用例问题".to_string()
            }
        );
    }

    #[test]
    fn 人工通过与打回() {
        let t = builtin_template();
        assert_eq!(
            gate_decision_transition(
                &t,
                PipelineStageKey::TestDesign,
                GateDecisionKind::Approve,
                None
            ),
            Transition::Advance {
                to: PipelineStageKey::Develop
            }
        );
        assert_eq!(
            gate_decision_transition(
                &t,
                PipelineStageKey::Requirement,
                GateDecisionKind::Reject,
                Some(" 补充验收标准 ")
            ),
            Transition::Rerun {
                stage: PipelineStageKey::Requirement,
                feedback: "补充验收标准".to_string()
            }
        );
    }

    #[test]
    fn 阶段链终点判定() {
        use ExecutionProcessRunReason::*;
        // 智能体没提交、cleanup 没跑：退出钩子报 chain_continues = false → 是终点（§2.3 的陷阱）。
        assert!(is_stage_terminal(&进程(CodingAgent, true, false)));
        // 智能体有提交、cleanup 已启动 → 不是终点，等 cleanup。
        assert!(!is_stage_terminal(&进程(CodingAgent, true, true)));
        assert!(is_stage_terminal(&进程(CleanupScript, false, false)));
        assert!(is_stage_terminal(&进程(PipelineStep, false, false)));
        // 顺序 setup 成功后接着起智能体 → 不是终点；失败没接上 → 是终点。
        assert!(!is_stage_terminal(&进程(SetupScript, true, true)));
        assert!(is_stage_terminal(&进程(SetupScript, true, false)));
        // 并行 setup（没有 next_action）与阶段结果无关。
        assert!(!is_stage_terminal(&进程(SetupScript, false, false)));
        assert!(!is_stage_terminal(&进程(DevServer, false, false)));
        assert!(!is_stage_terminal(&进程(ArchiveScript, false, false)));
    }

    #[test]
    fn 进程成功要求完成且退出码为零() {
        let mut p = 进程(ExecutionProcessRunReason::CodingAgent, false, false);
        assert!(process_succeeded(&p));
        p.exit_code = Some(1);
        assert!(!process_succeeded(&p));
        p.exit_code = Some(0);
        p.status = ExecutionProcessStatus::Killed;
        assert!(!process_succeeded(&p));
    }

    #[test]
    fn 阶段到看板列的映射() {
        assert_eq!(
            stage_column(PipelineStageKey::Requirement),
            StageType::Backlog
        );
        assert_eq!(stage_column(PipelineStageKey::Spec), StageType::Todo);
        assert_eq!(stage_column(PipelineStageKey::TestDesign), StageType::Todo);
        assert_eq!(stage_column(PipelineStageKey::Develop), StageType::Dev);
        assert_eq!(stage_column(PipelineStageKey::Review), StageType::Review);
        assert_eq!(stage_column(PipelineStageKey::Test), StageType::Test);
        assert_eq!(stage_column(PipelineStageKey::Deliver), StageType::Done);
    }

    #[test]
    fn 回退也按本阶段轮次计数_达到上限时运行失败() {
        let t = builtin_template();
        let verdict = StageVerdict::FailBackTo {
            stage: PipelineStageKey::Develop,
            reason: "测试失败".to_string(),
        };
        assert!(matches!(
            next_transition(&t, PipelineStageKey::Test, &verdict, 3),
            Transition::FailRun { reason } if reason.contains("test")
        ));
    }

    #[test]
    fn 人工关卡阶段也受轮次限制() {
        let t = builtin_template();
        let verdict = StageVerdict::Fail {
            reason: "技能没有产出 requirement.md".to_string(),
        };
        assert!(matches!(
            next_transition(&t, PipelineStageKey::Requirement, &verdict, 2),
            Transition::Rerun {
                stage: PipelineStageKey::Requirement,
                ..
            }
        ));
        assert!(matches!(
            next_transition(&t, PipelineStageKey::Requirement, &verdict, 3),
            Transition::FailRun { .. }
        ));
    }

    #[test]
    fn 轮次上限取模板里本阶段的配置() {
        let mut t = builtin_template();
        t.stages
            .iter_mut()
            .find(|s| s.key == PipelineStageKey::Review)
            .unwrap()
            .max_rounds = 1;
        let verdict = StageVerdict::Fail {
            reason: "阻断".to_string(),
        };
        assert!(matches!(
            next_transition(&t, PipelineStageKey::Review, &verdict, 1),
            Transition::FailRun { .. }
        ));
    }

    #[test]
    fn 人工打回不计入失败次数_不受轮次上限限制() {
        let mut t = builtin_template();
        for stage in &mut t.stages {
            stage.max_rounds = 1;
        }
        for _ in 0..5 {
            assert_eq!(
                gate_decision_transition(
                    &t,
                    PipelineStageKey::Spec,
                    GateDecisionKind::Reject,
                    Some("再改改")
                ),
                Transition::Rerun {
                    stage: PipelineStageKey::Spec,
                    feedback: "再改改".to_string()
                }
            );
        }
    }

    #[test]
    fn 最后阶段人工通过即完成() {
        let mut t = builtin_template();
        t.stages.last_mut().unwrap().gate = crate::services::pipeline::template::StageGate::Human {
            label: "交付确认".to_string(),
        };
        assert_eq!(
            gate_decision_transition(
                &t,
                PipelineStageKey::Deliver,
                GateDecisionKind::Approve,
                None
            ),
            Transition::Complete
        );
    }

    #[test]
    fn 进入阶段时运行暂停则新尝试留_pending() {
        assert_eq!(
            entered_stage_statuses(PipelineRunStatus::Paused),
            (PipelineStageStatus::Pending, PipelineRunStatus::Paused)
        );
        for run_status in [
            PipelineRunStatus::Running,
            PipelineRunStatus::WaitingGate,
            PipelineRunStatus::Failed,
        ] {
            assert_eq!(
                entered_stage_statuses(run_status),
                (PipelineStageStatus::Running, PipelineRunStatus::Running),
                "{run_status:?}"
            );
        }
    }

    #[test]
    fn 等人工时运行已暂停则保持暂停() {
        assert_eq!(
            waiting_gate_run_status(PipelineRunStatus::Paused),
            PipelineRunStatus::Paused
        );
        assert_eq!(
            waiting_gate_run_status(PipelineRunStatus::Running),
            PipelineRunStatus::WaitingGate
        );
    }

    #[test]
    fn 暂停中的关卡决策_通过建下一阶段_pending_打回建同阶段_pending() {
        // C12：暂停中照常受理决策；转移本身与运行状态无关，新尝试的状态由运行是否暂停决定。
        let t = builtin_template();
        let approve =
            gate_decision_transition(&t, PipelineStageKey::Spec, GateDecisionKind::Approve, None);
        assert_eq!(
            approve,
            Transition::Advance {
                to: PipelineStageKey::TestDesign
            }
        );
        let reject = gate_decision_transition(
            &t,
            PipelineStageKey::Spec,
            GateDecisionKind::Reject,
            Some("缺接口说明"),
        );
        assert_eq!(
            reject,
            Transition::Rerun {
                stage: PipelineStageKey::Spec,
                feedback: "缺接口说明".to_string()
            }
        );
        assert_eq!(
            entered_stage_statuses(PipelineRunStatus::Paused).0,
            PipelineStageStatus::Pending
        );
    }
}
