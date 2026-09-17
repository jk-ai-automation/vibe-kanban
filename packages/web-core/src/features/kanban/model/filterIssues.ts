import {
  KANBAN_ASSIGNEE_FILTER_VALUES,
  type KanbanFilterState,
} from '@/shared/stores/useUiPreferencesStore';
import type {
  Issue,
  IssueRelationship,
  IssuePriority,
} from 'shared/remote-types';

export const PRIORITY_ORDER: Record<IssuePriority, number> = {
  urgent: 0,
  high: 1,
  medium: 2,
  low: 3,
};

export type FilterIssuesParams = {
  issues: Issue[];
  /** issue_id -> user_id[]，由调用方预先算好。 */
  assigneesByIssue: Record<string, string[]>;
  /** issue_id -> tag_id[]，由调用方预先算好。 */
  tagsByIssue: Record<string, string[]>;
  issueRelationships: IssueRelationship[];
  issuesById: Map<string, Issue>;
  doneStatusIds: Set<string>;
  filters: KanbanFilterState;
  showSubIssues: boolean;
  hideBlocked: boolean;
  currentUserId: string | null;
};

/**
 * 看板筛选。纯函数：不读 hook、不改入参、同样的入参必然得到同样的结果。
 *
 * 排序不在这里做 —— KanbanContainer 先按状态列分组再在列内排序。
 */
export function filterIssues({
  issues,
  assigneesByIssue,
  tagsByIssue,
  issueRelationships,
  issuesById,
  doneStatusIds,
  filters,
  showSubIssues,
  hideBlocked,
  currentUserId,
}: FilterIssuesParams): Issue[] {
  let result = issues;

  // 子需求：按项目偏好决定是否显示
  if (!showSubIssues) {
    result = result.filter((issue) => issue.parent_issue_id === null);
  }

  // 关键字（标题 + simple_id + issue_number）
  const query = filters.searchQuery.trim().toLowerCase();
  if (query) {
    result = result.filter((issue) => {
      if (issue.title.toLowerCase().includes(query)) {
        return true;
      }

      const simpleId = issue.simple_id.toLowerCase();
      if (simpleId.includes(query)) {
        return true;
      }

      const issueNumber = String(issue.issue_number);
      return issueNumber.includes(query);
    });
  }

  // 优先级（组内 OR）
  if (filters.priorities.length > 0) {
    result = result.filter(
      (issue) =>
        issue.priority !== null && filters.priorities.includes(issue.priority)
    );
  }

  // 负责人（组内 OR）
  if (filters.assigneeIds.length > 0) {
    const includeUnassigned = filters.assigneeIds.includes(
      KANBAN_ASSIGNEE_FILTER_VALUES.UNASSIGNED
    );
    const selectedAssigneeIds = new Set(
      filters.assigneeIds.flatMap((assigneeId) => {
        if (assigneeId === KANBAN_ASSIGNEE_FILTER_VALUES.SELF) {
          return currentUserId ? [currentUserId] : [];
        }
        if (assigneeId === KANBAN_ASSIGNEE_FILTER_VALUES.UNASSIGNED) {
          return [];
        }
        return [assigneeId];
      })
    );

    result = result.filter((issue) => {
      const issueAssigneeIds = assigneesByIssue[issue.id] ?? [];

      // 'unassigned' 特例
      if (includeUnassigned) {
        if (issueAssigneeIds.length === 0) return true;
      }

      // 需求的任一负责人命中即可
      return issueAssigneeIds.some((assigneeId) =>
        selectedAssigneeIds.has(assigneeId)
      );
    });
  }

  // 标签（组内 OR）
  if (filters.tagIds.length > 0) {
    result = result.filter((issue) => {
      const issueTagIds = tagsByIssue[issue.id] ?? [];
      return issueTagIds.some((tagId) => filters.tagIds.includes(tagId));
    });
  }

  // 隐藏被阻塞：过滤掉被未完成需求阻塞的
  if (hideBlocked) {
    result = result.filter((issue) => {
      return !issueRelationships.some((r) => {
        if (r.relationship_type !== 'blocking') return false;
        if (r.related_issue_id !== issue.id) return false;
        const blockingIssue = issuesById.get(r.issue_id);
        if (blockingIssue == null) return false;
        // 阻塞者落在 done 状态列里就算已解除
        return !doneStatusIds.has(blockingIssue.status_id);
      });
    });
  }

  return result;
}

/** 把 issue_assignees 投影成 issue_id -> user_id[]。 */
export function groupAssigneesByIssue(
  issueAssignees: { issue_id: string; user_id: string }[]
): Record<string, string[]> {
  const map: Record<string, string[]> = {};
  for (const ia of issueAssignees) {
    if (!map[ia.issue_id]) {
      map[ia.issue_id] = [];
    }
    map[ia.issue_id].push(ia.user_id);
  }
  return map;
}

/** 把 issue_tags 投影成 issue_id -> tag_id[]。 */
export function groupTagsByIssue(
  issueTags: { issue_id: string; tag_id: string }[]
): Record<string, string[]> {
  const map: Record<string, string[]> = {};
  for (const it of issueTags) {
    if (!map[it.issue_id]) {
      map[it.issue_id] = [];
    }
    map[it.issue_id].push(it.tag_id);
  }
  return map;
}
