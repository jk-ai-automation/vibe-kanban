/**
 * 会话过期广播。
 *
 * 团队模式下后端对无效 / 过期会话返回 **401**，对 CSRF 校验失败返回 **403**。
 * 两者必须分开处理：403 再怎么重新登录也修不好（缺的是请求头，不是会话），
 * 把 403 也当会话过期会让用户陷在登录页反复登录。
 */

/** 登录与 bootstrap 自身的 401 是业务结果，不是会话过期。 */
const SESSION_EXPIRY_EXEMPT_PATHS = [
  '/api/local-auth/login',
  '/api/local-auth/bootstrap',
];

export interface SessionExpiryInput {
  status: number;
  /** 当前是否团队模式（`requiresLogin()`）。个人版没有会话概念。 */
  requiresLogin: boolean;
  /** 请求路径，用于豁免登录接口自身的 401。 */
  path?: string;
}

export function shouldTreatAsSessionExpired({
  status,
  requiresLogin,
  path,
}: SessionExpiryInput): boolean {
  if (!requiresLogin) return false;
  if (status !== 401) return false;
  if (path && SESSION_EXPIRY_EXEMPT_PATHS.some((p) => path.startsWith(p))) {
    return false;
  }
  return true;
}

type Listener = () => void;

const listeners = new Set<Listener>();

/** 订阅会话过期。返回取消订阅函数。 */
export function onSessionExpired(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function notifySessionExpired(): void {
  for (const listener of [...listeners]) {
    listener();
  }
}

/** 仅测试用。 */
export function resetSessionExpiryListenersForTests(): void {
  listeners.clear();
}
