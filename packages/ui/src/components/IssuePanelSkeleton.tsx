import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';
import { Skeleton } from './Skeleton';

export interface IssuePanelSkeletonProps {
  className?: string;
}

/**
 * 需求详情面板的加载骨架（设计文档 §7.5：加载态一律骨架屏）。
 * 版式对着 `KanbanIssuePanel` 的头部 / 属性行 / 标题 / 描述摆。
 */
export function IssuePanelSkeleton({ className }: IssuePanelSkeletonProps) {
  const { t } = useTranslation('common');

  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={t('states.loading')}
      className={cn('flex h-full flex-col bg-secondary', className)}
    >
      {/* 头部：simple_id + 操作按钮 */}
      <div className="flex shrink-0 items-center justify-between border-b px-base py-half">
        <Skeleton className="h-4 w-20" />
        <Skeleton className="h-4 w-12" />
      </div>
      {/* 属性行 */}
      <div className="flex gap-half border-b px-base py-base">
        <Skeleton className="h-5 w-20" />
        <Skeleton className="h-5 w-16" />
        <Skeleton className="h-5 w-16" />
      </div>
      {/* 标签行 */}
      <div className="flex gap-half border-b px-base py-base">
        <Skeleton className="h-5 w-14" />
        <Skeleton className="h-5 w-10" />
      </div>
      {/* 标题 + 描述 */}
      <div className="flex flex-col gap-base px-base py-base">
        <Skeleton className="h-6 w-3/4" />
        <Skeleton className="h-3 w-full" />
        <Skeleton className="h-3 w-full" />
        <Skeleton className="h-3 w-2/3" />
      </div>
    </div>
  );
}
