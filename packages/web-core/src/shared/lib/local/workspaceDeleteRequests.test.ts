import { describe, expect, it } from 'vitest';
import type { WorkspaceDeleteRequestInfo } from 'shared/types';
import {
  indexRequestsByWorkspaceId,
  resolveDeleteAffordance,
} from '@/shared/lib/local/workspaceDeleteRequests';

function 申请(
  overrides: Partial<WorkspaceDeleteRequestInfo> = {}
): WorkspaceDeleteRequestInfo {
  return {
    id: 'req-1',
    workspace_id: 'ws-1',
    workspace_name: 'ws',
    workspace_branch: 'feat-a',
    requested_by_user_id: 'bob',
    requested_by_username: 'bob',
    reason: null,
    status: 'pending',
    created_at: '2026-09-17T00:00:00+00:00',
    ...overrides,
  };
}

const 基础 = {
  isPersonal: false,
  isAdmin: false,
  currentUserId: 'bob',
  isOwnedByCurrentUser: true,
  pendingRequest: undefined,
};

describe('resolveDeleteAffordance（删除审批的前端权限矩阵）', () => {
  it('个人版返回 undefined：整套审批 UI 都不出现', () => {
    expect(
      resolveDeleteAffordance({ ...基础, isPersonal: true, isAdmin: true })
    ).toBeUndefined();
    expect(
      resolveDeleteAffordance({
        ...基础,
        isPersonal: true,
        pendingRequest: 申请(),
      })
    ).toBeUndefined();
  });

  it('管理员是直接删，且对不是自己建的工作区也能删', () => {
    const a = resolveDeleteAffordance({
      ...基础,
      isAdmin: true,
      currentUserId: 'amy',
      isOwnedByCurrentUser: false,
    });
    expect(a).toMatchObject({
      mode: 'delete',
      canInitiate: true,
      pending: false,
      canDecide: false,
      canWithdraw: false,
    });
  });

  it('成员是提交申请，且只在自己建的工作区上有入口', () => {
    expect(resolveDeleteAffordance(基础)).toMatchObject({
      mode: 'request',
      canInitiate: true,
    });
    expect(
      resolveDeleteAffordance({ ...基础, isOwnedByCurrentUser: false })
    ).toMatchObject({ mode: 'request', canInitiate: false });
  });

  it('有待处理申请时管理员能批/驳，成员不能', () => {
    const 管理员 = resolveDeleteAffordance({
      ...基础,
      isAdmin: true,
      currentUserId: 'amy',
      pendingRequest: 申请(),
    });
    expect(管理员).toMatchObject({ pending: true, canDecide: true });

    const 成员 = resolveDeleteAffordance({
      ...基础,
      pendingRequest: 申请(),
    });
    expect(成员).toMatchObject({ pending: true, canDecide: false });
  });

  it('只有申请人本人能撤回', () => {
    expect(
      resolveDeleteAffordance({
        ...基础,
        currentUserId: 'bob',
        pendingRequest: 申请({ requested_by_user_id: 'bob' }),
      })?.canWithdraw
    ).toBe(true);

    expect(
      resolveDeleteAffordance({
        ...基础,
        currentUserId: 'eve',
        pendingRequest: 申请({ requested_by_user_id: 'bob' }),
      })?.canWithdraw
    ).toBe(false);
  });

  it('管理员也撤不了别人的申请（他该用驳回）', () => {
    expect(
      resolveDeleteAffordance({
        ...基础,
        isAdmin: true,
        currentUserId: 'amy',
        pendingRequest: 申请({ requested_by_user_id: 'bob' }),
      })?.canWithdraw
    ).toBe(false);
  });

  it('申请人账号已被删（requested_by_user_id 为 null）时没人能撤回', () => {
    expect(
      resolveDeleteAffordance({
        ...基础,
        currentUserId: 'bob',
        pendingRequest: 申请({ requested_by_user_id: null }),
      })?.canWithdraw
    ).toBe(false);
  });

  it('未登录（currentUserId 为 null）不会误判成申请人', () => {
    expect(
      resolveDeleteAffordance({
        ...基础,
        currentUserId: null,
        pendingRequest: 申请({ requested_by_user_id: null }),
      })?.canWithdraw
    ).toBe(false);
  });
});

describe('indexRequestsByWorkspaceId', () => {
  it('按工作区 id 建索引', () => {
    const map = indexRequestsByWorkspaceId([
      申请({ id: 'a', workspace_id: 'ws-1' }),
      申请({ id: 'b', workspace_id: 'ws-2' }),
    ]);
    expect(map.get('ws-1')?.id).toBe('a');
    expect(map.get('ws-2')?.id).toBe('b');
    expect(map.get('ws-3')).toBeUndefined();
  });

  it('同一工作区出现多条时保留先到的那条', () => {
    const map = indexRequestsByWorkspaceId([
      申请({ id: 'a', workspace_id: 'ws-1' }),
      申请({ id: 'b', workspace_id: 'ws-1' }),
    ]);
    expect(map.get('ws-1')?.id).toBe('a');
  });
});
