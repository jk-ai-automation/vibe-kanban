//! 流水线提示词的固定格式（契约 §5）与 qa-mode 模拟产出物。
//!
//! 引擎（`services::services::pipeline::prompt`）按这里的常量拼提示词；qa-mode 模拟执行器
//! （`executors::executors::qa_mock`）与引擎集成测试按这里的函数解析提示词、写占位产出物。
//! 改格式只改这一处。

use std::path::{Path, PathBuf};

pub const SKILL_PREFIX: &str = "使用技能 ";
pub const REQUIREMENT_LABEL: &str = "需求：";
pub const ARTIFACTS_DIR_LABEL: &str = "产出物目录（绝对路径）：";
pub const REQUIRED_ARTIFACTS_LABEL: &str = "必须产出：";
pub const PREVIOUS_ARTIFACTS_LABEL: &str = "上一阶段产出：";
pub const FEEDBACK_LABEL: &str = "上一次被打回的意见：";
pub const NONE_VALUE: &str = "无";
pub const ARTIFACT_SEPARATOR: &str = ", ";

/// atp 旧 CSV 表头，原样取自 atp 仓库 `legacy/test_cases/api_custom/api_custom.csv` 第一行
/// （去掉 BOM，46 列）。列定义保持旧版不变；模拟器只用它写占位用例。
pub const MOCK_TEST_CASES_HEADER: &str = "CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinePrompt {
    /// 「需求：」行冒号后的内容：`{simple_id} {title}`。
    pub requirement: String,
    pub artifacts_dir: PathBuf,
    pub required: Vec<String>,
}

/// 解析流水线提示词。缺「产出物目录」或「必须产出」行、或目录不是绝对路径时返回 None
/// （即不是流水线提示词）。每种固定行只认第一次出现，防止打回意见里的同名行干扰。
pub fn parse_pipeline_prompt(prompt: &str) -> Option<PipelinePrompt> {
    let mut requirement: Option<String> = None;
    let mut artifacts_dir: Option<PathBuf> = None;
    let mut required: Option<Vec<String>> = None;
    for line in prompt.lines() {
        let line = line.trim();
        if requirement.is_none()
            && let Some(rest) = line.strip_prefix(REQUIREMENT_LABEL)
        {
            requirement = Some(rest.trim().to_string());
        } else if artifacts_dir.is_none()
            && let Some(rest) = line.strip_prefix(ARTIFACTS_DIR_LABEL)
        {
            artifacts_dir = Some(PathBuf::from(rest.trim()));
        } else if required.is_none()
            && let Some(rest) = line.strip_prefix(REQUIRED_ARTIFACTS_LABEL)
        {
            required = Some(parse_artifact_list(rest));
        }
    }
    let artifacts_dir = artifacts_dir?;
    if !artifacts_dir.is_absolute() {
        return None;
    }
    Some(PipelinePrompt {
        requirement: requirement.unwrap_or_default(),
        artifacts_dir,
        required: required?,
    })
}

pub fn parse_artifact_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() || value == NONE_VALUE {
        return Vec::new();
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn format_artifact_list(names: &[String]) -> String {
    if names.is_empty() {
        NONE_VALUE.to_string()
    } else {
        names.join(ARTIFACT_SEPARATOR)
    }
}

/// 端到端测试用的需求标题标记（契约 §5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaMarker {
    ReviewBlockerOnce,
    TestFailCodeOnce,
    TestFailCaseOnce,
    AlwaysFailReview,
}

impl QaMarker {
    pub const ALL: [QaMarker; 4] = [
        QaMarker::ReviewBlockerOnce,
        QaMarker::TestFailCodeOnce,
        QaMarker::TestFailCaseOnce,
        QaMarker::AlwaysFailReview,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            QaMarker::ReviewBlockerOnce => "[qa:review-blocker-once]",
            QaMarker::TestFailCodeOnce => "[qa:test-fail-code-once]",
            QaMarker::TestFailCaseOnce => "[qa:test-fail-case-once]",
            QaMarker::AlwaysFailReview => "[qa:always-fail-review]",
        }
    }
}

/// 从「需求：」行内容里找标记，按 [`QaMarker::ALL`] 的顺序返回。
pub fn qa_markers(requirement: &str) -> Vec<QaMarker> {
    QaMarker::ALL
        .into_iter()
        .filter(|marker| requirement.contains(marker.tag()))
        .collect()
}

/// 在产出物目录里写出「必须产出」的每个文件。已存在的旧文件先改名为 `*.prev`
/// ——模拟器靠「有没有旧文件」判断是不是第一次（契约 §5）。
/// 只接受纯文件名，带路径分隔符或 `..` 的跳过。
pub fn write_mock_artifacts(prompt: &PipelinePrompt) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(&prompt.artifacts_dir)?;
    let markers = qa_markers(&prompt.requirement);
    let mut written = Vec::new();
    for name in &prompt.required {
        if !is_plain_file_name(name) {
            continue;
        }
        let path = prompt.artifacts_dir.join(name);
        let first = !path.exists();
        if !first {
            std::fs::rename(&path, prompt.artifacts_dir.join(format!("{name}.prev")))?;
        }
        std::fs::write(
            &path,
            mock_artifact_content(name, first, &markers, &prompt.requirement),
        )?;
        written.push(path);
    }
    Ok(written)
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).file_name().is_some()
}

