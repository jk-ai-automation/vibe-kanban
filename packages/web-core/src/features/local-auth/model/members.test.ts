import { describe, expect, it } from 'vitest';

import {
  buildInviteLink,
  canChangeRole,
  canDisable,
  canRevokeInvite,
  countLoginCapableAdminsExcept,
  describeAdminError,
  getInviteStatus,
  isAccountFormValid,
  isEditMemberFormValid,
  isFixedLocalUser,
  isInviteFormValid,
  isLoginCapableAdmin,
  sortInvitesByCreatedAtDesc,
  sortMembers,
  validateAccountForm,
  validateEditMemberForm,
  validateInviteForm,
  validateResetPasswordForm,
  type MemberLike,
} from '@/features/local-auth/model/members';

function member(overrides: Partial<MemberLike> = {}): MemberLike {
  return {
    id: 'u-amy',
    username: 'amy',
    role: 'admin',
    status: 'active',
    ...overrides,
  };
}

describe('isFixedLocalUser', () => {
  it('username 为 local 时判定为本机固定用户', () => {
    expect(isFixedLocalUser({ username: 'local' })).toBe(true);
    expect(isFixedLocalUser({ username: 'amy' })).toBe(false);
  });
});

describe('isLoginCapableAdmin / countLoginCapableAdminsExcept', () => {
  it('本机固定用户即便是 active admin 也不算可登录管理员', () => {
    expect(
      isLoginCapableAdmin(
        member({ id: 'local-id', username: 'local', role: 'admin' })
      )
    ).toBe(false);
  });

  it('停用的 admin 不算可登录管理员', () => {
    expect(isLoginCapableAdmin(member({ status: 'disabled' }))).toBe(false);
  });

  it('member 角色不算可登录管理员', () => {
    expect(isLoginCapableAdmin(member({ role: 'member' }))).toBe(false);
  });

  it('排除自己后数其余可登录管理员', () => {
    const members = [
      member({ id: 'a', username: 'amy' }),
      member({ id: 'z', username: 'zoe' }),
      member({ id: 'local-id', username: 'local' }),
      member({ id: 'b', username: 'bob', role: 'member' }),
    ];
    expect(countLoginCapableAdminsExcept(members, 'a')).toBe(1);
    expect(countLoginCapableAdminsExcept(members, 'z')).toBe(1);
  });
});

describe('sortMembers', () => {
  it('admin 排在 member 前面，同角色按用户名排序', () => {
    const sorted = sortMembers([
      { role: 'member', username: 'zoe' },
      { role: 'admin', username: 'bob' },
      { role: 'member', username: 'amy' },
      { role: 'admin', username: 'amy' },
    ]);
    expect(sorted.map((m) => `${m.role}:${m.username}`)).toEqual([
      'admin:amy',
      'admin:bob',
      'member:amy',
      'member:zoe',
    ]);
  });

  it('不修改原数组', () => {
    const input = [{ role: 'member', username: 'z' }];
    const sorted = sortMembers(input);
    expect(sorted).not.toBe(input);
  });
});

describe('canChangeRole', () => {
  const amy = member({ id: 'amy', username: 'amy', role: 'admin' });
  const zoe = member({ id: 'zoe', username: 'zoe', role: 'admin' });
  const bob = member({ id: 'bob', username: 'bob', role: 'member' });
  const local = member({ id: 'local-id', username: 'local', role: 'admin' });

  it('自己不能改自己的角色', () => {
    expect(canChangeRole(amy, amy, [amy, zoe])).toBe(false);
  });

  it('actor 不是 admin 时不能改任何人的角色', () => {
    expect(canChangeRole(bob, zoe, [amy, zoe, bob])).toBe(false);
  });

  it('本机固定用户不能改角色', () => {
    expect(canChangeRole(amy, local, [amy, local])).toBe(false);
  });

  it('降级最后一个可登录管理员会清零，不允许', () => {
    // bob 是唯一发起者，但他不是 admin；换个有效场景：zoe 是唯一别的管理员，
    // 若把 zoe 降级，amy 会话之外就没有别的可登录管理员——但 amy 自己还在，
    // 所以只有当「唯一的其他管理员」是被改的目标本身时才会清零。
    const onlyAdmin = member({ id: 'only', username: 'only', role: 'admin' });
    expect(canChangeRole(amy, onlyAdmin, [amy, onlyAdmin])).toBe(true);
    // amy 自己也被排除在外时（amy 是 member 场景不成立，改用 local 顶替 amy）
    expect(canChangeRole(local, onlyAdmin, [local, onlyAdmin])).toBe(false);
  });

  it('还有其它可登录管理员时允许改角色', () => {
    expect(canChangeRole(amy, zoe, [amy, zoe])).toBe(true);
  });

  it('降级 member 或改角色到相同值不受最后管理员限制', () => {
    expect(canChangeRole(amy, bob, [amy, bob])).toBe(true);
  });
});

