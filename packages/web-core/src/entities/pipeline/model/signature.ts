import type { PipelineRun, PipelineStageRun } from 'shared/types';

/**
 * 集合数据的指纹：运行状态 / 当前阶段 / 阶段状态 / 尝试次数任一变化就变。
 *
 * 产出物与关卡决策不推送（契约 §3），前端靠这个指纹变化去失效
 * `GET /api/local/issues/{id}/pipeline` 与 pending 查询。
 */
export function pipelineSignature(
  runs: readonly PipelineRun[],
  stages: readonly PipelineStageRun[]
): string {
  const runIds = new Set(runs.map((run) => run.id));
  const parts = [
    ...runs.map((run) => `r:${run.id}:${run.status}:${run.current_stage_key}`),
    ...stages
      .filter((stage) => runIds.has(stage.run_id))
      .map((stage) => `s:${stage.id}:${stage.status}:${String(stage.attempt)}`),
  ];
  return parts.sort().join('|');
}
