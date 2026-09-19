import { describe, expect, it } from 'vitest';
import { buildTimeline, timelineTextKey } from './timeline';
import {
  at,
  makeDecision,
  makeStage,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('buildTimeline', () => {
  it('需求被打回后重跑再确认：事件顺序与演员', () => {
    const stages = [
      makeStage({
        id: 'r1',
        attempt: 1,
        status: 'rejected',
        started_at: at(0),
        finished_at: at(2),
        summary: '5 条验收标准',
      }),
      makeStage({
        id: 'r2',
        attempt: 2,
        status: 'passed',
        started_at: at(4),
        finished_at: at(6),
        summary: '补充了边界',
      }),
    ];
    const decisions = [
      makeDecision({
        id: 'd1',
        stage_run_id: 'r1',
        decision: 'reject',
        comment: '缺边界',
        decided_at: at(3),
      }),
      makeDecision({
        id: 'd2',
        stage_run_id: 'r2',
        decision: 'approve',
        decided_at: at(7),
      }),
    ];

    const events = buildTimeline(stages, decisions);
    expect(events.map((e) => [e.kind, e.actor, e.attempt])).toEqual([
      ['stage_started', 'ai', 1],
      ['gate_rejected', 'human', 1],
      ['stage_started', 'ai', 2],
      ['stage_passed', 'ai', 2],
      ['gate_approved', 'human', 2],
    ]);
    expect(events[1].text).toBe('缺边界');
    expect(events[3].durationMs).toBe(2 * 60_000);
    expect(events[3].text).toBe('补充了边界');
  });

  it('没有打回决策的 rejected 阶段仍然出「被打回」', () => {
    const events = buildTimeline(
      [makeStage({ status: 'rejected', finished_at: at(1) })],
      []
    );
    expect(events.map((e) => e.kind)).toEqual([
      'stage_started',
      'stage_rejected',
    ]);
  });

  it('失败事件的说明优先用 error', () => {
    const events = buildTimeline(
      [
        makeStage({
          stage_key: 'review',
          status: 'failed',
          finished_at: at(3),
          error: '3 轮后仍有 blocker',
          summary: '评审完成',
        }),
      ],
      []
    );
    expect(events[1]).toMatchObject({
      kind: 'stage_failed',
      text: '3 轮后仍有 blocker',
    });
  });

  it('等待确认且没有 finished_at：与开始同一时刻，排在开始之后', () => {
    const events = buildTimeline([makeStage({ status: 'waiting_gate' })], []);
    expect(events.map((e) => e.kind)).toEqual([
      'stage_started',
      'stage_waiting',
    ]);
  });

  it('还没开始的阶段与找不到阶段的决策都忽略', () => {
    const events = buildTimeline(
      [makeStage({ status: 'pending', started_at: null })],
      [makeDecision({ stage_run_id: 'nope' })]
    );
    expect(events).toEqual([]);
  });

  it('文案 key 是字面量', () => {
    expect(timelineTextKey('gate_approved')).toBe(
      'issueDetail.timeline.gateApproved'
    );
    expect(timelineTextKey('stage_started')).toBe(
      'issueDetail.timeline.stageStarted'
    );
  });
});

describe('buildTimeline：契约 C11 / C5', () => {
  it('进入等待确认时写了 finished_at：等待事件落在执行结束时刻并带耗时', () => {
    const events = buildTimeline(
      [
        makeStage({
          status: 'waiting_gate',
          started_at: at(0),
          finished_at: at(3),
          summary: '需求写好了',
        }),
      ],
      []
    );
    expect(events[1]).toMatchObject({
      kind: 'stage_waiting',
      at: Date.parse(at(3)),
      durationMs: 3 * 60_000,
      text: '需求写好了',
    });
  });

  it('用户手动停止：失败事件显示停止原因', () => {
    const events = buildTimeline(
      [
        makeStage({
          stage_key: 'develop',
          status: 'failed',
          finished_at: at(2),
          error: '用户手动停止',
        }),
      ],
      []
    );
    expect(events[1]).toMatchObject({
      kind: 'stage_failed',
      text: '用户手动停止',
    });
  });
});
