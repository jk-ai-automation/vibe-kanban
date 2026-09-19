import type { PipelineRun, PipelineStageRun } from 'shared/types';
import {
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  type PipelineCellState,
  type PipelineTone,
  type Translate,
} from './progress';
import { pipelineStageLabelKey } from './stages';

export interface PipelineCardCell {
  key: string;
  state: PipelineCellState;
  /** 已翻译：「开发: 进行中」，用于格子的 title 与无障碍说明。 */
  label: string;
}

/** 看板卡片 / 工作台卡片底部要的全部信息（已翻译）。 */
export interface PipelineCardInfo {
  runId: string;
  cells: PipelineCardCell[];
  tone: PipelineTone;
  statusText: string;
  progressLabel: string;
  /** 失败时卡片加红边框（设计文档 §8.5 三重编码）。 */
  isFailed: boolean;
}

const CELL_LABEL_KEYS: Record<PipelineCellState, string> = {
  done: 'pipeline.cell.done',
  running: 'pipeline.cell.running',
  gate: 'pipeline.cell.gate',
  failed: 'pipeline.cell.failed',
  pending: 'pipeline.cell.pending',
  paused: 'pipeline.cell.paused',
  cancelled: 'pipeline.cell.cancelled',
};

export function cellLabelKey(state: PipelineCellState): string {
  return CELL_LABEL_KEYS[state];
}

/** 每个需求取最新创建的一次运行。 */
export function latestRunByIssue(
  runs: readonly PipelineRun[]
): Map<string, PipelineRun> {
  const map = new Map<string, PipelineRun>();
  for (const run of runs) {
    const current = map.get(run.issue_id);
    if (
      !current ||
      (toMillis(run.created_at) ?? 0) > (toMillis(current.created_at) ?? 0)
    ) {
      map.set(run.issue_id, run);
    }
  }
  return map;
}

export function groupStagesByRun(
  stages: readonly PipelineStageRun[]
): Map<string, PipelineStageRun[]> {
  const map = new Map<string, PipelineStageRun[]>();
  for (const stage of stages) {
    const list = map.get(stage.run_id);
    if (list) {
      list.push(stage);
    } else {
      map.set(stage.run_id, [stage]);
    }
  }
  return map;
}

/**
 * 卡片信息。集合里没有模板，所以用默认七阶段顺序、不显示关卡专名
 * （回落到内置关卡名 key）。
 */
export function buildPipelineCardInfo(
  run: PipelineRun,
  stages: readonly PipelineStageRun[],
  t: Translate
): PipelineCardInfo {
  const cells = pipelineProgress(stages, null, run).map((cell) => ({
    key: cell.key,
    state: cell.state,
    label: `${t(pipelineStageLabelKey(cell.key))}: ${t(CELL_LABEL_KEYS[cell.state])}`,
  }));
  const info = pipelineStatus(stages, null, run);
  return {
    runId: run.id,
    cells,
    tone: info.tone,
    statusText: pipelineStatusText(info, t),
    progressLabel: t('pipeline.progressLabel', {
      summary: cells.map((cell) => cell.label).join(', '),
    }),
    isFailed: info.tone === 'failed',
  };
}
