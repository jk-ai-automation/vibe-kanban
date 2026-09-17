import { useMemo } from 'react';
import type { KanbanFilterState } from '@/shared/stores/useUiPreferencesStore';
import type {
  Issue,
  IssueAssignee,
  IssueRelationship,
  IssueTag,
} from 'shared/remote-types';
import {
  filterIssues,
  groupAssigneesByIssue,
  groupTagsByIssue,
} from '../filterIssues';

export { PRIORITY_ORDER } from '../filterIssues';

type UseKanbanFiltersParams = {
  issues: Issue[];
  issueAssignees: IssueAssignee[];
  issueTags: IssueTag[];
  issueRelationships: IssueRelationship[];
  issuesById: Map<string, Issue>;
  doneStatusIds: Set<string>;
  filters: KanbanFilterState;
  showSubIssues: boolean;
  hideBlocked: boolean;
  currentUserId: string | null;
};

type UseKanbanFiltersResult = {
  filteredIssues: Issue[];
};

/**
 * 看板筛选的 React 外壳：只负责两张查找表的 memo 与依赖数组，
 * 真正的筛选逻辑在 `../filterIssues.ts` 的纯函数里（可单测）。
 */
export function useKanbanFilters({
  issues,
  issueAssignees,
  issueTags,
  issueRelationships,
  issuesById,
  doneStatusIds,
  filters,
  showSubIssues,
  hideBlocked,
  currentUserId,
}: UseKanbanFiltersParams): UseKanbanFiltersResult {
  const assigneesByIssue = useMemo(
    () => groupAssigneesByIssue(issueAssignees),
    [issueAssignees]
  );

  const tagsByIssue = useMemo(() => groupTagsByIssue(issueTags), [issueTags]);

  const filteredIssues = useMemo(
    () =>
      filterIssues({
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
      }),
    [
      issues,
      filters,
      assigneesByIssue,
      tagsByIssue,
      showSubIssues,
      hideBlocked,
      issueRelationships,
      issuesById,
      doneStatusIds,
      currentUserId,
    ]
  );

  return {
    filteredIssues,
  };
}
