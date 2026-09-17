import type { WorkspaceDeleteRequestInfo } from 'shared/types';
import type { WorkspaceDeleteAffordance } from '@vibe/ui/components/IssueWorkspaceCard';

/**
 * 工作区删除审批的**前端权限矩阵**，纯函数，单独可测。
 *
 * 后端才是权威（`DELETE /api/workspaces/{id}` 无条件 `require_admin`，
 * 审批接口整组挂在 `/api/admin` 下）。这里算出来的只是「界面上给不给按钮」，
 * 算错了也只会多一次注定被拒的请求，绝不会放行。
 */
export interface DeleteAffordanceInput {
  /** `isLocalPersonalMode()` 的结果。个人版整套审批 UI 都不存在。 */
  isPersonal: boolean;
  isAdmin: boolean;
  currentUserId: string | null;
  /** 这个工作区是不是当前用户建的。 */
  isOwnedByCurrentUser: boolean;
  /** 这个工作区当前待处理的删除申请；没有就传 `undefined`。 */
  pendingRequest: WorkspaceDeleteRequestInfo | undefined;
}

/**
 * 算出一张工作区卡片上关于删除的可见能力。
 *
 * **个人版返回 `undefined`**：卡片据此退回历史行为（只有自己的工作区能删、
 * 没有徽标、没有审批菜单），个人版界面上不会出现这套东西的任何痕迹。
 */
export function resolveDeleteAffordance(
  input: DeleteAffordanceInput
): WorkspaceDeleteAffordance | undefined {
  if (input.isPersonal) {
    return undefined;
  }

  const pending = Boolean(input.pendingRequest);

  return {
    mode: input.isAdmin ? 'delete' : 'request',
    // 管理员对任何工作区都能发起删除（后端也这么判）；成员只对自己建的
    // 工作区发起申请，和历史上「删除入口只在自己的卡上」保持一致。
    canInitiate: input.isAdmin || input.isOwnedByCurrentUser,
    pending,
    canDecide: pending && input.isAdmin,
    // 撤回只属于申请人本人。管理员想否掉一条申请要用「驳回」，
    // 那会留下 decided_by 记录；撤回不会。
    canWithdraw:
      pending &&
      input.currentUserId !== null &&
      input.pendingRequest?.requested_by_user_id === input.currentUserId,
  };
}

/** 按 `workspace_id` 索引待处理申请，供卡片按本地工作区 id 直接查。 */
export function indexRequestsByWorkspaceId(
  requests: WorkspaceDeleteRequestInfo[]
): Map<string, WorkspaceDeleteRequestInfo> {
  const map = new Map<string, WorkspaceDeleteRequestInfo>();
  for (const request of requests) {
    // 后端的条件唯一索引保证同一工作区只有一条待处理，这里遇到重复
    // 保留先到的那条（与后端「后来者不顶替原申请人」一致）。
    if (!map.has(request.workspace_id)) {
      map.set(request.workspace_id, request);
    }
  }
  return map;
}
