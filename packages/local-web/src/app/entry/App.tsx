import type { ReactNode } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { HotkeysProvider } from 'react-hotkeys-hook';
import { UserSystemProvider } from '@web/app/providers/ConfigProvider';
import { ClickedElementsProvider } from '@web/app/providers/ClickedElementsProvider';
import { localAppNavigation } from '@web/app/navigation/AppNavigation';
import { LocalAuthProvider } from '@/shared/providers/auth/LocalAuthProvider';
import { LocalSessionProvider } from '@/shared/providers/auth/LocalSessionProvider';
import { requiresLogin } from '@/shared/lib/local/runtimeMode';
import { AppRuntimeProvider } from '@/shared/hooks/useAppRuntime';
import { AppNavigationProvider } from '@/shared/hooks/useAppNavigation';
import { useTauriNotificationNavigation } from '@web/app/hooks/useTauriNotificationNavigation';
import { useTauriUpdateReady } from '@web/app/hooks/useTauriUpdateReady';
import { AppSystemNotifications } from '@web/app/notifications/AppSystemNotifications';
import { router } from '@web/app/router';

function TauriListeners() {
  useTauriNotificationNavigation();
  useTauriUpdateReady();
  return null;
}

/**
 * 认证边界：两个 Provider 都往 `AuthContext` 里写，**不能同时挂**。
 *
 * - 团队模式（`VK_MODE=team`）→ `LocalSessionProvider`：未登录时就地渲染登录页。
 * - 其余（个人版、云端构建）→ 沿用原来的 `LocalAuthProvider`，一行没动。
 *
 * `requiresLogin()` 的值来自入口那一次 bootstrap，整个进程生命周期内不变，
 * 所以这里不会出现 Provider 来回切换导致的状态丢失。
 */
function AuthBoundary({ children }: { children: ReactNode }) {
  if (requiresLogin()) {
    return <LocalSessionProvider>{children}</LocalSessionProvider>;
  }
  return <LocalAuthProvider>{children}</LocalAuthProvider>;
}

function App() {
  return (
    <AppRuntimeProvider runtime="local">
      <AppNavigationProvider value={localAppNavigation}>
        <TauriListeners />
        <UserSystemProvider>
          <AuthBoundary>
            <AppSystemNotifications />
            <ClickedElementsProvider>
              <HotkeysProvider
                initiallyActiveScopes={[
                  'global',
                  'workspace',
                  'kanban',
                  'projects',
                ]}
              >
                <RouterProvider router={router} />
              </HotkeysProvider>
            </ClickedElementsProvider>
          </AuthBoundary>
        </UserSystemProvider>
      </AppNavigationProvider>
    </AppRuntimeProvider>
  );
}

export default App;
