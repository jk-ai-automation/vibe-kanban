import { describe, expect, it } from 'vitest';

import {
  describeInviteAcceptError,
  extractInviteCode,
  resolveInviteCode,
} from '@/features/local-auth/model/inviteAccept';

describe('extractInviteCode', () => {
  it('从 ?invite= 里取出邀请码', () => {
    expect(extractInviteCode('?invite=abc123')).toBe('abc123');
  });

  it('没有 invite 参数时返回 null', () => {
    expect(extractInviteCode('?foo=bar')).toBeNull();
    expect(extractInviteCode('')).toBeNull();
  });

  it('空值当作没有', () => {
    expect(extractInviteCode('?invite=')).toBeNull();
    expect(extractInviteCode('?invite=   ')).toBeNull();
  });

  it('两侧空白被裁剪', () => {
    expect(extractInviteCode('?invite=%20abc%20')).toBe('abc');
  });

  it('和其它参数共存时仍能取到', () => {
    expect(extractInviteCode('?redirect=/foo&invite=xyz')).toBe('xyz');
  });
});

describe('resolveInviteCode', () => {
  it('团队版透传邀请码', () => {
    expect(resolveInviteCode('?invite=abc123', false)).toBe('abc123');
  });

  it('个人版无条件忽略邀请码', () => {
    expect(resolveInviteCode('?invite=abc123', true)).toBeNull();
  });

  it('个人版且没有邀请码也是 null', () => {
    expect(resolveInviteCode('', true)).toBeNull();
  });
});

describe('describeInviteAcceptError', () => {
  it('400 直接展示后端文案（邀请码无效/过期/已用、弱密码、非法用户名共用这条路径）', () => {
    expect(describeInviteAcceptError(400, '邀请码无效或已过期')).toEqual({
      kind: 'message',
      text: '邀请码无效或已过期',
    });
  });

  it('409 直接展示后端文案（用户名或邮箱冲突）', () => {
    expect(describeInviteAcceptError(409, '用户名已被占用')).toEqual({
      kind: 'message',
      text: '用户名已被占用',
    });
  });

  it('429 直接展示后端文案（含剩余秒数）', () => {
    expect(
      describeInviteAcceptError(429, '尝试过于频繁，请 30 秒后再试')
    ).toEqual({
      kind: 'message',
      text: '尝试过于频繁，请 30 秒后再试',
    });
  });

  it('其它状态码（404/5xx/意外情况）给通用兜底文案', () => {
    expect(describeInviteAcceptError(404, '不应该出现')).toEqual({
      kind: 'i18nKey',
      key: 'localAuth.networkError',
    });
    expect(describeInviteAcceptError(500, 'boom')).toEqual({
      kind: 'i18nKey',
      key: 'localAuth.networkError',
    });
  });
});
