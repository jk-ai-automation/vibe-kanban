import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  notifySessionExpired,
  onSessionExpired,
  resetSessionExpiryListenersForTests,
  shouldTreatAsSessionExpired,
} from '@/shared/lib/local/sessionExpiry';

describe('会话过期判定', () => {
  it('团队模式的 401 是会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({ status: 401, requiresLogin: true })
    ).toBe(true);
  });

  it('个人版的 401 不是会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({ status: 401, requiresLogin: false })
    ).toBe(false);
  });

  it('403 不是会话过期：CSRF 头缺失再登录多少次也修不好', () => {
    expect(
      shouldTreatAsSessionExpired({ status: 403, requiresLogin: true })
    ).toBe(false);
  });

  it('500 不是会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({ status: 500, requiresLogin: true })
    ).toBe(false);
  });

  it('200 不是会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({ status: 200, requiresLogin: true })
    ).toBe(false);
  });

  it('登录接口自身的 401 是「用户名或密码错误」，不是会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({
        status: 401,
        requiresLogin: true,
        path: '/api/local-auth/login',
      })
    ).toBe(false);
  });

  it('bootstrap 接口的 401 也豁免', () => {
    expect(
      shouldTreatAsSessionExpired({
        status: 401,
        requiresLogin: true,
        path: '/api/local-auth/bootstrap',
      })
    ).toBe(false);
  });

  it('别的路径的 401 照常算会话过期', () => {
    expect(
      shouldTreatAsSessionExpired({
        status: 401,
        requiresLogin: true,
        path: '/api/local-auth/me',
      })
    ).toBe(true);
  });
});

describe('会话过期广播', () => {
  beforeEach(() => {
    resetSessionExpiryListenersForTests();
  });

  it('通知所有订阅者', () => {
    const a = vi.fn();
    const b = vi.fn();
    onSessionExpired(a);
    onSessionExpired(b);

    notifySessionExpired();

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });

  it('取消订阅后不再收到通知', () => {
    const listener = vi.fn();
    const unsubscribe = onSessionExpired(listener);
    unsubscribe();

    notifySessionExpired();

    expect(listener).not.toHaveBeenCalled();
  });
});
