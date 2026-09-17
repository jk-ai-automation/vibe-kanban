import { CaretRightIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export type WorkspaceBreadcrumbSegmentKind = 'issue' | 'workspace' | 'pr';

export interface WorkspaceBreadcrumbSegment {
  kind: WorkspaceBreadcrumbSegmentKind;
  label: string;
  actionable: boolean;
  href?: string;
}

export interface WorkspaceBreadcrumbProps {
  segments: WorkspaceBreadcrumbSegment[];
  onSegmentClick?: (kind: WorkspaceBreadcrumbSegmentKind) => void;
  /** 每一段的无障碍标签，按 kind 取。缺省时退回 label 自身。 */
  segmentAriaLabels?: Partial<Record<WorkspaceBreadcrumbSegmentKind, string>>;
  className?: string;
}

const segmentTextClass =
  'min-w-0 truncate rounded-sm px-1 py-0.5 text-sm text-normal transition-colors';
const segmentInteractiveClass =
  'hover:bg-panel hover:text-high focus:outline-none focus-visible:ring-1 focus-visible:ring-brand';

/**
 * 工作区详情顶部的「需求 → 工作区 → PR」路径条（设计文档 §7.4）。
 *
 * 纯展示：段落由 `buildWorkspaceBreadcrumb` 算好传进来，这里只渲染。
 * PR 段带 `href` 时渲染成外链，其余可点段走 `onSegmentClick`。
 * `min-w-0` + `truncate` 保证窄屏（1280px）下长分支名被截断而不是撑出横向滚动条。
 */
export function WorkspaceBreadcrumb({
  segments,
  onSegmentClick,
  segmentAriaLabels,
  className,
}: WorkspaceBreadcrumbProps) {
  if (segments.length === 0) {
    return null;
  }

  return (
    <nav
      aria-label="Workspace breadcrumb"
      className={cn(
        'flex min-w-0 items-center gap-half font-ibm-plex-mono',
        className
      )}
    >
      {segments.map((segment, index) => (
        <div
          key={`${segment.kind}-${index}`}
          className={cn(
            'flex min-w-0 items-center gap-half',
            segment.kind === 'workspace' ? 'min-w-0' : 'shrink-0'
          )}
        >
          {index > 0 && (
            <CaretRightIcon
              className="size-icon-2xs shrink-0 text-low"
              weight="bold"
              aria-hidden="true"
            />
          )}
          {segment.href ? (
            <a
              href={segment.href}
              target="_blank"
              rel="noopener noreferrer"
              aria-label={segmentAriaLabels?.[segment.kind] ?? segment.label}
              className={cn(segmentTextClass, segmentInteractiveClass)}
            >
              {segment.label}
            </a>
          ) : segment.actionable && onSegmentClick ? (
            <button
              type="button"
              onClick={() => onSegmentClick(segment.kind)}
              aria-label={segmentAriaLabels?.[segment.kind] ?? segment.label}
              className={cn(segmentTextClass, segmentInteractiveClass)}
            >
              {segment.label}
            </button>
          ) : (
            <span className={cn(segmentTextClass, 'text-low')}>
              {segment.label}
            </span>
          )}
        </div>
      ))}
    </nav>
  );
}
