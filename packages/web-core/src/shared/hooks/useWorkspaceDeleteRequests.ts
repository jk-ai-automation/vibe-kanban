import { useCallback, useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { WorkspaceDeleteRequestInfo } from 'shared/types';
import type { WorkspaceDeleteAffordance } from '@vibe/ui/components/IssueWorkspaceCard';
import { useAuth } from '@/shared/hooks/auth/useAuth';
import { useLocalSession } from '@/shared/hooks/auth/useLocalSession';
import { isLocalTeamMode } from '@/shared/lib/local/runtimeMode';
import {
  approveDeleteRequest,
  createDeleteRequest,
  listMyDeleteRequests,
  listPendingDeleteRequests,
  rejectDeleteRequest,
  withdrawDeleteRequest,
} from '@/shared/lib/local/workspaceDeleteRequestsApi';
import {
  indexRequestsByWorkspaceId,
  resolveDeleteAffordance,
} from '@/shared/lib/local/workspaceDeleteRequests';

export const DELETE_REQUESTS_QUERY_KEY = ['workspace-delete-requests'] as const;

/**
 * 工作区删除申请的读写入口。
 *
 * **个人版与云端构建彻底关掉**：`isLocalTeamMode()` 为假时查询
 * `enabled: false`，一个请求都不会发出去，`getAffordance` 恒返回
 * `undefined`，界面退回历史行为。
 *
 * 管理员拉审批队列（`/api/admin/*`），其他人拉「我的申请」——两个接口
 * 返回同一种行结构，因此 UI 只认一个 map。角色判断只影响拉哪个列表，
 * 真正的权限在后端。
 */
export function useWorkspaceDeleteRequests() {
  const queryClient = useQueryClient();
  // 只有「本地数据源 + 需要登录」的团队版才有这套接口。个人版没有，
  // 云端构建（数据源 remote）也没有——用 isLocalTeamMode 而不是
  // !isLocalPersonalMode，后者会把云端构建也算进来。
  const approvalEnabled = isLocalTeamMode();
  const localSession = useLocalSession();
  const { userId } = useAuth();
  const isAdmin = localSession?.user.role === 'admin';

  const query = useQuery({
    queryKey: [...DELETE_REQUESTS_QUERY_KEY, isAdmin ? 'queue' : 'mine'],
    queryFn: isAdmin ? listPendingDeleteRequests : listMyDeleteRequests,
    enabled: approvalEnabled,
    retry: false,
    // 申请与审批发生在不同的浏览器里，本地没有推送通道；轮询是最省事
    // 也最不容易出错的同步方式。30 秒对一个「等人审批」的流程足够。
    refetchInterval: approvalEnabled ? 30_000 : false,
  });

  const invalidate = useCallback(
    () =>
      queryClient.invalidateQueries({ queryKey: DELETE_REQUESTS_QUERY_KEY }),
    [queryClient]
  );

  const byWorkspaceId = useMemo(
    () => indexRequestsByWorkspaceId(query.data?.requests ?? []),
    [query.data]
  );

  const requests: WorkspaceDeleteRequestInfo[] = useMemo(
    () => query.data?.requests ?? [],
    [query.data]
  );

  const createMutation = useMutation({
    mutationFn: (workspaceId: string) =>
      createDeleteRequest({ workspace_id: workspaceId, reason: null }),
    onSettled: () => void invalidate(),
  });
  const withdrawMutation = useMutation({
    mutationFn: (requestId: string) => withdrawDeleteRequest(requestId),
    onSettled: () => void invalidate(),
  });
  const approveMutation = useMutation({
    mutationFn: (requestId: string) => approveDeleteRequest(requestId),
    onSettled: () => void invalidate(),
  });
  const rejectMutation = useMutation({
    mutationFn: (requestId: string) => rejectDeleteRequest(requestId),
    onSettled: () => void invalidate(),
  });

  /**
   * `isOwnedByCurrentUser` 必须由调用方传：成员只在自己建的工作区上有
   * 删除入口，而「是不是我的」这件事只有渲染那张卡的容器知道
   * （`workspace.owner_user_id === userId`）。
   */
  const getAffordance = useCallback(
    (
      workspaceId: string,
      isOwnedByCurrentUser: boolean
    ): WorkspaceDeleteAffordance | undefined =>
      resolveDeleteAffordance({
        approvalEnabled,
        isAdmin,
        currentUserId: userId,
        isOwnedByCurrentUser,
        pendingRequest: byWorkspaceId.get(workspaceId),
      }),
    [approvalEnabled, isAdmin, userId, byWorkspaceId]
  );

  return {
    approvalEnabled,
    isAdmin,
    requests,
    byWorkspaceId,
    isLoading: query.isPending && approvalEnabled,
    isError: query.isError,
    error: query.error,
    getAffordance,
    createRequest: createMutation.mutateAsync,
    withdrawRequest: withdrawMutation.mutateAsync,
    approveRequest: approveMutation.mutateAsync,
    rejectRequest: rejectMutation.mutateAsync,
    busyRequestId:
      (approveMutation.isPending ? approveMutation.variables : null) ??
      (rejectMutation.isPending ? rejectMutation.variables : null) ??
      null,
    refetch: invalidate,
  };
}
