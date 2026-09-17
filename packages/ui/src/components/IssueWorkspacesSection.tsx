import { useTranslation } from 'react-i18next';
import {
  IssueWorkspaceCard,
  IssueWorkspaceCreateCard,
  type WorkspaceDeleteAffordance,
  type WorkspaceWithStats,
} from './IssueWorkspaceCard';
import {
  CollapsibleSectionHeader,
  type SectionAction,
} from './CollapsibleSectionHeader';

export interface IssueWorkspacesSectionProps {
  workspaces: WorkspaceWithStats[];
  isLoading?: boolean;
  actions?: SectionAction[];
  onWorkspaceClick?: (localWorkspaceId: string | null) => void;
  onCreateWorkspace?: () => void;
  onUnlinkWorkspace?: (localWorkspaceId: string) => void;
  onDeleteWorkspace?: (localWorkspaceId: string) => void;
  /**
   * 按本地工作区 id 取这张卡的删除能力。返回 `undefined`（个人版，或
   * 还没加载出来）时卡片行为与历史版本逐字一致。
   */
  getDeleteAffordance?: (
    localWorkspaceId: string
  ) => WorkspaceDeleteAffordance | undefined;
  onWithdrawDeleteRequest?: (localWorkspaceId: string) => void;
  onApproveDeleteRequest?: (localWorkspaceId: string) => void;
  onRejectDeleteRequest?: (localWorkspaceId: string) => void;
  shouldAnimateCreateButton?: boolean;
}

/**
 * View component for the workspaces section in the issue panel.
 * Displays a collapsible list of workspace cards.
 */
export function IssueWorkspacesSection({
  workspaces,
  isLoading,
  actions = [],
  onWorkspaceClick,
  onCreateWorkspace,
  onUnlinkWorkspace,
  onDeleteWorkspace,
  getDeleteAffordance,
  onWithdrawDeleteRequest,
  onApproveDeleteRequest,
  onRejectDeleteRequest,
  shouldAnimateCreateButton = false,
}: IssueWorkspacesSectionProps) {
  const { t } = useTranslation('common');

  return (
    <CollapsibleSectionHeader
      title={t('workspaces.title')}
      persistKey="kanban-issue-workspaces"
      defaultExpanded={true}
      actions={actions}
    >
      <div className="px-base p-base flex flex-col gap-base border-t">
        {isLoading ? (
          <p className="text-low py-half">{t('workspaces.loading')}</p>
        ) : workspaces.length === 0 ? (
          <IssueWorkspaceCreateCard
            onClick={onCreateWorkspace}
            shouldAnimateCreateButton={shouldAnimateCreateButton}
          />
        ) : (
          workspaces.map((workspace) => {
            const { localWorkspaceId } = workspace;
            const affordance = localWorkspaceId
              ? getDeleteAffordance?.(localWorkspaceId)
              : undefined;
            // 没有 affordance（个人版，或还没加载出来）时退回历史判据：
            // 只有自己的工作区才给删除入口。
            const canInitiate = affordance
              ? affordance.canInitiate
              : workspace.isOwnedByCurrentUser;
            return (
              <IssueWorkspaceCard
                key={workspace.id}
                workspace={workspace}
                onClick={
                  onWorkspaceClick &&
                  localWorkspaceId &&
                  workspace.isOwnedByCurrentUser
                    ? () => onWorkspaceClick(localWorkspaceId)
                    : undefined
                }
                onUnlink={
                  onUnlinkWorkspace && localWorkspaceId
                    ? () => onUnlinkWorkspace(localWorkspaceId)
                    : undefined
                }
                onDelete={
                  onDeleteWorkspace && localWorkspaceId && canInitiate
                    ? () => onDeleteWorkspace(localWorkspaceId)
                    : undefined
                }
                deleteAffordance={affordance}
                onWithdrawDeleteRequest={
                  onWithdrawDeleteRequest && localWorkspaceId
                    ? () => onWithdrawDeleteRequest(localWorkspaceId)
                    : undefined
                }
                onApproveDeleteRequest={
                  onApproveDeleteRequest && localWorkspaceId
                    ? () => onApproveDeleteRequest(localWorkspaceId)
                    : undefined
                }
                onRejectDeleteRequest={
                  onRejectDeleteRequest && localWorkspaceId
                    ? () => onRejectDeleteRequest(localWorkspaceId)
                    : undefined
                }
              />
            );
          })
        )}
      </div>
    </CollapsibleSectionHeader>
  );
}
