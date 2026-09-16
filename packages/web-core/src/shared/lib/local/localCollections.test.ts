import { describe, expect, it, vi } from 'vitest';

import {
  buildLocalMutationHandlers,
  patchToWrites,
} from '@/shared/lib/local/localCollections';

describe('patchToWrites', () => {
  it('首帧的 replace 整表变成 truncate + 批量 insert', () => {
    const writes = patchToWrites(
      [
        {
          op: 'replace',
          path: '/issues',
          value: {
            a: { id: 'a', title: 'A' },
            b: { id: 'b', title: 'B' },
          },
        },
      ],
      'issues'
    );

    expect(writes).toEqual([
      { type: 'truncate' },
      { type: 'insert', value: { id: 'a', title: 'A' } },
      { type: 'insert', value: { id: 'b', title: 'B' } },
    ]);
  });

  it('单行 add 变成 insert', () => {
    expect(
      patchToWrites(
        [{ op: 'add', path: '/issues/a', value: { id: 'a', title: 'A' } }],
        'issues'
      )
    ).toEqual([{ type: 'insert', value: { id: 'a', title: 'A' } }]);
  });

  it('单行 replace 变成 update', () => {
    expect(
      patchToWrites(
        [{ op: 'replace', path: '/issues/a', value: { id: 'a', title: 'A2' } }],
        'issues'
      )
    ).toEqual([{ type: 'update', value: { id: 'a', title: 'A2' } }]);
  });

  it('remove 变成 delete，并带上 id 以便按主键删除', () => {
    expect(
      patchToWrites([{ op: 'remove', path: '/issues/a' }], 'issues')
    ).toEqual([{ type: 'delete', value: { id: 'a' } }]);
  });

  it('忽略其他表的 patch', () => {
    expect(
      patchToWrites(
        [{ op: 'add', path: '/project_statuses/s1', value: { id: 's1' } }],
        'issues'
      )
    ).toEqual([]);
  });

  it('对 JSON Pointer 转义做还原', () => {
    expect(
      patchToWrites([{ op: 'remove', path: '/issues/a~1b~0c' }], 'issues')
    ).toEqual([{ type: 'delete', value: { id: 'a/b~c' } }]);
  });

  it('未知操作被忽略而不是抛错', () => {
    expect(
      patchToWrites(
        [{ op: 'test', path: '/issues/a', value: {} } as never],
        'issues'
      )
    ).toEqual([]);
  });
});

describe('buildLocalMutationHandlers', () => {
  const okResponse = () =>
    new Response(JSON.stringify({ txid: 0 }), { status: 200 });

  it('新增走 POST，并在成功后刷新集合', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onInsert({
      transaction: { mutations: [{ modified: { id: 'a', title: 'A' } }] },
    });

    expect(request).toHaveBeenCalledWith('/api/local/issues', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: 'a', title: 'A' }),
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('单条更新走 PATCH /{id}', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onUpdate({
      transaction: { mutations: [{ key: 'a', changes: { title: 'A2' } }] },
    });

    expect(request).toHaveBeenCalledWith('/api/local/issues/a', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: 'A2' }),
    });
  });

  it('多条更新合并成一次 POST /bulk，保证拖拽排序是一个事务', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onUpdate({
      transaction: {
        mutations: [
          { key: 'a', changes: { sort_order: 1 } },
          { key: 'b', changes: { sort_order: 2 } },
        ],
      },
    });

    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenCalledWith('/api/local/issues/bulk', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        updates: [
          { id: 'a', sort_order: 1 },
          { id: 'b', sort_order: 2 },
        ],
      }),
    });
  });

  it('删除走 DELETE /{id}', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onDelete({ transaction: { mutations: [{ key: 'a' }] } });

    expect(request).toHaveBeenCalledWith('/api/local/issues/a', {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
    });
  });

  it('接口报错时抛出服务端文案，且不刷新集合', async () => {
    const request = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ message: '需求标题不能为空' }), {
        status: 400,
      })
    );
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await expect(
      handlers.onInsert({
        transaction: { mutations: [{ modified: { title: '' } }] },
      })
    ).rejects.toThrow('需求标题不能为空');
    expect(refresh).not.toHaveBeenCalled();
  });

  it('不支持的写操作直接抛出可读错误', async () => {
    const request = vi.fn();
    const refresh = vi.fn();
    const handlers = buildLocalMutationHandlers({
      name: 'PullRequestIssue',
      url: null,
      request,
      refresh,
    });

    await expect(
      handlers.onInsert({ transaction: { mutations: [{ modified: {} }] } })
    ).rejects.toThrow(/个人版不支持/);
    expect(request).not.toHaveBeenCalled();
  });
});
