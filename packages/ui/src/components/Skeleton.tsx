import { cn } from '../lib/cn';

export interface SkeletonProps {
  className?: string;
}

/**
 * 骨架屏基元（设计文档 §7.5：加载态一律用骨架屏而不是整页 spinner）。
 *
 * 只用设计令牌 `bg-panel`，浅色与暗色都跟着主题走，不硬编码颜色。
 * 对读屏软件是 `aria-hidden`：骨架本身没有信息量，由外层容器带
 * `role="status"` + `aria-label` 说明「正在加载」。
 */
export function Skeleton({ className }: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={cn('animate-pulse rounded-sm bg-panel', className)}
    />
  );
}
