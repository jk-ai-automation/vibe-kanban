import type {
  PipelineRun,
  PipelineRunStatus,
  PipelineStageKey,
  PipelineStageRun,
  PipelineStageStatus,
  PipelineTemplateView,
} from 'shared/types';
import {
  PIPELINE_STAGE_KEYS,
  fallbackGateLabelKey,
  pipelineStageLabelKey,
} from './stages';

/** 进度格状态（设计文档 §8.3：已完成绿、进行中蓝、等人工橙、失败红）。 */
export type PipelineCellState =
  | 'done'
  | 'running'
  | 'gate'
  | 'failed'
  | 'pending'
  | 'paused'
  | 'cancelled';

export interface PipelineCell {
  key: PipelineStageKey;
  state: PipelineCellState;
  /** 该阶段最新一次尝试的序号；还没跑过为 null。 */
  attempt: number | null;
}

/** 运行整体的状态色调（= 当前阶段格子的状态，永远不是 pending）。 */
export type PipelineTone = Exclude<PipelineCellState, 'pending'>;

type TemplateLike = Pick<PipelineTemplateView, 'stages'> | null | undefined;

/**
 * 时间字段一律用它读：ts-rs 可能把 `DateTime<Utc>` 声明成 `Date` 或 `string`，
 * 运行时 JSON 一定是 ISO 字符串。非法值返回 null，绝不抛错。
 */
export function toMillis(value: unknown): number | null {
  if (value instanceof Date) {
    const ms = value.getTime();
    return Number.isFinite(ms) ? ms : null;
  }
  if (typeof value === 'string') {
    const ms = Date.parse(value);
    return Number.isFinite(ms) ? ms : null;
  }
  return null;
}

/**
 * i64 字段一律用它读：契约 C2 已把流水线的 i64 声明成 `number`，但 ts-rs
 * 默认是 `bigint`，防御一次零成本。非法值返回 null，绝不抛错。
 */
export function toNumber(value: unknown): number | null {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value === 'bigint') return Number(value);
  return null;
}

export function stageKeysOf(template: TemplateLike): PipelineStageKey[] {
  const keys = template?.stages?.map((stage) => stage.key) ?? [];
  return keys.length > 0 ? keys : [...PIPELINE_STAGE_KEYS];
}

function isNewer(candidate: PipelineStageRun, current: PipelineStageRun) {
  const a = toNumber(candidate.attempt) ?? 0;
  const b = toNumber(current.attempt) ?? 0;
  if (a !== b) return a > b;
  return (
    (toMillis(candidate.started_at) ?? -1) >
    (toMillis(current.started_at) ?? -1)
  );
}

/** 每个阶段取「最新一次尝试」。 */
export function latestAttempts(
  stages: readonly PipelineStageRun[]
): Map<PipelineStageKey, PipelineStageRun> {
  const latest = new Map<PipelineStageKey, PipelineStageRun>();
  for (const stage of stages) {
    const current = latest.get(stage.stage_key);
    if (!current || isNewer(stage, current)) {
      latest.set(stage.stage_key, stage);
    }
  }
  return latest;
}

/**
 * 「用户手动停止」执行进程时后端写进阶段尝试 `error` 的固定文案。
 * 来源：`crates/db/src/models/pipeline.rs` 的 `MANUAL_STOP_ERROR`，两边必须逐字一致。
 * 这种尝试不计入 `max_rounds`（后端 `PipelineStageRuns::count_failed` 排除它）。
 */
export const MANUAL_STOP_ERROR = '用户手动停止';

export function isManualStop(stage: Pick<PipelineStageRun, 'error'>): boolean {
  return stage.error === MANUAL_STOP_ERROR;
}

/**
 * 这次尝试是本阶段的第几轮：尝试序号减去本运行、本阶段更早的
 * 「用户手动停止」尝试数，至少为 1。
 */
export function stageRound(
  stages: readonly PipelineStageRun[],
  stage: PipelineStageRun
): number {
  const attempt = toNumber(stage.attempt) ?? 1;
  const manualStopsBefore = stages.filter(
    (other) =>
      other.id !== stage.id &&
      other.run_id === stage.run_id &&
      other.stage_key === stage.stage_key &&
      (toNumber(other.attempt) ?? 0) < attempt &&
      isManualStop(other)
  ).length;
  return Math.max(1, attempt - manualStopsBefore);
}

