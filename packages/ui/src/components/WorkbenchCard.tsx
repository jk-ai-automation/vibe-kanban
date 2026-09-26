import type { ReactNode } from 'react';
import { cn } from '../lib/cn';
import { PrimaryButton } from './PrimaryButton';

export interface WorkbenchCardProps {
  issueId: string;
  simpleId: string;
  title: string;
  stageBadge?: ReactNode;
  statusTag?: ReactNode;
  /** 已翻译的相对时间。 */
  timeText?: string | null;
  meta?: string | null;
  progress?: ReactNode;
  actions?: ReactNode;
  /** 等人工描橙边，失败描红边（颜色之外还有文字状态标签，色弱可辨）。 */
  highlight?: 'gate' | 'failed' | null;
  openLabel: string;
  onOpen: () => void;
}

export function WorkbenchCard({
  issueId,
  simpleId,
  title,
  stageBadge,
  statusTag,
  timeText,
  meta,
  progress,
  actions,
  highlight = null,
  openLabel,
  onOpen,
}: WorkbenchCardProps) {
  return (
    <article
      data-testid="workbench-card"
      data-issue-id={issueId}
      className={cn(
        'flex flex-col gap-half rounded-sm border bg-primary p-base',
        highlight === 'failed'
          ? 'border-stage-failed'
          : highlight === 'gate'
            ? 'border-brand/60'
            : 'border-border'
      )}
    >
      <div className="flex min-w-0 items-center gap-half">
        <span className="shrink-0 font-ibm-plex-mono text-xs text-low">
          {simpleId}
        </span>
        {stageBadge}
        {statusTag}
        {timeText && (
          <span
            data-testid="relative-time"
            className="ml-auto shrink-0 text-xs text-low"
          >
            {timeText}
          </span>
        )}
      </div>
      <button
        type="button"
        onClick={onOpen}
        className="m-0 truncate text-left text-base text-high hover:underline"
      >
        {title}
      </button>
      {meta && <p className="m-0 text-sm text-low">{meta}</p>}
      {progress}
      <div className="flex items-center justify-end gap-half">
        {actions}
        <PrimaryButton variant="tertiary" value={openLabel} onClick={onOpen} />
      </div>
    </article>
  );
}
