import { cn } from '../lib/cn';
import { PlusIcon, UsersIcon, XIcon } from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import { PrimaryButton } from './PrimaryButton';
import { IconButton } from './IconButton';
import { StatusDot } from './StatusDot';
import { PriorityIcon, type PriorityLevel } from './PriorityIcon';
import { UserAvatar, type UserAvatarUser } from './UserAvatar';
import { KanbanAssignee, type KanbanAssigneeUser } from './KanbanAssignee';

export interface IssuePropertyStatus {
  id: string;
  name: string;
  color: string;
}

const priorityLabelKeys: Record<PriorityLevel, string> = {
  urgent: 'kanban.priorityLevel.urgent',
  high: 'kanban.priorityLevel.high',
  medium: 'kanban.priorityLevel.medium',
  low: 'kanban.priorityLevel.low',
};

export interface IssuePropertyRowProps {
  statusId: string;
  priority: PriorityLevel | null;
  assigneeIds: string[];
  assigneeUsers?: KanbanAssigneeUser[];
  /** 指派入口只在团队版出现；个人版传 false，整个按钮不渲染。 */
  showAssignees?: boolean;
  statuses: IssuePropertyStatus[];
  creatorUser?: UserAvatarUser | null;
  parentIssue?: { id: string; simpleId: string } | null;
  onParentIssueClick?: () => void;
  onRemoveParentIssue?: () => void;
  onStatusClick: () => void;
  onPriorityClick: () => void;
  onAssigneeClick: () => void;
  onAddClick?: () => void;
  disabled?: boolean;
  className?: string;
}

export function IssuePropertyRow({
  statusId,
  priority,
  assigneeUsers,
  showAssignees = true,
  statuses,
  creatorUser,
  parentIssue,
  onParentIssueClick,
  onRemoveParentIssue,
  onStatusClick,
  onPriorityClick,
  onAssigneeClick,
  onAddClick,
  disabled,
  className,
}: IssuePropertyRowProps) {
  const { t } = useTranslation('common');

  return (
    <div className={cn('flex items-center gap-half flex-wrap', className)}>
      <PrimaryButton
        variant="tertiary"
        onClick={onStatusClick}
        disabled={disabled}
      >
        <StatusDot
          color={statuses.find((s) => s.id === statusId)?.color ?? '0 0% 50%'}
        />
        {statuses.find((s) => s.id === statusId)?.name ??
          t('kanban.selectStatus')}
      </PrimaryButton>

      <PrimaryButton
        variant="tertiary"
        onClick={onPriorityClick}
        disabled={disabled}
      >
        <PriorityIcon priority={priority} />
        {priority ? t(priorityLabelKeys[priority]) : t('kanban.noPriority')}
      </PrimaryButton>

      {showAssignees && (
        <PrimaryButton
          variant="tertiary"
          onClick={onAssigneeClick}
          disabled={disabled}
        >
          {assigneeUsers && assigneeUsers.length > 0 ? (
            <KanbanAssignee assignees={assigneeUsers} />
          ) : (
            <>
              <UsersIcon className="size-icon-xs" weight="bold" />
              {t('kanban.assignee', 'Assignee')}
            </>
          )}
        </PrimaryButton>
      )}

      {creatorUser &&
        (creatorUser.first_name?.trim() || creatorUser.username?.trim()) && (
          <div className="flex items-center gap-half px-base py-half bg-panel rounded-sm text-sm whitespace-nowrap">
            <span className="text-low">
              {t('kanban.createdBy', 'Created by')}
            </span>
            <UserAvatar
              user={creatorUser}
              className="h-5 w-5 text-[9px] border border-border"
            />
            <span className="text-normal truncate max-w-[120px]">
              {creatorUser.first_name?.trim() || creatorUser.username?.trim()}
            </span>
          </div>
        )}

      {parentIssue && (
        <div className="flex items-center gap-half">
          <PrimaryButton
            variant="tertiary"
            onClick={onParentIssueClick}
            disabled={disabled}
            className="whitespace-nowrap text-sm"
          >
            <span className="text-low">
              {t('kanban.parentIssue', 'Parent')}:
            </span>
            <span className="font-ibm-plex-mono text-normal">
              {parentIssue.simpleId}
            </span>
          </PrimaryButton>
          {onRemoveParentIssue && (
            <IconButton
              icon={XIcon}
              onClick={onRemoveParentIssue}
              disabled={disabled}
              aria-label={t('kanban.removeParentIssue')}
              title={t('kanban.removeParentIssue')}
            />
          )}
        </div>
      )}

      {onAddClick && (
        <IconButton
          icon={PlusIcon}
          onClick={onAddClick}
          disabled={disabled}
          aria-label={t('buttons.add')}
          title={t('buttons.add')}
        />
      )}
    </div>
  );
}
