import { describe, expect, it } from 'vitest';
import {
  filtersToSearch,
  hasKanbanUrlFilters,
  isSameKanbanSearch,
  KANBAN_SEARCH_MAX_ARRAY_LEN,
  KANBAN_SEARCH_MAX_QUERY_LEN,
  mergeKanbanSearch,
  searchToFilters,
} from './kanbanUrlState';
import {
  DEFAULT_KANBAN_FILTER_STATE,
  KANBAN_ASSIGNEE_FILTER_VALUES,
  type KanbanFilterState,
} from '@/shared/stores/useUiPreferencesStore';

const 默认 = DEFAULT_KANBAN_FILTER_STATE;

function 筛选(overrides: Partial<KanbanFilterState>): KanbanFilterState {
  return { ...默认, ...overrides };
}

/** personal 视图的默认筛选（负责人 = 我 + 按优先级排） */
const personal默认 = 筛选({
  assigneeIds: [KANBAN_ASSIGNEE_FILTER_VALUES.SELF],
  sortField: 'priority',
});

describe('filtersToSearch', () => {
  it('默认筛选输出空对象，URL 保持干净', () => {
    expect(filtersToSearch(默认)).toEqual({});
  });

  it('只输出非默认值', () => {
    expect(filtersToSearch(筛选({ searchQuery: '登录' }))).toEqual({
      q: '登录',
    });
  });

  it('数组用逗号分隔', () => {
    expect(filtersToSearch(筛选({ priorities: ['urgent', 'high'] }))).toEqual({
      priority: 'urgent,high',
    });
  });

  it('排序字段与方向分成两个参数', () => {
    expect(
      filtersToSearch(筛选({ sortField: 'title', sortDirection: 'desc' }))
    ).toEqual({ sort: 'title', dir: 'desc' });
  });

  it('personal 视图的预设筛选不污染 URL', () => {
    expect(filtersToSearch(personal默认, personal默认)).toEqual({});
  });

  it('personal 视图下改成别的负责人才会写进 URL', () => {
    expect(
      filtersToSearch(
        筛选({ assigneeIds: ['u1'], sortField: 'priority' }),
        personal默认
      )
    ).toEqual({ assignee: 'u1' });
  });

  it('超长 q 被截断到 200 字符', () => {
    const long = 'a'.repeat(500);
    const search = filtersToSearch(筛选({ searchQuery: long }));
    expect(search.q).toHaveLength(KANBAN_SEARCH_MAX_QUERY_LEN);
  });

  it('数组元素个数有上限', () => {
    const many = Array.from({ length: 200 }, (_, i) => `t${i}`);
    const search = filtersToSearch(筛选({ tagIds: many }));
    expect(search.tag?.split(',')).toHaveLength(KANBAN_SEARCH_MAX_ARRAY_LEN);
  });

  it('清空成空数组时不写 key（与默认一致）', () => {
    expect(filtersToSearch(筛选({ priorities: [] }))).toEqual({});
  });

  it('默认非空却被显式清空时写哨兵值，否则读回来会被补成默认', () => {
    expect(
      filtersToSearch(
        筛选({ assigneeIds: [], sortField: 'priority' }),
        personal默认
      )
    ).toEqual({ assignee: 'none' });
  });
});

describe('searchToFilters', () => {
  it('空 query 补齐默认值', () => {
    expect(searchToFilters({})).toEqual(默认);
  });

  it('null / undefined 也返回默认值而不是抛错', () => {
    expect(searchToFilters(null)).toEqual(默认);
    expect(searchToFilters(undefined)).toEqual(默认);
  });

  it('逗号分隔的数组被拆开', () => {
    expect(searchToFilters({ tag: 't1,t2' }).tagIds).toEqual(['t1', 't2']);
  });

  it('非法 sortField 回落到默认值', () => {
    expect(searchToFilters({ sort: 'DROP TABLE' }).sortField).toBe(
      'sort_order'
    );
    expect(
      searchToFilters({ sort: 'DROP TABLE' }, personal默认).sortField
    ).toBe('priority');
  });

  it('非法 dir 回落到默认值', () => {
    expect(searchToFilters({ dir: '; rm -rf /' }).sortDirection).toBe('asc');
  });

  it('非法 priority 被丢弃而不是抛错', () => {
    expect(searchToFilters({ priority: 'urgent,nope' }).priorities).toEqual([
      'urgent',
    ]);
  });

  it('全是非法 priority 时得到空数组而不是默认值', () => {
    expect(searchToFilters({ priority: 'nope' }).priorities).toEqual([]);
  });

  it('assigneeIds 里的 __self__ 与 unassigned 原样保留', () => {
    expect(
      searchToFilters({ assignee: '__self__,unassigned,u1' }).assigneeIds
    ).toEqual(['__self__', 'unassigned', 'u1']);
  });

  it('超长 q 被截断到 200 字符', () => {
    expect(searchToFilters({ q: 'a'.repeat(500) }).searchQuery).toHaveLength(
      KANBAN_SEARCH_MAX_QUERY_LEN
    );
  });

  it('空字符串数组参数等于清空，而不是回落默认值', () => {
    expect(searchToFilters({ assignee: '' }, personal默认).assigneeIds).toEqual(
      []
    );
  });

  it('哨兵值 none 读成空数组', () => {
    expect(
      searchToFilters({ assignee: 'none' }, personal默认).assigneeIds
    ).toEqual([]);
    expect(searchToFilters({ priority: 'none' }).priorities).toEqual([]);
  });

  it('数组里的空段被忽略', () => {
    expect(searchToFilters({ tag: 't1,,,t2,' }).tagIds).toEqual(['t1', 't2']);
  });

  it('过长的数组元素被丢弃', () => {
    const 正常 = 'ok';
    const 超长 = 'x'.repeat(500);
    expect(searchToFilters({ tag: `${正常},${超长}` }).tagIds).toEqual([正常]);
  });

  it('未知参数被忽略', () => {
    expect(
      searchToFilters({ q: 'x', 乱来: '1' } as Record<string, string>)
    ).toEqual(筛选({ searchQuery: 'x' }));
  });
});

