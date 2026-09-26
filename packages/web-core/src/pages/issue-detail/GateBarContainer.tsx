import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { IssuePipelineView } from 'shared/types';
import { PipelineGateBar } from '@vibe/ui/components/PipelineGateBar';
import { useGateDecision } from '@/entities/pipeline/model/hooks/usePipelineData';
import { firstErrorLine } from '@/entities/pipeline/model/progress';
import {
  autoConditionLabelKey,
  fallbackGateLabelKey,
  pipelineStageLabelKey,
} from '@/entities/pipeline/model/stages';
import { gateBarState } from './issueDetailModel';

/** 底部关卡条（设计文档 §8.4）。调用方用 key 在阶段 / 状态变化时重置打回草稿。 */
export function GateBarContainer({
  issueId,
  view,
}: {
  issueId: string;
  view: IssuePipelineView;
}) {
  const { t } = useTranslation('common');
  const gate = useGateDecision();
  const [rejectDraft, setRejectDraft] = useState<string | null>(null);
  const state = gateBarState(view);

  if (state.kind === 'none') return null;

  const stage =
    'stageKey' in state ? t(pipelineStageLabelKey(state.stageKey)) : '';
  const round =
    'attempt' in state && state.maxRounds !== null
      ? t('pipeline.gate.round', {
          attempt: state.attempt,
          max: state.maxRounds,
        })
      : null;

  let title: string;
  let message: string;
  switch (state.kind) {
    case 'human':
      title = state.gateLabel ?? t(fallbackGateLabelKey(state.stageKey));
      message = t('pipeline.status.waitingGate', { gate: title });
      break;
    case 'auto': {
      // 判定条件取模板的 gate_condition（契约 C10），不按阶段 key 猜。
      const conditionKey = autoConditionLabelKey(state.gateCondition);
      title = t('pipeline.gate.autoGate');
      message = [conditionKey ? t(conditionKey) : stage, round]
        .filter(Boolean)
        .join(' · ');
      break;
    }
    case 'running':
      title = stage;
      message = t('pipeline.gate.runningHint', { stage });
      break;
    case 'paused':
      if (state.pendingGate) {
        // 暂停中仍可对待决人工关卡做决策（契约 C12）。
        title =
          state.pendingGate.gateLabel ??
          t(fallbackGateLabelKey(state.stageKey));
        message = [
          t('pipeline.gate.pausedHint'),
          t('pipeline.status.waitingGate', { gate: title }),
        ].join(' · ');
      } else {
        title = t('pipeline.status.paused');
        message = t('pipeline.gate.pausedHint');
      }
      break;
    case 'failed':
      title = t('pipeline.status.failed');
      // 失败原因含日志末尾，可能有几十行；关卡条只放第一行（认出已知接口报错时
      // 就是那句中文说明），完整原文在右栏时间线里。
      message = [
        t('pipeline.gate.failedHint'),
        round,
        firstErrorLine(state.error),
      ]
        .filter(Boolean)
        .join(' · ');
      break;
    case 'completed':
      title = t('pipeline.status.completed');
      message = t('pipeline.gate.completedHint');
      break;
    case 'cancelled':
      title = t('pipeline.status.cancelled');
      message = t('pipeline.gate.cancelledHint');
      break;
  }

  const decisionStageRunId =
    state.kind === 'human'
      ? state.stageRunId
      : state.kind === 'paused'
        ? (state.pendingGate?.stageRunId ?? null)
        : null;

  const decide = (decision: 'approve' | 'reject', comment: string | null) => {
    if (!decisionStageRunId) return;
    gate.mutate(
      {
        issueId,
        stageRunId: decisionStageRunId,
        request: { decision, comment },
      },
      { onSuccess: () => setRejectDraft(null) }
    );
  };

  return (
    <PipelineGateBar
      kind={state.kind}
      title={title}
      message={message}
      onConfirm={() => decide('approve', null)}
      onRejectSubmit={(comment) => {
        if (comment) decide('reject', comment);
      }}
      rejectDraft={rejectDraft}
      onRejectDraftChange={setRejectDraft}
      isBusy={gate.isPending}
      canDecide={decisionStageRunId !== null}
      error={
        gate.error
          ? t('pipeline.gate.error', { message: gate.error.message })
          : null
      }
    />
  );
}
