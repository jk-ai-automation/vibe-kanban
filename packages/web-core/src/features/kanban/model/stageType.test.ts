import { describe, expect, it } from 'vitest';
import {
  compareStages,
  FALLBACK_STAGE,
  isStageType,
  readStageType,
  STAGE_ORDER,
  stageLabelKey,
  type StageType,
} from './stageType';

describe('readStageType', () => {
  it('从带额外字段的对象里取出 stage_type', () => {
    expect(
      readStageType({
        id: 's1',
        name: '测试中',
        stage_type: 'test',
        anything: 1,
      } as unknown as { stage_type?: unknown })
    ).toBe('test');
  });

  it('六个已知阶段都能原样取出', () => {
    const stages: StageType[] = [
      'backlog',
      'todo',
      'dev',
      'review',
      'test',
      'done',
    ];
    for (const stage of stages) {
      expect(readStageType({ stage_type: stage })).toBe(stage);
    }
  });

  it('未知值归一成 todo', () => {
    expect(readStageType({ stage_type: 'qa' })).toBe('todo');
    expect(readStageType({ stage_type: 'TEST' })).toBe('todo');
  });

  it('字段缺失归一成 todo', () => {
    expect(readStageType({})).toBe('todo');
  });

  it('团队版 remote 数据源下 stage_type 一定缺失，函数不能抛错', () => {
    // remote 的 ProjectStatus 形状：没有 stage_type
    const remoteStatus = {
      id: 'a',
      project_id: 'p',
      name: 'Todo',
      color: '0 0% 50%',
      sort_order: 1,
      hidden: false,
      created_at: '2026-01-01T00:00:00Z',
    };
    expect(() => readStageType(remoteStatus)).not.toThrow();
    expect(readStageType(remoteStatus)).toBe('todo');
  });

  it('null / undefined 也不抛错', () => {
    expect(readStageType(null)).toBe('todo');
    expect(readStageType(undefined)).toBe('todo');
  });

  it('非字符串类型不抛错', () => {
    expect(readStageType({ stage_type: 42 })).toBe('todo');
    expect(readStageType({ stage_type: null })).toBe('todo');
    expect(readStageType({ stage_type: { toString: () => 'done' } })).toBe(
      'todo'
    );
  });

  it('回落阶段常量就是 todo', () => {
    expect(FALLBACK_STAGE).toBe('todo');
  });
});

describe('isStageType', () => {
  it('只认这六个字符串', () => {
    expect(isStageType('backlog')).toBe(true);
    expect(isStageType('test')).toBe(true);
    expect(isStageType('qa')).toBe(false);
    expect(isStageType(undefined)).toBe(false);
  });
});

describe('STAGE_ORDER', () => {
  it('顺序是 backlog < todo < dev < review < test < done', () => {
    expect(STAGE_ORDER.backlog).toBeLessThan(STAGE_ORDER.todo);
    expect(STAGE_ORDER.todo).toBeLessThan(STAGE_ORDER.dev);
    expect(STAGE_ORDER.dev).toBeLessThan(STAGE_ORDER.review);
    expect(STAGE_ORDER.review).toBeLessThan(STAGE_ORDER.test);
    expect(STAGE_ORDER.test).toBeLessThan(STAGE_ORDER.done);
  });

  it('compareStages 可以直接用于排序', () => {
    const shuffled: StageType[] = ['done', 'dev', 'backlog', 'test', 'review'];
    expect([...shuffled].sort(compareStages)).toEqual([
      'backlog',
      'dev',
      'review',
      'test',
      'done',
    ]);
  });
});

describe('stageLabelKey', () => {
  it('返回 i18n key 而不是硬编码中文', () => {
    expect(stageLabelKey('test')).toBe('kanban.stage.test');
    expect(stageLabelKey('backlog')).toBe('kanban.stage.backlog');
  });

  it('每个阶段都有唯一的 key', () => {
    const stages = Object.keys(STAGE_ORDER) as StageType[];
    const keys = stages.map(stageLabelKey);
    expect(new Set(keys).size).toBe(stages.length);
    for (const key of keys) {
      expect(key.startsWith('kanban.stage.')).toBe(true);
    }
  });
});
