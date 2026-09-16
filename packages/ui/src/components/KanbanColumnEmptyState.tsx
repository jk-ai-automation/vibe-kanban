import { useTranslation } from 'react-i18next';
import { FunnelSimpleIcon, PlusIcon, StackIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export type KanbanColumnEmptyStateKind = 'empty' | 'filtered';

export interface KanbanColumnEmptyStateProps {
  kind: KanbanColumnEmptyStateKind;
  /** 「新建需求」。`kind === 'empty'` 时显示。 */
  onCreateIssue?: () => void;
  /** 「从需求创建工作区」。没有需求可选时调用方不传，按钮就不显示。 */
  onCreateWorkspace?: () => void;
  /** 「清除筛选」。`kind === 'filtered'` 时显示。 */
  onClearFilters?: () => void;
  className?: string;
}

function EmptyStateButton({
  onClick,
  icon: Icon,
  children,
}: {
  onClick: () => void;
  icon: typeof PlusIcon;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex w-full items-center justify-center gap-half rounded-sm px-base py-half',
        'text-sm text-normal transition-colors',
        'border border-dashed border-border',
        'hover:border-brand hover:bg-brand/10 hover:text-high',
        'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand'
      )}
    >
      <Icon className="size-icon-xs shrink-0" weight="bold" />
      <span className="truncate">{children}</span>
    </button>
  );
}

/**
 * 空列引导（设计文档 §7.2：空列有明确的下一步引导按钮）。
 *
 * 两种形态：
 * - `empty`：这一列真的没有需求 → 「新建需求」「从需求创建工作区」。
 * - `filtered`：被筛选条件筛空了 → 说清楚原因 + 「清除筛选」。
 *
 * 纯展示组件，配色全走设计令牌（`text-low` / `text-normal` / `border-border` /
 * `brand`），暗色模式自动跟随。
 */
export function KanbanColumnEmptyState({
  kind,
  onCreateIssue,
  onCreateWorkspace,
  onClearFilters,
  className,
}: KanbanColumnEmptyStateProps) {
  const { t } = useTranslation('common');

  if (kind === 'filtered') {
    return (
      <div
        className={cn(
          'flex flex-col items-center gap-half rounded-sm px-base py-base text-center',
          className
        )}
      >
        <p className="m-0 text-sm text-low">
          {t('kanban.columnEmpty.filteredTitle')}
        </p>
        {onClearFilters && (
          <EmptyStateButton onClick={onClearFilters} icon={FunnelSimpleIcon}>
            {t('kanban.clearFilters')}
          </EmptyStateButton>
        )}
      </div>
    );
  }

  return (
    <div
      className={cn(
        'flex flex-col items-center gap-half rounded-sm px-base py-base text-center',
        className
      )}
    >
      <p className="m-0 text-sm text-low">
        {t('kanban.columnEmpty.emptyTitle')}
      </p>
      {onCreateIssue && (
        <EmptyStateButton onClick={onCreateIssue} icon={PlusIcon}>
          {t('kanban.columnEmpty.createIssue')}
        </EmptyStateButton>
      )}
      {onCreateWorkspace && (
        <EmptyStateButton onClick={onCreateWorkspace} icon={StackIcon}>
          {t('kanban.columnEmpty.createWorkspace')}
        </EmptyStateButton>
      )}
    </div>
  );
}
