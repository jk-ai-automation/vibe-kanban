import type {
  AcceptInviteRequest,
  ApiResponse,
  ChangePasswordRequest,
  LocalAuthBootstrap,
  LocalAuthUser,
  LocalLoginRequest,
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
} as const;

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
  const response = await makeLocalApiRequest(path, { method: 'GET' });
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
    { method: 'GET' }
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

export async function changePassword(
  payload: ChangePasswordRequest
): Promise<LocalAuthUser> {
  const response = await makeLocalApiRequest(LOCAL_AUTH_PATHS.password, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });

  if (response.status === 401) throw new InvalidCredentialsError();

  const envelope = await readEnvelope<LocalAuthUser>(
    response,
    LOCAL_AUTH_PATHS.password
  );
  return envelope.data as LocalAuthUser;
}
