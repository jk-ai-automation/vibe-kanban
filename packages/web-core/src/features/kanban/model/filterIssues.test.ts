import { describe, expect, it } from 'vitest';
import {
  filterIssues,
  groupAssigneesByIssue,
  groupTagsByIssue,
  PRIORITY_ORDER,
  type FilterIssuesParams,
} from './filterIssues';
import {
  DEFAULT_KANBAN_FILTER_STATE,
  KANBAN_ASSIGNEE_FILTER_VALUES,
  type KanbanFilterState,
} from '@/shared/stores/useUiPreferencesStore';
import type {
  Issue,
  IssuePriority,
  IssueRelationship,
} from 'shared/remote-types';

function 建需求(overrides: Partial<Issue> & { id: string }): Issue {
  return {
    project_id: 'p1',
    issue_number: 1,
    simple_id: 'VK-1',
    status_id: 'todo',
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

function 建参数(
  overrides: Partial<FilterIssuesParams> & { issues: Issue[] }
): FilterIssuesParams {
  const issues = overrides.issues;
  return {
    assigneesByIssue: {},
    tagsByIssue: {},
    issueRelationships: [],
    issuesById: new Map(issues.map((i) => [i.id, i])),
    doneStatusIds: new Set<string>(),
    filters: DEFAULT_KANBAN_FILTER_STATE,
    showSubIssues: true,
    hideBlocked: false,
    currentUserId: null,
    ...overrides,
  };
}

function 筛选(filters: Partial<KanbanFilterState>): KanbanFilterState {
  return { ...DEFAULT_KANBAN_FILTER_STATE, ...filters };
}

const ids = (issues: Issue[]) => issues.map((i) => i.id);

describe('filterIssues 子需求', () => {
  const issues = [
    建需求({ id: 'a' }),
    建需求({ id: 'b', parent_issue_id: 'a' }),
  ];

  it('不显示子需求时过滤掉 parent_issue_id 非空的', () => {
    const result = filterIssues(建参数({ issues, showSubIssues: false }));
    expect(ids(result)).toEqual(['a']);
  });

  it('显示子需求时全部保留', () => {
    const result = filterIssues(建参数({ issues, showSubIssues: true }));
    expect(ids(result)).toEqual(['a', 'b']);
  });
});

describe('filterIssues 关键字', () => {
  const issues = [
    建需求({
      id: 'a',
      title: '登录页改造',
      simple_id: 'VK-11',
      issue_number: 11,
    }),
    建需求({ id: 'b', title: 'Fix crash', simple_id: 'VK-2', issue_number: 2 }),
    建需求({ id: 'c', title: '无关', simple_id: 'ABC-9', issue_number: 9 }),
  ];

  it('匹配标题', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: '登录' }) })
    );
    expect(ids(result)).toEqual(['a']);
  });

  it('匹配 simple_id', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: 'ABC-9' }) })
    );
    expect(ids(result)).toEqual(['c']);
  });

  it('匹配 issue_number', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: '11' }) })
    );
    expect(ids(result)).toEqual(['a']);
  });

  it('大小写不敏感', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: 'FIX CRASH' }) })
    );
    expect(ids(result)).toEqual(['b']);
  });

  it('首尾空白被裁掉', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: '   登录   ' }) })
    );
    expect(ids(result)).toEqual(['a']);
  });

  it('全是空白等于没有关键字', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ searchQuery: '    ' }) })
    );
    expect(ids(result)).toEqual(['a', 'b', 'c']);
  });
});

describe('filterIssues 优先级', () => {
  const issues = [
    建需求({ id: 'a', priority: 'urgent' }),
    建需求({ id: 'b', priority: 'low' }),
    建需求({ id: 'c', priority: null }),
  ];

  it('组内是 OR', () => {
    const result = filterIssues(
      建参数({
        issues,
        filters: 筛选({ priorities: ['urgent', 'low'] as IssuePriority[] }),
      })
    );
    expect(ids(result)).toEqual(['a', 'b']);
  });

  it('没有优先级的需求不会被选中', () => {
    const result = filterIssues(
      建参数({ issues, filters: 筛选({ priorities: ['urgent'] }) })
    );
    expect(ids(result)).toEqual(['a']);
  });

  it('空优先级筛选不过滤', () => {
    const result = filterIssues(建参数({ issues }));
    expect(ids(result)).toEqual(['a', 'b', 'c']);
  });
});

