import type {
  ApproveWorkspaceDeleteRequestRequest,
  CreateWorkspaceDeleteRequestRequest,
  ListWorkspaceDeleteRequestsResponse,
  RejectWorkspaceDeleteRequestRequest,
  WorkspaceDeleteRequestDecisionResponse,
  WorkspaceDeleteRequestInfo,
} from 'shared/types';
import { jsonInit, requestLocalEnvelope } from '@/shared/lib/local/adminApi';

/**
 * 工作区删除申请的接口客户端。
 *
 * 路径分成两组，**这个区分就是权限模型本身**：
 * - `/api/workspace-delete-requests*`：申请人侧，任何登录用户可调，
 *   列表永远只返回自己的申请；
 * - `/api/admin/workspace-delete-requests*`：审批侧，后端整组挂在
 *   `require_admin_middleware` 下，member 调一律 403。
 *
 * 错误统一是 `AdminApiError`（见 `adminApi.ts`），`message` 可以直接展示。
 */
export const DELETE_REQUEST_API_PATHS = {
  mine: '/api/workspace-delete-requests',
  withdraw: (id: string) => `/api/workspace-delete-requests/${id}`,
  queue: '/api/admin/workspace-delete-requests',
  approve: (id: string) => `/api/admin/workspace-delete-requests/${id}/approve`,
  reject: (id: string) => `/api/admin/workspace-delete-requests/${id}/reject`,
} as const;

/** 我自己提交的、还没被处理的删除申请。 */
export function listMyDeleteRequests(): Promise<ListWorkspaceDeleteRequestsResponse> {
  return requestLocalEnvelope(DELETE_REQUEST_API_PATHS.mine);
}

export function createDeleteRequest(
  payload: CreateWorkspaceDeleteRequestRequest
): Promise<WorkspaceDeleteRequestInfo> {
  return requestLocalEnvelope(
    DELETE_REQUEST_API_PATHS.mine,
    jsonInit('POST', payload)
  );
}

export async function withdrawDeleteRequest(id: string): Promise<void> {
  await requestLocalEnvelope<string>(DELETE_REQUEST_API_PATHS.withdraw(id), {
    method: 'DELETE',
  });
}

/** 全部待处理申请。仅管理员可调（后端 403）。 */
export function listPendingDeleteRequests(): Promise<ListWorkspaceDeleteRequestsResponse> {
  return requestLocalEnvelope(DELETE_REQUEST_API_PATHS.queue);
}

/** 批准 = 当场删除工作区。仅管理员可调。 */
export function approveDeleteRequest(
  id: string,
  payload: ApproveWorkspaceDeleteRequestRequest = { delete_branches: null }
): Promise<WorkspaceDeleteRequestDecisionResponse> {
  return requestLocalEnvelope(
    DELETE_REQUEST_API_PATHS.approve(id),
    jsonInit('POST', payload)
  );
}

export function rejectDeleteRequest(
  id: string,
  payload: RejectWorkspaceDeleteRequestRequest = { note: null }
): Promise<WorkspaceDeleteRequestDecisionResponse> {
  return requestLocalEnvelope(
    DELETE_REQUEST_API_PATHS.reject(id),
    jsonInit('POST', payload)
  );
}
