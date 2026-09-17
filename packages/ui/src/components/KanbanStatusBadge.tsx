import type { ReactNode } from 'react';
import { cn } from '../lib/cn';

/**
 * 卡片底部状态徽标的色调（设计文档 §7.2）。
 *
 * 判定逻辑在 `@/features/kanban/model/cardBadges`，这里只负责把色调映射成 class。
 * 所有配色都走设计令牌（`brand` / `error` / `success` / `merged` / `panel` /
 * `text-low`），浅色与暗色自动跟随主题，没有硬编码颜色。
 */
export type KanbanStatusBadgeTone =
  | 'running'
  | 'stopped'
  | 'failed'
  | 'open'
  | 'merged'
  | 'closed'
  | 'pending';

const TONE_CLASSES: Record<KanbanStatusBadgeTone, string> = {
  running: 'bg-brand/10 text-brand',
  stopped: 'bg-panel text-low',
  failed: 'bg-error/10 text-error',
  open: 'bg-success/10 text-success',
  merged: 'bg-merged/10 text-merged',
  closed: 'bg-error/10 text-error',
  pending: 'bg-panel text-low',
};

export interface KanbanStatusBadgeProps {
  tone: KanbanStatusBadgeTone;
  label: string;
  icon?: ReactNode;
  /** > 1 时在标签后追加「×N」。 */
  count?: number;
  title?: string;
  className?: string;
}

export function KanbanStatusBadge({
  tone,
  label,
  icon,
  count,
  title,
  className,
}: KanbanStatusBadgeProps) {
  return (
    <span
      title={title ?? label}
      className={cn(
        'inline-flex max-w-full items-center gap-half',
        'rounded-sm px-1.5 py-0.5',
        'text-xs font-medium whitespace-nowrap',
        TONE_CLASSES[tone] ?? TONE_CLASSES.pending,
        className
      )}
    >
      {icon}
      <span className="truncate">{label}</span>
      {typeof count === 'number' && count > 1 && <span>×{count}</span>}
    </span>
  );
}