describe('filterIssues 负责人', () => {
  const issues = [
    建需求({ id: 'a' }),
    建需求({ id: 'b' }),
    建需求({ id: 'c' }),
  ];
  const assigneesByIssue = { a: ['u1'], b: ['u2', 'u3'] };

  it('按用户 id 匹配，组内 OR', () => {
    const result = filterIssues(
      建参数({
        issues,
        assigneesByIssue,
        filters: 筛选({ assigneeIds: ['u1', 'u3'] }),
      })
    );
    expect(ids(result)).toEqual(['a', 'b']);
  });

  it('unassigned 匹配没有负责人的需求', () => {
    const result = filterIssues(
      建参数({
        issues,
        assigneesByIssue,
        filters: 筛选({
          assigneeIds: [KANBAN_ASSIGNEE_FILTER_VALUES.UNASSIGNED],
        }),
      })
    );
    expect(ids(result)).toEqual(['c']);
  });

  it('unassigned 与具体用户可以并存', () => {
    const result = filterIssues(
      建参数({
        issues,
        assigneesByIssue,
        filters: 筛选({
          assigneeIds: [KANBAN_ASSIGNEE_FILTER_VALUES.UNASSIGNED, 'u1'],
        }),
      })
    );
    expect(ids(result)).toEqual(['a', 'c']);
  });

  it('__self__ 展开成当前用户', () => {
    const result = filterIssues(
      建参数({
        issues,
        assigneesByIssue,
        currentUserId: 'u2',
        filters: 筛选({ assigneeIds: [KANBAN_ASSIGNEE_FILTER_VALUES.SELF] }),
      })
    );
    expect(ids(result)).toEqual(['b']);
  });

  it('currentUserId 为 null 时 __self__ 匹配不到任何人', () => {
    const result = filterIssues(
      建参数({
        issues,
        assigneesByIssue,
        currentUserId: null,
        filters: 筛选({ assigneeIds: [KANBAN_ASSIGNEE_FILTER_VALUES.SELF] }),
      })
    );
    expect(ids(result)).toEqual([]);
  });
});

describe('filterIssues 标签', () => {
  const issues = [
    建需求({ id: 'a' }),
    建需求({ id: 'b' }),
    建需求({ id: 'c' }),
  ];
  const tagsByIssue = { a: ['t1'], b: ['t2', 't3'] };

  it('组内是 OR', () => {
    const result = filterIssues(
      建参数({ issues, tagsByIssue, filters: 筛选({ tagIds: ['t1', 't3'] }) })
    );
    expect(ids(result)).toEqual(['a', 'b']);
  });

  it('没有标签的需求不会被选中', () => {
    const result = filterIssues(
      建参数({ issues, tagsByIssue, filters: 筛选({ tagIds: ['t1'] }) })
    );
    expect(ids(result)).toEqual(['a']);
  });
});

describe('filterIssues 隐藏被阻塞', () => {
  const blocker = 建需求({ id: 'blocker', status_id: 'dev' });
  const doneBlocker = 建需求({ id: 'doneBlocker', status_id: 'done' });
  const blocked = 建需求({ id: 'blocked' });
  const blockedByDone = 建需求({ id: 'blockedByDone' });
  const free = 建需求({ id: 'free' });
  const issues = [blocker, doneBlocker, blocked, blockedByDone, free];

  const relationship = (
    id: string,
    issueId: string,
    relatedIssueId: string
  ): IssueRelationship => ({
    id,
    issue_id: issueId,
    related_issue_id: relatedIssueId,
    relationship_type: 'blocking',
    created_at: '2026-01-01T00:00:00Z',
  });

  const issueRelationships = [
    relationship('r1', 'blocker', 'blocked'),
    relationship('r2', 'doneBlocker', 'blockedByDone'),
  ];

  it('过滤掉被未完成需求阻塞的', () => {
    const result = filterIssues(
      建参数({
        issues,
        issueRelationships,
        doneStatusIds: new Set(['done']),
        hideBlocked: true,
      })
    );
    expect(ids(result)).toEqual([
      'blocker',
      'doneBlocker',
      'blockedByDone',
      'free',
    ]);
  });

  it('阻塞者已完成时不过滤', () => {
    const result = filterIssues(
      建参数({
        issues: [blockedByDone],
        issuesById: new Map([
          ['doneBlocker', doneBlocker],
          ['blockedByDone', blockedByDone],
        ]),
        issueRelationships,
        doneStatusIds: new Set(['done']),
        hideBlocked: true,
      })
    );
    expect(ids(result)).toEqual(['blockedByDone']);
  });

  it('阻塞者不在 issuesById 里时不过滤', () => {
    const result = filterIssues(
      建参数({
        issues: [blocked],
        issuesById: new Map([['blocked', blocked]]),
        issueRelationships,
        doneStatusIds: new Set(['done']),
        hideBlocked: true,
      })
    );
    expect(ids(result)).toEqual(['blocked']);
  });

  it('hideBlocked 关闭时完全不看关系', () => {
    const result = filterIssues(
      建参数({
        issues,
        issueRelationships,
        doneStatusIds: new Set(['done']),
        hideBlocked: false,
      })
    );
    expect(result).toHaveLength(issues.length);
  });

  it('非 blocking 关系不参与', () => {
    const result = filterIssues(
      建参数({
        issues: [blocker, blocked],
        issuesById: new Map([
          ['blocker', blocker],
          ['blocked', blocked],
        ]),
        issueRelationships: [
          {
            ...relationship('r3', 'blocker', 'blocked'),
            relationship_type: 'related',
          },
        ],
        doneStatusIds: new Set(['done']),
        hideBlocked: true,
      })
    );
    expect(ids(result)).toEqual(['blocker', 'blocked']);
  });
});

