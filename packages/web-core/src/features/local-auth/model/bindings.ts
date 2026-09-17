/**
 * 第三方账号绑定的纯逻辑层。
 *
 * 这里不碰网络也不碰 React：界面要显示哪几行、哪一行能解绑、
 * 失败了该说什么，全是可以单独穷举的判定。
 */
import type { OAuthBindings } from 'shared/types';

/**
 * 提供方 id → i18n key 的**字面量映射表**。
 *
 * 刻意不写 `t(\`localAuth.providers.${id}\`)`：模板字面量 key 是
 * `scripts/check-unused-i18n-keys.mjs` 抓不到的，那些文案会被当成
 * 无人使用而在某次清理里被删掉。这张表同时也是一道白名单——
 * `provider` 来自后端响应，拿它去索引一个普通对象时
 * `__proto__` / `constructor` 这类键会命中原型链上的东西。
 */
export const PROVIDER_LABEL_KEYS = {
  feishu: 'localAuth.providers.feishu',
  lark: 'localAuth.providers.lark',
  google: 'localAuth.providers.google',
} as const;

export type KnownProviderId = keyof typeof PROVIDER_LABEL_KEYS;

/** 解绑失败的固定文案，同样是字面量映射表。 */
export const BINDING_ERROR_KEYS = {
  lastLoginMethod: 'localAuth.bindings.errorLastLoginMethod',
  notFound: 'localAuth.bindings.errorNotFound',
  forbidden: 'localAuth.bindings.errorForbidden',
  generic: 'localAuth.bindings.errorGeneric',
} as const;

export type BindingErrorKey =
  (typeof BINDING_ERROR_KEYS)[keyof typeof BINDING_ERROR_KEYS];

/** 绑定成功后后端固定回跳 `/?oauth=bound`（见 `oauth_routes.rs` 的 `BIND_REDIRECT`）。 */
const OAUTH_QUERY_PARAM = 'oauth';
const OAUTH_BOUND_VALUE = 'bound';

export interface OAuthBindingRow {
  provider: string;
  /** 已经绑上了。 */
  bound: boolean;
  /** 绑定时间（RFC 3339）；未绑定为 null。 */
  boundAt: string | null;
  /** 提供方给的邮箱，仅供辨认绑的是哪个账号。 */
  email: string | null;
  /** 本机配齐了凭据，可以发起绑定。 */
  canBind: boolean;
  /** 解绑了就再也登不进来——界面要提前把话说清，并禁用解绑按钮。 */
  isLastLoginMethod: boolean;
  /** 解绑按钮可不可点。 */
  canUnbind: boolean;
}

/**
 * 用**字面量映射表**查提供方显示名的 i18n key。
 *
 * 用 `Object.hasOwn` 而不是 `in`：后者会顺着原型链找到 `toString` 之类，
 * 而 `provider` 是来自响应的字符串。
 */
export function providerLabelKey(provider: string): string | null {
  return Object.hasOwn(PROVIDER_LABEL_KEYS, provider)
    ? PROVIDER_LABEL_KEYS[provider as KnownProviderId]
    : null;
}

/**
 * 把后端视图摊成一行一个提供方。
 *
 * 行的来源是「可绑的」∪「已绑的」：管理员撤掉某个提供方的凭据之后，
 * 已经绑上的那一行必须继续显示（只是不给「绑定」按钮），
 * 否则用户看不到它，也就永远解不掉。
 */
export function buildBindingRows(view: OAuthBindings): OAuthBindingRow[] {
  const bound = new Map(view.bindings.map((item) => [item.provider, item]));
  const providers = [
    ...new Set([...view.available_providers, ...bound.keys()]),
  ].sort();

  // 没有密码时，最后一个已绑身份就是唯一的登录方式，解不得。
  const 只剩一个登录方式 = !view.has_password && bound.size === 1;

  return providers.map((provider) => {
    const hit = bound.get(provider) ?? null;
    const isLastLoginMethod = hit !== null && 只剩一个登录方式;
    return {
      provider,
      bound: hit !== null,
      boundAt: hit?.bound_at ?? null,
      email: hit?.email ?? null,
      canBind: view.available_providers.includes(provider),
      isLastLoginMethod,
      canUnbind: hit !== null && !isLastLoginMethod,
    };
  });
}

/**
 * 按**状态码**分流失败文案。
 *
 * 不去匹配后端的 message 文本：那是中文固定串，一改就失配；而且
 * 「找不到」与「不是你的」后端刻意合并成了同一个 404，前端也不该
 * 把它们重新区分开。
 */
export function describeBindingError(status: number): BindingErrorKey {
  switch (status) {
    case 409:
      return BINDING_ERROR_KEYS.lastLoginMethod;
    case 404:
      return BINDING_ERROR_KEYS.notFound;
    case 403:
      return BINDING_ERROR_KEYS.forbidden;
    default:
      return BINDING_ERROR_KEYS.generic;
  }
}

/** 这次回跳是不是「绑定成功」。**精确匹配**，不做大小写折叠。 */
export function isOAuthBoundRedirect(search: string): boolean {
  return (
    new URLSearchParams(search).get(OAUTH_QUERY_PARAM) === OAUTH_BOUND_VALUE
  );
}

/** 提示过一次就把 `oauth` 参数摘掉，免得刷新页面又提示一遍。 */
export function stripOAuthQueryFlag(search: string): string {
  const params = new URLSearchParams(search);
  params.delete(OAUTH_QUERY_PARAM);
  const rest = params.toString();
  return rest ? `?${rest}` : '';
}
