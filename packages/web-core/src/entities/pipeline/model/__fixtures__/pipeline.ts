import type {
  IssueArtifactSummary,
  IssuePipelineView,
  PipelineGateDecision,
  PipelineRun,
  PipelineStageRun,
  PipelineTemplateView,
} from 'shared/types';

/**
 * 流水线测试夹具。
 *
 * 按**运行时真实形状**构造（时间是 ISO 字符串、i64 是 number，契约 C2），
 * 再 `as unknown as` 成生成类型，避免每个夹具都要写满全部字段。
 */
const T0 = Date.parse('2026-09-18T02:00:00.000Z');

/** 以 T0 为零点的第 n 分钟，ISO 字符串。 */
export function at(minutes: number): string {
  return new Date(T0 + minutes * 60_000).toISOString();
}

export const T0_MS = T0;

type Loose<T> = { [K in keyof T]?: unknown };

export function makeRun(overrides: Loose<PipelineRun> = {}): PipelineRun {
  return {
    id: 'run-1',
    issue_id: 'issue-1',
    project_id: 'project-1',
    workspace_id: null,
    template_key: 'standard',
    template_version: 1,
    status: 'running',
    current_stage_key: 'requirement',
    created_at: at(0),
    updated_at: at(0),
    finished_at: null,
    ...overrides,
  } as unknown as PipelineRun;
}

export function makeStage(
  overrides: Loose<PipelineStageRun> = {}
): PipelineStageRun {
  return {
    id: 'stage-1',
    run_id: 'run-1',
    project_id: 'project-1',
    stage_key: 'requirement',
    attempt: 1,
    status: 'running',
    gate_kind: 'human',
    session_id: null,
    execution_process_id: null,
    started_at: at(0),
    finished_at: null,
    summary: null,
    error: null,
    ...overrides,
  } as unknown as PipelineStageRun;
}

export function makeDecision(
  overrides: Loose<PipelineGateDecision> = {}
): PipelineGateDecision {
  return {
    id: 'decision-1',
    stage_run_id: 'stage-1',
    decision: 'approve',
    comment: null,
    decided_by: null,
    decided_at: at(1),
    ...overrides,
  } as unknown as PipelineGateDecision;
}

export function makeArtifact(
  overrides: Loose<IssueArtifactSummary> = {}
): IssueArtifactSummary {
  return {
    id: 'artifact-1',
    issue_id: 'issue-1',
    stage_run_id: 'stage-1',
    kind: 'requirement',
    rel_path: 'requirement.md',
    version: 1,
    truncated: false,
    created_at: at(1),
    ...overrides,
  } as unknown as IssueArtifactSummary;
}

/** 与后端内置模板一致（`crates/services/src/services/pipeline/template.rs`）。 */
export const STANDARD_TEMPLATE = {
  key: 'standard',
  version: 1,
  stages: [
    {
      key: 'requirement',
      skill: 'vk-requirement',
      gate_kind: 'human',
      gate_label: '需求确认',
      gate_condition: null,
      max_rounds: 3,
    },
    {
      key: 'spec',
      skill: 'vk-spec',
      gate_kind: 'human',
      gate_label: '设计规格确认',
      gate_condition: null,
      max_rounds: 3,
    },
    {
      key: 'test_design',
      skill: 'prd2testcase',
      gate_kind: 'human',
      gate_label: '用例设计确认',
      gate_condition: null,
      max_rounds: 3,
    },
    {
      key: 'develop',
      skill: 'vk-develop',
      gate_kind: 'auto',
      gate_label: null,
      gate_condition: 'checks_passed',
      max_rounds: 3,
    },
    {
      key: 'review',
      skill: 'vk-review',
      gate_kind: 'auto',
      gate_label: null,
      gate_condition: 'no_blocking_findings',
      max_rounds: 3,
    },
    {
      key: 'test',
      skill: 'atp-run',
      gate_kind: 'auto',
      gate_label: null,
      gate_condition: 'all_cases_passed',
      max_rounds: 3,
    },
    {
      key: 'deliver',
      skill: 'vk-deliver',
      gate_kind: 'auto',
      gate_label: null,
      gate_condition: 'artifacts_present',
      max_rounds: 3,
    },
  ],
} as unknown as PipelineTemplateView;

export function makeView(
  overrides: Partial<IssuePipelineView> = {}
): IssuePipelineView {
  return {
    run: makeRun(),
    stages: [makeStage()],
    decisions: [],
    artifacts: [],
    template: STANDARD_TEMPLATE,
    ...overrides,
  };
}
