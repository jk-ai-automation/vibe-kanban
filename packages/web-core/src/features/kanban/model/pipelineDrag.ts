import type { PipelineRunStatus } from 'shared/types';
import { STAGE_ORDER, type StageType } from './stageType';

export type DropDecision =
  | { allowed: true }
  | { allowed: false; reasonKey: string };

const ACTIVE_STATUSES: ReadonlySet<PipelineRunStatus> = new Set([
  'running',
  'waiting_gate',
]);

/**
 * 个人版流水线看板的拖拽规则（设计文档 §8.3 + 计划 §3 决策 7）。
 *
 * - 没有流水线的手工需求：不限制（保持旧行为）。
 * - 流水线在跑：跨列一律拦下——契约没有「打回到指定阶段」的接口，
 *   只改列会和引擎写的阶段打架。
 * - 流水线已停：只允许往回拖。
 */
export function canDropIssue(args: {
  fromStage: StageType;
  toStage: StageType;
  runStatus: PipelineRunStatus | null;
}): DropDecision {
  const { fromStage, toStage, runStatus } = args;
  if (fromStage === toStage || runStatus === null) {
    return { allowed: true };
  }
  if (ACTIVE_STATUSES.has(runStatus)) {
    return { allowed: false, reasonKey: 'pipeline.drag.pauseFirst' };
  }
  if (STAGE_ORDER[toStage] > STAGE_ORDER[fromStage]) {
    return { allowed: false, reasonKey: 'pipeline.drag.forwardBlocked' };
  }
  return { allowed: true };
}
