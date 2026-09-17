import { useMemo, useCallback } from 'react';
import { useParams } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { LinkIcon, PlusIcon } from '@phosphor-icons/react';
import { useProjectContext } from '@/shared/hooks/useProjectContext';
import { useAuth } from '@/shared/hooks/auth/useAuth';
import { useOrgContext } from '@/shared/hooks/useOrgContext';
import { useUserContext } from '@/shared/hooks/useUserContext';
import { useWorkspaceContext } from '@/shared/hooks/useWorkspaceContext';
import { useWorkspaceDeleteRequests } from '@/shared/hooks/useWorkspaceDeleteRequests';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { useProjectWorkspaceCreateDraft } from '@/shared/hooks/useProjectWorkspaceCreateDraft';
import { workspacesApi } from '@/shared/lib/api';
import { getWorkspaceDefaults } from '@/shared/lib/workspaceDefaults';
import {
  buildLinkedIssueCreateState,
  buildLocalWorkspaceIdSet,
  buildWorkspaceCreateInitialState,
  buildWorkspaceCreatePrompt,
} from '@/shared/lib/workspaceCreateState';
import { ConfirmDialog } from '@vibe/ui/components/ConfirmDialog';
import { DeleteWorkspaceDialog } from '@vibe/ui/components/DeleteWorkspaceDialog';
import type { WorkspaceWithStats } from '@vibe/ui/components/IssueWorkspaceCard';
import { IssueWorkspacesSection } from '@vibe/ui/components/IssueWorkspacesSection';
import type { SectionAction } from '@vibe/ui/components/CollapsibleSectionHeader';

interface IssueWorkspacesSectionContainerProps {
  issueId: string;
}

/**
 * Container component for the workspaces section.
 * Fetches workspace data from ProjectContext and transforms it for display.
 */