describe('canDisable', () => {
  const amy = member({ id: 'amy', username: 'amy', role: 'admin' });
  const zoe = member({ id: 'zoe', username: 'zoe', role: 'admin' });
  const bob = member({ id: 'bob', username: 'bob', role: 'member' });
  const local = member({ id: 'local-id', username: 'local', role: 'admin' });

  it('不能停用自己', () => {
    expect(canDisable(amy, amy, [amy, zoe])).toBe(false);
  });

  it('不能停用本机固定用户', () => {
    expect(canDisable(amy, local, [amy, local])).toBe(false);
  });

  it('不能停用最后一个可登录管理员', () => {
    const onlyAdmin = member({ id: 'only', username: 'only', role: 'admin' });
    expect(canDisable(local, onlyAdmin, [local, onlyAdmin])).toBe(false);
  });

  it('还有别的可登录管理员时可以停用', () => {
    expect(canDisable(amy, zoe, [amy, zoe])).toBe(true);
  });

  it('可以停用普通成员', () => {
    expect(canDisable(amy, bob, [amy, bob])).toBe(true);
  });
});

describe('validateAccountForm', () => {
  it('合法输入没有错误', () => {
    expect(
      isAccountFormValid(
        validateAccountForm({
          username: 'amy',
          password: 'hunter2hunter2',
          email: '',
        })
      )
    ).toBe(true);
  });

  it('用户名为空', () => {
    expect(
      validateAccountForm({
        username: '  ',
        password: 'hunter2hunter2',
        email: '',
      }).username
    ).toBe('errors.usernameRequired');
  });

  it('用户名包含非法字符', () => {
    expect(
      validateAccountForm({
        username: 'ad min',
        password: 'hunter2hunter2',
        email: '',
      }).username
    ).toBe('errors.usernameInvalid');
  });

  it('用户名超长', () => {
    expect(
      validateAccountForm({
        username: 'a'.repeat(65),
        password: 'hunter2hunter2',
        email: '',
      }).username
    ).toBe('errors.usernameInvalid');
  });

  it('密码按字节数算，emoji 密码不足 8 字节应报错', () => {
    // 单个 emoji 在 UTF-8 里是 4 字节，两个 emoji = 8 字节，刚好达标。
    expect(
      validateAccountForm({ username: 'amy', password: '🙂🙂', email: '' })
        .password
    ).toBeUndefined();
    expect(
      validateAccountForm({ username: 'amy', password: '🙂', email: '' })
        .password
    ).toBe('errors.passwordTooShort');
  });

  it('密码过长（超过 1024 字节）', () => {
    expect(
      validateAccountForm({
        username: 'amy',
        password: 'a'.repeat(1025),
        email: '',
      }).password
    ).toBe('errors.passwordTooLong');
  });

  it('邮箱留空合法', () => {
    expect(
      validateAccountForm({
        username: 'amy',
        password: 'hunter2hunter2',
        email: '',
      }).email
    ).toBeUndefined();
  });

  it('邮箱格式不对', () => {
    expect(
      validateAccountForm({
        username: 'amy',
        password: 'hunter2hunter2',
        email: 'not-an-email',
      }).email
    ).toBe('errors.emailInvalid');
  });
});

describe('validateEditMemberForm', () => {
  it('显示名为空报错', () => {
    expect(
      isEditMemberFormValid(
        validateEditMemberForm({ displayName: '  ', email: '' })
      )
    ).toBe(false);
  });

  it('合法输入通过', () => {
    expect(
      isEditMemberFormValid(
        validateEditMemberForm({ displayName: 'Amy', email: 'a@b.com' })
      )
    ).toBe(true);
  });
});

