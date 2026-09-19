//! 流水线模板：内置「标准七阶段」+ 仓库 `.vibe/pipeline.yaml`（设计 §6.1）。
//!
//! 运行启动时把解析好的模板序列化进 `pipeline_runs.template_json`，运行中途改 YAML
//! 不影响在跑的运行。

use std::path::Path;

use db::models::pipeline::{
    ArtifactKind, GateKind, PipelineStageKey, PipelineTemplateStageView, PipelineTemplateView,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BUILTIN_TEMPLATE_KEY: &str = "standard";
pub const REPO_TEMPLATE_KEY: &str = "repo";
/// 相对仓库根目录。
pub const REPO_TEMPLATE_PATH: &str = ".vibe/pipeline.yaml";
pub const DEFAULT_MAX_ROUNDS: i64 = 3;
pub const MAX_ROUNDS_LIMIT: i64 = 10;

/// 自动判定只有四种，不做表达式引擎（设计 §6.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoGate {
    ChecksPassed,
    NoBlockingFindings,
    AllCasesPassed,
    ArtifactsPresent,
}

impl AutoGate {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "checks_passed" => Some(AutoGate::ChecksPassed),
            "no_blocking_findings" => Some(AutoGate::NoBlockingFindings),
            "all_cases_passed" => Some(AutoGate::AllCasesPassed),
            "artifacts_present" => Some(AutoGate::ArtifactsPresent),
            _ => None,
        }
    }

    /// 判定名，与 YAML 里的写法一致；也是契约 `PipelineTemplateStageView.gate_condition` 的取值（C10）。
    pub fn as_str(self) -> &'static str {
        match self {
            AutoGate::ChecksPassed => "checks_passed",
            AutoGate::NoBlockingFindings => "no_blocking_findings",
            AutoGate::AllCasesPassed => "all_cases_passed",
            AutoGate::ArtifactsPresent => "artifacts_present",
        }
    }

    /// 判定要读的产出物；模板解析时要求本阶段声明了它，否则判定必然失败。
    pub fn required_artifact(self) -> Option<&'static str> {
        match self {
            AutoGate::NoBlockingFindings => Some("review.json"),
            AutoGate::AllCasesPassed => Some("test-report.json"),
            AutoGate::ChecksPassed | AutoGate::ArtifactsPresent => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageGate {
    Human { label: String },
    Auto { rule: AutoGate },
    None,
}

impl StageGate {
    pub fn kind(&self) -> GateKind {
        match self {
            StageGate::Human { .. } => GateKind::Human,
            StageGate::Auto { .. } => GateKind::Auto,
            StageGate::None => GateKind::None,
        }
    }

    pub fn human_label(&self) -> Option<&str> {
        match self {
            StageGate::Human { label } => Some(label.as_str()),
            _ => None,
        }
    }

    pub fn auto_rule(&self) -> Option<AutoGate> {
        match self {
            StageGate::Auto { rule } => Some(*rule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTemplate {
    pub key: PipelineStageKey,
    /// U3 前技能不存在，引擎只把名字写进提示词。
    pub skill: String,
    /// 本阶段必须产出的文件名（契约 §4）。
    pub artifacts: Vec<String>,
    /// 检查命令（契约修正 C3）；非空时智能体成功后在同会话里依次执行，任一失败即不通过。
    pub checks: Vec<String>,
    pub gate: StageGate,
    pub max_rounds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineTemplate {
    pub key: String,
    pub version: i64,
    /// 解析时保证非空、按标准顺序、不重复。
    pub stages: Vec<StageTemplate>,
}

impl PipelineTemplate {
    pub fn stage(&self, key: PipelineStageKey) -> Option<&StageTemplate> {
        self.stages.iter().find(|stage| stage.key == key)
    }

    /// 首阶段。`parse_template_yaml` 保证非空，但从 `template_json` 直接反序列化的快照
    /// 不经过校验，所以返回 Option，由调用方处理空模板，杜绝 panic。
    pub fn first_stage(&self) -> Option<&StageTemplate> {
        self.stages.first()
    }

    pub fn next_stage(&self, key: PipelineStageKey) -> Option<&StageTemplate> {
        let index = self.stages.iter().position(|stage| stage.key == key)?;
        self.stages.get(index + 1)
    }

    pub fn to_view(&self) -> PipelineTemplateView {
        PipelineTemplateView {
            key: self.key.clone(),
            version: self.version,
            stages: self
                .stages
                .iter()
                .map(|stage| PipelineTemplateStageView {
                    key: stage.key,
                    skill: stage.skill.clone(),
                    gate_kind: stage.gate.kind(),
                    gate_label: stage.gate.human_label().map(str::to_string),
                    // C10：自动关卡填判定名，人工与无关卡为 None。
                    gate_condition: stage.gate.auto_rule().map(|rule| rule.as_str().to_string()),
                    max_rounds: stage.max_rounds,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("YAML 解析失败：{0}")]
    Yaml(String),
    #[error("不支持的模板版本 {0}，目前只支持 1")]
    UnsupportedVersion(i64),
    #[error("模板至少要有一个阶段")]
    Empty,
    #[error("未知阶段：{0}")]
    UnknownStage(String),
    #[error(
        "阶段 {0} 重复或顺序不对（必须按 requirement → spec → test_design → develop → review → test → deliver 排列）"
    )]
    BadOrder(String),
    #[error("阶段 {stage} 的产出物 {file} 不在约定清单里")]
    UnknownArtifact { stage: String, file: String },
    #[error("阶段 {0} 的关卡只能写 human 或 auto 其中一个")]
    AmbiguousGate(String),
    #[error("阶段 {stage} 的自动关卡 {rule} 不存在")]
    UnknownAutoGate { stage: String, rule: String },
    #[error("阶段 {0} 的人工关卡名称不能为空")]
    EmptyHumanLabel(String),
    #[error("阶段 {stage} 的 max_rounds 必须在 1 到 10 之间，实际 {value}")]
    BadMaxRounds { stage: String, value: i64 },
    #[error("阶段 {stage} 的产出物 {file} 重复声明")]
    DuplicateArtifact { stage: String, file: String },
    #[error("产出物 {file} 属于阶段 {owner}，不能写在阶段 {stage} 里")]
    ArtifactStageMismatch {
        stage: String,
        file: String,
        owner: String,
    },
    #[error("阶段 {stage} 的自动关卡 {rule} 依赖产出物 {file}，请在 artifacts 里声明")]
    GateNeedsArtifact {
        stage: String,
        rule: String,
        file: String,
    },
    #[error("阶段 {0} 的 skill 不能为空")]
    EmptySkill(String),
    #[error("阶段 {0} 的 checks 里有空命令")]
    EmptyCheck(String),
}

pub fn builtin_template() -> PipelineTemplate {
    fn stage(
        key: PipelineStageKey,
        skill: &str,
        artifacts: &[&str],
        gate: StageGate,
    ) -> StageTemplate {
        StageTemplate {
            key,
            skill: skill.to_string(),
            artifacts: artifacts.iter().map(|name| name.to_string()).collect(),
            checks: Vec::new(),
            gate,
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }
    fn human(label: &str) -> StageGate {
        StageGate::Human {
            label: label.to_string(),
        }
    }
    fn auto(rule: AutoGate) -> StageGate {
        StageGate::Auto { rule }
    }

    PipelineTemplate {
        key: BUILTIN_TEMPLATE_KEY.to_string(),
        version: 1,
        stages: vec![
            stage(
                PipelineStageKey::Requirement,
                "vk-requirement",
                &["requirement.md"],
                human("需求确认"),
            ),
            stage(
                PipelineStageKey::Spec,
                "vk-spec",
                &["spec.md", "plan.md"],
                human("设计规格确认"),
            ),
            stage(
                PipelineStageKey::TestDesign,
                "prd2testcase",
                &["test-cases.csv", "trace-matrix.md"],
                human("用例设计确认"),
            ),
            stage(
                PipelineStageKey::Develop,
                "vk-develop",
                &[],
                auto(AutoGate::ChecksPassed),
            ),
            stage(
                PipelineStageKey::Review,
                "vk-review",
                &["review.json"],
                auto(AutoGate::NoBlockingFindings),
            ),
            stage(
                PipelineStageKey::Test,
                "atp-run",
                &["test-report.json"],
                auto(AutoGate::AllCasesPassed),
            ),
            stage(
                PipelineStageKey::Deliver,
                "vk-deliver",
                &["delivery-report.md"],
                auto(AutoGate::ArtifactsPresent),
            ),
        ],
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    version: i64,
    stages: Vec<RawStage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStage {
    key: String,
    skill: String,
    #[serde(default)]
    artifacts: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    retry: Option<RawRetry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate {
    #[serde(default)]
    human: Option<String>,
    #[serde(default)]
    auto: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRetry {
    max_rounds: i64,
}

pub fn parse_template_yaml(text: &str) -> Result<PipelineTemplate, TemplateError> {
    let raw: RawTemplate =
        serde_yaml::from_str(text).map_err(|e| TemplateError::Yaml(e.to_string()))?;
    if raw.version != 1 {
        return Err(TemplateError::UnsupportedVersion(raw.version));
    }
    if raw.stages.is_empty() {
        return Err(TemplateError::Empty);
    }

    let mut stages = Vec::with_capacity(raw.stages.len());
    let mut last_order: Option<usize> = None;
    for raw_stage in raw.stages {
        let name = raw_stage.key.clone();
        let key = PipelineStageKey::parse(&name)
            .ok_or_else(|| TemplateError::UnknownStage(name.clone()))?;
        if last_order.is_some_and(|last| key.order() <= last) {
            return Err(TemplateError::BadOrder(name));
        }
        last_order = Some(key.order());

        if raw_stage.skill.trim().is_empty() {
            return Err(TemplateError::EmptySkill(name));
        }
        if raw_stage.checks.iter().any(|check| check.trim().is_empty()) {
            return Err(TemplateError::EmptyCheck(name));
        }

        for (index, file) in raw_stage.artifacts.iter().enumerate() {
            let Some(kind) = ArtifactKind::from_file_name(file) else {
                return Err(TemplateError::UnknownArtifact {
                    stage: name.clone(),
                    file: file.clone(),
                });
            };
            if kind.stage() != key {
                return Err(TemplateError::ArtifactStageMismatch {
                    stage: name.clone(),
                    file: file.clone(),
                    owner: kind.stage().as_str().to_string(),
                });
            }
            if raw_stage.artifacts[..index].contains(file) {
                return Err(TemplateError::DuplicateArtifact {
                    stage: name.clone(),
                    file: file.clone(),
                });
            }
        }

        let gate = match raw_stage.gate {
            None
            | Some(RawGate {
                human: None,
                auto: None,
            }) => StageGate::None,
            Some(RawGate {
                human: Some(_),
                auto: Some(_),
            }) => return Err(TemplateError::AmbiguousGate(name)),
            Some(RawGate {
                human: Some(label),
                auto: None,
            }) => {
                let label = label.trim().to_string();
                if label.is_empty() {
                    return Err(TemplateError::EmptyHumanLabel(name));
                }
                StageGate::Human { label }
            }
            Some(RawGate {
                human: None,
                auto: Some(rule),
            }) => StageGate::Auto {
                rule: AutoGate::parse(&rule).ok_or_else(|| TemplateError::UnknownAutoGate {
                    stage: name.clone(),
                    rule: rule.clone(),
                })?,
            },
        };

        if let Some(rule) = gate.auto_rule()
            && let Some(file) = rule.required_artifact()
            && !raw_stage.artifacts.iter().any(|name| name == file)
        {
            return Err(TemplateError::GateNeedsArtifact {
                stage: name,
                rule: rule.as_str().to_string(),
                file: file.to_string(),
            });
        }

        let max_rounds = raw_stage
            .retry
            .map(|retry| retry.max_rounds)
            .unwrap_or(DEFAULT_MAX_ROUNDS);
        if !(1..=MAX_ROUNDS_LIMIT).contains(&max_rounds) {
            return Err(TemplateError::BadMaxRounds {
                stage: name,
                value: max_rounds,
            });
        }

        stages.push(StageTemplate {
            key,
            skill: raw_stage.skill,
            artifacts: raw_stage.artifacts,
            checks: raw_stage.checks,
            gate,
            max_rounds,
        });
    }

    Ok(PipelineTemplate {
        key: REPO_TEMPLATE_KEY.to_string(),
        version: raw.version,
        stages,
    })
}

/// 读仓库模板：文件不存在用内置模板；读取或解析失败回落内置模板并返回警告（设计 §6.1）。
pub fn load_template(repo_root: &Path) -> (PipelineTemplate, Option<String>) {
    let path = repo_root.join(REPO_TEMPLATE_PATH);
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse_template_yaml(&text) {
            Ok(template) => (template, None),
            Err(e) => (
                builtin_template(),
                Some(format!(
                    "{REPO_TEMPLATE_PATH} 解析失败，已改用内置模板：{e}"
                )),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (builtin_template(), None),
        Err(e) => (
            builtin_template(),
            Some(format!(
                "读取 {REPO_TEMPLATE_PATH} 失败，已改用内置模板：{e}"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use db::models::pipeline::{GateKind, PipelineStageKey};

    use super::*;

    /// 设计文档 §6.1 的 YAML 原文（checks 按契约修正 C3 改成 shell 命令）。
    const DESIGN_YAML: &str = r#"
version: 1
stages:
  - key: requirement
    skill: vk-requirement
    artifacts: [requirement.md]
    gate: { human: 需求确认 }
  - key: spec
    skill: vk-spec
    artifacts: [spec.md, plan.md]
    gate: { human: 设计规格确认 }
  - key: test_design
    skill: prd2testcase
    artifacts: [test-cases.csv, trace-matrix.md]
    gate: { human: 用例设计确认 }
  - key: develop
    skill: vk-develop
    checks: ["pnpm run lint", "cargo test"]
    gate: { auto: checks_passed }
    retry: { max_rounds: 3 }
  - key: review
    skill: vk-review
    artifacts: [review.json]
    gate: { auto: no_blocking_findings }
    retry: { max_rounds: 3 }
  - key: test
    skill: atp-run
    artifacts: [test-report.json]
    gate: { auto: all_cases_passed }
    retry: { max_rounds: 3 }
  - key: deliver
    skill: vk-deliver
    artifacts: [delivery-report.md]
    gate: { auto: artifacts_present }
"#;

    #[test]
    fn 内置模板是标准七阶段且三道人工关卡() {
        let template = builtin_template();
        assert_eq!(template.key, BUILTIN_TEMPLATE_KEY);
        assert_eq!(template.version, 1);
        assert_eq!(
            template.stages.iter().map(|s| s.key).collect::<Vec<_>>(),
            PipelineStageKey::ALL.to_vec()
        );
        let humans: Vec<_> = template
            .stages
            .iter()
            .filter_map(|s| s.gate.human_label())
            .collect();
        assert_eq!(humans, vec!["需求确认", "设计规格确认", "用例设计确认"]);
        assert!(
            template
                .stages
                .iter()
                .all(|s| s.max_rounds == DEFAULT_MAX_ROUNDS)
        );
        assert!(
            template
                .stage(PipelineStageKey::Develop)
                .unwrap()
                .checks
                .is_empty(),
            "内置模板不带检查命令（repos 表没有 lint/test 脚本）"
        );
    }

    #[test]
    fn 设计文档里的_yaml_能解析且与内置模板只差_key_和_checks() {
        let parsed = parse_template_yaml(DESIGN_YAML).expect("设计文档 YAML 应能解析");
        assert_eq!(parsed.key, REPO_TEMPLATE_KEY);
        let mut builtin = builtin_template();
        builtin.key = REPO_TEMPLATE_KEY.to_string();
        builtin
            .stages
            .iter_mut()
            .find(|s| s.key == PipelineStageKey::Develop)
            .unwrap()
            .checks = vec!["pnpm run lint".to_string(), "cargo test".to_string()];
        assert_eq!(parsed, builtin);
    }

    #[test]
    fn 下一阶段与首阶段() {
        let template = builtin_template();
        assert_eq!(
            template.first_stage().map(|s| s.key),
            Some(PipelineStageKey::Requirement)
        );
        assert_eq!(
            template.next_stage(PipelineStageKey::Test).map(|s| s.key),
            Some(PipelineStageKey::Deliver)
        );
        assert!(template.next_stage(PipelineStageKey::Deliver).is_none());
    }

    #[test]
    fn 视图只暴露契约字段() {
        let view = builtin_template().to_view();
        assert_eq!(view.stages.len(), 7);
        assert_eq!(view.stages[0].gate_kind, GateKind::Human);
        assert_eq!(view.stages[0].gate_label.as_deref(), Some("需求确认"));
        assert_eq!(view.stages[0].gate_condition, None, "人工关卡没有判定名");
        assert_eq!(view.stages[3].gate_kind, GateKind::Auto);
        assert_eq!(view.stages[3].gate_label, None);
        assert_eq!(
            view.stages[3].gate_condition.as_deref(),
            Some("checks_passed")
        );
        for stage in &view.stages {
            assert_eq!(
                stage.gate_condition.is_some(),
                stage.gate_kind == GateKind::Auto,
                "只有自动关卡有 gate_condition：{stage:?}"
            );
        }
    }

    #[test]
    fn 省略阶段也能解析且默认轮次为三() {
        let parsed = parse_template_yaml(
            "version: 1\nstages:\n  - key: develop\n    skill: vk-develop\n    gate: { auto: checks_passed }\n  - key: deliver\n    skill: vk-deliver\n    artifacts: [delivery-report.md]\n",
        )
        .unwrap();
        assert_eq!(parsed.stages.len(), 2);
        assert_eq!(parsed.stages[0].max_rounds, DEFAULT_MAX_ROUNDS);
        assert_eq!(parsed.stages[1].gate, StageGate::None);
    }

    fn 断言错误(yaml: &str, expected: impl Fn(&TemplateError) -> bool) {
        let error = parse_template_yaml(yaml).expect_err(yaml);
        assert!(expected(&error), "YAML:\n{yaml}\n实际错误：{error:?}");
    }

    #[test]
    fn 非法模板给出明确错误() {
        断言错误("version: 2\nstages: []\n", |e| {
            matches!(e, TemplateError::UnsupportedVersion(2))
        });
        断言错误("version: 1\nstages: []\n", |e| {
            matches!(e, TemplateError::Empty)
        });
        断言错误(
            "version: 1\nstages:\n  - key: design\n    skill: x\n",
            |e| matches!(e, TemplateError::UnknownStage(k) if k == "design"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n  - key: requirement\n    skill: y\n",
            |e| matches!(e, TemplateError::BadOrder(k) if k == "requirement"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n  - key: spec\n    skill: y\n",
            |e| matches!(e, TemplateError::BadOrder(k) if k == "spec"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    artifacts: [notes.md]\n",
            |e| matches!(e, TemplateError::UnknownArtifact { file, .. } if file == "notes.md"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { human: 确认, auto: checks_passed }\n",
            |e| matches!(e, TemplateError::AmbiguousGate(_)),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { auto: tests_green }\n",
            |e| matches!(e, TemplateError::UnknownAutoGate { rule, .. } if rule == "tests_green"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { human: \"  \" }\n",
            |e| matches!(e, TemplateError::EmptyHumanLabel(_)),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    retry: { max_rounds: 0 }\n",
            |e| matches!(e, TemplateError::BadMaxRounds { value: 0, .. }),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    timeout: 3\n",
            |e| matches!(e, TemplateError::Yaml(_)),
        );
    }

    #[test]
    fn 仓库没有模板文件时用内置模板且无警告() {
        let dir = tempfile::tempdir().unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template, builtin_template());
        assert!(warning.is_none());
    }

    #[test]
    fn 仓库模板解析失败时回落内置模板并给警告() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
        std::fs::write(
            dir.path().join(REPO_TEMPLATE_PATH),
            "version: 9\nstages: []\n",
        )
        .unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template, builtin_template());
        let warning = warning.expect("应给出警告");
        assert!(warning.contains(REPO_TEMPLATE_PATH), "{warning}");
    }

    #[test]
    fn 仓库模板合法时以仓库为准() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
        std::fs::write(dir.path().join(REPO_TEMPLATE_PATH), DESIGN_YAML).unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template.key, REPO_TEMPLATE_KEY);
        assert!(warning.is_none());
    }

    #[test]
    fn 模板可以序列化成_json_再读回() {
        let template = builtin_template();
        let json = serde_json::to_string(&template).unwrap();
        let back: PipelineTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, template);
    }

    #[test]
    fn 快照里阶段为空时首阶段为_none_不会_panic() {
        // template_json 是 serde 直接反序列化的，不经过 parse_template_yaml 的非空校验。
        let empty: PipelineTemplate =
            serde_json::from_str(r#"{"key":"repo","version":1,"stages":[]}"#).unwrap();
        assert!(empty.first_stage().is_none());
    }

    #[test]
    fn 自动关卡依赖的产出物必须在本阶段声明() {
        断言错误(
            "version: 1\nstages:\n  - key: review\n    skill: vk-review\n    gate: { auto: no_blocking_findings }\n",
            |e| {
                matches!(e, TemplateError::GateNeedsArtifact { stage, file, .. }
                    if stage == "review" && file == "review.json")
            },
        );
        断言错误(
            "version: 1\nstages:\n  - key: test\n    skill: atp-run\n    gate: { auto: all_cases_passed }\n",
            |e| {
                matches!(e, TemplateError::GateNeedsArtifact { stage, file, .. }
                    if stage == "test" && file == "test-report.json")
            },
        );
        // review.json 只属于评审阶段，所以 all_cases_passed 放到评审阶段也缺 test-report.json。
        断言错误(
            "version: 1\nstages:\n  - key: review\n    skill: vk-review\n    artifacts: [review.json]\n    gate: { auto: all_cases_passed }\n",
            |e| matches!(e, TemplateError::GateNeedsArtifact { file, .. } if file == "test-report.json"),
        );
    }

    #[test]
    fn 关卡缺产出物的仓库模板回落内置模板并给警告() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
        std::fs::write(
            dir.path().join(REPO_TEMPLATE_PATH),
            "version: 1\nstages:\n  - key: review\n    skill: vk-review\n    gate: { auto: no_blocking_findings }\n",
        )
        .unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template, builtin_template());
        let warning = warning.expect("应给出警告");
        assert!(warning.contains("review.json"), "{warning}");
    }

    #[test]
    fn 产出物重复或不属于本阶段时拒绝() {
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    artifacts: [spec.md, plan.md, spec.md]\n",
            |e| {
                matches!(e, TemplateError::DuplicateArtifact { stage, file }
                    if stage == "spec" && file == "spec.md")
            },
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    artifacts: [requirement.md]\n",
            |e| {
                matches!(e, TemplateError::ArtifactStageMismatch { stage, file, owner }
                    if stage == "spec" && file == "requirement.md" && owner == "requirement")
            },
        );
        断言错误(
            "version: 1\nstages:\n  - key: develop\n    skill: x\n    artifacts: [review.json]\n",
            |e| matches!(e, TemplateError::ArtifactStageMismatch { .. }),
        );
    }

    #[test]
    fn 技能为空或检查命令为空时拒绝() {
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: \"\"\n",
            |e| matches!(e, TemplateError::EmptySkill(stage) if stage == "spec"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: \"  \"\n",
            |e| matches!(e, TemplateError::EmptySkill(_)),
        );
        断言错误(
            "version: 1\nstages:\n  - key: develop\n    skill: x\n    checks: [\"cargo test\", \" \"]\n",
            |e| matches!(e, TemplateError::EmptyCheck(stage) if stage == "develop"),
        );
    }

    #[test]
    fn 内置模板每个阶段的产出物都属于本阶段() {
        for stage in builtin_template().stages {
            for file in &stage.artifacts {
                assert_eq!(
                    ArtifactKind::from_file_name(file).map(ArtifactKind::stage),
                    Some(stage.key),
                    "{file}"
                );
            }
        }
    }
}
