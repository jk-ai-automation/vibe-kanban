//! 交付流水线引擎（设计 docs/superpowers/specs/2026-09-18-personal-pipeline-design.md §6）。
//! 对外只暴露 [`PipelineService`]（任务 15 起）；其余子模块是它的纯函数零件。

pub mod artifacts;
pub mod gates;
pub mod prompt;
pub mod template;
pub mod transition;

#[cfg(test)]
mod test_support;

pub use transition::{FinishedProcess, PipelineExitEvent};
