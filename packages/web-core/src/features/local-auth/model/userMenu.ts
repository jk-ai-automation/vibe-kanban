import type { RuntimeMode } from '@/shared/lib/local/runtimeMode';

/** 用户菜单里和本地账号体系有关的条目。 */
export type UserMenuItemId = 'signIn' | 'signOut' | 'members';

export interface UserMenuInput {
  mode: RuntimeMode;
  /** 当前登录用户；未登录传 `null`。 */
  user: { role: string } | null;
}

/**
 * 个人版没有登录概念，所以既不显示「登录」也不显示「退出」。
 * 成员管理只对 admin 可见（界面在 G7，这里先把判定钉住）。
 */
export function buildUserMenuItems({
  mode,
  user,
}: UserMenuInput): UserMenuItemId[] {
  if (mode === 'personal') return [];
  if (!user) return ['signIn'];
  if (user.role === 'admin') return ['members', 'signOut'];
  return ['signOut'];
}

/**
 * 头像首字母。中文取第一个字，拉丁字母大写。
 * 用 `Array.from` 而不是 `[0]`，避免把 emoji / 代理对切一半。
 */
export function initials(displayName: string | null | undefined): string {
  if (!displayName) return '?';
  const trimmed = displayName.trim();
  if (trimmed.length === 0) return '?';
  const first = Array.from(trimmed)[0];
  return first.toUpperCase();
}
