import { useCallback, useEffect, useMemo, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import type { LocalAuthBootstrap } from 'shared/types';
import { SetupRequiredPanel } from '@vibe/ui/components/SetupRequiredPanel';
import {
  AuthContext,
  type AuthContextValue,
} from '@/shared/hooks/auth/useAuth';
import {
  LocalSessionContext,
  type LocalSessionContextValue,
} from '@/shared/hooks/auth/useLocalSession';
import {
  fetchBootstrap,
  fetchMe,
  logout,
} from '@/shared/lib/local/bootstrapApi';
import { getBootstrapSnapshot } from '@/shared/lib/local/runtimeMode';
import { onSessionExpired } from '@/shared/lib/local/sessionExpiry';
import { LoginPageContainer } from '@/features/local-auth/ui/LoginPageContainer';

export const LOCAL_AUTH_QUERY_KEY = ['local-auth'] as const;
const BOOTSTRAP_QUERY_KEY = ['local-auth', 'bootstrap'] as const;
const ME_QUERY_KEY = ['local-auth', 'me'] as const;

function FullScreenMessage({ message }: { message: string }) {
  return (
    <div className="min-h-screen w-full flex items-center justify-center bg-primary text-low text-base">
      {message}
    </div>
  );
}

/**
 * 团队模式的会话上下文。
 *
 * 只在 `requiresLogin()` 为真时挂载，所以**个人版完全走不到这里**——这是
 * 「个人版零回退」的结构性保证，而不是靠运行时判断。
 *
 * 未登录时它**就地**渲染登录页而不是跳路由：URL 保持不变，登录成功后重载
 * 回同一个地址，原本要去的页面自然就保住了。
 */
export function LocalSessionProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const bootstrapQuery = useQuery<LocalAuthBootstrap>({
    queryKey: BOOTSTRAP_QUERY_KEY,
    queryFn: fetchBootstrap,
    initialData: getBootstrapSnapshot() ?? undefined,
    retry: false,
    refetchOnWindowFocus: false,
  });

  const bootstrap = bootstrapQuery.data;
  const needsSetup = bootstrap?.needs_setup === true;
  const authenticated = bootstrap?.authenticated === true && !needsSetup;

  const meQuery = useQuery({
    queryKey: ME_QUERY_KEY,
    queryFn: fetchMe,
    enabled: authenticated,
    retry: false,
    refetchOnWindowFocus: false,
  });

  // 会话过期（团队模式下的 401，不含 403）：把 bootstrap 快照翻成未登录，
  // 下一帧就渲染登录页。URL 不动，所以原目标路由被保留。
  useEffect(
    () =>
      onSessionExpired(() => {
        queryClient.setQueryData<LocalAuthBootstrap>(
          BOOTSTRAP_QUERY_KEY,
          (previous) =>
            previous ? { ...previous, authenticated: false } : previous
        );
        queryClient.removeQueries({ queryKey: ME_QUERY_KEY });
      }),
    [queryClient]
  );

  const handleSignOut = useCallback(async () => {
    try {
      await logout();
    } finally {
      // 清掉所有本地缓存，避免下一个登录的人看到上一个人的数据。
      queryClient.clear();
      window.location.assign('/');
    }
  }, [queryClient]);

  const refetch = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: LOCAL_AUTH_QUERY_KEY });
  }, [queryClient]);

  const user = meQuery.data;

  const authValue = useMemo<AuthContextValue>(
    () => ({
      isSignedIn: user !== undefined,
      isLoaded: user !== undefined,
      userId: user?.id ?? null,
    }),
    [user]
  );

  const sessionValue = useMemo<LocalSessionContextValue | undefined>(
    () => (user ? { user, refetch, signOut: handleSignOut } : undefined),
    [handleSignOut, refetch, user]
  );

  if (needsSetup) {
    return (
      <SetupRequiredPanel
        isChecking={bootstrapQuery.isFetching}
        onRetry={refetch}
      />
    );
  }

  if (!bootstrap && bootstrapQuery.isLoading) {
    return <FullScreenMessage message={t('localAuth.loading')} />;
  }

  if (!authenticated) {
    // bootstrap 拉不到时也渲染登录页：团队模式下反正要登录，
    // 直接给一个可操作的界面比白屏或崩溃页有用。
    return (
      <LoginPageContainer
        providers={bootstrap?.providers ?? []}
        initialError={
          bootstrapQuery.isError ? t('localAuth.networkError') : null
        }
      />
    );
  }

  if (!user) {
    return (
      <FullScreenMessage
        message={
          meQuery.isError ? t('localAuth.networkError') : t('localAuth.loading')
        }
      />
    );
  }

  return (
    <AuthContext.Provider value={authValue}>
      <LocalSessionContext.Provider value={sessionValue}>
        {children}
      </LocalSessionContext.Provider>
    </AuthContext.Provider>
  );
}
