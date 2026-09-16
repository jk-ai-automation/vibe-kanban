import { useState } from 'react';
import type { OrganizationWithRole } from 'shared/types';
import { AppBarUserPopover } from '@vibe/ui/components/AppBarUserPopover';
import { SettingsDialog } from '@/shared/dialogs/settings/SettingsDialog';
import { useAuth } from '@/shared/hooks/auth/useAuth';
import { useUserSystem } from '@/shared/hooks/useUserSystem';
import { useOrganizationStore } from '@/shared/stores/useOrganizationStore';
import { useActions } from '@/shared/hooks/useActions';
import { Actions } from '@/shared/actions';
import { useLocalSession } from '@/shared/hooks/auth/useLocalSession';
import {
  isLocalPersonalMode,
  requiresLogin,
} from '@/shared/lib/local/runtimeMode';
import {
  buildUserMenuItems,
  initials,
} from '@/features/local-auth/model/userMenu';

interface AppBarUserPopoverContainerProps {
  organizations: OrganizationWithRole[];
  selectedOrgId: string;
  onOrgSelect: (orgId: string) => void;
}

export function AppBarUserPopoverContainer({
  organizations,
  selectedOrgId,
  onOrgSelect,
}: AppBarUserPopoverContainerProps) {
  const { executeAction } = useActions();
  const { isSignedIn } = useAuth();
  const { loginStatus } = useUserSystem();
  const localSession = useLocalSession();
  const setSelectedOrgId = useOrganizationStore((s) => s.setSelectedOrgId);
  const [open, setOpen] = useState(false);
  const [avatarError, setAvatarError] = useState(false);

  // Extract avatar URL from first provider
  const avatarUrl =
    loginStatus?.status === 'loggedin'
      ? (loginStatus.profile?.providers[0]?.avatar_url ?? null)
      : null;

  // 个人版没有登录概念，两个入口都不出现；团队模式按当前用户与角色决定。
  // `members` 条目的界面在计划任务 G7，这里先只消费登录 / 退出。
  const menuItems = buildUserMenuItems({
    mode: requiresLogin() ? 'team' : 'personal',
    user: localSession?.user ?? null,
  });
  const isTeam = requiresLogin();
  const isPersonal = isLocalPersonalMode();
  const showSignIn = isTeam ? menuItems.includes('signIn') : !isPersonal;
  const showSignOut = isTeam ? menuItems.includes('signOut') : !isPersonal;
  const showMembers = isTeam && menuItems.includes('members');

  const handleSignIn = async () => {
    await executeAction(Actions.SignIn);
  };

  const handleMembers = () => {
    setOpen(false);
    // `/members` 是团队模式独有的路由，走整页跳转而不是 `useAppNavigation`：
    // 那套抽象是本地/远端两端共享的目的地枚举，个人版与云端构建都没有这个
    // 页面，不值得为一个团队模式专属入口扩出一个新的 AppDestination 分支。
    window.location.assign('/members');
  };

  const handleLogout = async () => {
    if (localSession) {
      await localSession.signOut();
      return;
    }
    await executeAction(Actions.SignOut);
  };

  const handleOrgSettings = async (orgId: string) => {
    setSelectedOrgId(orgId);
    await SettingsDialog.show({ initialSection: 'organizations' });
  };

  const handleSettings = async () => {
    setOpen(false);
    await SettingsDialog.show();
  };

  return (
    <AppBarUserPopover
      isSignedIn={isSignedIn}
      avatarUrl={avatarUrl}
      avatarError={avatarError}
      organizations={organizations}
      selectedOrgId={selectedOrgId}
      open={open}
      onOpenChange={setOpen}
      onOrgSelect={onOrgSelect}
      onOrgSettings={handleOrgSettings}
      onSignIn={handleSignIn}
      onLogout={handleLogout}
      onAvatarError={() => setAvatarError(true)}
      onSettings={handleSettings}
      onMembers={showMembers ? handleMembers : undefined}
      showSignIn={showSignIn}
      showSignOut={showSignOut}
      accountName={localSession?.user.display_name ?? null}
      accountInitials={
        localSession ? initials(localSession.user.display_name) : null
      }
    />
  );
}
