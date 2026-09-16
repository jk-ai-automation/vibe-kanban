import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';
import { Skeleton } from './Skeleton';

export interface KanbanBoardSkeletonProps {
  /** 骨架列数。默认 3 列，跟真实看板首屏差不多宽。 */
  columns?: number;
  /** 每列的骨架卡片数。 */
  cardsPerColumn?: number;
  className?: string;
}

function KanbanCardSkeleton() {
  return (
    <div className="flex flex-col gap-half rounded-sm bg-secondary p-base">
      {/* 第 1 行：simple_id */}
      <Skeleton className="h-3 w-16" />
      {/* 第 2 行：标题 */}
      <Skeleton className="h-4 w-full" />
      {/* 第 3 行：描述预览 */}
      <Skeleton className="h-3 w-4/5" />
      {/* 第 4 行：底部徽标 */}
      <div className="flex items-center gap-half pt-half">
        <Skeleton className="h-4 w-12" />
        <Skeleton className="h-4 w-10" />
      </div>
    </div>
  );
}

/**
 * 看板加载骨架：N 列 × 每列 M 张卡片。
 *
 * 宽度用 `min-w-0` + `flex-1`，1280px 窄屏下列会自己变窄而不是撑出横向滚动条。
 */
export function KanbanBoardSkeleton({
  columns = 3,
  cardsPerColumn = 4,
  className,
}: KanbanBoardSkeletonProps) {
  const { t } = useTranslation('common');
  const columnIndexes = Array.from({ length: Math.max(1, columns) });
  const cardIndexes = Array.from({ length: Math.max(1, cardsPerColumn) });

  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={t('states.loading')}
      className={cn('flex flex-1 gap-base overflow-hidden px-double', className)}
    >
      {columnIndexes.map((_, columnIndex) => (
        <div
          key={columnIndex}
          className="flex min-w-0 flex-1 flex-col gap-base"
        >
          <div className="flex items-center justify-between gap-2 border-b border-t bg-secondary p-base">
            <Skeleton className="h-3 w-24" />
            <Skeleton className="h-3 w-6" />
          </div>
          <div className="flex flex-col gap-base">
            {cardIndexes.map((__, cardIndex) => (
              <KanbanCardSkeleton key={cardIndex} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
