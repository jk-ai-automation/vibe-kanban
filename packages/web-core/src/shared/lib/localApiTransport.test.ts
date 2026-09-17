import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  makeLocalApiRequest,
  setLocalApiTransport,
  type LocalApiRequestOptions,
} from '@/shared/lib/localApiTransport';
import {
  configureRuntimeMode,
  resetRuntimeModeForTests,
} from '@/shared/lib/local/runtimeMode';
import {
  onSessionExpired,
  resetSessionExpiryListenersForTests,
} from '@/shared/lib/local/sessionExpiry';

interface RecordedCall {
  path: string;
  init: LocalApiRequestOptions;
}

let calls: RecordedCall[] = [];
let nextStatus = 200;

function headerOf(call: RecordedCall, name: string): string | null {
  return new Headers(call.init.headers ?? {}).get(name);
}

beforeEach(() => {
  calls = [];
  nextStatus = 200;
  resetRuntimeModeForTests();
  resetSessionExpiryListenersForTests();
  setLocalApiTransport({
    request: (path, init = {}) => {
      calls.push({ path, init });
      return Promise.resolve(new Response(null, { status: nextStatus }));
    },
    openWebSocket: () => {
      throw new Error('not used');
    },
  });
});

afterEach(() => {
  setLocalApiTransport(null);
  vi.unstubAllGlobals();
  resetRuntimeModeForTests();
  resetSessionExpiryListenersForTests();
});

describe('写请求携带双提交 CSRF 令牌', () => {
  it('写方法会带上 X-VK-CSRF 头', async () => {
    vi.stubGlobal('document', { cookie: 'a=1; vk_csrf=tok-123' });

    await makeLocalApiRequest('/api/projects', { method: 'POST' });

    expect(headerOf(calls[0], 'X-VK-CSRF')).toBe('tok-123');
  });

  it('GET 不带 CSRF 头', async () => {
    vi.stubGlobal('document', { cookie: 'vk_csrf=tok-123' });

    await makeLocalApiRequest('/api/projects');

    expect(headerOf(calls[0], 'X-VK-CSRF')).toBeNull();
  });

  it('没有 vk_csrf Cookie 时不加头，也不报错（个人版就是这种情况）', async () => {
    vi.stubGlobal('document', { cookie: 'a=1' });

    await makeLocalApiRequest('/api/projects', { method: 'DELETE' });

    expect(headerOf(calls[0], 'X-VK-CSRF')).toBeNull();
  });

  it('不覆盖调用方已有的请求头', async () => {
    vi.stubGlobal('document', { cookie: 'vk_csrf=tok-123' });
    const headers = new Headers({ 'Content-Type': 'application/json' });

    await makeLocalApiRequest('/api/projects', { method: 'POST', headers });

    expect(headerOf(calls[0], 'Content-Type')).toBe('application/json');
    expect(headerOf(calls[0], 'X-VK-CSRF')).toBe('tok-123');
    // 调用方传进来的 Headers 实例不被就地改写。
    expect(headers.get('X-VK-CSRF')).toBeNull();
  });

  it('不设置 credentials：同源 fetch 默认就带 Cookie', async () => {
    vi.stubGlobal('document', { cookie: 'vk_csrf=tok-123' });

    await makeLocalApiRequest('/api/projects', { method: 'POST' });

    expect(calls[0].init.credentials).toBeUndefined();
  });
});

describe('401 / 403 分流', () => {
  it('团队模式的 401 触发一次会话过期广播', async () => {
    configureRuntimeMode('team');
    nextStatus = 401;
    const listener = vi.fn();
    onSessionExpired(listener);

    await makeLocalApiRequest('/api/projects');

    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('团队模式的 403（CSRF 失败）不触发会话过期', async () => {
    configureRuntimeMode('team');
    nextStatus = 403;
    const listener = vi.fn();
    onSessionExpired(listener);

    await makeLocalApiRequest('/api/projects', { method: 'POST' });

    expect(listener).not.toHaveBeenCalled();
  });

  it('个人版的 401 不触发会话过期', async () => {
    configureRuntimeMode('personal');
    nextStatus = 401;
    const listener = vi.fn();
    onSessionExpired(listener);

    await makeLocalApiRequest('/api/projects');

    expect(listener).not.toHaveBeenCalled();
  });

  it('登录接口自身的 401 不触发会话过期', async () => {
    configureRuntimeMode('team');
    nextStatus = 401;
    const listener = vi.fn();
    onSessionExpired(listener);

    await makeLocalApiRequest('/api/local-auth/login', { method: 'POST' });

    expect(listener).not.toHaveBeenCalled();
  });

  it('请求仍然原样返回响应', async () => {
    configureRuntimeMode('team');
    nextStatus = 401;

    const response = await makeLocalApiRequest('/api/projects');

    expect(response.status).toBe(401);
  });
});
