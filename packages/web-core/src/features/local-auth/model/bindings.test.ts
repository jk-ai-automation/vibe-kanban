import { describe, expect, it } from 'vitest';
import type { OAuthBindings } from 'shared/types';
import {
  BINDING_ERROR_KEYS,
  PROVIDER_LABEL_KEYS,
  buildBindingRows,
  describeBindingError,
  isOAuthBoundRedirect,
  providerLabelKey,
  stripOAuthQueryFlag,
} from '@/features/local-auth/model/bindings';

function view(overrides: Partial<OAuthBindings> = {}): OAuthBindings {
  return {
    available_providers: ['feishu', 'google'],
    bindings: [],
    has_password: true,
    ...overrides,
  };
}

const 绑定 = (provider: string, email: string | null = null) => ({
  provider,
  bound_at: '2026-09-17T03:00:00Z',
  email,
});

describe('buildBindingRows（把后端视图摊成一行一个提供方）', () => {
  it('可绑的都列出来，没绑的标成未绑定', () => {
    const rows = buildBindingRows(view());
    expect(rows.map((r) => r.provider)).toEqual(['feishu', 'google']);
    expect(rows.every((r) => !r.bound)).toBe(true);
    expect(rows.every((r) => r.canBind)).toBe(true);
  });

  it('已绑的带上绑定时间与邮箱', () => {
    const rows = buildBindingRows(
      view({ bindings: [绑定('feishu', 'a@x.com')] })
    );
    const feishu = rows.find((r) => r.provider === 'feishu');
    expect(feishu).toMatchObject({
      bound: true,
      boundAt: '2026-09-17T03:00:00Z',
      email: 'a@x.com',
    });
  });

  /**
   * 管理员撤掉凭据之后那条绑定还在账号上，界面必须继续显示它，
   * 否则用户永远看不到、也就永远解不掉。
   */
  it('凭据已被撤掉的提供方仍然列出，但不给绑定按钮', () => {
    const rows = buildBindingRows(view({ bindings: [绑定('lark')] }));
    const lark = rows.find((r) => r.provider === 'lark');
    expect(lark?.bound).toBe(true);
    expect(lark?.canBind).toBe(false);
    expect(rows.map((r) => r.provider)).toEqual(['feishu', 'google', 'lark']);
  });

  it('不会把同一个提供方列两遍', () => {
    const rows = buildBindingRows(view({ bindings: [绑定('feishu')] }));
    expect(rows.filter((r) => r.provider === 'feishu')).toHaveLength(1);
  });

  describe('「最后一个登录方式」的提前告知', () => {
    it('没密码且只有一个绑定时，那一条不允许解绑', () => {
      const rows = buildBindingRows(
        view({ bindings: [绑定('feishu')], has_password: false })
      );
      const feishu = rows.find((r) => r.provider === 'feishu');
      expect(feishu?.isLastLoginMethod).toBe(true);
      expect(feishu?.canUnbind).toBe(false);
    });

    it('有密码时最后一个绑定也能解', () => {
      const rows = buildBindingRows(
        view({ bindings: [绑定('feishu')], has_password: true })
      );
      expect(rows.find((r) => r.provider === 'feishu')?.canUnbind).toBe(true);
    });

    it('没密码但有两个绑定时两条都能解', () => {
      const rows = buildBindingRows(
        view({
          bindings: [绑定('feishu'), 绑定('google')],
          has_password: false,
        })
      );
      expect(rows.filter((r) => r.bound).every((r) => r.canUnbind)).toBe(true);
      expect(rows.some((r) => r.isLastLoginMethod)).toBe(false);
    });

    it('没绑的那条不会被标成最后一个登录方式', () => {
      const rows = buildBindingRows(
        view({ bindings: [绑定('feishu')], has_password: false })
      );
      expect(rows.find((r) => r.provider === 'google')?.isLastLoginMethod).toBe(
        false
      );
    });
  });
});

describe('providerLabelKey（字面量映射，不是模板字面量）', () => {
  it('三个预置提供方各有一条固定 key', () => {
    expect(providerLabelKey('feishu')).toBe('localAuth.providers.feishu');
    expect(providerLabelKey('lark')).toBe('localAuth.providers.lark');
    expect(providerLabelKey('google')).toBe('localAuth.providers.google');
  });

  /** 后端将来多一个提供方时，界面显示 id 本身而不是崩掉或显示空白。 */
  it('未知提供方返回 null，由调用方退回显示 id', () => {
    expect(providerLabelKey('github')).toBeNull();
    expect(providerLabelKey('')).toBeNull();
    expect(providerLabelKey('__proto__')).toBeNull();
  });

  it('映射表里只有白名单里的三个', () => {
    expect(Object.keys(PROVIDER_LABEL_KEYS).sort()).toEqual([
      'feishu',
      'google',
      'lark',
    ]);
  });
});

describe('describeBindingError（按状态码分流，不匹配文案）', () => {
  it('409 是「这是你唯一的登录方式」', () => {
    expect(describeBindingError(409)).toBe(BINDING_ERROR_KEYS.lastLoginMethod);
  });

  it('404 是「这条绑定已经不在了」', () => {
    expect(describeBindingError(404)).toBe(BINDING_ERROR_KEYS.notFound);
  });

  it('403 是「请刷新页面重试」（CSRF 或账号被停用）', () => {
    expect(describeBindingError(403)).toBe(BINDING_ERROR_KEYS.forbidden);
  });

  it('其余一律落到通用文案', () => {
    for (const status of [0, 400, 401, 429, 500, 502]) {
      expect(describeBindingError(status)).toBe(BINDING_ERROR_KEYS.generic);
    }
  });
});

describe('isOAuthBoundRedirect（认出绑定成功的回跳）', () => {
  it('认出 ?oauth=bound', () => {
    expect(isOAuthBoundRedirect('?oauth=bound')).toBe(true);
    expect(isOAuthBoundRedirect('?a=1&oauth=bound')).toBe(true);
  });

  it('别的值一律不认', () => {
    for (const search of [
      '',
      '?oauth=',
      '?oauth=bound2',
      '?oauth=BOUND',
      '?notoauth=bound',
      '?oauth=bound%00',
    ]) {
      expect(isOAuthBoundRedirect(search)).toBe(false);
    }
  });

  it('提示过一次就把参数从地址栏摘掉，刷新不再重复提示', () => {
    expect(stripOAuthQueryFlag('?oauth=bound')).toBe('');
    expect(stripOAuthQueryFlag('?a=1&oauth=bound')).toBe('?a=1');
    expect(stripOAuthQueryFlag('?oauth=bound&b=2')).toBe('?b=2');
    expect(stripOAuthQueryFlag('')).toBe('');
  });
});
