import type { PipelineRun, PipelineStageRun } from 'shared/types';
import type { ShapeDefinition } from 'shared/remote-types';

/**
 * 流水线两张表的集合形状（契约 §3）。
 *
 * 云端（`shared/remote-types.ts`）没有这两张表，`defineShape` 也没导出，
 * 这里手写同样的结构。`url` / `fallbackUrl` 只在云端数据源下被读，
 * 本地数据源下 `createShapeCollection` 直接走 `createLocalShapeCollection`
 * （`shared/lib/electric/collections.ts:741-751`），读的是 `localEndpoints.ts`。
 * 使用方（`usePipelineData.ts`）只在个人版启用这两个集合。
 */
export const PIPELINE_RUNS_SHAPE = {
  table: 'pipeline_runs',
  params: ['project_id'],
  url: '',
  fallbackUrl: '',
} as unknown as ShapeDefinition<PipelineRun>;

export const PIPELINE_STAGE_RUNS_SHAPE = {
  table: 'pipeline_stage_runs',
  params: ['project_id'],
  url: '',
  fallbackUrl: '',
} as unknown as ShapeDefinition<PipelineStageRun>;
