/**
 * 看板列头与空态的判定逻辑。
 *
 * 全是纯函数：不碰 DOM、不读 store，方便在 `environment: 'node'` 的 Vitest 下直接测。
 * （渲染方只负责把返回值映射成 class 与文案。）
 */

/** 设计文档 §7.2：单列超过 5 张卡片就变色提示。**只提示不阻止。** */
export const DEFAULT_WIP_LIMIT = 5;

export type WipState = 'ok' | 'over';

/**
 * 列内卡片数相对 WIP 上限的状态。
 *
 * - 恰好等于上限算 `'ok'`（「超过 5 张」才提示）。
 * - `limit <= 0` 视为关闭提示，恒为 `'ok'`。
 * - 非有限数（NaN / Infinity）一律当成关闭提示，绝不抛错。
 */
export function wipState(
  count: number,
  limit: number = DEFAULT_WIP_LIMIT
): WipState {
  if (!Number.isFinite(count) || !Number.isFinite(limit)) {
    return 'ok';
  }
  if (limit <= 0) {
    return 'ok';
  }
  return count > limit ? 'over' : 'ok';
}

/**
 * 空列要显示哪种引导。
 *
 * - `'none'`：列里有卡片，不显示空态。
 * - `'filtered'`：列为空**且**当前有生效的筛选 → 提示「没有符合筛选的需求」+ 清除筛选。
 * - `'empty'`：列为空且没有筛选 → 提示「新建需求 / 从需求创建工作区」。
 */
export type ColumnEmptyStateKind = 'none' | 'empty' | 'filtered';

export function columnEmptyStateKind(
  count: number,
  hasActiveFilters: boolean
): ColumnEmptyStateKind {
  if (!Number.isFinite(count) || count > 0) {
    return 'none';
  }
  return hasActiveFilters ? 'filtered' : 'empty';
}
