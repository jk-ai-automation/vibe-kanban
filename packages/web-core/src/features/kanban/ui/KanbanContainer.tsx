import {
  useMemo,
  useCallback,
  useState,
  useEffect,
  useRef,
  type MouseEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from '@tanstack/react-router';
import { useProjectContext } from '@/shared/hooks/useProjectContext';
import { useOrgContext } from '@/shared/hooks/useOrgContext';
import { useWorkspaceContext } from '@/shared/hooks/useWorkspaceContext';
import { useActions } from '@/shared/hooks/useActions';
import { useAuth } from '@/shared/hooks/auth/useAuth';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { useIsMobile } from '@/shared/hooks/useIsMobile';
import { cn } from '@/shared/lib/utils';
import { useCurrentKanbanRouteState } from '@/shared/hooks/useCurrentKanbanRouteState';
import {
  useUiPreferencesStore,
  resolveKanbanProjectState,
  KANBAN_ASSIGNEE_FILTER_VALUES,
  KANBAN_PROJECT_VIEW_IDS,
  type KanbanFilterState,
  type KanbanSortField,
} from '@/shared/stores/useUiPreferencesStore';
import { useKanbanFilters } from '../model/hooks/useKanbanFilters';
import {
  buildBoardColumns,
  groupIssueIdsByStatus,
  selectVisibleStatuses,
  sortStatusesByOrder,
} from '../model/boardModel';
import {
  filtersToSearch,
  hasKanbanUrlFilters,
  isSameKanbanSearch,
  mergeKanbanSearch,
  searchToFilters,
  type KanbanUrlSearch,
} from '../model/kanbanUrlState';
import { KanbanBoardView } from './KanbanBoardView';
import {
  bulkUpdateIssues,
  type BulkUpdateIssueItem,
} from '@/shared/lib/remoteApi';
import { DotsThreeIcon } from '@phosphor-icons/react';
import { Actions } from '@/shared/actions';
import {
  buildKanbanIssueComposerKey,
  closeKanbanIssueComposer,
  openKanbanIssueComposer,
  type ProjectIssueCreateOptions,
  useKanbanIssueComposer,
} from '@/shared/stores/useKanbanIssueComposerStore';
import type { OrganizationMemberWithProfile } from 'shared/types';
import {
  KanbanProvider,
  type DropResult,
} from '@vibe/ui/components/KanbanBoard';
import type {
  WorkspaceWithStats,
  WorkspacePr,
} from '@vibe/ui/components/IssueWorkspaceCard';
import { resolveRelationshipsForIssue } from '@/shared/lib/resolveRelationships';
import { KanbanFilterBar } from '@vibe/ui/components/KanbanFilterBar';
import { ViewNavTabs } from '@vibe/ui/components/ViewNavTabs';
import { IssueListView } from '@vibe/ui/components/IssueListView';
import { CommandBarDialog } from '@/shared/dialogs/command-bar/CommandBarDialog';
import { KanbanFiltersDialog } from '@/shared/dialogs/kanban/KanbanFiltersDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@vibe/ui/components/Dropdown';
import type { IssuePriority } from 'shared/remote-types';
import { useIssueMultiSelect } from '@/shared/hooks/useIssueMultiSelect';
import { useIssueSelectionStore } from '@/shared/stores/useIssueSelectionStore';
import { BulkActionBarContainer } from './BulkActionBarContainer';

const areStringSetsEqual = (left: string[], right: string[]): boolean => {
  if (left.length !== right.length) {
    return false;
  }

  const rightSet = new Set(right);
  return left.every((value) => rightSet.has(value));
};

const areKanbanFiltersEqual = (
  left: KanbanFilterState,
  right: KanbanFilterState
): boolean => {
  if (left.searchQuery.trim() !== right.searchQuery.trim()) {
    return false;
  }

  if (!areStringSetsEqual(left.priorities, right.priorities)) {
    return false;
  }

  if (!areStringSetsEqual(left.assigneeIds, right.assigneeIds)) {
    return false;
  }

  if (!areStringSetsEqual(left.tagIds, right.tagIds)) {
    return false;
  }

  return (
    left.sortField === right.sortField &&
    left.sortDirection === right.sortDirection
  );
};

/**
 * 只改 search、不改 path 的导航。
 *
 * web-core 是 local-web 与 remote-web 共用的库，没有注册具体的路由树，
 * `useNavigate()` 在这里拿到的是 `AnyRouter` 版本，search 的 reducer 被推导成
 * `never`，无法直接用。这里收窄成一个最小签名。
 * （`useAppNavigation` 的 goTo* 方法全都不接受 search 参数；
 * 用当前路由的点号写法做 to 又被 scripts/check-legacy-frontend-paths.sh 禁用，
 * 所以这里只传 search，不传 to。）
 */
type SearchOnlyNavigate = (options: {
  search: (previous: Record<string, unknown>) => Record<string, unknown>;
  replace?: boolean;
}) => void;

function LoadingState() {
  const { t } = useTranslation('common');
  return (
    <div className="flex items-center justify-center h-full">
      <p className="text-low">{t('states.loading')}</p>
    </div>
  );
}

/**
 * KanbanContainer displays the kanban board using data from ProjectContext and OrgContext.
 * Must be rendered within both OrgProvider and ProjectProvider.
 */
export function KanbanContainer() {
  const isMobile = useIsMobile();
  const { t } = useTranslation('common');
  const appNavigation = useAppNavigation();
  const routeState = useCurrentKanbanRouteState();

  // Get data from contexts (set up by WorkspacesLayout)
  const {
    projectId,
    issues,
    statuses,
    tags,
    issueAssignees,
    issueTags,
    issueRelationships,
    getTagObjectsForIssue,
    getTagsForIssue,
    getPullRequestsForIssue,
    getWorkspacesForIssue,
    getRelationshipsForIssue,
    issuesById,
    insertIssueTag,
    removeIssueTag,
    insertTag,
    pullRequests,
    isLoading: projectLoading,
  } = useProjectContext();

  const {
    projects,
    membersWithProfilesById,
    isLoading: orgLoading,
  } = useOrgContext();
  const { activeWorkspaces } = useWorkspaceContext();
  const { userId } = useAuth();

  // Get project name by finding the project matching current projectId
  const projectName = projects.find((p) => p.id === projectId)?.name ?? '';

  const selectedKanbanIssueId = routeState.issueId;
  const issueComposerKey = useMemo(
    () => buildKanbanIssueComposerKey(routeState.hostId, projectId),
    [routeState.hostId, projectId]
  );
  const issueComposer = useKanbanIssueComposer(issueComposerKey);
  const isIssueComposerOpen = issueComposer !== null;
  const openIssue = useCallback(
    (issueId: string) => {
      if (isIssueComposerOpen) {
        closeKanbanIssueComposer(issueComposerKey);
      }

      appNavigation.goToProjectIssue(projectId, issueId);
    },
    [isIssueComposerOpen, issueComposerKey, appNavigation, projectId]
  );
  const openIssueWorkspace = useCallback(
    (issueId: string, workspaceAttemptId: string) => {
      appNavigation.goToProjectIssueWorkspace(
        projectId,
        issueId,
        workspaceAttemptId
      );
    },
    [appNavigation, projectId]
  );
  const startCreate = useCallback(
    (options?: ProjectIssueCreateOptions) => {
      openKanbanIssueComposer(issueComposerKey, options);
    },
    [issueComposerKey]
  );

  // Get setter and executor from ActionsContext
  const {
    setDefaultCreateStatusId,
    executeAction,
    openPrioritySelection,
    openAssigneeSelection,
  } = useActions();
  const openProjectsGuide = useCallback(() => {
    executeAction(Actions.ProjectsGuide);
  }, [executeAction]);

  const projectViewSelection = useUiPreferencesStore(
    (s) => s.kanbanProjectViewSelections[projectId]
  );
  const projectViewPreferencesById = useUiPreferencesStore(
    (s) => s.kanbanProjectViewPreferences[projectId]
  );
  const setKanbanProjectView = useUiPreferencesStore(
    (s) => s.setKanbanProjectView
  );
  const setKanbanProjectViewFilters = useUiPreferencesStore(
    (s) => s.setKanbanProjectViewFilters
  );
  const setKanbanProjectViewShowSubIssues = useUiPreferencesStore(
    (s) => s.setKanbanProjectViewShowSubIssues
  );
  const setKanbanProjectViewShowWorkspaces = useUiPreferencesStore(
    (s) => s.setKanbanProjectViewShowWorkspaces
  );
  const setKanbanProjectViewHideBlocked = useUiPreferencesStore(
    (s) => s.setKanbanProjectViewHideBlocked
  );
  const clearKanbanProjectViewPreferences = useUiPreferencesStore(
    (s) => s.clearKanbanProjectViewPreferences
  );
  const resolvedProjectState = useMemo(
    () => resolveKanbanProjectState(projectViewSelection),
    [projectViewSelection]
  );
  const {
    activeViewId,
    filters: defaultKanbanFilters,
    showSubIssues: defaultShowSubIssues,
    showWorkspaces: defaultShowWorkspaces,
    hideBlocked: defaultHideBlocked,
  } = resolvedProjectState;
  const projectViewPreferences = projectViewPreferencesById?.[activeViewId];
  const kanbanFilters = projectViewPreferences?.filters ?? defaultKanbanFilters;
  const showSubIssues =
    projectViewPreferences?.showSubIssues ?? defaultShowSubIssues;
  const showWorkspaces =
    projectViewPreferences?.showWorkspaces ?? defaultShowWorkspaces;
  const hideBlocked = projectViewPreferences?.hideBlocked ?? defaultHideBlocked;

  const hasActiveFilters = useMemo(
    () =>
      !areKanbanFiltersEqual(kanbanFilters, defaultKanbanFilters) ||
      showSubIssues !== defaultShowSubIssues ||
      showWorkspaces !== defaultShowWorkspaces ||
      hideBlocked !== defaultHideBlocked,
    [
      kanbanFilters,
      defaultKanbanFilters,
      showSubIssues,
      defaultShowSubIssues,
      showWorkspaces,
      defaultShowWorkspaces,
      hideBlocked,
      defaultHideBlocked,
    ]
  );
  const shouldAnimateCreateButton = issues.length === 0;

  // Compute resolved status IDs for the blocked filter.
  // A blocking issue is considered resolved when it's in:
  // - The last visible status (rightmost kanban column, e.g. "Done")
  // - Any hidden status (terminal states like "Cancelled" that don't appear as columns)
  const doneStatusIds = useMemo(() => {
    const ids = new Set<string>();
    for (const s of statuses) {
      if (s.hidden) ids.add(s.id);
    }
    const sorted = statuses
      .filter((s) => !s.hidden)
      .sort((a, b) => a.sort_order - b.sort_order);
    const lastVisible = sorted[sorted.length - 1];
    if (lastVisible) ids.add(lastVisible.id);
    return ids;
  }, [statuses]);

  const { filteredIssues } = useKanbanFilters({
    issues,
    issueAssignees,
    issueTags,
    issueRelationships,
    issuesById,
    doneStatusIds,
    filters: kanbanFilters,
    showSubIssues,
    hideBlocked,
    currentUserId: userId,
  });

  const setKanbanSearchQuery = useCallback(
    (searchQuery: string) => {
      setKanbanProjectViewFilters(projectId, activeViewId, {
        ...kanbanFilters,
        searchQuery,
      });
    },
    [activeViewId, kanbanFilters, projectId, setKanbanProjectViewFilters]
  );

  const setKanbanPriorities = useCallback(
    (priorities: IssuePriority[]) => {
      setKanbanProjectViewFilters(projectId, activeViewId, {
        ...kanbanFilters,
        priorities,
      });
    },
    [activeViewId, kanbanFilters, projectId, setKanbanProjectViewFilters]
  );

  const setKanbanAssignees = useCallback(
    (assigneeIds: string[]) => {
      setKanbanProjectViewFilters(projectId, activeViewId, {
        ...kanbanFilters,
        assigneeIds,
      });
    },
    [activeViewId, kanbanFilters, projectId, setKanbanProjectViewFilters]
  );

  const setKanbanTags = useCallback(
    (tagIds: string[]) => {
      setKanbanProjectViewFilters(projectId, activeViewId, {
        ...kanbanFilters,
        tagIds,
      });
    },
    [activeViewId, kanbanFilters, projectId, setKanbanProjectViewFilters]
  );

  const setKanbanSort = useCallback(
    (sortField: KanbanSortField, sortDirection: 'asc' | 'desc') => {
      setKanbanProjectViewFilters(projectId, activeViewId, {
        ...kanbanFilters,
        sortField,
        sortDirection,
      });
    },
    [activeViewId, kanbanFilters, projectId, setKanbanProjectViewFilters]
  );

  const setShowSubIssues = useCallback(
    (show: boolean) => {
      setKanbanProjectViewShowSubIssues(projectId, activeViewId, show);
    },
    [activeViewId, projectId, setKanbanProjectViewShowSubIssues]
  );

  const setShowWorkspaces = useCallback(
    (show: boolean) => {
      setKanbanProjectViewShowWorkspaces(projectId, activeViewId, show);
    },
    [activeViewId, projectId, setKanbanProjectViewShowWorkspaces]
  );

  const setHideBlocked = useCallback(
    (hide: boolean) => {
      setKanbanProjectViewHideBlocked(projectId, activeViewId, hide);
    },
    [activeViewId, projectId, setKanbanProjectViewHideBlocked]
  );

  const clearKanbanFilters = useCallback(() => {
    clearKanbanProjectViewPreferences(projectId, activeViewId);
  }, [activeViewId, clearKanbanProjectViewPreferences, projectId]);

  // ---- 筛选与 URL 的双向同步 ----
  // 优先级：URL > store（zustand + 服务端 scratch）。
  // 首次进入某个「项目 + 视图」时，URL 带了筛选参数就以 URL 为准（链接可分享、可刷新）；
  // URL 干净则把 store 里记住的筛选写回 URL。之后界面上改筛选一律 store -> URL（replace，
  // 不往历史里灌记录）；URL 被外部改动（前进/后退/改地址栏）则反向拉回 store。
  const location = useLocation();
  const navigate = useNavigate() as unknown as SearchOnlyNavigate;
  const urlSearch = location.search as KanbanUrlSearch;
  const urlSyncKey = `${projectId}::${activeViewId}`;
  const urlSyncedKeyRef = useRef<string | null>(null);
  const lastPushedSearchRef = useRef<KanbanUrlSearch | null>(null);

  useEffect(() => {
    const fromStore = filtersToSearch(kanbanFilters, defaultKanbanFilters);

    // 两边已经一致：这是打断双向回环的唯一出口。
    if (isSameKanbanSearch(fromStore, urlSearch)) {
      urlSyncedKeyRef.current = urlSyncKey;
      lastPushedSearchRef.current = fromStore;
      return;
    }

    const isFirstPass = urlSyncedKeyRef.current !== urlSyncKey;
    const urlChangedElsewhere =
      lastPushedSearchRef.current !== null &&
      !isSameKanbanSearch(lastPushedSearchRef.current, urlSearch);

    if (
      (isFirstPass && hasKanbanUrlFilters(urlSearch)) ||
      urlChangedElsewhere
    ) {
      urlSyncedKeyRef.current = urlSyncKey;
      lastPushedSearchRef.current = urlSearch;
      setKanbanProjectViewFilters(
        projectId,
        activeViewId,
        searchToFilters(urlSearch, defaultKanbanFilters)
      );
      return;
    }

    urlSyncedKeyRef.current = urlSyncKey;
    lastPushedSearchRef.current = fromStore;
    navigate({
      search: (prev: Record<string, unknown>) =>
        mergeKanbanSearch(prev, fromStore),
      replace: true,
    });
  }, [
    urlSyncKey,
    urlSearch,
    kanbanFilters,
    defaultKanbanFilters,
    projectId,
    activeViewId,
    setKanbanProjectViewFilters,
    navigate,
  ]);

  const handleKanbanProjectViewChange = useCallback(
    (viewId: string) => {
      // 视图切换由界面发起：先把同步标记推到新视图，避免上一视图残留在 URL 里的
      // 筛选参数被当成「首次进入」而倒灌进新视图。
      urlSyncedKeyRef.current = `${projectId}::${viewId}`;
      setKanbanProjectView(projectId, viewId);
    },
    [projectId, setKanbanProjectView]
  );
  const kanbanViewMode = useUiPreferencesStore((s) => s.kanbanViewMode);
  const listViewStatusFilter = useUiPreferencesStore(
    (s) => s.listViewStatusFilter
  );
  const setKanbanViewMode = useUiPreferencesStore((s) => s.setKanbanViewMode);
  const setListViewStatusFilter = useUiPreferencesStore(
    (s) => s.setListViewStatusFilter
  );
  // Reset view mode when navigating projects
  const prevProjectIdRef = useRef<string | null>(null);

  // Track when drag-drop sync is in progress to prevent flicker
  const isSyncingRef = useRef(false);

  useEffect(() => {
    if (
      prevProjectIdRef.current !== null &&
      prevProjectIdRef.current !== projectId
    ) {
      setKanbanViewMode('kanban');
      setListViewStatusFilter(null);
    }

    prevProjectIdRef.current = projectId;
  }, [projectId, setKanbanViewMode, setListViewStatusFilter]);

  // Sort all statuses for display settings
  const sortedStatuses = useMemo(
    () => sortStatusesByOrder(statuses),
    [statuses]
  );

  // Filter statuses: visible (non-hidden) for kanban, hidden for tabs
  const visibleStatuses = useMemo(
    () => selectVisibleStatuses(statuses),
    [statuses]
  );

  // Map status ID to 1-based column index for sort_order calculation
  const statusColumnIndexMap = useMemo(() => {
    const map = new Map<string, number>();
    visibleStatuses.forEach((status, index) => {
      map.set(status.id, index + 1);
    });
    return map;
  }, [visibleStatuses]);

  const hiddenStatuses = useMemo(
    () => sortedStatuses.filter((s) => s.hidden),
    [sortedStatuses]
  );

  const defaultCreateStatusId = useMemo(() => {
    if (kanbanViewMode === 'kanban') {
      return visibleStatuses[0]?.id;
    }
    if (listViewStatusFilter) {
      return listViewStatusFilter;
    }
    return sortedStatuses[0]?.id;
  }, [kanbanViewMode, visibleStatuses, listViewStatusFilter, sortedStatuses]);

  // Update default create status for command bar based on current tab
  useEffect(() => {
    setDefaultCreateStatusId(defaultCreateStatusId);
  }, [defaultCreateStatusId, setDefaultCreateStatusId]);

  const createAssigneeIds = useMemo(() => {
    const assigneeIds = new Set<string>();

    for (const assigneeId of kanbanFilters.assigneeIds) {
      if (assigneeId === KANBAN_ASSIGNEE_FILTER_VALUES.UNASSIGNED) {
        continue;
      }

      if (assigneeId === KANBAN_ASSIGNEE_FILTER_VALUES.SELF) {
        if (userId) {
          assigneeIds.add(userId);
        }
        continue;
      }

      assigneeIds.add(assigneeId);
    }

    return [...assigneeIds];
  }, [kanbanFilters.assigneeIds, userId]);

  // Get statuses to display in list view (all or filtered to one)
  const listViewStatuses = useMemo(() => {
    if (listViewStatusFilter) {
      return sortedStatuses.filter((s) => s.id === listViewStatusFilter);
    }
    return sortedStatuses;
  }, [sortedStatuses, listViewStatusFilter]);

  // Track items as arrays of IDs grouped by status
  const [items, setItems] = useState<Record<string, string[]>>({});
  const [isFiltersDialogOpen, setIsFiltersDialogOpen] = useState(false);

  // Sync items from filtered issues when they change
  useEffect(() => {
    // Skip rebuild during drag-drop sync to prevent flicker
    if (isSyncingRef.current) {
      return;
    }

    const { sortField, sortDirection } = kanbanFilters;
    setItems(
      groupIssueIdsByStatus(statuses, filteredIssues, sortField, sortDirection)
    );
  }, [filteredIssues, statuses, kanbanFilters]);

  // 看板要渲染的列（含流程阶段）。列表视图仍然直接用 items。
  const boardColumns = useMemo(
    () => buildBoardColumns(visibleStatuses, items),
    [visibleStatuses, items]
  );

  // Create a lookup map for issue data
  const issueMap = useMemo(() => {
    const map: Record<string, (typeof issues)[0]> = {};
    for (const issue of issues) {
      map[issue.id] = issue;
    }
    return map;
  }, [issues]);

  // Create a lookup map for issue assignees (issue_id -> OrganizationMemberWithProfile[])
  const issueAssigneesMap = useMemo(() => {
    const map: Record<string, OrganizationMemberWithProfile[]> = {};
    for (const assignee of issueAssignees) {
      const member = membersWithProfilesById.get(assignee.user_id);
      if (member) {
        if (!map[assignee.issue_id]) {
          map[assignee.issue_id] = [];
        }
        map[assignee.issue_id].push(member);
      }
    }
    return map;
  }, [issueAssignees, membersWithProfilesById]);

  const membersWithProfiles = useMemo(
    () => [...membersWithProfilesById.values()],
    [membersWithProfilesById]
  );

  const localWorkspacesById = useMemo(() => {
    const map = new Map<string, (typeof activeWorkspaces)[number]>();

    for (const workspace of activeWorkspaces) {
      map.set(workspace.id, workspace);
    }

    return map;
  }, [activeWorkspaces]);

  const prsByWorkspaceId = useMemo(() => {
    const map = new Map<string, WorkspacePr[]>();

    for (const pr of pullRequests) {
      if (!pr.workspace_id) continue;

      const prs = map.get(pr.workspace_id) ?? [];
      prs.push({
        number: pr.number,
        url: pr.url,
        status: pr.status as 'open' | 'merged' | 'closed',
      });
      map.set(pr.workspace_id, prs);
    }

    return map;
  }, [pullRequests]);

  const workspacesByIssueId = useMemo(() => {
    if (!showWorkspaces) {
      return new Map<string, WorkspaceWithStats[]>();
    }

    const map = new Map<string, WorkspaceWithStats[]>();

    for (const issue of issues) {
      const nonArchivedWorkspaces = getWorkspacesForIssue(issue.id)
        .filter(
          (workspace) =>
            !workspace.archived &&
            !!workspace.local_workspace_id &&
            localWorkspacesById.has(workspace.local_workspace_id)
        )
        .map((workspace) => {
          const localWorkspace = localWorkspacesById.get(
            workspace.local_workspace_id!
          );

          return {
            id: workspace.id,
            localWorkspaceId: workspace.local_workspace_id,
            name: workspace.name,
            archived: workspace.archived,
            filesChanged: workspace.files_changed ?? 0,
            linesAdded: workspace.lines_added ?? 0,
            linesRemoved: workspace.lines_removed ?? 0,
            prs: prsByWorkspaceId.get(workspace.id) ?? [],
            owner: membersWithProfilesById.get(workspace.owner_user_id) ?? null,
            updatedAt: workspace.updated_at,
            isOwnedByCurrentUser: workspace.owner_user_id === userId,
            isRunning: localWorkspace?.isRunning,
            hasPendingApproval: localWorkspace?.hasPendingApproval,
            hasRunningDevServer: localWorkspace?.hasRunningDevServer,
            hasUnseenActivity: localWorkspace?.hasUnseenActivity,
            latestProcessCompletedAt: localWorkspace?.latestProcessCompletedAt,
            latestProcessStatus: localWorkspace?.latestProcessStatus,
          };
        });

      if (nonArchivedWorkspaces.length > 0) {
        map.set(issue.id, nonArchivedWorkspaces);
      }
    }

    return map;
  }, [
    showWorkspaces,
    issues,
    getWorkspacesForIssue,
    localWorkspacesById,
    prsByWorkspaceId,
    membersWithProfilesById,
    userId,
  ]);

  // Calculate sort_order based on column index and issue position
  // Formula: 1000 * [COLUMN_INDEX] + [ISSUE_INDEX] (both 1-based)
  const calculateSortOrder = useCallback(
    (statusId: string, issueIndex: number): number => {
      const columnIndex = statusColumnIndexMap.get(statusId) ?? 1;
      return 1000 * columnIndex + (issueIndex + 1);
    },
    [statusColumnIndexMap]
  );

  // Simple onDragEnd handler - the library handles all visual movement
  const handleDragEnd = useCallback(
    (result: DropResult) => {
      const { source, destination } = result;

      // Dropped outside a valid droppable
      if (!destination) return;

      // No movement
      if (
        source.droppableId === destination.droppableId &&
        source.index === destination.index
      ) {
        return;
      }

      const isManualSort = kanbanFilters.sortField === 'sort_order';

      // Block within-column reordering when not in manual sort mode
      // (cross-column moves are always allowed for status changes)
      if (source.droppableId === destination.droppableId && !isManualSort) {
        return;
      }

      const sourceId = source.droppableId;
      const destId = destination.droppableId;
      const isCrossColumn = sourceId !== destId;

      // Update local state and capture new items for bulk update
      let newItems: Record<string, string[]> = {};
      setItems((prev) => {
        const sourceItems = [...(prev[sourceId] ?? [])];
        const [moved] = sourceItems.splice(source.index, 1);

        if (!isCrossColumn) {
          // Within-column reorder
          sourceItems.splice(destination.index, 0, moved);
          newItems = { ...prev, [sourceId]: sourceItems };
        } else {
          // Cross-column move
          const destItems = [...(prev[destId] ?? [])];
          destItems.splice(destination.index, 0, moved);
          newItems = {
            ...prev,
            [sourceId]: sourceItems,
            [destId]: destItems,
          };
        }
        return newItems;
      });

      // Build bulk updates for all issues in affected columns
      const updates: BulkUpdateIssueItem[] = [];

      // Always update destination column
      const destIssueIds = newItems[destId] ?? [];
      destIssueIds.forEach((issueId, index) => {
        updates.push({
          id: issueId,
          changes: {
            status_id: destId,
            sort_order: calculateSortOrder(destId, index),
          },
        });
      });

      // Update source column if cross-column move
      if (isCrossColumn) {
        const sourceIssueIds = newItems[sourceId] ?? [];
        sourceIssueIds.forEach((issueId, index) => {
          updates.push({
            id: issueId,
            changes: {
              sort_order: calculateSortOrder(sourceId, index),
            },
          });
        });
      }

      // Perform bulk update
      isSyncingRef.current = true;
      bulkUpdateIssues(updates)
        .catch((err) => {
          console.error('Failed to bulk update sort order:', err);
        })
        .finally(() => {
          // Delay clearing flag to let Electric sync complete
          setTimeout(() => {
            isSyncingRef.current = false;
          }, 500);
        });
    },
    [kanbanFilters.sortField, calculateSortOrder]
  );

  // Multi-select support
  const {
    selectedIssueIds,
    isMultiSelectActive,
    handleIssueClick,
    handleCheckboxChange,
    clearSelection,
  } = useIssueMultiSelect();
  const setOrderedIssueIds = useIssueSelectionStore(
    (s) => s.setOrderedIssueIds
  );
  const setAnchor = useIssueSelectionStore((s) => s.setAnchor);

  // Compute ordered issue IDs for range selection
  const orderedIssueIds = useMemo(() => {
    const statusOrder =
      kanbanViewMode === 'kanban' ? visibleStatuses : listViewStatuses;
    return statusOrder.flatMap((status) => items[status.id] ?? []);
  }, [kanbanViewMode, visibleStatuses, listViewStatuses, items]);

  // Keep the store's ordered IDs in sync
  useEffect(() => {
    setOrderedIssueIds(orderedIssueIds);
  }, [orderedIssueIds, setOrderedIssueIds]);

  // Clear multi-selection when project or view mode changes
  useEffect(() => {
    clearSelection();
  }, [projectId, kanbanViewMode, clearSelection]);

  // Keep anchor in sync with the currently opened issue (e.g. from URL on
  // page load) so Shift/Cmd+Click on another issue includes it.
  useEffect(() => {
    if (selectedKanbanIssueId) {
      setAnchor(selectedKanbanIssueId);
    }
  }, [selectedKanbanIssueId, setAnchor]);

  const handleCardClick = useCallback(
    (issueId: string, e?: MouseEvent) => {
      if (e && (e.metaKey || e.ctrlKey || e.shiftKey)) {
        handleIssueClick(issueId, e);
      } else {
        if (selectedIssueIds.size > 0) {
          clearSelection();
        }
        // Set as anchor so Shift+Click from this issue works
        setAnchor(issueId);
        openIssue(issueId);
      }
    },
    [
      openIssue,
      handleIssueClick,
      selectedIssueIds.size,
      clearSelection,
      setAnchor,
    ]
  );

  const handleAddTask = useCallback(
    (statusId?: string) => {
      const createPayload = {
        statusId: statusId ?? defaultCreateStatusId,
        ...(createAssigneeIds.length > 0
          ? { assigneeIds: createAssigneeIds }
          : {}),
      };
      startCreate(createPayload);
    },
    [createAssigneeIds, defaultCreateStatusId, startCreate]
  );

  // Inline editing callbacks for kanban cards
  // When multi-select is active, apply to all selected issues
  const handleCardPriorityClick = useCallback(
    (issueId: string) => {
      const ids = isMultiSelectActive ? [...selectedIssueIds] : [issueId];
      openPrioritySelection(projectId, ids);
    },
    [projectId, openPrioritySelection, selectedIssueIds, isMultiSelectActive]
  );

  const handleCardAssigneeClick = useCallback(
    (issueId: string) => {
      const ids = isMultiSelectActive ? [...selectedIssueIds] : [issueId];
      openAssigneeSelection(projectId, ids);
    },
    [projectId, openAssigneeSelection, selectedIssueIds, isMultiSelectActive]
  );

  const handleCardMoreActionsClick = useCallback(
    (issueId: string) => {
      const ids = isMultiSelectActive ? [...selectedIssueIds] : [issueId];
      CommandBarDialog.show({
        page: 'issueActions',
        projectId,
        issueIds: ids,
      });
    },
    [projectId, selectedIssueIds, isMultiSelectActive]
  );

  const handleCardTagToggle = useCallback(
    (issueId: string, tagId: string) => {
      const currentIssueTags = getTagsForIssue(issueId);
      const existing = currentIssueTags.find((it) => it.tag_id === tagId);
      if (existing) {
        removeIssueTag(existing.id);
      } else {
        insertIssueTag({ issue_id: issueId, tag_id: tagId });
      }
    },
    [getTagsForIssue, insertIssueTag, removeIssueTag]
  );

  const getResolvedRelationshipsForIssue = useCallback(
    (issueId: string) =>
      resolveRelationshipsForIssue(
        issueId,
        getRelationshipsForIssue(issueId),
        issuesById
      ),
    [getRelationshipsForIssue, issuesById]
  );

  const handleCreateTag = useCallback(
    (data: { name: string; color: string }): string => {
      const { data: newTag } = insertTag({
        project_id: projectId,
        name: data.name,
        color: data.color,
      });
      return newTag.id;
    },
    [insertTag, projectId]
  );

  const isLoading = projectLoading || orgLoading;

  if (isLoading) {
    return <LoadingState />;
  }

  return (
    <div className="flex flex-col h-full space-y-base">
      <div
        className={cn(
          'px-double pt-double space-y-base',
          isMobile && 'px-base pt-base'
        )}
      >
        <div className="flex items-center gap-half">
          <h2 className={cn('text-2xl font-medium', isMobile && 'text-lg')}>
            {projectName}
          </h2>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                className="p-half rounded-sm text-low hover:text-normal hover:bg-secondary transition-colors"
                aria-label="Project menu"
              >
                <DotsThreeIcon className="size-icon-sm" weight="bold" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={openProjectsGuide}>
                {t('kanban.openProjectsGuide', 'Projects guide')}
              </DropdownMenuItem>
              <DropdownMenuItem
                onClick={() => executeAction(Actions.ProjectSettings)}
              >
                {t('kanban.editProjectSettings', 'Edit project settings')}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        <div
          className={cn(
            'flex items-start gap-base',
            isMobile ? 'flex-col' : 'flex-wrap'
          )}
        >
          <ViewNavTabs
            activeView={kanbanViewMode}
            onViewChange={setKanbanViewMode}
            hiddenStatuses={hiddenStatuses}
            selectedStatusId={listViewStatusFilter}
            onStatusSelect={setListViewStatusFilter}
          />
          <KanbanFilterBar
            isFiltersDialogOpen={isFiltersDialogOpen}
            onFiltersDialogOpenChange={setIsFiltersDialogOpen}
            tags={tags}
            users={membersWithProfiles}
            activeViewId={activeViewId}
            onViewChange={handleKanbanProjectViewChange}
            viewIds={KANBAN_PROJECT_VIEW_IDS}
            projectId={projectId}
            currentUserId={userId}
            filters={kanbanFilters}
            showSubIssues={showSubIssues}
            showWorkspaces={showWorkspaces}
            hasActiveFilters={hasActiveFilters}
            onSearchQueryChange={setKanbanSearchQuery}
            onPrioritiesChange={setKanbanPriorities}
            onAssigneesChange={setKanbanAssignees}
            onTagsChange={setKanbanTags}
            onSortChange={setKanbanSort}
            onShowSubIssuesChange={setShowSubIssues}
            onShowWorkspacesChange={setShowWorkspaces}
            hideBlocked={hideBlocked}
            onHideBlockedChange={setHideBlocked}
            onClearFilters={clearKanbanFilters}
            onCreateIssue={handleAddTask}
            shouldAnimateCreateButton={shouldAnimateCreateButton}
            renderFiltersDialog={(props) => <KanbanFiltersDialog {...props} />}
            isMobile={isMobile}
          />
        </div>
      </div>

      {kanbanViewMode === 'kanban' ? (
        visibleStatuses.length === 0 ? (
          <div className="flex-1 flex items-center justify-center">
            <p className="text-low">{t('kanban.noVisibleStatuses')}</p>
          </div>
        ) : (
          <KanbanBoardView
            columns={boardColumns}
            onDragEnd={handleDragEnd}
            issueMap={issueMap}
            issueAssigneesMap={issueAssigneesMap}
            workspacesByIssueId={workspacesByIssueId}
            tags={tags}
            selectedIssueId={selectedKanbanIssueId}
            selectedIssueIds={selectedIssueIds}
            isMultiSelectActive={isMultiSelectActive}
            isMobile={isMobile}
            getPullRequestsForIssue={getPullRequestsForIssue}
            getTagObjectsForIssue={getTagObjectsForIssue}
            getTagsForIssue={getTagsForIssue}
            getResolvedRelationshipsForIssue={getResolvedRelationshipsForIssue}
            onAddIssue={handleAddTask}
            onCardClick={handleCardClick}
            onCardPriorityClick={handleCardPriorityClick}
            onCardAssigneeClick={handleCardAssigneeClick}
            onCardMoreActionsClick={handleCardMoreActionsClick}
            onCardTagToggle={handleCardTagToggle}
            onCreateTag={handleCreateTag}
            onOpenIssueWorkspace={openIssueWorkspace}
          />
        )
      ) : (
        <div className="flex-1 overflow-y-auto px-double">
          <KanbanProvider onDragEnd={handleDragEnd} className="!block !w-full">
            <IssueListView
              statuses={listViewStatuses}
              items={items}
              issueMap={issueMap}
              issueAssigneesMap={issueAssigneesMap}
              getTagObjectsForIssue={getTagObjectsForIssue}
              getResolvedRelationshipsForIssue={
                getResolvedRelationshipsForIssue
              }
              onIssueClick={handleCardClick}
              selectedIssueId={selectedKanbanIssueId}
              selectedIssueIds={selectedIssueIds}
              isMultiSelectActive={isMultiSelectActive}
              onIssueCheckboxChange={handleCheckboxChange}
            />
          </KanbanProvider>
        </div>
      )}

      {isMultiSelectActive && <BulkActionBarContainer projectId={projectId} />}
    </div>
  );
}
