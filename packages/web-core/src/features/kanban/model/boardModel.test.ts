import { describe, expect, it } from 'vitest';
import {
  buildBoardColumns,
  groupIssueIdsByStatus,
  selectVisibleStatuses,
  sortStatusesByOrder,
} from './boardModel';
import type { Issue, ProjectStatus } from 'shared/remote-types';
import type { StatusWithStage } from './stageType';

function 建状态列(
  overrides: Partial<StatusWithStage> & { id: string }
): StatusWithStage {
  return {
    project_id: 'p1',
    name: 'Column',
    color: '0 0% 50%',
    sort_order: 0,
    hidden: false,
    created_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as StatusWithStage;
}

function 建需求(overrides: Partial<Issue> & { id: string }): Issue {
  return {
    project_id: 'p1',
    issue_number: 1,
    simple_id: 'VK-1',
    status_id: 's1',
    title: 'Title',
    description: null,
    priority: null,
    start_date: null,
    target_date: null,
    completed_at: null,
    sort_order: 0,
    parent_issue_id: null,
    parent_issue_sort_order: null,
    extension_metadata: {},
    creator_user_id: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

describe('sortStatusesByOrder', () => {
  it('按 sort_order 升序', () => {
    const statuses = [
      建状态列({ id: 'c', sort_order: 2 }),
      建状态列({ id: 'a', sort_order: 0 }),
      建状态列({ id: 'b', sort_order: 1 }),
    ];
    expect(sortStatusesByOrder(statuses).map((s) => s.id)).toEqual([
      'a',
      'b',
      'c',
    ]);
  });

  it('不修改入参数组', () => {
    const statuses = [
      建状态列({ id: 'c', sort_order: 2 }),
      建状态列({ id: 'a', sort_order: 0 }),
    ];
    sortStatusesByOrder(statuses);
    expect(statuses.map((s) => s.id)).toEqual(['c', 'a']);
  });
});

describe('selectVisibleStatuses', () => {
  it('隐藏列不出现，其余按 sort_order 升序', () => {
    const statuses = [
      建状态列({ id: 'done', sort_order: 5 }),
      建状态列({ id: 'cancelled', sort_order: 6, hidden: true }),
      建状态列({ id: 'backlog', sort_order: 0 }),
    ];
    expect(selectVisibleStatuses(statuses).map((s) => s.id)).toEqual([
      'backlog',
      'done',
    ]);
  });

  it('全部隐藏时返回空数组', () => {
    const statuses = [建状态列({ id: 'a', hidden: true })];
    expect(selectVisibleStatuses(statuses)).toEqual([]);
  });
});

describe('groupIssueIdsByStatus', () => {
  const statuses = [
    建状态列({ id: 's1', sort_order: 0 }),
    建状态列({ id: 's2', sort_order: 1 }),
  ];

  it('每个状态列都有一条记录，没有需求时是空数组', () => {
    const grouped = groupIssueIdsByStatus(statuses, [], 'sort_order', 'asc');
    expect(grouped).toEqual({ s1: [], s2: [] });
  });

  it('按 status_id 分组', () => {
    const issues = [
      建需求({ id: 'a', status_id: 's1' }),
      建需求({ id: 'b', status_id: 's2' }),
      建需求({ id: 'c', status_id: 's1' }),
    ];
    const grouped = groupIssueIdsByStatus(
      statuses,
      issues,
      'sort_order',
      'asc'
    );
    expect(grouped.s1).toEqual(['a', 'c']);
    expect(grouped.s2).toEqual(['b']);
  });

  it('不属于任何状态列的需求被丢弃', () => {
    const issues = [建需求({ id: 'x', status_id: '不存在' })];
    const grouped = groupIssueIdsByStatus(
      statuses,
      issues,
      'sort_order',
      'asc'
    );
    expect(grouped).toEqual({ s1: [], s2: [] });
  });

  it('sort_order 升序', () => {
    const issues = [
      建需求({ id: 'b', sort_order: 2 }),
      建需求({ id: 'a', sort_order: 1 }),
    ];
    expect(
      groupIssueIdsByStatus(statuses, issues, 'sort_order', 'asc').s1
    ).toEqual(['a', 'b']);
  });

  it('降序把比较结果取反', () => {
    const issues = [
      建需求({ id: 'a', sort_order: 1 }),
      建需求({ id: 'b', sort_order: 2 }),
    ];
    expect(
      groupIssueIdsByStatus(statuses, issues, 'sort_order', 'desc').s1
    ).toEqual(['b', 'a']);
  });

  it('优先级排序：urgent 在前，没有优先级的排最后', () => {
    const issues = [
      建需求({ id: 'low', priority: 'low' }),
      建需求({ id: 'none', priority: null }),
      建需求({ id: 'urgent', priority: 'urgent' }),
    ];
    expect(
      groupIssueIdsByStatus(statuses, issues, 'priority', 'asc').s1
    ).toEqual(['urgent', 'low', 'none']);
  });

  it('按创建时间排序', () => {
    const issues = [
      建需求({ id: 'new', created_at: '2026-03-01T00:00:00Z' }),
      建需求({ id: 'old', created_at: '2026-01-01T00:00:00Z' }),
    ];
    expect(
      groupIssueIdsByStatus(statuses, issues, 'created_at', 'asc').s1
    ).toEqual(['old', 'new']);
  });

  it('按更新时间排序', () => {
    const issues = [
      建需求({ id: 'new', updated_at: '2026-03-01T00:00:00Z' }),
      建需求({ id: 'old', updated_at: '2026-01-01T00:00:00Z' }),
    ];
    expect(
      groupIssueIdsByStatus(statuses, issues, 'updated_at', 'asc').s1
    ).toEqual(['old', 'new']);
  });

  it('按标题排序', () => {
    const issues = [
      建需求({ id: 'b', title: 'Beta' }),
      建需求({ id: 'a', title: 'Alpha' }),
    ];
    expect(groupIssueIdsByStatus(statuses, issues, 'title', 'asc').s1).toEqual([
      'a',
      'b',
    ]);
  });

  it('不修改入参数组', () => {
    const issues = [
      建需求({ id: 'b', sort_order: 2 }),
      建需求({ id: 'a', sort_order: 1 }),
    ];
    const 之前 = issues.map((i) => i.id);
    groupIssueIdsByStatus(statuses, issues, 'sort_order', 'asc');
    expect(issues.map((i) => i.id)).toEqual(之前);
  });
});

describe('buildBoardColumns', () => {
  it('列顺序跟随传入的状态列顺序', () => {
    const columns = buildBoardColumns(
      [
        建状态列({ id: 's1', name: '待规划', stage_type: 'backlog' }),
        建状态列({ id: 's2', name: '测试中', stage_type: 'test' }),
      ],
      { s1: ['a'], s2: ['b', 'c'] }
    );
    expect(columns.map((c) => c.status.id)).toEqual(['s1', 's2']);
  });

  it('带出流程阶段', () => {
    const columns = buildBoardColumns(
      [建状态列({ id: 's1', stage_type: 'test' })],
      {}
    );
    expect(columns[0].stage).toBe('test');
  });

  it('缺 stage_type（团队版 remote 数据源）时回落到 todo', () => {
    const remoteStatus: ProjectStatus = {
      id: 's1',
      project_id: 'p1',
      name: 'Todo',
      color: '0 0% 50%',
      sort_order: 0,
      hidden: false,
      created_at: '2026-01-01T00:00:00Z',
    };
    const columns = buildBoardColumns([remoteStatus], {});
    expect(columns[0].stage).toBe('todo');
  });

  it('每列的卡片顺序与计数来自分组结果', () => {
    const columns = buildBoardColumns([建状态列({ id: 's1' })], {
      s1: ['a', 'b', 'c'],
    });
    expect(columns[0].issueIds).toEqual(['a', 'b', 'c']);
    expect(columns[0].count).toBe(3);
  });

  it('分组结果里没有该状态列时是空列', () => {
    const columns = buildBoardColumns([建状态列({ id: 's1' })], {});
    expect(columns[0].issueIds).toEqual([]);
    expect(columns[0].count).toBe(0);
  });

  it('没有可见状态列时返回空数组', () => {
    expect(buildBoardColumns([], { s1: ['a'] })).toEqual([]);
  });
});

describe('分组 + 建列 串起来', () => {
  it('隐藏列不出现在看板上，但它的需求也不会串到别的列', () => {
    const all = [
      建状态列({ id: 'todo', sort_order: 0, stage_type: 'todo' }),
      建状态列({ id: 'done', sort_order: 1, stage_type: 'done' }),
      建状态列({
        id: 'cancelled',
        sort_order: 2,
        hidden: true,
        stage_type: 'done',
      }),
    ];
    const issues = [
      建需求({ id: 'a', status_id: 'todo', sort_order: 1 }),
      建需求({ id: 'b', status_id: 'cancelled' }),
      建需求({ id: 'c', status_id: 'todo', sort_order: 0 }),
    ];

    const items = groupIssueIdsByStatus(all, issues, 'sort_order', 'asc');
    const columns = buildBoardColumns(selectVisibleStatuses(all), items);

    expect(columns.map((c) => c.status.id)).toEqual(['todo', 'done']);
    expect(columns[0].issueIds).toEqual(['c', 'a']);
    expect(columns[1].issueIds).toEqual([]);
    // 隐藏列的分组仍然算出来了（列表视图要用）
    expect(items.cancelled).toEqual(['b']);
  });
});
