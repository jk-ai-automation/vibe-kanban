import { describe, expect, it } from 'vitest';
import {
  buildUserMenuItems,
  initials,
} from '@/features/local-auth/model/userMenu';

describe('用户菜单条目', () => {
  it('个人版不含登录、退出、成员管理', () => {
    expect(buildUserMenuItems({ mode: 'personal', user: null })).toEqual([]);
    expect(
      buildUserMenuItems({ mode: 'personal', user: { role: 'admin' } })
    ).toEqual([]);
  });

  it('团队版未登录只含登录', () => {
    expect(buildUserMenuItems({ mode: 'team', user: null })).toEqual([
      'signIn',
    ]);
  });

  it('团队版普通成员含退出、不含成员管理', () => {
    const items = buildUserMenuItems({
      mode: 'team',
      user: { role: 'member' },
    });
    expect(items).toContain('signOut');
    expect(items).not.toContain('members');
  });

  it('团队版管理员含成员管理', () => {
    const items = buildUserMenuItems({ mode: 'team', user: { role: 'admin' } });
    expect(items).toContain('members');
    expect(items).toContain('signOut');
  });
});

describe('头像首字母', () => {
  it('中文取第一个字', () => {
    expect(initials('张三')).toBe('张');
  });

  it('拉丁字母大写', () => {
    expect(initials('alice')).toBe('A');
  });

  it('空与全空白回退成问号', () => {
    expect(initials('')).toBe('?');
    expect(initials('  ')).toBe('?');
    expect(initials(null)).toBe('?');
    expect(initials(undefined)).toBe('?');
  });

  it('不把代理对切一半', () => {
    expect(initials('😀abc')).toBe('😀');
  });
});