export function IssueWorkspacesSectionContainer({
  issueId,
}: IssueWorkspacesSectionContainerProps) {
  const { t } = useTranslation('common');
  const { projectId } = useParams({ strict: false });
  const appNavigation = useAppNavigation();
  const { openWorkspaceCreateFromState } = useProjectWorkspaceCreateDraft();
  const { userId } = useAuth();
  const { workspaces } = useUserContext();

  const {
    pullRequests,
    getIssue,
    getWorkspacesForIssue,
    issues,
    isLoading: projectLoading,
  } = useProjectContext();
  const { activeWorkspaces, archivedWorkspaces } = useWorkspaceContext();
  const { membersWithProfilesById, isLoading: orgLoading } = useOrgContext();
  const deleteRequests = useWorkspaceDeleteRequests();

  const localWorkspacesById = useMemo(() => {
    const map = new Map<string, (typeof activeWorkspaces)[number]>();

    for (const workspace of activeWorkspaces) {
      map.set(workspace.id, workspace);
    }

    for (const workspace of archivedWorkspaces) {
      map.set(workspace.id, workspace);
    }

    return map;
  }, [activeWorkspaces, archivedWorkspaces]);

  // Get workspaces for the issue, with PR info
  const workspacesWithStats: WorkspaceWithStats[] = useMemo(() => {
    const rawWorkspaces = getWorkspacesForIssue(issueId);

    return rawWorkspaces.map((workspace) => {
      const localWorkspace = workspace.local_workspace_id
        ? localWorkspacesById.get(workspace.local_workspace_id)
        : undefined;

      // Find all linked PRs for this workspace
      const linkedPrs = pullRequests
        .filter((pr) => pr.workspace_id === workspace.id)
        .map((pr) => ({
          number: pr.number,
          url: pr.url,
          status: pr.status as 'open' | 'merged' | 'closed',
        }));

      // Get owner
      const owner =
        membersWithProfilesById.get(workspace.owner_user_id) ?? null;

      return {
        id: workspace.id,
        localWorkspaceId: workspace.local_workspace_id,
        name: workspace.name,
        archived: workspace.archived,
        filesChanged: workspace.files_changed ?? 0,
        linesAdded: workspace.lines_added ?? 0,
        linesRemoved: workspace.lines_removed ?? 0,
        prs: linkedPrs,
        owner,
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
  }, [
    issueId,
    getWorkspacesForIssue,
    pullRequests,
    membersWithProfilesById,
    userId,
    localWorkspacesById,
  ]);

  const isLoading = projectLoading || orgLoading;
  const shouldAnimateCreateButton = useMemo(() => {
    if (issues.length !== 1) {
      return false;
    }

    return issues.every(
      (issue) => getWorkspacesForIssue(issue.id).length === 0
    );
  }, [issues, getWorkspacesForIssue]);

  // Handle clicking '+' to create and link a new workspace directly
  const handleAddWorkspace = useCallback(async () => {
    if (!projectId) {
      return;
    }

    const issue = getIssue(issueId);
    const initialPrompt = buildWorkspaceCreatePrompt(
      issue?.title ?? null,
      issue?.description ?? null
    );
    const localWorkspaceIds = buildLocalWorkspaceIdSet(
      activeWorkspaces,
      archivedWorkspaces
    );

    const defaults = await getWorkspaceDefaults(
      workspaces,
      localWorkspaceIds,
      projectId
    );
    const createState = buildWorkspaceCreateInitialState({
      prompt: initialPrompt,
      defaults,
      linkedIssue: buildLinkedIssueCreateState(issue, projectId),
    });

    const draftId = await openWorkspaceCreateFromState(createState, {
      issueId,
    });
    if (!draftId) {
      await ConfirmDialog.show({
        title: t('common:error'),
        message: t(
          'workspaces.createDraftError',
          'Failed to prepare workspace draft. Please try again.'
        ),
        confirmText: t('common:ok'),
        showCancelButton: false,
      });
    }
  }, [
    projectId,
    openWorkspaceCreateFromState,
    getIssue,
    issueId,
    activeWorkspaces,
    archivedWorkspaces,
    workspaces,
    t,
  ]);

  // Handle clicking link action to link an existing workspace
  const handleLinkWorkspace = useCallback(async () => {
    if (!projectId) {
      return;
    }

    const { WorkspaceSelectionDialog } = await import(
      '@/shared/dialogs/command-bar/WorkspaceSelectionDialog'
    );
    await WorkspaceSelectionDialog.show({ projectId, issueId });
  }, [projectId, issueId]);

  // Handle clicking a workspace card to open it
  const handleWorkspaceClick = useCallback(
    (localWorkspaceId: string | null) => {
      if (projectId && localWorkspaceId) {
        appNavigation.goToProjectIssueWorkspace(
          projectId,
          issueId,
          localWorkspaceId
        );
      }
    },
    [projectId, issueId, appNavigation]
  );

  // Handle unlinking a workspace from the issue
  const handleUnlinkWorkspace = useCallback(
    async (localWorkspaceId: string) => {
      const result = await ConfirmDialog.show({
        title: t('workspaces.unlinkFromIssue'),
        message: t('workspaces.unlinkConfirmMessage'),
        confirmText: t('workspaces.unlink'),
        variant: 'destructive',
      });

      if (result === 'confirmed') {
        try {
          await workspacesApi.unlinkFromIssue(localWorkspaceId);
        } catch (error) {
          ConfirmDialog.show({
            title: t('common:error'),
            message:
              error instanceof Error
                ? error.message
                : t('workspaces.unlinkError'),
            confirmText: t('common:ok'),
            showCancelButton: false,
          });
        }
      }
    },
    [t]
  );

  // 每张卡上的删除能力。个人版全是 undefined，卡片退回历史行为。
  const deleteAffordanceById = useMemo(() => {
    const map = new Map<
      string,
      ReturnType<typeof deleteRequests.getAffordance>
    >();
    for (const workspace of workspacesWithStats) {
      if (!workspace.localWorkspaceId) continue;
      map.set(
        workspace.localWorkspaceId,
        deleteRequests.getAffordance(
          workspace.localWorkspaceId,
          workspace.isOwnedByCurrentUser
        )
      );
    }
    return map;
    // 只依赖 getAffordance（useCallback，身份稳定），不要整个 deleteRequests
    // 对象——那是每次渲染都新建的，会让这张表和下游所有回调白白重算。
  }, [workspacesWithStats, deleteRequests.getAffordance]);

  const getDeleteAffordance = useCallback(
    (localWorkspaceId: string) => deleteAffordanceById.get(localWorkspaceId),
    [deleteAffordanceById]
  );

  const showError = useCallback(
    (error: unknown, fallbackKey: string) => {
      ConfirmDialog.show({
        title: t('common:error'),
        message: error instanceof Error ? error.message : t(fallbackKey),
        confirmText: t('common:ok'),
        showCancelButton: false,
      });
    },
    [t]
  );

  // 非管理员点删除 → 提交删除申请（后端对非管理员一律 403，这里只是把
  // 用户引到正确的动作上）。
  const handleRequestDelete = useCallback(
    async (localWorkspaceId: string) => {
      const result = await ConfirmDialog.show({
        title: t('workspaces.requestDelete'),
        message: t('workspaces.requestDeleteConfirmMessage'),
        confirmText: t('workspaces.requestDelete'),
      });
      if (result !== 'confirmed') return;
      try {
        await deleteRequests.createRequest(localWorkspaceId);
      } catch (error) {
        showError(error, 'workspaces.requestDeleteError');
      }
    },
    [t, deleteRequests, showError]
  );

  const handleWithdrawDeleteRequest = useCallback(
    async (localWorkspaceId: string) => {
      const request = deleteRequests.byWorkspaceId.get(localWorkspaceId);
      if (!request) return;
      try {
        await deleteRequests.withdrawRequest(request.id);
      } catch (error) {
        showError(error, 'workspaces.withdrawDeleteError');
      }
    },
    [deleteRequests, showError]
  );

  const handleApproveDeleteRequest = useCallback(
    async (localWorkspaceId: string) => {
      const request = deleteRequests.byWorkspaceId.get(localWorkspaceId);
      if (!request) return;
      const result = await ConfirmDialog.show({
        title: t('workspaces.approveDelete'),
        message: t('workspaces.approveDeleteConfirmMessage'),
        confirmText: t('workspaces.approveDelete'),
        variant: 'destructive',
      });
      if (result !== 'confirmed') return;
      try {
        await deleteRequests.approveRequest(request.id);
      } catch (error) {
        showError(error, 'workspaces.approveDeleteError');
      }
    },
    [t, deleteRequests, showError]
  );

  const handleRejectDeleteRequest = useCallback(
    async (localWorkspaceId: string) => {
      const request = deleteRequests.byWorkspaceId.get(localWorkspaceId);
      if (!request) return;
      try {
        await deleteRequests.rejectRequest(request.id);
      } catch (error) {
        showError(error, 'workspaces.rejectDeleteError');
      }
    },
    [deleteRequests, showError]
  );

  // Handle deleting a workspace (unlinks first, then deletes local)
  const handleDeleteWorkspace = useCallback(
    async (localWorkspaceId: string) => {
      // 团队版的非管理员走申请流程；个人版没有 affordance，逐字保持原行为。
      if (getDeleteAffordance(localWorkspaceId)?.mode === 'request') {
        await handleRequestDelete(localWorkspaceId);
        return;
      }

      const localWorkspace = localWorkspacesById.get(localWorkspaceId);
      if (!localWorkspace) {
        ConfirmDialog.show({
          title: t('common:error'),
          message: t('workspaces.deleteError'),
          confirmText: t('common:ok'),
          showCancelButton: false,
        });
        return;
      }

      const result = await DeleteWorkspaceDialog.show({
        branchName: localWorkspace.branch,
        hasOpenPR:
          workspacesWithStats
            .find(
              (workspace) => workspace.localWorkspaceId === localWorkspaceId
            )
            ?.prs.some((pr) => pr.status === 'open') ?? false,
        isLinkedToIssue: true,
        linkedIssueSimpleId: getIssue(issueId)?.simple_id,
      });

      if (result.action !== 'confirmed') {
        return;
      }

      try {
        // Delete local workspace first
        await workspacesApi.delete(localWorkspaceId, result.deleteBranches);
        // Unlink from remote after successful deletion
        if (result.unlinkFromIssue) {
          await workspacesApi.unlinkFromIssue(localWorkspaceId);
        }
      } catch (error) {
        ConfirmDialog.show({
          title: t('common:error'),
          message:
            error instanceof Error
              ? error.message
              : t('workspaces.deleteError'),
          confirmText: t('common:ok'),
          showCancelButton: false,
        });
      }
    },
    [
      localWorkspacesById,
      workspacesWithStats,
      t,
      issueId,
      getIssue,
      getDeleteAffordance,
      handleRequestDelete,
    ]
  );

  // Actions for the section header
  const actions: SectionAction[] = useMemo(
    () => [
      {
        icon: PlusIcon,
        onClick: handleAddWorkspace,
      },
      {
        icon: LinkIcon,
        onClick: handleLinkWorkspace,
      },
    ],
    [handleAddWorkspace, handleLinkWorkspace]
  );

  return (
    <IssueWorkspacesSection
      workspaces={workspacesWithStats}
      isLoading={isLoading}
      actions={actions}
      onWorkspaceClick={handleWorkspaceClick}
      onCreateWorkspace={handleAddWorkspace}
      onUnlinkWorkspace={handleUnlinkWorkspace}
      onDeleteWorkspace={handleDeleteWorkspace}
      getDeleteAffordance={getDeleteAffordance}
      onWithdrawDeleteRequest={handleWithdrawDeleteRequest}
      onApproveDeleteRequest={handleApproveDeleteRequest}
      onRejectDeleteRequest={handleRejectDeleteRequest}
      shouldAnimateCreateButton={shouldAnimateCreateButton}
    />
  );
}
