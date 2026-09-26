import { cn } from '../lib/cn';

/** 与 web-core `PipelineStageTone` 逐字一致（设计文档 §8.5 阶段语义色）。 */
export type PipelineStageBadgeTone =
  | 'neutral'
  | 'dev'
  | 'review'
  | 'test'
  | 'done';

const TONE_CLASSES: Record<PipelineStageBadgeTone, string> = {
  neutral: 'bg-stage-neutral/15 text-normal',
  dev: 'bg-stage-dev/15 text-stage-dev',
  review: 'bg-stage-review/15 text-stage-review',
  test: 'bg-stage-test/15 text-stage-test',
  done: 'bg-stage-done/15 text-stage-done',
};

export function PipelineStageBadge({
  tone,
  label,
  className,
}: {
  tone: PipelineStageBadgeTone;
  label: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center rounded-sm px-1.5 py-0.5 text-xs font-medium whitespace-nowrap',
        TONE_CLASSES[tone],
        className
      )}
    >
      {label}
    </span>
  );
}
