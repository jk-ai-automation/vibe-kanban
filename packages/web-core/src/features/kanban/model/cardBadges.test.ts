import { describe, expect, it } from 'vitest';
import {
  buildPrBadge,
  buildTestBadge,
  buildWorkspaceBadge,
  shouldShowAssignees,
  shouldShowStageBadges,
} from './cardBadges';
import type { StageType } from './stageType';

describe('buildWorkspaceBadge（工作区运行状态）', () => {
  it('没有工作区返回 null（不留空占位）', () => {
    expect(buildWorkspaceBadge([])).toBeNull();
    expect(buildWorkspaceBadge(null)).toBeNull();
    expect(buildWorkspaceBadge(undefined)).toBeNull();
  });

  it('有正在跑的工作区 → running', () => {
    const badge = buildWorkspaceBadge([{ isRunning: true }]);
    expect(badge?.tone).toBe('running');
    expect(badge?.labelKey).toBe('kanban.badge.workspaceRunning');
  });

  it('latestProcessStatus 为 running 也算 running', () => {
    expect(
      buildWorkspaceBadge([{ latestProcessStatus: 'running' }])?.tone
    ).toBe('running');
  });

  it('全部停了 → stopped', () => {
    const badge = buildWorkspaceBadge([
      { isRunning: false, latestProcessStatus: 'completed' },
      { isRunning: false, latestProcessStatus: 'killed' },
    ]);
    expect(badge?.tone).toBe('stopped');
    expect(badge?.labelKey).toBe('kanban.badge.workspaceStopped');
  });

  it('有失败的 → failed，且优先级高于 running', () => {
    const badge = buildWorkspaceBadge([
      { isRunning: true },
      { latestProcessStatus: 'failed' },
    ]);
    expect(badge?.tone).toBe('failed');
    expect(badge?.labelKey).toBe('kanban.badge.workspaceFailed');
  });

  it('count 是工作区个数', () => {
    expect(buildWorkspaceBadge([{}, {}, {}])?.count).toBe(3);
  });

  it('字段全缺失时按 stopped 处理，不抛错', () => {
    expect(buildWorkspaceBadge([{}])?.tone).toBe('stopped');
  });
});

describe('buildPrBadge（PR 状态）', () => {
  it('没有 PR 返回 null', () => {
    expect(buildPrBadge([])).toBeNull();
    expect(buildPrBadge(undefined)).toBeNull();
  });

  it('单个 PR 按自身状态', () => {
    expect(buildPrBadge([{ status: 'open' }])?.tone).toBe('open');
    expect(buildPrBadge([{ status: 'merged' }])?.tone).toBe('merged');
    expect(buildPrBadge([{ status: 'closed' }])?.tone).toBe('closed');
  });

  it('labelKey 是字面量 key', () => {
    expect(buildPrBadge([{ status: 'merged' }])?.labelKey).toBe(
      'kanban.badge.prMerged'
    );
    expect(buildPrBadge([{ status: 'open' }])?.labelKey).toBe(
      'kanban.badge.prOpen'
    );
    expect(buildPrBadge([{ status: 'closed' }])?.labelKey).toBe(
      'kanban.badge.prClosed'
    );
  });

  it('多个 PR 取最靠后的状态：merged > open', () => {
    expect(buildPrBadge([{ status: 'open' }, { status: 'merged' }])?.tone).toBe(
      'merged'
    );
  });

  it('多个 PR 取最靠后的状态：open > closed', () => {
    expect(buildPrBadge([{ status: 'closed' }, { status: 'open' }])?.tone).toBe(
      'open'
    );
  });

  it('全部关闭时才是 closed', () => {
    expect(
      buildPrBadge([{ status: 'closed' }, { status: 'closed' }])?.tone
    ).toBe('closed');
  });

  it('count 是 PR 个数', () => {
    expect(
      buildPrBadge([{ status: 'open' }, { status: 'merged' }])?.count
    ).toBe(2);
  });

  it('无法识别的状态被忽略，不抛错', () => {
    const prs = [{ status: 'weird' as unknown as 'open' }];
    expect(buildPrBadge(prs)?.tone).toBe('closed');
  });
});

describe('buildTestBadge（测试结果占位）', () => {
  it('本期恒为「尚未接入」', () => {
    expect(buildTestBadge()).toEqual({
      tone: 'pending',
      labelKey: 'kanban.badge.testNotConnected',
      count: 0,
    });
  });
});

describe('shouldShowAssignees（个人版隐藏头像）', () => {
  it('个人版不显示', () => {
    expect(shouldShowAssignees(true)).toBe(false);
  });

  it('团队版 / 云端显示', () => {
    expect(shouldShowAssignees(false)).toBe(true);
  });
});

describe('shouldShowStageBadges（泳道阶段徽标）', () => {
  const stages = (...values: StageType[]) => values;

  it('阶段各不相同时显示', () => {
    expect(shouldShowStageBadges(stages('backlog', 'todo', 'dev'))).toBe(true);
  });

  it('团队版所有列都回落成 todo 时不显示（否则是六个一样的徽标）', () => {
    expect(
      shouldShowStageBadges(stages('todo', 'todo', 'todo', 'todo', 'todo'))
    ).toBe(false);
  });

  it('只有一列时不显示', () => {
    expect(shouldShowStageBadges(stages('dev'))).toBe(false);
  });

  it('空数组 / 缺失不抛错', () => {
    expect(shouldShowStageBadges([])).toBe(false);
    expect(shouldShowStageBadges(null)).toBe(false);
    expect(shouldShowStageBadges(undefined)).toBe(false);
  });

  it('只要有一列不同就显示', () => {
    expect(shouldShowStageBadges(stages('todo', 'todo', 'done'))).toBe(true);
  });
});
