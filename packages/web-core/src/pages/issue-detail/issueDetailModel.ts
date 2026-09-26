import type {
  ArtifactKind,
  IssueArtifactSummary,
  IssuePipelineView,
  PipelineStageKey,
} from 'shared/types';
import {
  latestAttempts,
  stageRound,
  toMillis,
  toNumber,
} from '@/entities/pipeline/model/progress';

export type IssueDetailTab =
  | 'requirement'
  | 'spec'
  | 'cases'
  | 'code'
  | 'test'
  | 'delivery';

export const ISSUE_DETAIL_TABS: readonly IssueDetailTab[] = [
  'requirement',
  'spec',
  'cases',
  'code',
  'test',
  'delivery',
];

export function isIssueDetailTab(value: unknown): value is IssueDetailTab {
  return (
    typeof value === 'string' &&
    (ISSUE_DETAIL_TABS as readonly string[]).includes(value)
  );
}

const TAB_LABEL_KEYS: Record<IssueDetailTab, string> = {
  requirement: 'issueDetail.tab.requirement',
  spec: 'issueDetail.tab.spec',
  cases: 'issueDetail.tab.cases',
  code: 'issueDetail.tab.code',
  test: 'issueDetail.tab.test',
  delivery: 'issueDetail.tab.delivery',
};

export function issueDetailTabLabelKey(tab: IssueDetailTab): string {
  return TAB_LABEL_KEYS[tab];
}

const TAB_ARTIFACT_KINDS: Record<IssueDetailTab, readonly ArtifactKind[]> = {
  requirement: ['requirement'],
  spec: ['spec', 'plan'],
  cases: ['test_cases', 'trace_matrix'],
  code: ['review'],
  test: ['test_report'],
  delivery: ['delivery_report'],
};

export function tabArtifactKinds(tab: IssueDetailTab): readonly ArtifactKind[] {
  return TAB_ARTIFACT_KINDS[tab];
}

export function tabOfArtifactKind(kind: ArtifactKind): IssueDetailTab {
  return (
    ISSUE_DETAIL_TABS.find((tab) => TAB_ARTIFACT_KINDS[tab].includes(kind)) ??
    'requirement'
  );
}

const STAGE_DEFAULT_TAB: Record<PipelineStageKey, IssueDetailTab> = {
  requirement: 'requirement',
  spec: 'spec',
  test_design: 'cases',
  develop: 'code',
  review: 'code',
  test: 'test',
  deliver: 'delivery',
};

/** 打开详情时默认落在当前阶段对应的页签。 */
export function defaultIssueDetailTab(
  stage: PipelineStageKey | null
): IssueDetailTab {
  return stage ? STAGE_DEFAULT_TAB[stage] : 'requirement';
}

function isNewerArtifact(a: IssueArtifactSummary, b: IssueArtifactSummary) {
  const va = toNumber(a.version) ?? 0;
  const vb = toNumber(b.version) ?? 0;
  if (va !== vb) return va > vb;
  return (toMillis(a.created_at) ?? 0) > (toMillis(b.created_at) ?? 0);
}

export function latestArtifactOfKind(
  artifacts: readonly IssueArtifactSummary[],
  kind: ArtifactKind
): IssueArtifactSummary | null {
  let latest: IssueArtifactSummary | null = null;
  for (const artifact of artifacts) {
    if (artifact.kind !== kind) continue;
    if (!latest || isNewerArtifact(artifact, latest)) latest = artifact;
  }
  return latest;
}

/** 右栏「产出物」列表：每种取最新版，按页签顺序排。 */
export function latestArtifactsByKind(
  artifacts: readonly IssueArtifactSummary[]
): IssueArtifactSummary[] {
  return ISSUE_DETAIL_TABS.flatMap((tab) => TAB_ARTIFACT_KINDS[tab])
    .map((kind) => latestArtifactOfKind(artifacts, kind))
    .filter((artifact): artifact is IssueArtifactSummary => artifact !== null);
}