describe('filterIssues 不变量', () => {
  it('空筛选返回原数组内容', () => {
    const issues = [建需求({ id: 'a' }), 建需求({ id: 'b' })];
    const result = filterIssues(建参数({ issues }));
    expect(ids(result)).toEqual(['a', 'b']);
  });

  it('筛选函数不修改入参数组', () => {
    const issues = [
      建需求({ id: 'a', priority: 'urgent' }),
      建需求({ id: 'b', priority: 'low' }),
    ];
    const snapshot = [...issues];
    const params = 建参数({
      issues,
      filters: 筛选({ priorities: ['urgent'], searchQuery: 'Title' }),
      showSubIssues: false,
      hideBlocked: true,
    });
    const assigneesSnapshot = { ...params.assigneesByIssue };

    filterIssues(params);

    expect(issues).toHaveLength(2);
    expect(issues).toEqual(snapshot);
    expect(params.assigneesByIssue).toEqual(assigneesSnapshot);
  });

  it('多个筛选条件是 AND 关系', () => {
    const issues = [
      建需求({ id: 'a', title: '登录', priority: 'urgent' }),
      建需求({ id: 'b', title: '登录', priority: 'low' }),
      建需求({ id: 'c', title: '注册', priority: 'urgent' }),
    ];
    const result = filterIssues(
      建参数({
        issues,
        filters: 筛选({ searchQuery: '登录', priorities: ['urgent'] }),
      })
    );
    expect(ids(result)).toEqual(['a']);
  });
});

describe('分组辅助函数', () => {
  it('groupAssigneesByIssue 把同一需求的负责人聚到一起', () => {
    expect(
      groupAssigneesByIssue([
        { issue_id: 'a', user_id: 'u1' },
        { issue_id: 'a', user_id: 'u2' },
        { issue_id: 'b', user_id: 'u1' },
      ])
    ).toEqual({ a: ['u1', 'u2'], b: ['u1'] });
  });

  it('groupTagsByIssue 把同一需求的标签聚到一起', () => {
    expect(
      groupTagsByIssue([
        { issue_id: 'a', tag_id: 't1' },
        { issue_id: 'b', tag_id: 't2' },
        { issue_id: 'a', tag_id: 't3' },
      ])
    ).toEqual({ a: ['t1', 't3'], b: ['t2'] });
  });

  it('空输入得到空对象', () => {
    expect(groupAssigneesByIssue([])).toEqual({});
    expect(groupTagsByIssue([])).toEqual({});
  });
});

describe('PRIORITY_ORDER', () => {
  it('urgent < high < medium < low', () => {
    expect(PRIORITY_ORDER.urgent).toBeLessThan(PRIORITY_ORDER.high);
    expect(PRIORITY_ORDER.high).toBeLessThan(PRIORITY_ORDER.medium);
    expect(PRIORITY_ORDER.medium).toBeLessThan(PRIORITY_ORDER.low);
  });
});
