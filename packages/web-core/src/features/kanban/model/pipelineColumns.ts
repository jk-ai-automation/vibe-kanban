import type { StageType } from './stageType';

/**
 * 个人版新建项目时自动创建的列名（`crates/db/src/models/local_project.rs:14-21`
 * 的 `DEFAULT_STATUSES`）。列名等于它，说明用户没改过名，可以换成流水线列名。
 */
export const LEGACY_DEFAULT_STATUS_NAMES: Record<StageType, string> = {
  backlog: '待规划',
  todo: '待开发',
  dev: '开发中',
  review: '待评审',
  test: '测试中',
  done: '已完成',
};

const COLUMN_TITLE_KEYS: Record<StageType, string> = {
  backlog: 'pipeline.column.backlog',
  todo: 'pipeline.column.todo',
  dev: 'pipeline.column.dev',
  review: 'pipeline.column.review',
  test: 'pipeline.column.test',
  done: 'pipeline.column.done',
};

/** 个人版列头标题：默认列名 → 流水线列名 key；用户改过名 → null（显示原名）。 */
export function pipelineColumnTitleKey(
  statusName: string,
  stage: StageType
): string | null {
  return statusName.trim() === LEGACY_DEFAULT_STATUS_NAMES[stage]
    ? COLUMN_TITLE_KEYS[stage]
    : null;
}

const COLUMN_EMPTY_KEYS: Record<StageType, string> = {
  backlog: 'pipeline.columnEmpty.backlog',
  todo: 'pipeline.columnEmpty.todo',
  dev: 'pipeline.columnEmpty.dev',
  review: 'pipeline.columnEmpty.review',
  test: 'pipeline.columnEmpty.test',
  done: 'pipeline.columnEmpty.done',
};

/** 空列写明「为什么空」（设计文档 §8.3）。 */
export function pipelineColumnEmptyKey(stage: StageType): string {
  return COLUMN_EMPTY_KEYS[stage];
}

const COLUMN_HINT_KEYS: Record<StageType, string | null> = {
  backlog: 'pipeline.column.humanGate',
  todo: 'pipeline.column.humanGate',
  dev: null,
  review: null,
  test: null,
  done: 'pipeline.column.auto',
};

/** 列名旁的小提示（草图：「需求 · 人工确认」「交付 · 自动」）。 */
export function pipelineColumnHintKey(stage: StageType): string | null {
  return COLUMN_HINT_KEYS[stage];
}
