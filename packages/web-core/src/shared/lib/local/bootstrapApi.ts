import type {
  AcceptInviteRequest,
  ApiResponse,
  ChangePasswordRequest,
  LocalAuthBootstrap,
  LocalAuthUser,
  LocalLoginRequest,
  OAuthBindStart,
  OAuthBindings,
  SetupAdminRequest,
  SetupStatusResponse,
} from 'shared/types';
import { makeLocalApiRequest } from '@/shared/lib/localApiTransport';
import { parseEnvelopeError } from '@/shared/lib/local/apiEnvelope';

export const LOCAL_AUTH_PATHS = {
  bootstrap: '/api/local-auth/bootstrap',
  login: '/api/local-auth/login',
  logout: '/api/local-auth/logout',
  me: '/api/local-auth/me',
  password: '/api/local-auth/password',
  setup: '/api/local-auth/setup',
  inviteAccept: '/api/local-auth/invites/accept',
  /** 只读，只返回当前用户自己的第三方绑定。 */
  oauthBindings: '/api/local-auth/oauth/bindings',
} as const;

/**
 * `/api/local-auth/*` **每一条都永远打本机后端**，所以这个文件里的每次
 * `makeLocalApiRequest` 都要带上它。
 *
 * 默认的 `hostScope: 'current'` 会把 `/api/xxx` 改写成
 * `/api/host/<id>/xxx` 转发到配对的另一台机器；而会话 Cookie 是本机这一份，
 * 转过去必然 401，`makeLocalApiRequest` 随即广播「会话过期」，
 * 用户就被莫名其妙踢回登录页了。
 *
 * 忘了写会被 `localOnlyHostScope.test.ts` 当场拦下。
 */
const 本机 = { hostScope: 'none' } as const;

/** 绑定 / 解绑的路径。`provider` 来自界面，进 URL 前一律编码。 */
function oauthProviderPath(provider: string, action: 'bind' | 'binding') {
  return `/api/local-auth/oauth/${encodeURIComponent(provider)}/${action}`;
}

/**
 * 登录失败。
 *
 * 后端对「用户不存在」「密码错误」「账号停用」「只有第三方登录」四种情况
 * 返回**完全相同**的 401 与英文固定文案——这是刻意的安全前提。所以前端
 * **只按状态码分支**，绝不去匹配 message 文本，否则会把四种情况又变得可区分。
 */
export class InvalidCredentialsError extends Error {
  constructor() {
    super('Invalid credentials');
    this.name = 'InvalidCredentialsError';
  }
}

/** 登录被限速（429）。 */
export class RateLimitedError extends Error {
  constructor() {
    super('Too many login attempts');
    this.name = 'RateLimitedError';
  }
}

/** 其它请求失败，错误信息里带上状态码方便排查。 */
export class LocalAuthRequestError extends Error {
  constructor(
    message: string,
    public readonly status: number
  ) {
    super(message);
    this.name = 'LocalAuthRequestError';
  }
}

async function readEnvelope<T>(
  response: Response,
  path: string
): Promise<ApiResponse<T>> {
  let envelope: ApiResponse<T> | null = null;
  try {
    envelope = (await response.json()) as ApiResponse<T>;
  } catch {
    envelope = null;
  }

  if (!response.ok) {
    throw new LocalAuthRequestError(
      `${path} failed with status ${response.status}`,
      response.status
    );
  }

  if (!envelope || !envelope.success) {
    throw new LocalAuthRequestError(
      envelope?.message ?? `${path} returned an unsuccessful response`,
      response.status
    );
  }

  return envelope;
}

async function getJson<T>(path: string): Promise<T> {
  const response = await makeLocalApiRequest(path, { method: 'GET', ...本机 });
  const envelope = await readEnvelope<T>(response, path);
  return envelope.data as T;
}

/**
 * 读运行时模式与可用登录方式。**免鉴权**，且响应里刻意不含任何用户信息。
 */
export function fetchBootstrap(): Promise<LocalAuthBootstrap> {
  return getJson<LocalAuthBootstrap>(LOCAL_AUTH_PATHS.bootstrap);
}

export function fetchMe(): Promise<LocalAuthUser> {
  return getJson<LocalAuthUser>(LOCAL_AUTH_PATHS.me);
}

export async function login(
  payload: LocalLoginRequest
): Promise<LocalAuthUser> {
  const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.login, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    ...本机,
  });

  if (response.status === 401) throw new InvalidCredentialsError();
  if (response.status === 429) throw new RateLimitedError();

  const envelope = await readEnvelope<LocalAuthUser>(
    response,
    LOCAL_AUTH_PATHS.login
  );
  return envelope.data as LocalAuthUser;
}

/**
 * 登出。**幂等**：会话已经没了（401）也算成功，不要因此把用户卡在界面上。
 */
export async function logout(): Promise<void> {
  try {
    const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.logout, {
      method: 'POST',
      ...本机,
    });
    if (response.status === 401) return;
    await readEnvelope<string>(response, LOCAL_AUTH_PATHS.logout);
  } catch (error) {
    if (error instanceof LocalAuthRequestError && error.status === 401) return;
    throw error;
  }
}

/**
 * 首启令牌缺失 / 错误 / 过期 / 已用——四种情况后端统一返回 401，
 * 前端也统一按「链接已失效」处理，不细分。
 */
