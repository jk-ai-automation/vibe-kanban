import { describe, expect, it } from 'vitest';

import {
  extractSetupToken,
  isAlreadyInitializedMessage,
} from '@/features/local-auth/model/setupWizard';

describe('extractSetupToken', () => {
  it('从 ?setup= 里取出令牌', () => {
    expect(extractSetupToken('?setup=abc123')).toBe('abc123');
  });

  it('没有 setup 参数时返回 null', () => {
    expect(extractSetupToken('?foo=bar')).toBeNull();
    expect(extractSetupToken('')).toBeNull();
  });

  it('空值当作没有', () => {
    expect(extractSetupToken('?setup=')).toBeNull();
    expect(extractSetupToken('?setup=   ')).toBeNull();
  });

  it('两侧空白被裁剪', () => {
    expect(extractSetupToken('?setup=%20abc%20')).toBe('abc');
  });

  it('和其它参数共存时仍能取到', () => {
    expect(extractSetupToken('?redirect=/foo&setup=xyz')).toBe('xyz');
  });
});

describe('isAlreadyInitializedMessage', () => {
  it('精确匹配后端常量文案', () => {
    expect(isAlreadyInitializedMessage('已初始化')).toBe(true);
  });

  it('其它文案（包括用户名冲突）不匹配', () => {
    expect(isAlreadyInitializedMessage('用户名已被占用')).toBe(false);
    expect(isAlreadyInitializedMessage(null)).toBe(false);
    expect(isAlreadyInitializedMessage(undefined)).toBe(false);
    expect(isAlreadyInitializedMessage('')).toBe(false);
  });
});