function currentCellState(
  runStatus: PipelineRunStatus,
  stageStatus: PipelineStageStatus | null
): PipelineTone {
  switch (runStatus) {
    case 'completed':
      return 'done';
    case 'failed':
      return 'failed';
    case 'waiting_gate':
      return 'gate';
    case 'paused':
      return 'paused';
    case 'cancelled':
      return 'cancelled';
    case 'running':
      if (stageStatus === 'waiting_gate') return 'gate';
      if (stageStatus === 'failed') return 'failed';
      return 'running';
  }
}

/**
 * 七格进度条（设计文档 §8.3）。只看 `run.id` 这一次运行的阶段记录。
 */
export function pipelineProgress(
  stages: readonly PipelineStageRun[],
  template: TemplateLike,
  run: PipelineRun
): PipelineCell[] {
  const keys = stageKeysOf(template);
  const latest = latestAttempts(stages.filter((s) => s.run_id === run.id));
  const currentIndex = keys.indexOf(run.current_stage_key);

  return keys.map((key, index): PipelineCell => {
    const attempt = latest.get(key) ?? null;
    const attemptNo = attempt ? toNumber(attempt.attempt) : null;

    if (run.status === 'completed') {
      return { key, state: 'done', attempt: attemptNo };
    }
    if (currentIndex === -1) {
      const passed =
        attempt?.status === 'passed' || attempt?.status === 'skipped';
      return { key, state: passed ? 'done' : 'pending', attempt: attemptNo };
    }
    if (index < currentIndex) {
      return { key, state: 'done', attempt: attemptNo };
    }
    if (index > currentIndex) {
      return { key, state: 'pending', attempt: attemptNo };
    }
    return {
      key,
      state: currentCellState(run.status, attempt?.status ?? null),
      attempt: attemptNo,
    };
  });
}

export interface PipelineStatusInfo {
  tone: PipelineTone;
  stageKey: PipelineStageKey;
  /** 模板里的人工关卡名（如「需求确认」）；没有模板或不是人工关卡为 null。 */
  gateLabel: string | null;
  /**
   * 当前阶段的轮次（`stageRound`：尝试序号扣掉更早的「用户手动停止」），
   * 至少为 1。
   */
  attempt: number;
  /**
   * 当前阶段最多轮次；没有模板、或不是自动关卡阶段时为 null。
   * 人工关卡阶段的 max_rounds 只管「缺产出物 / 进程失败」的重试，人工打回
   * 不计入（计划 A），显示「第 2/3 轮」会误导，所以只给自动阶段。
   */
  maxRounds: number | null;
}

export function pipelineStatus(
  stages: readonly PipelineStageRun[],
  template: TemplateLike,
  run: PipelineRun
): PipelineStatusInfo {
  const runStages = stages.filter((s) => s.run_id === run.id);
  const latest = latestAttempts(runStages).get(run.current_stage_key) ?? null;
  const templateStage =
    template?.stages?.find((stage) => stage.key === run.current_stage_key) ??
    null;

  return {
    tone: currentCellState(run.status, latest?.status ?? null),
    stageKey: run.current_stage_key,
    gateLabel: templateStage?.gate_label ?? null,
    attempt: latest ? stageRound(runStages, latest) : 1,
    maxRounds:
      templateStage && templateStage.gate_kind === 'auto'
        ? toNumber(templateStage.max_rounds)
        : null,
  };
}

/** 翻译函数的最小形状，方便在纯函数里注入与在测试里替身。 */
export type Translate = (
  key: string,
  params?: Record<string, string | number>
) => string;

const FIXED_STATUS_KEYS: Record<
  Exclude<PipelineTone, 'gate' | 'running'>,
  string
> = {
  failed: 'pipeline.status.failed',
  paused: 'pipeline.status.paused',
  done: 'pipeline.status.completed',
  cancelled: 'pipeline.status.cancelled',
};

/** 状态标签文案（卡片、工作台、详情头部共用）。 */
export function pipelineStatusText(
  info: PipelineStatusInfo,
  t: Translate
): string {
  const stage = t(pipelineStageLabelKey(info.stageKey));
  switch (info.tone) {
    case 'gate':
      return t('pipeline.status.waitingGate', {
        gate: info.gateLabel ?? t(fallbackGateLabelKey(info.stageKey)),
      });
    case 'running':
      if (info.attempt > 1 && info.maxRounds !== null && info.maxRounds > 1) {
        return t('pipeline.status.runningRound', {
          stage,
          attempt: info.attempt,
          max: info.maxRounds,
        });
      }
      return t('pipeline.status.running', { stage });
    default:
      return t(FIXED_STATUS_KEYS[info.tone]);
  }
}
