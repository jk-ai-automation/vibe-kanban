import type { ProjectStatus } from 'shared/remote-types';

/**
 * 流程阶段。与后端 `db::models::local_project_status::StageType` 逐字对应。
 *
 * 背景：`shared/remote-types.ts` 的 `ProjectStatus` 由 remote crate 生成，**没有**
 * `stage_type` 字段；本地 `/api/local/project_statuses` 的响应里有
 * （`crates/db/src/models/local_project_status.rs` 的 `LocalProjectStatus`）。
 * 所以前端只能把它当成「可能缺失的额外字段」来读，缺失时一律归一成 'todo'。
 */
export type StageType = 'backlog' | 'todo' | 'dev' | 'review' | 'test' | 'done';

/** 带 stage_type 的状态列。团队版（remote 数据源）下该字段缺失。 */
export type StatusWithStage = ProjectStatus & { stage_type?: unknown };

/** 流程先后顺序。数值只用于比较大小，不要持久化。 */
export const STAGE_ORDER: Record<StageType, number> = {
  backlog: 0,
  todo: 1,
  dev: 2,
  review: 3,
  test: 4,
  done: 5,
};

/** 缺失或无法识别时的回落阶段。 */
export const FALLBACK_STAGE: StageType = 'todo';

const KNOWN_STAGES = Object.keys(STAGE_ORDER) as StageType[];

export function isStageType(value: unknown): value is StageType {
  return (
    typeof value === 'string' && (KNOWN_STAGES as string[]).includes(value)
  );
}

/**
 * 从状态列对象里读出流程阶段。
 * 字段缺失、类型不对、值不认识，一律回落到 'todo'，绝不抛错
 * （团队版 remote 数据源下这个字段一定缺失）。
 */
export function readStageType(
  status: { stage_type?: unknown } | null | undefined
): StageType {
  const raw = status?.stage_type;
  return isStageType(raw) ? raw : FALLBACK_STAGE;
}

/**
 * 阶段的 i18n key（common 命名空间）。
 * 返回 key 而不是文案，渲染方自己 `t(stageLabelKey(stage))`。
 */
const STAGE_LABEL_KEYS: Record<StageType, string> = {
  backlog: 'kanban.stage.backlog',
  todo: 'kanban.stage.todo',
  dev: 'kanban.stage.dev',
  review: 'kanban.stage.review',
  test: 'kanban.stage.test',
  done: 'kanban.stage.done',
};

export function stageLabelKey(stage: StageType): string {
  return STAGE_LABEL_KEYS[stage];
}

/** 按流程顺序比较两个阶段，可直接喂给 Array.prototype.sort。 */
export function compareStages(left: StageType, right: StageType): number {
  return STAGE_ORDER[left] - STAGE_ORDER[right];
}
