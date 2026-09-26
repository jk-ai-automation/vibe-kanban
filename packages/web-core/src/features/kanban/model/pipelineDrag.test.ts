import { describe, expect, it } from 'vitest';
import { canDropIssue } from './pipelineDrag';

describe('canDropIssue（设计文档 §8.3：只允许往回拖）', () => {
  it('同列内调整顺序永远允许', () => {
    expect(
      canDropIssue({ fromStage: 'dev', toStage: 'dev', runStatus: 'running' })
    ).toEqual({ allowed: true });
  });

  it('没有流水线的手工需求：前后都能拖', () => {
    expect(
      canDropIssue({ fromStage: 'todo', toStage: 'dev', runStatus: null })
    ).toEqual({ allowed: true });
    expect(
      canDropIssue({ fromStage: 'dev', toStage: 'todo', runStatus: null })
    ).toEqual({ allowed: true });
  });

  it('流水线在跑：任何跨列都拦，提示先暂停', () => {
    for (const runStatus of ['running', 'waiting_gate'] as const) {
      expect(
        canDropIssue({ fromStage: 'test', toStage: 'dev', runStatus })
      ).toEqual({ allowed: false, reasonKey: 'pipeline.drag.pauseFirst' });
    }
  });

  it('流水线已停：往回允许，往后拦', () => {
    for (const runStatus of [
      'paused',
      'failed',
      'completed',
      'cancelled',
    ] as const) {
      expect(
        canDropIssue({ fromStage: 'review', toStage: 'dev', runStatus })
      ).toEqual({ allowed: true });
      expect(
        canDropIssue({ fromStage: 'review', toStage: 'test', runStatus })
      ).toEqual({ allowed: false, reasonKey: 'pipeline.drag.forwardBlocked' });
    }
  });
});
