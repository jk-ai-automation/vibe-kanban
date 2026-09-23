//! 交付流水线引擎（设计 docs/superpowers/specs/2026-09-18-personal-pipeline-design.md §6）。
//! 对外只暴露 [`PipelineService`]；其余子模块是它的零件。

pub mod artifacts;
pub mod engine;
pub mod gates;
pub mod launcher;
pub mod plugin;
pub mod prompt;
pub mod template;
pub mod transition;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use engine::{PipelineError, PipelineService, StartPipelineInput};
pub use launcher::{ContainerStageLauncher, LaunchedStep, NoopStageLauncher, StageLauncher};
pub use plugin::{PipelineSkillInfo, ensure_extracted, qualify_skill, skills_view};
pub use transition::{FinishedProcess, PipelineExitEvent};
