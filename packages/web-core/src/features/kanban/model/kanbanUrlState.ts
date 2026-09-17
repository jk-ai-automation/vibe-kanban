import {
  DEFAULT_KANBAN_FILTER_STATE,
  type KanbanFilterState,
  type KanbanSortField,
} from '@/shared/stores/useUiPreferencesStore';
import type { IssuePriority } from 'shared/remote-types';

/**
 * 看板筛选在 URL query 里的形态。
 *
 * 全部是可选字符串：数组用逗号分隔（`?priority=urgent,high`），
 * 这样链接肉眼可读，也不用把 JSON 塞进 query。
 * 只有「和当前视图默认值不同」的条件才会出现，默认状态下 URL 是干净的。
 */
export type KanbanUrlSearch = {
  q?: string;
  priority?: string;
  assignee?: string;
  tag?: string;
  sort?: string;
  dir?: string;
};

/** 本模块会读写的 query key。合并时按这份清单剔除旧值，其余参数原样保留。 */
export const KANBAN_SEARCH_KEYS = [
  'q',
  'priority',
  'assignee',
  'tag',
  'sort',
  'dir',
] as const;

/** 关键字长度上限，防止畸形超长 URL。 */
export const KANBAN_SEARCH_MAX_QUERY_LEN = 200;
/** 单个数组参数的元素个数上限。 */
export const KANBAN_SEARCH_MAX_ARRAY_LEN = 50;
/** 数组参数里单个元素的长度上限（UUID 是 36 位）。 */
export const KANBAN_SEARCH_MAX_ITEM_LEN = 100;

/**
 * 「显式清空」哨兵。
 * personal 视图默认「负责人 = 我」，用户手动清空成 0 个负责人时，
 * 省略参数会被 searchToFilters 补回默认值，所以必须写一个显式值进 URL。
 * 取值不会和 tag / 负责人 id（UUID）或 `__self__` / `unassigned` 撞。
 */
export const KANBAN_SEARCH_NONE = 'none';

const PRIORITIES: IssuePriority[] = ['urgent', 'high', 'medium', 'low'];
const SORT_FIELDS: KanbanSortField[] = [
  'sort_order',
  'priority',
  'created_at',
  'updated_at',
  'title',
];

function joinList(values: string[]): string {
  const cleaned = values
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
    .slice(0, KANBAN_SEARCH_MAX_ARRAY_LEN);
  return cleaned.length > 0 ? cleaned.join(',') : KANBAN_SEARCH_NONE;
}

function splitList(raw: string | undefined): string[] {
  if (typeof raw !== 'string' || raw.length === 0) {
    return [];
  }
  if (raw === KANBAN_SEARCH_NONE) {
    return [];
  }
  return raw
    .split(',')
    .map((value) => value.trim())
    .filter(
      (value) => value.length > 0 && value.length <= KANBAN_SEARCH_MAX_ITEM_LEN
    )
    .slice(0, KANBAN_SEARCH_MAX_ARRAY_LEN);
}

function sameList(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

/**
 * 把筛选状态写成 URL query，只输出「与 defaults 不同」的项。
 * defaults 传当前视图（team / personal）的默认筛选，这样 personal 视图下
 * 「负责人 = 我」这种预设不会污染 URL。
 */
export function filtersToSearch(
  filters: KanbanFilterState,
  defaults: KanbanFilterState = DEFAULT_KANBAN_FILTER_STATE
): KanbanUrlSearch {
  const search: KanbanUrlSearch = {};

  if (filters.searchQuery !== defaults.searchQuery) {
    const q = filters.searchQuery.slice(0, KANBAN_SEARCH_MAX_QUERY_LEN);
    if (q.length > 0) {
      search.q = q;
    }
  }

  if (!sameList(filters.priorities, defaults.priorities)) {
    search.priority = joinList(filters.priorities);
  }

  if (!sameList(filters.assigneeIds, defaults.assigneeIds)) {
    search.assignee = joinList(filters.assigneeIds);
  }

  if (!sameList(filters.tagIds, defaults.tagIds)) {
    search.tag = joinList(filters.tagIds);
  }

  if (filters.sortField !== defaults.sortField) {
    search.sort = filters.sortField;
  }

  if (filters.sortDirection !== defaults.sortDirection) {
    search.dir = filters.sortDirection;
  }

  return search;
}

/**
 * 把 URL query 读回筛选状态。缺失项补 defaults，非法值一律丢弃或回落，
 * **绝不抛错**——URL 是用户可以随手改的，抛错会把整页打死。
 */
export function searchToFilters(
  search: KanbanUrlSearch | null | undefined,
  defaults: KanbanFilterState = DEFAULT_KANBAN_FILTER_STATE
): KanbanFilterState {
  const raw = search ?? {};

  const q = typeof raw.q === 'string' ? raw.q : undefined;
  const searchQuery =
    q === undefined
      ? defaults.searchQuery
      : q.slice(0, KANBAN_SEARCH_MAX_QUERY_LEN);

  const priorityList = splitList(raw.priority).filter(
    (value): value is IssuePriority => (PRIORITIES as string[]).includes(value)
  );
  const priorities =
    raw.priority === undefined ? [...defaults.priorities] : priorityList;

  const assigneeIds =
    raw.assignee === undefined
      ? [...defaults.assigneeIds]
      : splitList(raw.assignee);

  const tagIds =
    raw.tag === undefined ? [...defaults.tagIds] : splitList(raw.tag);

  const sortField = (SORT_FIELDS as string[]).includes(raw.sort ?? '')
    ? (raw.sort as KanbanSortField)
    : defaults.sortField;

  const sortDirection =
    raw.dir === 'asc' || raw.dir === 'desc' ? raw.dir : defaults.sortDirection;

  return {
    searchQuery,
    priorities,
    assigneeIds,
    tagIds,
    sortField,
    sortDirection,
  };
}

/** URL 里是否带了看板筛选参数。用来决定「首次进入时以 URL 为准还是以 store 为准」。 */
export function hasKanbanUrlFilters(
  search: KanbanUrlSearch | null | undefined
): boolean {
  if (!search) return false;
  return KANBAN_SEARCH_KEYS.some((key) => {
    const value = search[key];
    return typeof value === 'string' && value.length > 0;
  });
}

/** 只比较看板筛选相关的 query key，其余参数不参与。 */
export function isSameKanbanSearch(
  left: KanbanUrlSearch | null | undefined,
  right: KanbanUrlSearch | null | undefined
): boolean {
  return KANBAN_SEARCH_KEYS.every((key) => {
    const a = left?.[key];
    const b = right?.[key];
    const normalizedA = typeof a === 'string' && a.length > 0 ? a : undefined;
    const normalizedB = typeof b === 'string' && b.length > 0 ? b : undefined;
    return normalizedA === normalizedB;
  });
}

/**
 * 把新的看板筛选参数合并进现有 query：先剔掉旧的看板 key，
 * 再塞进新的，**其它路由自己的 query 参数原样保留**。
 */
export function mergeKanbanSearch(
  previous: Record<string, unknown> | null | undefined,
  next: KanbanUrlSearch
): Record<string, unknown> {
  const merged: Record<string, unknown> = { ...(previous ?? {}) };
  for (const key of KANBAN_SEARCH_KEYS) {
    delete merged[key];
  }
  for (const [key, value] of Object.entries(next)) {
    if (typeof value === 'string' && value.length > 0) {
      merged[key] = value;
    }
  }
  return merged;
}
