import { CheckIcon, XIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import type { PipelineProgressCellState } from './PipelineProgressBar';

export interface PipelineStepperStep {
  key: string;
  label: string;
  state: PipelineProgressCellState;
  /** 已翻译的状态说明（title）。 */
  stateLabel: string;
}

const MARKER_CLASSES: Record<PipelineProgressCellState, string> = {
  done: 'bg-stage-done text-on-brand',
  running: 'bg-stage-dev text-on-brand',
  gate: 'bg-brand text-on-brand',
  failed: 'bg-stage-failed text-on-brand',
  pending: 'bg-panel text-low',
  paused: 'bg-stage-neutral text-on-brand',
  cancelled: 'bg-panel text-low',
};

/** 七段步进条：已完成打勾、当前高亮、失败标红（设计文档 §8.4）。 */
export function PipelineStepper({
  steps,
  currentKey,
}: {
  steps: PipelineStepperStep[];
  currentKey: string | null;
}) {
  return (
    <ol
      data-testid="pipeline-stepper"
      data-current={currentKey ?? ''}
      className="m-0 flex list-none items-center gap-half overflow-x-auto p-0"
    >
      {steps.map((step, index) => (
        <li
          key={step.key}
          data-state={step.state}
          title={step.stateLabel}
          className="flex shrink-0 items-center gap-half"
        >
          <span
            className={cn(
              'flex size-5 items-center justify-center rounded-full font-ibm-plex-mono text-xs',
              MARKER_CLASSES[step.state]
            )}
          >
            {step.state === 'done' ? (
              <CheckIcon className="size-3" weight="bold" />
            ) : step.state === 'failed' ? (
              <XIcon className="size-3" weight="bold" />
            ) : (
              index + 1
            )}
          </span>
          <span
            className={cn(
              'text-sm',
              step.key === currentKey ? 'font-medium text-high' : 'text-low'
            )}
          >
            {step.label}
          </span>
          {index < steps.length - 1 && (
            <span aria-hidden="true" className="h-px w-double bg-border" />
          )}
        </li>
      ))}
    </ol>
  );
}
