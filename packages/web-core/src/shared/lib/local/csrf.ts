/**
 * 双提交 CSRF 令牌。
 *
 * 登录成功后服务端下发两条 Cookie：`vk_session`（HttpOnly，前端读不到）与
 * `vk_csrf`（**非** HttpOnly，就是给前端读的）。团队模式下所有写方法必须带
 * `X-VK-CSRF` 头，且要和 `vk_csrf` Cookie 一致，否则后端返回 **403**。
 */

export const CSRF_COOKIE_NAME = 'vk_csrf';
export const CSRF_HEADER_NAME = 'X-VK-CSRF';

/**
 * 从 Cookie 字符串里取出 `vk_csrf`。纯函数，node 环境可直接测。
 *
 * **不做 `decodeURIComponent`**：服务端令牌字符集固定为 `[A-Za-z0-9_-]`
 * （base64url），本来就不需要百分号编码；解码反而会把恰好像编码的值改坏，
 * 导致和 Cookie 里的原值对不上而被判 403。
 */
export function readCsrfTokenFrom(cookieString: string | null): string | null {
  if (!cookieString) return null;

  for (const part of cookieString.split(';')) {
    const segment = part.trim();
    if (!segment.startsWith(`${CSRF_COOKIE_NAME}=`)) continue;
    const value = segment.slice(CSRF_COOKIE_NAME.length + 1).trim();
    // 空值当作没有；重复时取第一个（浏览器也是按最具体的排在前）。
    return value.length > 0 ? value : null;
  }

  return null;
}

/**
 * 读当前文档的 CSRF 令牌。
 * node 环境（vitest、SSR）没有 `document`，返回 `null` 而不是抛错。
 */
export function readCsrfToken(): string | null {
  if (typeof document === 'undefined') return null;
  return readCsrfTokenFrom(document.cookie);
}

/** 写方法才需要带 CSRF 头；GET/HEAD/OPTIONS 是安全方法。 */
export function methodNeedsCsrf(method: string | undefined): boolean {
  const normalized = (method ?? 'GET').toUpperCase();
  return (
    normalized !== 'GET' && normalized !== 'HEAD' && normalized !== 'OPTIONS'
  );
}
