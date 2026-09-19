import { cn } from '../lib/cn';

/** 与 `web-core/src/entities/pipeline/model/progress.ts` 的 `PipelineCellState` 逐字一致。 */
export type PipelineProgressCellState =
  | 'done'
  | 'running'
  | 'gate'
  | 'failed'
  | 'pending'
  | 'paused'
  | 'cancelled';

export interface PipelineProgressCell {
  key: string;
  state: PipelineProgressCellState;
  /** 已翻译，作为格子的 title。 */
  label: string;
}

/** 设计文档 §8.3：已完成绿、进行中蓝、等人工橙、失败红。 */
const CELL_CLASSES: Record<PipelineProgressCellState, string> = {
  done: 'bg-stage-done',
  running: 'bg-stage-dev animate-pulse',
  gate: 'bg-brand',
  failed: 'bg-stage-failed',
  pending: 'bg-panel',
  paused: 'bg-stage-neutral',
  cancelled: 'bg-stage-neutral opacity-50',
};

export interface PipelineProgressBarProps {
  cells: PipelineProgressCell[];
  ariaLabel: string;
  className?: string;
}

export function PipelineProgressBar({
  cells,
  ariaLabel,
  className,
}: PipelineProgressBarProps) {
  return (
    <div
      role="img"
      aria-label={ariaLabel}
      data-testid="pipeline-progress"
      data-cells={cells.map((cell) => cell.state).join(',')}
      className={cn('flex w-full items-center gap-0.5', className)}
    >
      {cells.map((cell) => (
        <span
          key={cell.key}
          title={cell.label}
          className={cn('h-1 flex-1 rounded-sm', CELL_CLASSES[cell.state])}
        />
      ))}
    </div>
  );
}