export class SetupTokenInvalidError extends Error {
  constructor() {
    super('Setup token missing, wrong, or expired');
    this.name = 'SetupTokenInvalidError';
  }
}

/**
 * 查询首启令牌是否有效。**不消费令牌**，前端可以随便刷新这个请求。
 */
export async function fetchSetupStatus(
  token: string
): Promise<SetupStatusResponse> {
  const response = await makeLocalApiRequest(
    `${LOCAL_AUTH_PATHS.setup}?token=${encodeURIComponent(token)}`,
    { method: 'GET', ...本机 }
  );
  if (response.status === 401) throw new SetupTokenInvalidError();
  const envelope = await readEnvelope<SetupStatusResponse>(
    response,
    LOCAL_AUTH_PATHS.setup
  );
  return envelope.data as SetupStatusResponse;
}

/**
 * 建第一个管理员。
 *
 * 和登录不同，这里的失败文案是**特意写给人看的**（“已初始化”“用户名已被
 * 占用”“密码至少需要 8 个字节”……），所以 400/409 直接把 `message` 透出
 * 给调用方，而不是像 `readEnvelope` 那样丢弃——`SetupWizardPanel` 需要
 * 靠这句文案区分「该去登录页」和「换个用户名重试」（见
 * `features/local-auth/model/setupWizard.ts::isAlreadyInitializedMessage`）。
 */
export async function submitSetupAdmin(
  payload: SetupAdminRequest
): Promise<LocalAuthUser> {
  const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.setup, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    ...本机,
  });

  if (response.status === 401) throw new SetupTokenInvalidError();
  if (response.status === 429) throw new RateLimitedError();
  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new LocalAuthRequestError(
      message ??
        `${LOCAL_AUTH_PATHS.setup} failed with status ${response.status}`,
      response.status
    );
  }

  const envelope = await readEnvelope<LocalAuthUser>(
    response,
    LOCAL_AUTH_PATHS.setup
  );
  return envelope.data as LocalAuthUser;
}

/**
 * 邀请码自助注册（`POST /api/local-auth/invites/accept`，免鉴权）。
 *
 * 和 `submitSetupAdmin` 一样：失败文案是**特意写给人看的**（“邀请码无效或
 * 已过期”“用户名已被占用”“尝试过于频繁，请 N 秒后再试”……），400/409/429
 * 都把 `message` 透出给调用方，由
 * `features/local-auth/model/inviteAccept.ts::describeInviteAcceptError`
 * 决定怎么展示；调用方不应该再按状态码之外的东西（比如 message 文本）
 * 去猜是三种邀请码失败里的哪一种——那是后端刻意做的防枚举设计。
 *
 * 成功即登录：响应会带上 `vk_session` / `vk_csrf` 两条 Cookie，
 * 和登录、首启建号一样不需要调用方另外再登录一次。
 */
export async function acceptInvite(
  payload: AcceptInviteRequest
): Promise<LocalAuthUser> {
  const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.inviteAccept, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    ...本机,
  });

  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new LocalAuthRequestError(
      message ??
        `${LOCAL_AUTH_PATHS.inviteAccept} failed with status ${response.status}`,
      response.status
    );
  }

  const envelope = await readEnvelope<LocalAuthUser>(
    response,
    LOCAL_AUTH_PATHS.inviteAccept
  );
  return envelope.data as LocalAuthUser;
}

/**
 * 读当前用户的第三方绑定。**只返回自己的**——后端的查询条件里写死了
 * 会话用户 id，前端既传不了也问不到别人的。
 */
export async function fetchOAuthBindings(): Promise<OAuthBindings> {
  const path = LOCAL_AUTH_PATHS.oauthBindings;
  const response = await makeLocalApiRequest(path, {
    method: 'GET',
    ...本机,
  });
  const envelope = await readEnvelope<OAuthBindings>(response, path);
  return envelope.data as OAuthBindings;
}

/**
 * 发起绑定，拿回授权链接。
 *
 * 后端**刻意不回 302**：`fetch` 会跟着重定向去提供方的域名，拿回来的是一个
 * 不能用的跨域响应。调用方拿到链接之后自己 `window.location.assign`。
 */
export async function startOAuthBind(provider: string): Promise<string> {
  const path = oauthProviderPath(provider, 'bind');
  const response = await makeLocalApiRequest(path, {
    method: 'POST',
    ...本机,
  });
  const envelope = await readEnvelope<OAuthBindStart>(response, path);
  return (envelope.data as OAuthBindStart).authorize_url;
}

/**
 * 解绑。失败时把**状态码**原样抛出，由
 * `features/local-auth/model/bindings.ts::describeBindingError` 决定文案；
 * 这里不去读 message 文本（409「唯一登录方式」与 404「没有这条绑定」
 * 都靠状态码就能分清）。
 */
export async function unbindOAuth(provider: string): Promise<void> {
  const path = oauthProviderPath(provider, 'binding');
  const response = await makeLocalApiRequest(path, {
    method: 'DELETE',
    ...本机,
  });
  await readEnvelope<string>(response, path);
}

export async function changePassword(
  payload: ChangePasswordRequest
): Promise<LocalAuthUser> {
  const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.password, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    ...本机,
  });

  if (response.status === 401) throw new InvalidCredentialsError();

  const envelope = await readEnvelope<LocalAuthUser>(
    response,
    LOCAL_AUTH_PATHS.password
  );
  return envelope.data as LocalAuthUser;
}
