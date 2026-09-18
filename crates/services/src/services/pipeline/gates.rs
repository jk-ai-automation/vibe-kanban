//! 关卡判定（设计 §6.1、§6.3；契约 §4）。全部是纯函数。

use std::collections::BTreeMap;

use db::models::pipeline::PipelineStageKey;
use serde::Deserialize;

use super::template::{AutoGate, StageGate, StageTemplate};

pub const REVIEW_FILE: &str = "review.json";
pub const TEST_REPORT_FILE: &str = "test-report.json";

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewReport {
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFinding {
    pub severity: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestReport {
    pub total: i64,
    pub passed: i64,
    pub failed: i64,
    #[serde(default)]
    pub cases: Vec<TestCaseResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestCaseResult {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub attribution: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageVerdict {
    /// 产出齐全，等人工确认。
    WaitHuman,
    /// 自动通过。
    Pass,
    /// 本阶段不通过：按轮次重跑本阶段。
    Fail { reason: String },
    /// 测试不通过且归因明确：回到指定阶段。
    FailBackTo {
        stage: PipelineStageKey,
        reason: String,
    },
}

impl StageVerdict {
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            StageVerdict::Fail { .. } | StageVerdict::FailBackTo { .. }
        )
    }
}

/// 声明了但没读到（不存在或内容为空白）的产出物。
pub fn missing_artifacts(required: &[String], present: &BTreeMap<String, String>) -> Vec<String> {
    required
        .iter()
        .filter(|name| {
            present
                .get(name.as_str())
                .is_none_or(|content| content.trim().is_empty())
        })
        .cloned()
        .collect()
}

/// 阶段判定：进程失败 → 缺产出物 → 按关卡类型。
///
/// `process_ok` 已经包含智能体与检查脚本（若有）的退出码，所以 `checks_passed`
/// 走到这里就是通过。
pub fn evaluate_stage(
    stage: &StageTemplate,
    process_ok: bool,
    process_error: Option<&str>,
    artifacts: &BTreeMap<String, String>,
) -> StageVerdict {
    if !process_ok {
        return StageVerdict::Fail {
            reason: process_error.unwrap_or("执行进程失败").to_string(),
        };
    }
    let missing = missing_artifacts(&stage.artifacts, artifacts);
    if !missing.is_empty() {
        return StageVerdict::Fail {
            reason: format!("技能没有产出 {}", missing.join("、")),
        };
    }
    match &stage.gate {
        StageGate::Human { .. } => StageVerdict::WaitHuman,
        StageGate::None => StageVerdict::Pass,
        StageGate::Auto { rule } => evaluate_auto(*rule, artifacts),
    }
}

pub fn evaluate_auto(rule: AutoGate, artifacts: &BTreeMap<String, String>) -> StageVerdict {
    match rule {
        AutoGate::ChecksPassed | AutoGate::ArtifactsPresent => StageVerdict::Pass,
        AutoGate::NoBlockingFindings => match artifacts.get(REVIEW_FILE) {
            Some(text) => check_no_blocking_findings(text),
            None => StageVerdict::Fail {
                reason: format!("缺少 {REVIEW_FILE}，无法判定评审结果"),
            },
        },
        AutoGate::AllCasesPassed => match artifacts.get(TEST_REPORT_FILE) {
            Some(text) => check_all_cases_passed(text),
            None => StageVerdict::Fail {
                reason: format!("缺少 {TEST_REPORT_FILE}，无法判定测试结果"),
            },
        },
    }
}

/// `no_blocking_findings` = 没有 `severity == "blocker"`（契约 §4）。
pub fn check_no_blocking_findings(text: &str) -> StageVerdict {
    let report: ReviewReport = match serde_json::from_str(text) {
        Ok(report) => report,
        Err(e) => {
            return StageVerdict::Fail {
                reason: format!("{REVIEW_FILE} 不是合法的评审结果：{e}"),
            };
        }
    };
    let blockers: Vec<&ReviewFinding> = report
        .findings
        .iter()
        .filter(|finding| finding.severity == "blocker")
        .collect();
    if blockers.is_empty() {
        return StageVerdict::Pass;
    }
    let details = blockers
        .iter()
        .map(|finding| describe_finding(finding))
        .collect::<Vec<_>>()
        .join("；");
    StageVerdict::Fail {
        reason: format!("评审发现 {} 个阻断项：{details}", blockers.len()),
    }
}

fn describe_finding(finding: &ReviewFinding) -> String {
    let location = match (&finding.file, finding.line) {
        (Some(file), Some(line)) => format!("{file}:{line} "),
        (Some(file), None) => format!("{file} "),
        _ => String::new(),
    };
    format!(
        "{location}{}",
        finding.message.as_deref().unwrap_or("（无说明）")
    )
}

/// `all_cases_passed` = `failed == 0 && total > 0`（契约 §4）。失败时按归因分派回流目标。
pub fn check_all_cases_passed(text: &str) -> StageVerdict {
    let report: TestReport = match serde_json::from_str(text) {
        Ok(report) => report,
        Err(e) => {
            return StageVerdict::Fail {
                reason: format!("{TEST_REPORT_FILE} 不是合法的测试报告：{e}"),
            };
        }
    };
    if report.total <= 0 {
        return StageVerdict::Fail {
            reason: "测试报告里没有任何用例（total = 0）".to_string(),
        };
    }
    if report.failed == 0 {
        return StageVerdict::Pass;
    }
    let details = report
        .cases
        .iter()
        .filter(|case| case.status != "passed")
        .map(|case| format!("{}：{}", case.id, case.message.as_deref().unwrap_or("失败")))
        .collect::<Vec<_>>()
        .join("；");
    StageVerdict::FailBackTo {
        stage: dispatch_test_failure(&report),
        reason: format!(
            "测试失败 {} 条（共 {} 条）：{details}",
            report.failed, report.total
        ),
    }
}

/// 测试归因分派（设计 §6.3、契约 §4）：任一 `attribution == "case"` → 回到用例设计
/// （重新走人工关卡）；否则 → 回到开发。
pub fn dispatch_test_failure(report: &TestReport) -> PipelineStageKey {
    let any_case = report
        .cases
        .iter()
        .any(|case| case.attribution.as_deref() == Some("case"));
    if any_case {
        PipelineStageKey::TestDesign
    } else {
        PipelineStageKey::Develop
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use db::models::pipeline::PipelineStageKey;

    use super::*;
    use crate::services::pipeline::template::{PipelineTemplate, builtin_template};

    fn 模板() -> PipelineTemplate {
        builtin_template()
    }

    fn 产出(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn 进程失败直接不通过且带原因() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Requirement).unwrap();
        let verdict = evaluate_stage(stage, false, Some("编码智能体 进程未成功结束"), &产出(&[]));
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "编码智能体 进程未成功结束".to_string()
            }
        );
    }

    #[test]
    fn 缺产出物按失败处理并列出缺哪些() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Spec).unwrap();
        let verdict = evaluate_stage(stage, true, None, &产出(&[("spec.md", "# 规格")]));
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "技能没有产出 plan.md".to_string()
            }
        );
        let blank = evaluate_stage(
            stage,
            true,
            None,
            &产出(&[("spec.md", "# 规格"), ("plan.md", "  \n")]),
        );
        assert!(
            matches!(blank, StageVerdict::Fail { .. }),
            "空白文件等于没产出"
        );
    }

    #[test]
    fn 人工关卡产出齐全后等人确认() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Requirement).unwrap();
        assert_eq!(
            evaluate_stage(stage, true, None, &产出(&[("requirement.md", "# 需求")])),
            StageVerdict::WaitHuman
        );
    }

    #[test]
    fn 开发阶段进程成功即通过() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Develop).unwrap();
        assert_eq!(
            evaluate_stage(stage, true, None, &产出(&[])),
            StageVerdict::Pass
        );
    }

    #[test]
    fn 交付阶段产出齐全即通过() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Deliver).unwrap();
        assert_eq!(
            evaluate_stage(
                stage,
                true,
                None,
                &产出(&[("delivery-report.md", "# 交付")])
            ),
            StageVerdict::Pass
        );
    }

    #[test]
    fn 评审没有阻断项即通过() {
        assert_eq!(
            check_no_blocking_findings(
                r#"{"findings":[{"severity":"major","file":"a.rs","line":3,"message":"命名"}]}"#
            ),
            StageVerdict::Pass
        );
        assert_eq!(
            check_no_blocking_findings(r#"{"findings":[]}"#),
            StageVerdict::Pass
        );
    }

    #[test]
    fn 评审有阻断项不通过并列出位置() {
        let verdict = check_no_blocking_findings(
            r#"{"findings":[{"severity":"blocker","file":"src/lib.rs","line":7,"message":"空指针"},{"severity":"minor","message":"格式"}]}"#,
        );
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "评审发现 1 个阻断项：src/lib.rs:7 空指针".to_string()
            }
        );
    }

    #[test]
    fn 评审结果不是合法_json_时不通过() {
        assert!(matches!(
            check_no_blocking_findings("not json"),
            StageVerdict::Fail { reason } if reason.contains("review.json")
        ));
    }

    #[test]
    fn 测试全部通过且至少一条才算通过() {
        assert_eq!(
            check_all_cases_passed(r#"{"total":3,"passed":3,"failed":0,"cases":[]}"#),
            StageVerdict::Pass
        );
        assert!(matches!(
            check_all_cases_passed(r#"{"total":0,"passed":0,"failed":0,"cases":[]}"#),
            StageVerdict::Fail { reason } if reason.contains("total = 0")
        ));
    }

    #[test]
    fn 测试失败归因代码缺陷回到开发() {
        let verdict = check_all_cases_passed(
            r#"{"total":3,"passed":2,"failed":1,"cases":[{"id":"TC-003","status":"failed","attribution":"code","message":"金额算错"}]}"#,
        );
        assert_eq!(
            verdict,
            StageVerdict::FailBackTo {
                stage: PipelineStageKey::Develop,
                reason: "测试失败 1 条（共 3 条）：TC-003：金额算错".to_string()
            }
        );
    }

    #[test]
    fn 测试失败任一归因用例问题回到用例设计() {
        let verdict = check_all_cases_passed(
            r#"{"total":3,"passed":1,"failed":2,"cases":[{"id":"TC-1","status":"failed","attribution":"code"},{"id":"TC-2","status":"failed","attribution":"case"}]}"#,
        );
        assert!(matches!(
            verdict,
            StageVerdict::FailBackTo {
                stage: PipelineStageKey::TestDesign,
                ..
            }
        ));
    }

    #[test]
    fn 测试失败没有归因时默认回到开发() {
        let verdict = check_all_cases_passed(
            r#"{"total":1,"passed":0,"failed":1,"cases":[{"id":"TC-1","status":"failed","attribution":null}]}"#,
        );
        assert!(matches!(
            verdict,
            StageVerdict::FailBackTo {
                stage: PipelineStageKey::Develop,
                ..
            }
        ));
    }

    #[test]
    fn 评审阶段走到阻断判定() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Review).unwrap();
        let verdict = evaluate_stage(
            stage,
            true,
            None,
            &产出(&[(
                "review.json",
                r#"{"findings":[{"severity":"blocker","message":"x"}]}"#,
            )]),
        );
        assert!(verdict.is_failure());
    }

    #[test]
    fn 用例问题回流的目标阶段走人工关卡() {
        let t = 模板();
        let verdict = evaluate_stage(
            t.stage(PipelineStageKey::Test).unwrap(),
            true,
            None,
            &产出(&[(
                "test-report.json",
                r#"{"total":2,"passed":1,"failed":1,"cases":[{"id":"TC-1","status":"failed","attribution":"case","message":"预期写错"}]}"#,
            )]),
        );
        let StageVerdict::FailBackTo { stage, reason } = verdict else {
            panic!("用例归因应回流：{verdict:?}");
        };
        assert_eq!(stage, PipelineStageKey::TestDesign);
        assert!(reason.contains("TC-1：预期写错"), "{reason}");
        assert!(
            t.stage(stage).unwrap().gate.human_label().is_some(),
            "回到用例设计后要重新过人工关卡"
        );
    }

    #[test]
    fn 检查脚本失败时开发阶段不通过() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Develop).unwrap();
        assert_eq!(
            evaluate_stage(stage, false, Some("检查命令 cargo test 失败"), &产出(&[])),
            StageVerdict::Fail {
                reason: "检查命令 cargo test 失败".to_string()
            }
        );
        assert_eq!(
            evaluate_stage(stage, false, None, &产出(&[])),
            StageVerdict::Fail {
                reason: "执行进程失败".to_string()
            }
        );
    }

    #[test]
    fn 测试阶段缺报告或报告非法时不通过且不回流() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Test).unwrap();
        assert!(matches!(
            evaluate_stage(stage, true, None, &产出(&[])),
            StageVerdict::Fail { reason } if reason.contains("test-report.json")
        ));
        assert!(matches!(
            check_all_cases_passed("{"),
            StageVerdict::Fail { reason } if reason.contains("test-report.json")
        ));
    }

    #[test]
    fn 无关卡阶段产出齐全即通过() {
        let mut stage = 模板().stage(PipelineStageKey::Spec).unwrap().clone();
        stage.gate = crate::services::pipeline::template::StageGate::None;
        assert_eq!(
            evaluate_stage(
                &stage,
                true,
                None,
                &产出(&[("spec.md", "# 规格"), ("plan.md", "# 计划")])
            ),
            StageVerdict::Pass
        );
    }
}
