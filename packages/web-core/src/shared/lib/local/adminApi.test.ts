import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  AdminApiError,
  createInvite,
  createMember,
  deleteInvite,
  listInvites,
  listMembers,
  resetMemberPassword,
  updateMember,
} from '@/shared/lib/local/adminApi';
import {
  setLocalApiTransport,
  type LocalApiRequestOptions,
} from '@/shared/lib/localApiTransport';
import { resetRuntimeModeForTests } from '@/shared/lib/local/runtimeMode';

interface RecordedCall {
  path: string;
  init: LocalApiRequestOptions;
}

let calls: RecordedCall[] = [];
let respond: () => Response = () => new Response(null, { status: 200 });

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  resetRuntimeModeForTests();
  setLocalApiTransport({
    request: (path, init = {}) => {
      calls.push({ path, init });
      return Promise.resolve(respond());
    },
    openWebSocket: () => {
      throw new Error('not used');
    },
  });
  // 裸 fetch 一律视为错误：管理接口也必须走 makeLocalApiRequest 才能带上 CSRF 头。
  vi.stubGlobal('fetch', () => {
    throw new Error('bare fetch must not be used');
  });
});

afterEach(() => {
  setLocalApiTransport(null);
  vi.unstubAllGlobals();
});

describe('listMembers', () => {
  it('解析 ApiResponse 包裹，走 makeLocalApiRequest', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { users: [{ id: 'u1', username: 'amy' }] },
        error_data: null,
        message: null,
      });

    const result = await listMembers();

    expect(result.users).toHaveLength(1);
    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe('/api/admin/users');
  });

  it('403 时抛 AdminApiError 且携带后端文案', async () => {
    respond = () =>
      jsonResponse(
        {
          success: false,
          data: null,
          error_data: null,
          message: '需要管理员权限',
        },
        403
      );

    const error = await listMembers().catch((e) => e);
    expect(error).toBeInstanceOf(AdminApiError);
    expect((error as AdminApiError).status).toBe(403);
    expect((error as AdminApiError).message).toBe('需要管理员权限');
  });
});

describe('createMember', () => {
  it('POST 到 /api/admin/users', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { id: 'u2', username: 'bob' },
        error_data: null,
        message: null,
      });

    await createMember({
      username: 'bob',
      display_name: 'Bob',
      email: null,
      password: 'hunter2hunter2',
      role: 'member',
    });

    expect(calls[0].path).toBe('/api/admin/users');
    expect(calls[0].init.method).toBe('POST');
  });

  it('409 时携带「用户名已被占用」这类可直接展示的文案', async () => {
    respond = () =>
      jsonResponse(
        {
          success: false,
          data: null,
          error_data: null,
          message: '用户名已被占用',
        },
        409
      );

    const error = await createMember({
      username: 'bob',
      display_name: 'Bob',
      email: null,
      password: 'hunter2hunter2',
      role: 'member',
    }).catch((e) => e);
    expect(error).toBeInstanceOf(AdminApiError);
    expect((error as AdminApiError).status).toBe(409);
    expect((error as AdminApiError).message).toBe('用户名已被占用');
  });

  it('400 弱密码', async () => {
    respond = () => new Response('boom', { status: 400 });

    const error = await createMember({
      username: 'bob',
      display_name: 'Bob',
      email: null,
      password: 'x',
      role: 'member',
    }).catch((e) => e);
    expect(error).toBeInstanceOf(AdminApiError);
    expect((error as AdminApiError).status).toBe(400);
  });
});

describe('updateMember', () => {
  it('PATCH 到 /api/admin/users/{id}', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { id: 'u1', username: 'amy', role: 'member' },
        error_data: null,
        message: null,
      });

    await updateMember('u1', {
      role: 'member',
      status: null,
      display_name: null,
    });

    expect(calls[0].path).toBe('/api/admin/users/u1');
    expect(calls[0].init.method).toBe('PATCH');
  });

  it('403「不能修改自己的角色」直接透出文案', async () => {
    respond = () =>
      jsonResponse(
        {
          success: false,
          data: null,
          error_data: null,
          message: '不能修改自己的角色',
        },
        403
      );

    const error = await updateMember('u1', {
      role: 'member',
      status: null,
      display_name: null,
    }).catch((e) => e);
    expect((error as AdminApiError).message).toBe('不能修改自己的角色');
  });

  it('email 省略时请求体里不带 email 键', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {},
        error_data: null,
        message: null,
      });

    await updateMember('u1', { display_name: 'x', role: null, status: null });

    const body = JSON.parse(calls[0].init.body as string);
    expect('email' in body).toBe(false);
  });

  it('email 传 null 会保留在请求体里（清空邮箱）', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {},
        error_data: null,
        message: null,
      });

    await updateMember('u1', {
      display_name: null,
      role: null,
      status: null,
      email: null,
    });

    const body = JSON.parse(calls[0].init.body as string);
    expect(body.email).toBeNull();
  });
});

describe('resetMemberPassword', () => {
  it('POST 到 /api/admin/users/{id}/password 并 resolve', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 'OK',
        error_data: null,
        message: null,
      });

    await expect(
      resetMemberPassword('u1', { new_password: 'hunter2hunter2' })
    ).resolves.toBeUndefined();
    expect(calls[0].path).toBe('/api/admin/users/u1/password');
  });
});

describe('listInvites / createInvite / deleteInvite', () => {
  it('listInvites 走 GET /api/admin/invites', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { invites: [] },
        error_data: null,
        message: null,
      });

    await listInvites();
    expect(calls[0].path).toBe('/api/admin/invites');
  });

  it('createInvite 返回明文邀请码', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {
          invite: { id: 'i1', role: 'member' },
          code: 'plain-code',
        },
        error_data: null,
        message: null,
      });

    const result = await createInvite({
      role: 'member',
      // ts-rs 把 Rust i64 映射成 TS `bigint`，但仓库里一律按既有惯例
      // （见 `useCreateAttachments.ts`）用普通 number 做类型转换来避免
      // `JSON.stringify` 遇到真正的 BigInt 抛错——这里照做。
      expires_in_days: 7 as unknown as bigint,
    });
    expect(result.code).toBe('plain-code');
    expect(calls[0].init.method).toBe('POST');
  });

  it('deleteInvite 走 DELETE 并 resolve', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 'OK',
        error_data: null,
        message: null,
      });

    await expect(deleteInvite('i1')).resolves.toBeUndefined();
    expect(calls[0].path).toBe('/api/admin/invites/i1');
    expect(calls[0].init.method).toBe('DELETE');
  });

  it('作废不存在的邀请返回 404', async () => {
    respond = () => new Response(null, { status: 404 });

    const error = await deleteInvite('missing').catch((e) => e);
    expect(error).toBeInstanceOf(AdminApiError);
    expect((error as AdminApiError).status).toBe(404);
  });
});
