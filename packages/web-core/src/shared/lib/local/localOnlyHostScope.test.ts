import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  makeLocalApiRequest,
  setLocalApiTransport,
  type LocalApiRequestOptions,
} from '@/shared/lib/localApiTransport';
import { resetRuntimeModeForTests } from '@/shared/lib/local/runtimeMode';

/**
 * 选中远端 host 时 `getCurrentHostId()` 返回那台机器的 id，
 * `makeLocalApiRequest` 就会把 `/api/xxx` 改写成 `/api/host/<id>/xxx`。
 * 这里把它钉成常量，用来检验哪些接口**不该**被改写。
 */
const 远端 = 'remote-host-1';

vi.mock('@/shared/providers/HostIdProvider', () => ({
  getCurrentHostId: () => 远端,
}));

const 被转发前缀 = `/api/host/${远端}/`;

let 请求路径: string[] = [];

beforeEach(() => {
  请求路径 = [];
  resetRuntimeModeForTests();
  setLocalApiTransport({
    request: (path: string, _init: LocalApiRequestOptions = {}) => {
      请求路径.push(path);
      // 空 body：调用方解信封时会抛错，测试只关心请求路径。
      return Promise.resolve(new Response(null, { status: 200 }));
    },
    openWebSocket: () => {
      throw new Error('not used');
    },
  });
});

afterEach(() => {
  setLocalApiTransport(null);
  resetRuntimeModeForTests();
});

/** 把整个模块的导出函数挨个调一遍，只为了收集它们真正发出的请求路径。 */
async function 调用全部导出(模块: Record<string, unknown>): Promise<void> {
  for (const [名字, 值] of Object.entries(模块)) {
    if (typeof 值 !== 'function') continue;
    // 首字母大写的是 Error 子类，不是接口函数。
    if (/^[A-Z]/.test(名字)) continue;
    try {
      await (值 as (...args: unknown[]) => unknown)('dummy-id', {}, {});
    } catch {
      // 假响应解不出信封是预期的；这里只收集路径。
    }
  }
}

describe('本机专属接口不能被改写到远端 host', () => {
  it('GET /api/local-auth/me 必须原样打本机', async () => {
    const { fetchMe } = await import('@/shared/lib/local/bootstrapApi');

    await fetchMe().catch(() => undefined);

    expect(请求路径).toEqual(['/api/local-auth/me']);
  });

  it('GET /api/admin/users 必须原样打本机', async () => {
    const { listMembers } = await import('@/shared/lib/local/adminApi');

    await listMembers().catch(() => undefined);

    expect(请求路径).toEqual(['/api/admin/users']);
  });

  it('工作区删除申请（申请侧与审批侧）必须原样打本机', async () => {
    const { listMyDeleteRequests, listPendingDeleteRequests } = await import(
      '@/shared/lib/local/workspaceDeleteRequestsApi'
    );

    await listMyDeleteRequests().catch(() => undefined);
    await listPendingDeleteRequests().catch(() => undefined);

    expect(请求路径).toEqual([
      '/api/workspace-delete-requests',
      '/api/admin/workspace-delete-requests',
    ]);
  });

  /**
   * 遍历导出而不是逐个列举：**将来新加的接口函数自动被这条覆盖**，
   * 忘了标 `hostScope: 'none'` 当场红。
   */
  it.each([
    ['bootstrapApi', () => import('@/shared/lib/local/bootstrapApi')],
    ['adminApi', () => import('@/shared/lib/local/adminApi')],
    [
      'workspaceDeleteRequestsApi',
      () => import('@/shared/lib/local/workspaceDeleteRequestsApi'),
    ],
  ] as const)('%s 的每一个导出函数都不走 host 转发', async (_名字, 载入) => {
    await 调用全部导出(await 载入());

    expect(请求路径.length).toBeGreaterThan(0);
    expect(请求路径.filter((p) => p.startsWith(被转发前缀))).toEqual([]);
  });
});

/**
 * 纵深防御的第二层：`localApiTransport.ts` 里的 `LOCAL_ONLY_API_PREFIXES`。
 *
 * 上面那组测的是「调用点标了 `hostScope: 'none'`，所以没被转发」。这组把
 * 标注**整个拿掉**（直接用默认的 `hostScope: 'current'` 调一次），证明中央
 * 名单自己就能把这些路径按在本机 —— 将来谁新加接口忘了标注，也不会出事。
 */
describe('中央名单独立生效（调用点不标注也挡得住）', () => {
  it.each([
    '/api/local-auth/me',
    '/api/local-auth/login',
    '/api/local-auth/oauth/feishu/bind',
    '/api/admin/users',
    '/api/admin/invites/abc',
    '/api/admin/workspace-delete-requests',
    '/api/workspace-delete-requests',
    '/api/workspace-delete-requests/abc',
    '/api/workspace-delete-requests?scope=mine',
  ])('%s 不带 hostScope 也不会被改写', async (路径) => {
    // 刻意不传 hostScope：走默认的 'current'，也就是「跟随选中的 host」。
    await makeLocalApiRequest(路径);

    expect(请求路径).toEqual([路径]);
  });

  /**
   * 前缀匹配的边界。`/api/workspace-delete-requests` 没有尾斜杠，如果拿
   * `startsWith` 裸比，`/api/workspace-delete-requests-foo` 会被一起误伤，
   * 那条本该跟着 host 走的接口就永远打不到远端了。
   */
  it.each([
    '/api/workspace-delete-requests-foo',
    '/api/administration/x',
    '/api/local-authority/x',
  ])('%s 不在名单上，仍然跟随选中的 host', async (路径) => {
    await makeLocalApiRequest(路径);

    expect(请求路径).toEqual([`${被转发前缀}${路径.slice('/api/'.length)}`]);
  });

  it('本来就该跟随 host 的普通接口没被波及', async () => {
    await makeLocalApiRequest('/api/workspaces');

    expect(请求路径).toEqual([`${被转发前缀}workspaces`]);
  });
});
