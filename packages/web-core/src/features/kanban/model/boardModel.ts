import type { KanbanFilterState } from '@/shared/stores/useUiPreferencesStore';
import type { Issue, ProjectStatus } from 'shared/remote-types';
import { PRIORITY_ORDER } from './filterIssues';
import {
  readStageType,
  type StageType,
  type StatusWithStage,
} from './stageType';

/** 看板上的一列：状态列本身 + 它对应的流程阶段 + 列内卡片顺序。 */
export type BoardColumn = {
  status: ProjectStatus;
  /** 流程阶段。团队版（remote）缺 stage_type 时统一是 'todo'。 */
  stage: StageType;
  /** 该列的需求 id，已按当前排序规则排好。 */
  issueIds: string[];
  /** 列内卡片数，列头计数与 WIP 提示用。 */
  count: number;
};

/** 按 sort_order 升序排列状态列。不改入参数组。 */
export function sortStatusesByOrder<T extends { sort_order: number }>(
  statuses: T[]
): T[] {
  return [...statuses].sort((a, b) => a.sort_order - b.sort_order);
}

/**
 * 把筛选后的需求按状态列分组，并在列内按用户选择的规则排序。
 *
 * 返回值以 status.id 为 key，**每个传入的状态列都会有一条记录**（可能是空数组），
 * 这样列表视图与看板视图都能直接查。不改入参数组。
 */
export function groupIssueIdsByStatus(
  statuses: { id: string }[],
  issues: Issue[],
  sortField: KanbanFilterState['sortField'],
  sortDirection: KanbanFilterState['sortDirection']
): Record<string, string[]> {
  const grouped: Record<string, string[]> = {};

  for (const status of statuses) {
    const statusIssues = issues.filter((i) => i.status_id === status.id);

    statusIssues.sort((a, b) => {
      let comparison = 0;
      switch (sortField) {
        case 'priority':
          comparison =
            (a.priority ? PRIORITY_ORDER[a.priority] : Infinity) -
            (b.priority ? PRIORITY_ORDER[b.priority] : Infinity);
          break;
        case 'created_at':
          comparison =
            new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
          break;
        case 'updated_at':
          comparison =
            new Date(a.updated_at).getTime() - new Date(b.updated_at).getTime();
          break;
        case 'title':
          comparison = a.title.localeCompare(b.title);
          break;
        case 'sort_order':
        default:
          comparison = a.sort_order - b.sort_order;
      }
      return sortDirection === 'desc' ? -comparison : comparison;
    });

    grouped[status.id] = statusIssues.map((i) => i.id);
  }

  return grouped;
}

/**
 * 把「可见状态列 + 分组结果」拼成看板要渲染的列。
 * 传进来的状态列顺序就是渲染顺序（调用方已经排好并过滤掉隐藏列）。
 */
export function buildBoardColumns(
  visibleStatuses: StatusWithStage[],
  itemsByStatusId: Record<string, string[]>
): BoardColumn[] {
  return visibleStatuses.map((status) => {
    const issueIds = itemsByStatusId[status.id] ?? [];
    return {
      status,
      stage: readStageType(status),
      issueIds,
      count: issueIds.length,
    };
  });
}

/** 从所有状态列里挑出看板要展示的列：非隐藏，按 sort_order 升序。 */
export function selectVisibleStatuses<
  T extends { sort_order: number; hidden: boolean },
>(statuses: T[]): T[] {
  return sortStatusesByOrder(statuses).filter((s) => !s.hidden);
}
