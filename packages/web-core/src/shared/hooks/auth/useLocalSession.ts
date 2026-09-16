import { useContext } from 'react';
import type { LocalAuthUser } from 'shared/types';
import { createHmrContext } from '@/shared/lib/hmrContext';

export interface LocalSessionContextValue {
  user: LocalAuthUser;
  /** 重新拉一次 bootstrap 与当前用户。 */
  refetch: () => void;
  signOut: () => Promise<void>;
}

export const LocalSessionContext = createHmrContext<
  LocalSessionContextValue | undefined
>('LocalSessionContext', undefined);

/**
 * 团队模式的当前会话。
 *
 * 个人版与云端模式下没有这个 Provider，返回 `null` 而不是抛错，
 * 这样共享组件可以用 `?? null` 无条件调用。
 */
export function useLocalSession(): LocalSessionContextValue | null {
  return useContext(LocalSessionContext) ?? null;
}
