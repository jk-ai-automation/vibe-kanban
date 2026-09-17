import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  InvalidCredentialsError,
  LocalAuthRequestError,
  RateLimitedError,
  SetupTokenInvalidError,
  fetchBootstrap,
  fetchMe,
  fetchOAuthBindings,
  fetchSetupStatus,
  login,
  logout,
  startOAuthBind,
  submitSetupAdmin,
  unbindOAuth,
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

describe('第三方账号绑定', () => {
  const 视图 = {
    available_providers: ['feishu', 'google'],
    bindings: [
      { provider: 'feishu', bound_at: '2026-09-17T03:00:00Z', email: null },
    ],
    has_password: true,
  };

  it('fetchOAuthBindings 打的是只读的绑定列表端点', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 视图,
        error_data: null,
        message: null,
      });

    const view = await fetchOAuthBindings();

    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe('/api/local-auth/oauth/bindings');
    expect(calls[0].init.method).toBe('GET');
    expect(view.bindings[0].provider).toBe('feishu');
  });

  /**
   * 这三条只跟**本机**的账号有关，绝不能被改写成
   * `/api/host/<id>/local-auth/...` 转发到别的机器上——
   * 那边没有这条会话，回来的 401 会把用户直接踢去登录页。
   */
  it('三条请求都不跟随机器作用域', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 视图,
        error_data: null,
        message: null,
      });
    await fetchOAuthBindings();

    respond = () =>
      jsonResponse({
        success: true,
        data: { authorize_url: 'https://idp.example/authorize' },
        error_data: null,
        message: null,
      });
    await startOAuthBind('feishu');

    respond = () =>
      jsonResponse({
        success: true,
        data: 'OK',
        error_data: null,
        message: null,
      });
    await unbindOAuth('feishu');

    expect(calls.map((call) => call.init.hostScope)).toEqual([
      'none',
      'none',
      'none',
    ]);
  });

  it('startOAuthBind 是 POST，并把授权链接原样带回来', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: { authorize_url: 'https://idp.example/authorize?state=x' },
        error_data: null,
        message: null,
      });

    const url = await startOAuthBind('feishu');

    expect(calls[0].path).toBe('/api/local-auth/oauth/feishu/bind');
    expect(calls[0].init.method).toBe('POST');
    expect(url).toBe('https://idp.example/authorize?state=x');
  });

  it('unbindOAuth 用 DELETE，路径落在 /binding 上', async () => {
    respond = () =>
      jsonResponse({
        success: true,
        data: 'OK',
        error_data: null,
        message: null,
      });

    await unbindOAuth('google');

    expect(calls[0].path).toBe('/api/local-auth/oauth/google/binding');
    expect(calls[0].init.method).toBe('DELETE');
  });

  /** provider 进了 URL 路径，必须编码；后端白名单会把它挡下来。 */
  it('provider 被编码进路径，不会拼出别的路由', async () => {
    respond = () => new Response(null, { status: 404 });

    await unbindOAuth('../../admin/users').catch(() => undefined);

    expect(calls[0].path).toBe(
      '/api/local-auth/oauth/..%2F..%2Fadmin%2Fusers/binding'
    );
  });

  it('解绑失败时把状态码透出来，供界面按码分流文案', async () => {
    for (const status of [401, 403, 404, 409, 500]) {
      calls = [];
      respond = () =>
        jsonResponse(
          { success: false, data: null, error_data: null, message: '不行' },
          status
        );

      const error = await unbindOAuth('feishu').catch((e) => e);
      expect(error).toBeInstanceOf(LocalAuthRequestError);
      expect((error as LocalAuthRequestError).status).toBe(status);
    }
  });
});