/// 单个占位产出物的内容。`first` = 目录里原本没有这个文件。
pub fn mock_artifact_content(
    name: &str,
    first: bool,
    markers: &[QaMarker],
    requirement: &str,
) -> String {
    match name {
        "review.json" => {
            let blocker = markers.contains(&QaMarker::AlwaysFailReview)
                || (first && markers.contains(&QaMarker::ReviewBlockerOnce));
            if blocker {
                serde_json::json!({
                    "findings": [{
                        "severity": "blocker",
                        "file": "src/lib.rs",
                        "line": 1,
                        "message": "模拟评审阻断项"
                    }]
                })
                .to_string()
            } else {
                r#"{"findings":[]}"#.to_string()
            }
        }
        "test-report.json" => {
            let attribution = if first && markers.contains(&QaMarker::TestFailCodeOnce) {
                Some("code")
            } else if first && markers.contains(&QaMarker::TestFailCaseOnce) {
                Some("case")
            } else {
                None
            };
            match attribution {
                Some(attribution) => serde_json::json!({
                    "total": 3,
                    "passed": 2,
                    "failed": 1,
                    "cases": [
                        passed_case("TC-001"),
                        passed_case("TC-002"),
                        {"id": "TC-003", "status": "failed", "attribution": attribution, "message": "模拟失败"}
                    ]
                })
                .to_string(),
                None => serde_json::json!({
                    "total": 3,
                    "passed": 3,
                    "failed": 0,
                    "cases": [passed_case("TC-001"), passed_case("TC-002"), passed_case("TC-003")]
                })
                .to_string(),
            }
        }
        name if name.ends_with(".csv") => mock_test_cases_csv(),
        name if name.ends_with(".json") => "{}".to_string(),
        name => format!(
            "# {}\n\n由 qa-mode 模拟执行器生成的占位内容。\n\n{REQUIREMENT_LABEL}{requirement}\n",
            markdown_title(name)
        ),
    }
}

fn passed_case(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "status": "passed",
        "attribution": null,
        "message": ""
    })
}

/// 表头 + 一行占位用例，字段数与表头一致（占位值不含逗号与引号）。
fn mock_test_cases_csv() -> String {
    let row: Vec<&str> = MOCK_TEST_CASES_HEADER
        .split(',')
        .map(|column| match column {
            "CaseID" => "TC-001",
            "ScenarioName" => "占位场景",
            "Priority" => "P1",
            "stepkw" => "GET",
            "StepSummary" => "由 qa-mode 模拟执行器生成",
            "ExpectedResult" => "返回成功",
            _ => "",
        })
        .collect();
    format!("{MOCK_TEST_CASES_HEADER}\n{}\n", row.join(","))
}

