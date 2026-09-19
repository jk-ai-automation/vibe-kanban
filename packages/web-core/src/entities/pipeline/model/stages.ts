import type { PipelineStageKey } from 'shared/types';

/**
 * 标准七阶段的固定顺序（设计文档 §6.1）。
 *
 * 拿得到模板（`PipelineTemplateView.stages`）时以模板为准；看板卡片只有
 * 集合数据、没有模板，就用这份默认顺序。
 */
export const PIPELINE_STAGE_KEYS: readonly PipelineStageKey[] = [
  'requirement',
  'spec',
  'test_design',
  'develop',
  'review',
  'test',
  'deliver',
];

/**
 * 看板列对应的流程阶段，与 `features/kanban/model/stageType.ts` 的
 * `StageType` 逐字一致。entities 层不能反向依赖 features，所以这里重复
 * 声明一次，由 `features/kanban/model/pipelineColumns.test.ts` 钉住两边一致。
 */
export type PipelineBoardStage =
  | 'backlog'
  | 'todo'
  | 'dev'
  | 'review'
  | 'test'
  | 'done';

const STAGE_TO_BOARD: Record<PipelineStageKey, PipelineBoardStage> = {
  requirement: 'backlog',
  spec: 'todo',
  test_design: 'todo',
  develop: 'dev',
  review: 'review',
  test: 'test',
  deliver: 'done',
};

/** 流水线阶段落在看板哪一列（设计文档 §6.4）。 */
export function boardStageOf(key: PipelineStageKey): PipelineBoardStage {
  return STAGE_TO_BOARD[key];
}

export function isPipelineStageKey(value: unknown): value is PipelineStageKey {
  return (
    typeof value === 'string' &&
    (PIPELINE_STAGE_KEYS as readonly string[]).includes(value)
  );
}

const STAGE_LABEL_KEYS: Record<PipelineStageKey, string> = {
  requirement: 'pipeline.stage.requirement',
  spec: 'pipeline.stage.spec',
  test_design: 'pipeline.stage.test_design',
  develop: 'pipeline.stage.develop',
  review: 'pipeline.stage.review',
  test: 'pipeline.stage.test',
  deliver: 'pipeline.stage.deliver',
};

/** 阶段名的 i18n key（common 命名空间）。 */
export function pipelineStageLabelKey(key: PipelineStageKey): string {
  return STAGE_LABEL_KEYS[key];
}

const GATE_LABEL_KEYS: Record<PipelineStageKey, string | null> = {
  requirement: 'pipeline.gateLabel.requirement',
  spec: 'pipeline.gateLabel.spec',
  test_design: 'pipeline.gateLabel.test_design',
  develop: null,
  review: null,
  test: null,
  deliver: null,
};

/**
 * 模板没给 `gate_label` 时用的关卡名 key：三道人工关卡有专名，
 * 其它阶段回落到阶段名。
 */
export function fallbackGateLabelKey(key: PipelineStageKey): string {
  return GATE_LABEL_KEYS[key] ?? STAGE_LABEL_KEYS[key];
}

/**
 * 自动关卡的判定条件文案（设计文档 §6.1 的四种判定）。
 *
 * 按模板阶段的 `gate_condition`（契约 C10）对应，不按阶段 key 写死——
 * 仓库 `.vibe/pipeline.yaml` 可以给任意阶段配任意判定。
 */
const AUTO_CONDITION_KEYS = new Map<string, string>([
  ['checks_passed', 'pipeline.gate.autoCondition.checksPassed'],
  ['no_blocking_findings', 'pipeline.gate.autoCondition.noBlockingFindings'],
  ['all_cases_passed', 'pipeline.gate.autoCondition.allCasesPassed'],
  ['artifacts_present', 'pipeline.gate.autoCondition.artifactsPresent'],
]);

/**
 * `PipelineTemplateStageView.gate_condition` → 文案 key。
 * 人工关卡 / 无关卡（null）或不认识的判定名返回 null（界面不显示条件）。
 */
export function autoConditionLabelKey(
  condition: string | null | undefined
): string | null {
  if (!condition) return null;
  return AUTO_CONDITION_KEYS.get(condition) ?? null;
}

/** 阶段语义色（设计文档 §8.5）。渲染方映射成 `stage-*` 颜色令牌。 */
export type PipelineStageTone = 'neutral' | 'dev' | 'review' | 'test' | 'done';

const STAGE_TONES: Record<PipelineStageKey, PipelineStageTone> = {
  requirement: 'neutral',
  spec: 'neutral',
  test_design: 'neutral',
  develop: 'dev',
  review: 'review',
  test: 'test',
  deliver: 'done',
};

export function pipelineStageTone(key: PipelineStageKey): PipelineStageTone {
  return STAGE_TONES[key];
}
