import { cn } from '../lib/cn';

/** 与 web-core `PipelineTone` 逐字一致。 */
export type PipelineStatusTagTone =
  | 'running'
  | 'gate'
  | 'failed'
  | 'paused'
  | 'done'
  | 'cancelled';

const TONE_CLASSES: Record<PipelineStatusTagTone, string> = {
  running: 'bg-stage-dev/10 text-stage-dev',
  gate: 'bg-brand/10 text-brand',
  failed: 'bg-stage-failed/10 text-stage-failed',
  paused: 'bg-panel text-low',
  done: 'bg-stage-done/10 text-stage-done',
  cancelled: 'bg-panel text-low',
};

export interface PipelineStatusTagProps {
  tone: PipelineStatusTagTone;
  label: string;
  className?: string;
}

/** 状态标签；运行中带呼吸点（设计文档 §8.3）。 */
export function PipelineStatusTag({
  tone,
  label,
  className,
}: PipelineStatusTagProps) {
  return (
    <span
      data-testid="pipeline-status-tag"
      data-tone={tone}
      title={label}
      className={cn(
        'inline-flex max-w-full items-center gap-half rounded-sm px-1.5 py-0.5',
        'whitespace-nowrap text-xs font-medium',
        TONE_CLASSES[tone],
        className
      )}
    >
      {tone === 'running' && (
        <span
          aria-hidden="true"
          className="size-dot shrink-0 rounded-full bg-current animate-running-dot-1"
        />
      )}
      <span className="truncate">{label}</span>
    </span>
  );
}