fn markdown_title(name: &str) -> &str {
    match name {
        "requirement.md" => "需求说明",
        "spec.md" => "设计规格",
        "plan.md" => "实施计划",
        "trace-matrix.md" => "追踪矩阵",
        "delivery-report.md" => "交付报告",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn 提示词(dir: &std::path::Path, required: &str, title: &str) -> String {
        format!(
            "使用技能 vk-review。\n需求：VK-7 {title}\n产出物目录（绝对路径）：{}\n必须产出：{required}\n上一阶段产出：spec.md\n这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。",
            dir.display()
        )
    }

    #[test]
    fn 解析两行固定格式() {
        let parsed = parse_pipeline_prompt(&提示词(
            std::path::Path::new("/abs/ws/.vk/runs/VK-7"),
            "requirement.md, spec.md",
            "导出报表",
        ))
        .unwrap();
        assert_eq!(parsed.artifacts_dir, PathBuf::from("/abs/ws/.vk/runs/VK-7"));
        assert_eq!(parsed.required, vec!["requirement.md", "spec.md"]);
        assert_eq!(parsed.requirement, "VK-7 导出报表");
    }

    #[test]
    fn 必须产出为无时是空清单() {
        let parsed = parse_pipeline_prompt(&提示词(
            std::path::Path::new("/abs/x"),
            NONE_VALUE,
            "开发",
        ))
        .unwrap();
        assert!(parsed.required.is_empty());
    }

    #[test]
    fn 普通提示词不是流水线提示词() {
        assert!(parse_pipeline_prompt("帮我修个 bug").is_none());
        assert!(
            parse_pipeline_prompt("产出物目录（绝对路径）：relative/dir\n必须产出：a.md").is_none(),
            "相对路径不接受"
        );
        assert!(
            parse_pipeline_prompt("产出物目录（绝对路径）：/abs\n").is_none(),
            "缺「必须产出」行不接受"
        );
    }

    #[test]
    fn 只认第一次出现的固定行() {
        let prompt = format!(
            "{}\n上一次被打回的意见：\n必须产出：evil.md\n",
            提示词(std::path::Path::new("/abs/x"), "review.json", "t")
        );
        assert_eq!(
            parse_pipeline_prompt(&prompt).unwrap().required,
            vec!["review.json"]
        );
    }

    #[test]
    fn 标记从需求行解析() {
        assert_eq!(
            qa_markers("VK-1 评审 [qa:review-blocker-once] 与 [qa:test-fail-case-once]"),
            vec![QaMarker::ReviewBlockerOnce, QaMarker::TestFailCaseOnce]
        );
        assert!(qa_markers("VK-1 普通需求").is_empty());
    }

    #[test]
    fn 用例表头是_atp_旧版_46_列() {
        let columns: Vec<&str> = MOCK_TEST_CASES_HEADER.split(',').collect();
        assert_eq!(columns.len(), 46);
        assert_eq!(columns.first(), Some(&"CaseID"));
        assert_eq!(columns.last(), Some(&"ActualOutput4LoopScenario"));
    }

    #[test]
    fn 写出占位产出物且第二次把旧文件改名为_prev() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts_dir = dir.path().join(".vk/runs/VK-7");
        let parsed = PipelinePrompt {
            requirement: "VK-7 导出 [qa:review-blocker-once]".to_string(),
            artifacts_dir: artifacts_dir.clone(),
            required: vec![
                "requirement.md".to_string(),
                "test-cases.csv".to_string(),
                "review.json".to_string(),
            ],
        };

        let written = write_mock_artifacts(&parsed).unwrap();
        assert_eq!(written.len(), 3);
        let md = std::fs::read_to_string(artifacts_dir.join("requirement.md")).unwrap();
        assert!(md.starts_with("# 需求说明"), "{md}");
        let csv = std::fs::read_to_string(artifacts_dir.join("test-cases.csv")).unwrap();
        assert!(csv.starts_with(MOCK_TEST_CASES_HEADER));
        assert_eq!(csv.lines().count(), 2, "表头加一行");
        let header_fields = csv.lines().next().unwrap().split(',').count();
        let row_fields = csv.lines().nth(1).unwrap().split(',').count();
        assert_eq!(header_fields, row_fields, "占位行字段数与表头一致");
        let first_review = std::fs::read_to_string(artifacts_dir.join("review.json")).unwrap();
        assert!(first_review.contains("\"blocker\""), "第一次评审带阻断项");

        write_mock_artifacts(&parsed).unwrap();
        assert!(artifacts_dir.join("review.json.prev").exists());
        assert_eq!(
            std::fs::read_to_string(artifacts_dir.join("review.json.prev")).unwrap(),
            first_review,
            "旧文件原样改名为 *.prev"
        );
        let second_review = std::fs::read_to_string(artifacts_dir.join("review.json")).unwrap();
        assert_eq!(second_review, r#"{"findings":[]}"#, "第二次起为空");
    }

    #[test]
    fn 测试报告按标记与轮次变化() {
        let pass: serde_json::Value = serde_json::from_str(&mock_artifact_content(
            "test-report.json",
            true,
            &[],
            "VK-1",
        ))
        .unwrap();
        assert_eq!(
            (pass["total"].as_i64(), pass["failed"].as_i64()),
            (Some(3), Some(0))
        );

        for (marker, attribution) in [
            (QaMarker::TestFailCodeOnce, "code"),
            (QaMarker::TestFailCaseOnce, "case"),
        ] {
            let first: serde_json::Value = serde_json::from_str(&mock_artifact_content(
                "test-report.json",
                true,
                &[marker],
                "VK-1",
            ))
            .unwrap();
            assert_eq!(first["failed"].as_i64(), Some(1));
            assert!(
                first["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["attribution"] == attribution)
            );
            let later: serde_json::Value = serde_json::from_str(&mock_artifact_content(
                "test-report.json",
                false,
                &[marker],
                "VK-1",
            ))
            .unwrap();
            assert_eq!(later["failed"].as_i64(), Some(0), "第二次起全过");
        }
    }

    #[test]
    fn 评审始终阻断标记每次都有阻断项() {
        for first in [true, false] {
            let text =
                mock_artifact_content("review.json", first, &[QaMarker::AlwaysFailReview], "VK-1");
            assert!(text.contains("\"blocker\""));
        }
    }

    #[test]
    fn 带路径的文件名被跳过() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = PipelinePrompt {
            requirement: "VK-1".to_string(),
            artifacts_dir: dir.path().join("out"),
            required: vec!["../escape.md".to_string(), "spec.md".to_string()],
        };
        let written = write_mock_artifacts(&parsed).unwrap();
        assert_eq!(written, vec![dir.path().join("out/spec.md")]);
        assert!(!dir.path().join("escape.md").exists());
    }
}