const ARTIFACT_KIND_LABEL_KEYS: Record<ArtifactKind, string> = {
  requirement: 'issueDetail.artifactKind.requirement',
  spec: 'issueDetail.artifactKind.spec',
  plan: 'issueDetail.artifactKind.plan',
  test_cases: 'issueDetail.artifactKind.test_cases',
  trace_matrix: 'issueDetail.artifactKind.trace_matrix',
  review: 'issueDetail.artifactKind.review',
  test_report: 'issueDetail.artifactKind.test_report',
  delivery_report: 'issueDetail.artifactKind.delivery_report',
};

export function artifactKindLabelKey(kind: ArtifactKind): string {
  return ARTIFACT_KIND_LABEL_KEYS[kind];
}

export type ArtifactRenderMode = 'markdown' | 'csv' | 'review' | 'test_report';

export function artifactRenderMode(kind: ArtifactKind): ArtifactRenderMode {
  switch (kind) {
    case 'test_cases':
      return 'csv';
    case 'review':
      return 'review';
    case 'test_report':
      return 'test_report';
    default:
      return 'markdown';
  }
}

/** 暂停中仍在等人工确认的关卡（契约 C12：暂停时关卡决策照常受理）。 */
export interface PendingGate {
  stageRunId: string;
  gateLabel: string | null;
}

export type GateBarState =
  | { kind: 'none' }
  | { kind: 'completed' }
  | { kind: 'cancelled' }
  | {
      kind: 'paused';
      runId: string;
      stageKey: PipelineStageKey;
      /** 当前阶段最新尝试在等人工确认时给出，界面照常显示确认 / 打回。 */
      pendingGate: PendingGate | null;
    }
  | {
      kind: 'failed';
      runId: string;
      stageKey: PipelineStageKey;
      attempt: number;
      maxRounds: number | null;
      /**
       * 当前阶段最新尝试的失败原因（后端已含日志末尾，可能很长）。关卡条只显示
       * 第一行；完整原文在右栏时间线里。
       */
      error: string | null;
    }
  | {
      kind: 'human';
      runId: string;
      stageRunId: string;
      stageKey: PipelineStageKey;
      gateLabel: string | null;
    }
  | {
      kind: 'auto';
      runId: string;
      stageKey: PipelineStageKey;
      attempt: number;
      maxRounds: number | null;
      /**
       * 模板里的判定名（契约 C10），交给 `autoConditionLabelKey` 取文案；
       * 不按阶段 key 猜。
       */
      gateCondition: string | null;
    }
  | { kind: 'running'; runId: string; stageKey: PipelineStageKey };

/** 底部关卡条显示什么（设计文档 §8.4）。 */
export function gateBarState(view: IssuePipelineView | null): GateBarState {
  if (!view) return { kind: 'none' };
  const { run } = view;
  const stageKey = run.current_stage_key;
  const runStages = view.stages.filter((s) => s.run_id === run.id);
  const current = latestAttempts(runStages).get(stageKey) ?? null;
  const templateStage =
    view.template.stages.find((stage) => stage.key === stageKey) ?? null;
  const attempt = current ? stageRound(runStages, current) : 1;
  const maxRounds = templateStage ? toNumber(templateStage.max_rounds) : null;
  const gateLabel = templateStage?.gate_label ?? null;
  const waitingHuman =
    current !== null &&
    current.status === 'waiting_gate' &&
    current.gate_kind === 'human';

  switch (run.status) {
    case 'completed':
      return { kind: 'completed' };
    case 'cancelled':
      return { kind: 'cancelled' };
    case 'paused':
      return {
        kind: 'paused',
        runId: run.id,
        stageKey,
        pendingGate:
          waitingHuman && current
            ? { stageRunId: current.id, gateLabel }
            : null,
      };
    case 'failed':
      return {
        kind: 'failed',
        runId: run.id,
        stageKey,
        attempt,
        maxRounds,
        error: current?.error ?? null,
      };
    case 'waiting_gate':
    case 'running':
      if (waitingHuman && current) {
        return {
          kind: 'human',
          runId: run.id,
          stageRunId: current.id,
          stageKey,
          gateLabel,
        };
      }
      if (templateStage?.gate_kind === 'auto') {
        return {
          kind: 'auto',
          runId: run.id,
          stageKey,
          attempt,
          maxRounds,
          gateCondition: templateStage.gate_condition ?? null,
        };
      }
      return { kind: 'running', runId: run.id, stageKey };
  }
}
