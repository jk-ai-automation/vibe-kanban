import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  InvalidCredentialsError,
  LocalAuthRequestError,
  RateLimitedError,
  SetupTokenInvalidError,
  fetchBootstrap,
  fetchMe,
  fetchSetupStatus,
  login,
  logout,
  submitSetupAdmin,
} from '@/shared/lib/local/bootstrapApi';
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
  // 裸 fetch 一律视为错误：所有本地认证请求都必须走 makeLocalApiRequest，
  // 否则拿不到 CSRF 头与 401 分流。
  vi.stubGlobal('fetch', () => {
    throw new Error('bare fetch must not be used');
  });
});

afterEach(() => {
  setLocalApiTransport(null);
  vi.unstubAllGlobals();
});

describe('fetchBootstrap', () => {
  it('解析 ApiResponse 包裹，返回 data', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {
          mode: 'team',
          require_login: true,
          authenticated: false,
          needs_setup: false,
          providers: ['feishu'],
          allow_oauth_signup: false,
        },
        error_data: null,
        message: null,
      });

    const bootstrap = await fetchBootstrap();

    expect(bootstrap.mode).toBe('team');
    expect(bootstrap.providers).toEqual(['feishu']);
  });

  it('走 makeLocalApiRequest 而不是裸 fetch', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {},
        error_data: null,
        message: null,
      });

    await fetchBootstrap();

    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe('/api/local-auth/bootstrap');
  });

  it('success=false 时抛错', async () => {
    respond = () =>
      jsonResponse({
        success: false,
        data: null,
        error_data: null,
        message: 'nope',
      });

    await expect(fetchBootstrap()).rejects.toThrow('nope');
  });

  it('5xx 时抛错且错误信息含状态码', async () => {
    respond = () => new Response('boom', { status: 503 });

    await expect(fetchBootstrap()).rejects.toThrow(/503/);
  });
});

describe('login', () => {
  it('成功返回用户', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {
          id: 'u1',
          username: 'alice',
          display_name: 'Alice',
          email: null,
          role: 'admin',
          avatar_color: '#123456',
        },
        error_data: null,
        message: null,
      });

    const user = await login({ username: 'alice', password: 'pw' });

    expect(user.username).toBe('alice');
    expect(calls[0].init.method).toBe('POST');
  });

  it('401 时抛 InvalidCredentialsError（按状态码，不看 message 文本）', async () => {
    respond = () =>
      jsonResponse(
        {
          success: false,
          data: null,
          error_data: null,
          message: 'Unauthorized. Please sign in again.',
        },
        401
      );

    await expect(
      login({ username: 'a', password: 'b' })
    ).rejects.toBeInstanceOf(InvalidCredentialsError);
  });

  it('429 时抛 RateLimitedError', async () => {
    respond = () => new Response(null, { status: 429 });

    await expect(
      login({ username: 'a', password: 'b' })
    ).rejects.toBeInstanceOf(RateLimitedError);
  });

  it('500 时抛通用错误', async () => {
    respond = () => new Response(null, { status: 500 });

    const error = await login({ username: 'a', password: 'b' }).catch((e) => e);
    expect(error).toBeInstanceOf(LocalAuthRequestError);
    expect(error.status).toBe(500);
  });
});

describe('logout', () => {
  it('成功时 resolve', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 'OK',
        error_data: null,
        message: null,
      });

    await expect(logout()).resolves.toBeUndefined();
  });

  it('401 也 resolve（幂等）', async () => {
    respond = () => new Response(null, { status: 401 });

    await expect(logout()).resolves.toBeUndefined();
  });

  it('500 仍然抛错', async () => {
    respond = () => new Response(null, { status: 500 });

    await expect(logout()).rejects.toBeInstanceOf(LocalAuthRequestError);
  });
});

describe('fetchSetupStatus', () => {
  it('有效令牌返回 valid:true，走 GET 并带上 token 查询参数', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { valid: true },
        error_data: null,
        message: null,
      });

    const result = await fetchSetupStatus('the-token');

    expect(result.valid).toBe(true);
    expect(calls[0].path).toBe('/api/local-auth/setup?token=the-token');
    expect(calls[0].init.method).toBe('GET');
  });

  it('401 时抛 SetupTokenInvalidError（缺失/错误/过期/已用一律如此）', async () => {
    respond = () => new Response(null, { status: 401 });

    await expect(fetchSetupStatus('wrong')).rejects.toBeInstanceOf(
      SetupTokenInvalidError
    );
  });
});

describe('submitSetupAdmin', () => {
  const payload = {
    token: 't',
    username: 'amy',
    display_name: 'amy',
    password: 'hunter2hunter2',
    email: null,
  };

  it('成功建号返回用户并走 POST', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: {
          id: 'u1',
          username: 'amy',
          display_name: 'amy',
          email: null,
          role: 'admin',
          avatar_color: '#123456',
        },
        error_data: null,
        message: null,
      });

    const user = await submitSetupAdmin(payload);

    expect(user.role).toBe('admin');
    expect(calls[0].path).toBe('/api/local-auth/setup');
    expect(calls[0].init.method).toBe('POST');
  });

  it('401 时抛 SetupTokenInvalidError', async () => {
    respond = () => new Response(null, { status: 401 });

    await expect(submitSetupAdmin(payload)).rejects.toBeInstanceOf(
      SetupTokenInvalidError
    );
  });

  it('429 时抛 RateLimitedError', async () => {
    respond = () => new Response(null, { status: 429 });

    await expect(submitSetupAdmin(payload)).rejects.toBeInstanceOf(
      RateLimitedError
    );
  });

  it('409「已初始化」把后端原文透出（供上层区分该去登录页）', async () => {
    respond = () =>
      jsonResponse(
        { success: false, data: null, error_data: null, message: '已初始化' },
        409
      );

    const error = await submitSetupAdmin(payload).catch((e) => e);
    expect(error).toBeInstanceOf(LocalAuthRequestError);
    expect((error as LocalAuthRequestError).message).toBe('已初始化');
  });

  it('409 用户名冲突同样透出原文（与「已初始化」区分）', async () => {
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

    const error = await submitSetupAdmin(payload).catch((e) => e);
    expect((error as LocalAuthRequestError).message).toBe('用户名已被占用');
  });

  it('400 弱密码透出文案', async () => {
    respond = () =>
      jsonResponse(
        {
          success: false,
          data: null,
          error_data: null,
          message: '密码至少需要 8 个字节',
        },
        400
      );

    const error = await submitSetupAdmin(payload).catch((e) => e);
    expect((error as LocalAuthRequestError).message).toBe(
      '密码至少需要 8 个字节'
    );
  });
});

describe('fetchMe', () => {
  it('401 时抛带状态码的错误，交给会话上下文处理', async () => {
    respond = () => new Response(null, { status: 401 });

    const error = await fetchMe().catch((e) => e);
    expect(error).toBeInstanceOf(LocalAuthRequestError);
    expect(error.status).toBe(401);
  });
});
