import { useContext } from 'react';
import { createHmrContext } from '@/shared/lib/hmrContext';
import { isLocalMode } from '@/shared/lib/local/dataSource';
import { LOCAL_USER_ID } from '@/shared/lib/local/identity';

export interface AuthContextValue {
  isSignedIn: boolean;
  isLoaded: boolean;
  userId: string | null;
}

export const AuthContext = createHmrContext<AuthContextValue | undefined>(
  'AuthContext',
  undefined
);

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);

  // 个人版没有登录概念：固定返回已登录的本机用户，
  // 这样所有依赖 isSignedIn 的组件与 shape 订阅都能正常工作。
  if (isLocalMode()) {
    return { isSignedIn: true, isLoaded: true, userId: LOCAL_USER_ID };
  }

  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
