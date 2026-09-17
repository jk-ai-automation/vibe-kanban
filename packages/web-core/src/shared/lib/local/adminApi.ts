import type {
  AdminResetPasswordRequest,
  AdminUserInfo,
  ApiResponse,
  CreateInviteRequest,
  CreateInviteResponse,
  CreateLocalUserRequest,
  ListInvitesResponse,
  ListLocalUsersResponse,
  UpdateLocalUserRequest,
} from 'shared/types';
import { makeLocalApiRequest } from '@/shared/lib/localApiTransport';
import { parseEnvelopeError } from '@/shared/lib/local/apiEnvelope';

/**
 * 成员 / 邀请码管理接口客户端（`/api/admin/*`，需登录 + admin）。
 *
 * 照 `bootstrapApi.ts` 的写法：一律走 `makeLocalApiRequest`（拿 CSRF 头、
 * 401 自动触发会话过期广播），**按状态码分支**——但和登录不同，这里的
 * 400/403/404/409 文案是后端特意写给人看的，允许直接展示，所以错误对象
 * 里带上 `message`（见 `features/local-auth/model/members.ts::describeAdminError`）。
 */
export const ADMIN_API_PATHS = {
  users: '/api/admin/users',
  user: (id: string) => `/api/admin/users/${id}`,
  userPassword: (id: string) => `/api/admin/users/${id}/password`,
  invites: '/api/admin/invites',
  invite: (id: string) => `/api/admin/invites/${id}`,
} as const;

export class AdminApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = 'AdminApiError';
  }
}

/**
 * 走本地会话信封协议的通用请求。
 *
 * 也被 `workspaceDeleteRequestsApi.ts` 复用（那里既有 `/api/admin/*` 也有
 * 申请人侧的普通路径）：两边的错误语义完全一样——后端的 400/403/404/409
 * 文案是特意写给人看的，可以直接展示。
 */
export async function requestLocalEnvelope<T>(
  path: string,
  init: RequestInit = {}
): Promise<T> {
  const response = await makeLocalApiRequest(path, init);

  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new AdminApiError(
      response.status,
      message ?? `${path} failed with status ${response.status}`
    );
  }

  let envelope: ApiResponse<T> | null = null;
  try {
    envelope = (await response.json()) as ApiResponse<T>;
  } catch {
    envelope = null;
  }

  if (!envelope || !envelope.success) {
    throw new AdminApiError(
      response.status,
      envelope?.message ?? `${path} returned an unsuccessful response`
    );
  }

  return envelope.data as T;
}

const request = requestLocalEnvelope;

export const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
});

export function listMembers(): Promise<ListLocalUsersResponse> {
  return request(ADMIN_API_PATHS.users);
}

export function createMember(
  payload: CreateLocalUserRequest
): Promise<AdminUserInfo> {
  return request(ADMIN_API_PATHS.users, jsonInit('POST', payload));
}

export function updateMember(
  id: string,
  payload: UpdateLocalUserRequest
): Promise<AdminUserInfo> {
  return request(ADMIN_API_PATHS.user(id), jsonInit('PATCH', payload));
}

export async function resetMemberPassword(
  id: string,
  payload: AdminResetPasswordRequest
): Promise<void> {
  await request<string>(
    ADMIN_API_PATHS.userPassword(id),
    jsonInit('POST', payload)
  );
}

export function listInvites(): Promise<ListInvitesResponse> {
  return request(ADMIN_API_PATHS.invites);
}

export function createInvite(
  payload: CreateInviteRequest
): Promise<CreateInviteResponse> {
  return request(ADMIN_API_PATHS.invites, jsonInit('POST', payload));
}

export async function deleteInvite(id: string): Promise<void> {
  await request<string>(ADMIN_API_PATHS.invite(id), { method: 'DELETE' });
}
