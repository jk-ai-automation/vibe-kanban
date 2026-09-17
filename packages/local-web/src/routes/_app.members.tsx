import { createFileRoute, redirect } from '@tanstack/react-router';
import { isPersonalMode } from '@/shared/lib/local/runtimeMode';
import { MembersPageContainer } from '@/features/local-auth/ui/MembersPageContainer';

/**
 * 成员管理页（`/members`）。
 *
 * 个人版没有账号体系，直接把路由挡在门外，不渲染任何东西——真正的权限
 * 判断（是不是 admin）在 `MembersPageContainer` 里做，因为那需要已经拿到
 * 手的本地会话，`beforeLoad` 这一层拿不到。
 */
export const Route = createFileRoute('/_app/members')({
  beforeLoad: () => {
    if (isPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: MembersPageContainer,
});