describe('validateResetPasswordForm', () => {
  it('弱密码报错', () => {
    expect(validateResetPasswordForm('short').password).toBe(
      'errors.passwordTooShort'
    );
  });

  it('合法密码没有错误', () => {
    expect(validateResetPasswordForm('hunter2hunter2')).toEqual({});
  });
});

describe('validateInviteForm', () => {
  it('合法天数通过', () => {
    expect(
      isInviteFormValid(
        validateInviteForm({ role: 'member', expiresInDays: 7 })
      )
    ).toBe(true);
  });

  it('0 天、负数、超过 30 天、非整数都报错', () => {
    for (const days of [0, -1, 31, 1.5]) {
      expect(
        validateInviteForm({ role: 'member', expiresInDays: days })
          .expiresInDays
      ).toBe('errors.inviteTtlInvalid');
    }
  });

  it('边界值 1 与 30 合法', () => {
    expect(
      validateInviteForm({ role: 'admin', expiresInDays: 1 }).expiresInDays
    ).toBeUndefined();
    expect(
      validateInviteForm({ role: 'admin', expiresInDays: 30 }).expiresInDays
    ).toBeUndefined();
  });
});

describe('getInviteStatus', () => {
  const now = new Date('2026-01-01T00:00:00Z');

  it('已使用优先于过期', () => {
    expect(
      getInviteStatus(
        {
          used_at: '2025-01-01T00:00:00Z',
          expires_at: '2025-01-02T00:00:00Z',
        },
        now
      )
    ).toBe('used');
  });

  it('未使用且已过期', () => {
    expect(
      getInviteStatus(
        { used_at: null, expires_at: '2025-12-31T00:00:00Z' },
        now
      )
    ).toBe('expired');
  });

  it('未使用且未过期', () => {
    expect(
      getInviteStatus(
        { used_at: null, expires_at: '2026-06-01T00:00:00Z' },
        now
      )
    ).toBe('pending');
  });
});

describe('canRevokeInvite', () => {
  it('已使用的邀请不能作废', () => {
    expect(
      canRevokeInvite({ used_at: '2026-01-01T00:00:00Z', expires_at: 'x' })
    ).toBe(false);
  });

  it('未使用的邀请（无论是否过期）可以作废', () => {
    expect(canRevokeInvite({ used_at: null, expires_at: 'x' })).toBe(true);
  });
});

describe('sortInvitesByCreatedAtDesc', () => {
  it('新邀请排在前面', () => {
    const sorted = sortInvitesByCreatedAtDesc([
      { created_at: '2026-01-01T00:00:00Z' },
      { created_at: '2026-03-01T00:00:00Z' },
      { created_at: '2026-02-01T00:00:00Z' },
    ]);
    expect(sorted.map((i) => i.created_at)).toEqual([
      '2026-03-01T00:00:00Z',
      '2026-02-01T00:00:00Z',
      '2026-01-01T00:00:00Z',
    ]);
  });
});

describe('buildInviteLink', () => {
  it('拼出 /login?invite= 链接', () => {
    expect(buildInviteLink('http://127.0.0.1:8080', 'abc')).toBe(
      'http://127.0.0.1:8080/login?invite=abc'
    );
  });

  it('origin 末尾多个斜杠不会拼出多余的 //', () => {
    expect(buildInviteLink('http://127.0.0.1:8080/', 'abc')).toBe(
      'http://127.0.0.1:8080/login?invite=abc'
    );
  });
});

describe('describeAdminError', () => {
  it('400/403/404/409 直接展示后端文案', () => {
    for (const status of [400, 403, 404, 409]) {
      expect(describeAdminError({ status, message: '不能停用自己' })).toEqual({
        kind: 'message',
        text: '不能停用自己',
      });
    }
  });

  it('其它状态码给通用兜底 i18n key', () => {
    expect(describeAdminError({ status: 500, message: 'boom' })).toEqual({
      kind: 'i18nKey',
      key: 'localAuth.networkError',
    });
    expect(describeAdminError({ status: 401, message: 'boom' })).toEqual({
      kind: 'i18nKey',
      key: 'localAuth.networkError',
    });
  });
});
