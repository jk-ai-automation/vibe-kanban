import { useEffect } from 'react';

import { SettingsDialog } from '@/shared/dialogs/settings/SettingsDialog';
import { isOAuthBoundRedirect } from '@/features/local-auth/model/bindings';
import { isLocalTeamMode } from '@/shared/lib/local/runtimeMode';

/**
 * 绑定第三方账号成功后，后端固定把浏览器送回 `/?oauth=bound`
 * （`oauth_routes.rs` 的 `BIND_REDIRECT`）。那是一次整页导航，
 * 用户落回看板，除非有人告诉他，否则他不知道刚才那一趟成没成。
 *
 * 这里认出那个参数，把设置对话框直接开到「账号」那一节：
 * 成功提示与刷新过的绑定列表都在那里。
 *
 * **参数故意不在这里摘掉**，留给账号设置那一块读完再摘——
 * 万一对话框没能打开，用户自己点进设置也还能看到提示。
 *
 * 个人版与云端构建下整个不存在：`isLocalTeamMode()` 为假时直接返回。
 */
export function OAuthBoundRedirectHandler() {
  useEffect(() => {
    if (!isLocalTeamMode()) return;
    if (typeof window === 'undefined') return;
    if (!isOAuthBoundRedirect(window.location.search)) return;
    void SettingsDialog.show({ initialSection: 'account' });
  }, []);

  return null;
}
