import type {
  PipelineGateDecision,
  PipelineStageKey,
  PipelineStageRun,
} from 'shared/types';
import { toMillis, toNumber } from '@/entities/pipeline/model/progress';

export type TimelineKind =
  | 'stage_started'
  | 'stage_passed'
  | 'stage_failed'
  | 'stage_waiting'
  | 'stage_rejected'
  | 'gate_approved'
  | 'gate_rejected';

export interface TimelineEvent {
  id: string;
  at: number;
  kind: TimelineKind;
  stageKey: PipelineStageKey;
  attempt: number;
  actor: 'ai' | 'human';
  durationMs: number | null;
  /** 阶段摘要 / 错误 / 打回意见，原样展示。 */
  text: string | null;
}

/** 同一时刻的先后：开始 < 结束 < 人工决策。 */
const KIND_ORDER: Record<TimelineKind, number> = {
  stage_started: 0,
  stage_passed: 1,
  stage_failed: 1,
  stage_waiting: 1,
  stage_rejected: 1,
  gate_approved: 2,
  gate_rejected: 2,
};

const END_KIND: Partial<Record<PipelineStageRun['status'], TimelineKind>> = {
  passed: 'stage_passed',
  failed: 'stage_failed',
  waiting_gate: 'stage_waiting',
  rejected: 'stage_rejected',
};

/** 由阶段记录与关卡决策合成时间线（设计文档 §8.4 右栏）。 */
export function buildTimeline(
  stages: readonly PipelineStageRun[],
  decisions: readonly PipelineGateDecision[]
): TimelineEvent[] {
  const stageById = new Map(stages.map((stage) => [stage.id, stage]));
  const rejectedByDecision = new Set(
    decisions
      .filter((decision) => decision.decision === 'reject')
      .map((decision) => decision.stage_run_id)
  );
  const events: TimelineEvent[] = [];

  for (const stage of stages) {
    const attempt = toNumber(stage.attempt) ?? 1;
    const started = toMillis(stage.started_at);
    const finished = toMillis(stage.finished_at);
    if (started !== null) {
      events.push({
        id: `${stage.id}:started`,
        at: started,
        kind: 'stage_started',
        stageKey: stage.stage_key,
        attempt,
        actor: 'ai',
        durationMs: null,
        text: null,
      });
    }

    const endKind = END_KIND[stage.status];
    const endAt = finished ?? started;
    if (!endKind || endAt === null) continue;
    if (endKind === 'stage_rejected' && rejectedByDecision.has(stage.id))
      continue;

    events.push({
      id: `${stage.id}:${endKind}`,
      at: endAt,
      kind: endKind,
      stageKey: stage.stage_key,
      attempt,
      actor: 'ai',
      durationMs:
        started !== null && finished !== null
          ? Math.max(0, finished - started)
          : null,
      text:
        endKind === 'stage_failed'
          ? (stage.error ?? stage.summary)
          : stage.summary,
    });
  }

  for (const decision of decisions) {
    const stage = stageById.get(decision.stage_run_id);
    const decidedAt = toMillis(decision.decided_at);
    if (!stage || decidedAt === null) continue;
    events.push({
      id: decision.id,
      at: decidedAt,
      kind: decision.decision === 'approve' ? 'gate_approved' : 'gate_rejected',
      stageKey: stage.stage_key,
      attempt: toNumber(stage.attempt) ?? 1,
      actor: 'human',
      durationMs: null,
      text: decision.comment,
    });
  }

  return events.sort(
    (a, b) => a.at - b.at || KIND_ORDER[a.kind] - KIND_ORDER[b.kind]
  );
}

const TIMELINE_TEXT_KEYS: Record<TimelineKind, string> = {
  stage_started: 'issueDetail.timeline.stageStarted',
  stage_passed: 'issueDetail.timeline.stagePassed',
  stage_failed: 'issueDetail.timeline.stageFailed',
  stage_waiting: 'issueDetail.timeline.stageWaiting',
  stage_rejected: 'issueDetail.timeline.stageRejected',
  gate_approved: 'issueDetail.timeline.gateApproved',
  gate_rejected: 'issueDetail.timeline.gateRejected',
};

export function timelineTextKey(kind: TimelineKind): string {
  return TIMELINE_TEXT_KEYS[kind];
}