describe('往返一致', () => {
  const 样例: KanbanFilterState[] = [
    默认,
    筛选({ searchQuery: '登录 & 注册' }),
    筛选({ priorities: ['urgent', 'low'] }),
    筛选({ assigneeIds: ['__self__', 'unassigned'] }),
    筛选({ tagIds: ['t1', 't2', 't3'] }),
    筛选({ sortField: 'updated_at', sortDirection: 'desc' }),
    筛选({ assigneeIds: [], sortField: 'priority' }),
    筛选({
      searchQuery: 'bug',
      priorities: ['high'],
      assigneeIds: ['u1'],
      tagIds: ['t9'],
      sortField: 'title',
      sortDirection: 'desc',
    }),
  ];

  it('searchToFilters(filtersToSearch(f)) 等于 f', () => {
    for (const f of 样例) {
      expect(searchToFilters(filtersToSearch(f))).toEqual(f);
    }
  });

  it('personal 视图默认值下同样往返一致', () => {
    for (const f of 样例) {
      expect(
        searchToFilters(filtersToSearch(f, personal默认), personal默认)
      ).toEqual(f);
    }
  });
});

describe('hasKanbanUrlFilters', () => {
  it('没有任何筛选 key 时为 false', () => {
    expect(hasKanbanUrlFilters({})).toBe(false);
    expect(hasKanbanUrlFilters(null)).toBe(false);
    expect(hasKanbanUrlFilters({ 别的: 'x' } as Record<string, string>)).toBe(
      false
    );
  });

  it('空字符串不算', () => {
    expect(hasKanbanUrlFilters({ q: '' })).toBe(false);
  });

  it('任一筛选 key 非空即为 true', () => {
    expect(hasKanbanUrlFilters({ dir: 'desc' })).toBe(true);
  });
});

describe('isSameKanbanSearch', () => {
  it('只比较筛选 key，别的参数不参与', () => {
    expect(
      isSameKanbanSearch(
        { q: 'a', 别的: '1' } as Record<string, string>,
        { q: 'a', 别的: '2' } as Record<string, string>
      )
    ).toBe(true);
  });

  it('空字符串等同于缺失', () => {
    expect(isSameKanbanSearch({ q: '' }, {})).toBe(true);
  });

  it('值不同则为 false', () => {
    expect(isSameKanbanSearch({ q: 'a' }, { q: 'b' })).toBe(false);
  });
});

describe('mergeKanbanSearch', () => {
  it('剔掉旧的筛选参数再塞新的', () => {
    expect(
      mergeKanbanSearch({ q: '旧', priority: 'low' }, { q: '新' })
    ).toEqual({
      q: '新',
    });
  });

  it('其它路由自己的参数原样保留', () => {
    expect(mergeKanbanSearch({ ref: 'slack', q: '旧' }, { tag: 't1' })).toEqual(
      {
        ref: 'slack',
        tag: 't1',
      }
    );
  });

  it('空筛选把所有筛选参数清掉', () => {
    expect(mergeKanbanSearch({ q: 'x', tag: 't', ref: 'a' }, {})).toEqual({
      ref: 'a',
    });
  });

  it('不修改入参对象', () => {
    const prev = { q: '旧', ref: 'a' };
    mergeKanbanSearch(prev, { q: '新' });
    expect(prev).toEqual({ q: '旧', ref: 'a' });
  });

  it('previous 为 null 时也能用', () => {
    expect(mergeKanbanSearch(null, { q: 'x' })).toEqual({ q: 'x' });
  });
});
