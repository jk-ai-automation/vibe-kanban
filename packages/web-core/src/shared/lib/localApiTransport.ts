import { getCurrentHostId } from '@/shared/providers/HostIdProvider';
import {
  CSRF_HEADER_NAME,
  methodNeedsCsrf,
  readCsrfToken,
} from '@/shared/lib/local/csrf';
import { requiresLogin } from '@/shared/lib/local/runtimeMode';
import {
  notifySessionExpired,
  shouldTreatAsSessionExpired,
} from '@/shared/lib/local/sessionExpiry';

export type LocalApiHostScope = 'current' | 'explicit' | 'none';

export interface LocalApiRequestOptions extends RequestInit {
  hostScope?: LocalApiHostScope;
  hostId?: string | null;
  relayHostId?: string | null;
}

export interface LocalApiWebSocketOptions {
  hostScope?: LocalApiHostScope;
  hostId?: string | null;
  relayHostId?: string | null;
}

export interface LocalApiTransport {
  request: (
    pathOrUrl: string,
    init?: LocalApiRequestOptions
  ) => Promise<Response>;
  openWebSocket: (
    pathOrUrl: string,
    options?: LocalApiWebSocketOptions
  ) => Promise<WebSocket> | WebSocket;
}

/**
 * **只存在于本机后端、绝不能被转发到远端 host 的接口前缀。**
 *
 * 这份名单是唯一的**强制点**：不管调用方怎么写、有没有记得标
 * `hostScope: 'none'`、把选项包在什么变量里，走到这里都会被挡下。
 * 调用点上的 `hostScope: 'none'` 保留着当文档（读代码的人一眼能看出
 * 这条是本机专属，不用回来翻这份名单），但真正兜底的是这里。
 *
 * 后 3 条是本机账号体系：会话 Cookie（`vk_session`）是本机这一份，
 * 而 `/api/host/<id>/{*tail}` 代理会把请求头**原样**转给对面那台机器
 * （`crates/server/src/routes/host_relay/proxy.rs`），对面是同一个后端
 * 二进制、同样注册了这些路由，只是不认这份 Cookie —— 于是回 401，
 * 传输层广播「会话过期」，用户被莫名其妙踢回登录页。
 *
 * 写法：结尾带 `/` 的按纯前缀匹配（历史写法，保持不变）；不带 `/` 的
 * 要求后面紧跟路径结束、`/` 或 `?`，免得 `/api/admin` 把
 * `/api/administration` 之类的也一起误伤。
 */
const LOCAL_ONLY_API_PREFIXES = [
  '/api/open-remote-editor/',
  '/api/relay-auth/server/',
  '/api/relay-auth/client/',
  '/api/local-auth',
  '/api/admin',
  '/api/workspace-delete-requests',
];

function isLocalOnlyApiPath(path: string): boolean {
  return LOCAL_ONLY_API_PREFIXES.some((prefix) => {
    if (!path.startsWith(prefix)) return false;
    if (prefix.endsWith('/')) return true;
    const 边界 = path.charAt(prefix.length);
    return 边界 === '' || 边界 === '/' || 边界 === '?';
  });
}

function isAbsoluteUrl(pathOrUrl: string): boolean {
  return /^https?:\/\//i.test(pathOrUrl) || /^wss?:\/\//i.test(pathOrUrl);
}

function toPathAndQuery(pathOrUrl: string): string {
  if (isAbsoluteUrl(pathOrUrl)) {
    const url = new URL(pathOrUrl);
    return `${url.pathname}${url.search}`;
  }
  return pathOrUrl.startsWith('/') ? pathOrUrl : `/${pathOrUrl}`;
}

function toAbsoluteWsUrl(pathOrUrl: string): string {
  if (/^wss?:\/\//i.test(pathOrUrl)) return pathOrUrl;
  if (/^https?:\/\//i.test(pathOrUrl)) return pathOrUrl.replace(/^http/i, 'ws');

  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const path = pathOrUrl.startsWith('/') ? pathOrUrl : `/${pathOrUrl}`;
  return `${protocol}//${window.location.host}${path}`;
}

function scopeLocalApiPath(pathOrUrl: string, hostId: string | null): string {
  if (!hostId) return pathOrUrl;
  const path = toPathAndQuery(pathOrUrl);
  // These endpoints must always hit the local backend because they rely on
  // local-only credentials/state.
  if (isLocalOnlyApiPath(path)) {
    return pathOrUrl;
  }

  if (!path.startsWith('/api/') || path.startsWith('/api/host/'))
    return pathOrUrl;

  const suffix = path.slice('/api'.length);
  return `/api/host/${hostId}${suffix}`;
}

function resolveScopedPath(
  pathOrUrl: string,
  options: {
    hostScope?: LocalApiHostScope;
    hostId?: string | null;
  } = {}
): string {
  const hostScope = options.hostScope ?? 'current';

  if (hostScope === 'none') {
    return pathOrUrl;
  }

  if (hostScope === 'explicit') {
    return scopeLocalApiPath(pathOrUrl, options.hostId ?? null);
  }

  return scopeLocalApiPath(pathOrUrl, getCurrentHostId());
}

const defaultTransport: LocalApiTransport = {
  request: (pathOrUrl, init = {}) => {
    const {
      hostScope: _hostScope,
      hostId: _hostId,
      relayHostId: _relayHostId,
      ...requestInit
    } = init;
    return fetch(pathOrUrl, requestInit);
  },
  openWebSocket: (pathOrUrl, _options = {}) =>
    new WebSocket(toAbsoluteWsUrl(pathOrUrl)),
};

let transport: LocalApiTransport = defaultTransport;

export function setLocalApiTransport(nextTransport: LocalApiTransport | null) {
  transport = nextTransport ?? defaultTransport;
}

/**
 * 本地 `/api/*` 请求的唯一收口。
 *
 * 这里做两件跟认证有关的事：
 * 1. **写方法自动带上 `X-VK-CSRF` 头**（值取自非 HttpOnly 的 `vk_csrf` Cookie）。
 *    Cookie 本身是同源 fetch 默认就会带的（`credentials: 'same-origin'`），
 *    **不需要** `credentials: 'include'`——加了反而会触发跨域预检。
 * 2. **识别会话过期**（团队模式下的 401）并广播一次，由会话上下文决定跳登录页。
 *    403（CSRF 校验失败）**不**走这条路，见 `sessionExpiry.ts`。
 */
export async function makeLocalApiRequest(
  pathOrUrl: string,
  init: LocalApiRequestOptions = {}
): Promise<Response> {
  const scopedPath = resolveScopedPath(pathOrUrl, init);

  let requestInit = init;
  if (methodNeedsCsrf(init.method)) {
    const token = readCsrfToken();
    if (token) {
      const headers = new Headers(init.headers ?? {});
      headers.set(CSRF_HEADER_NAME, token);
      requestInit = { ...init, headers };
    }
  }

  const response = await transport.request(scopedPath, requestInit);

  if (
    shouldTreatAsSessionExpired({
      status: response.status,
      requiresLogin: requiresLogin(),
      path: scopedPath,
    })
  ) {
    notifySessionExpired();
  }

  return response;
}

export async function openLocalApiWebSocket(
  pathOrUrl: string,
  options: LocalApiWebSocketOptions = {}
): Promise<WebSocket> {
  return transport.openWebSocket(
    resolveScopedPath(pathOrUrl, options),
    options
  );
}
