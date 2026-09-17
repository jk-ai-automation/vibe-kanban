import { beforeEach, describe, expect, it } from 'vitest';
import type { LocalAuthBootstrap } from 'shared/types';

import {
  configureDataSource,
  getDataSourceMode,
} from '@/shared/lib/local/dataSource';
import {
  applyBootstrap,
  configureRuntimeMode,
  getBootstrapSnapshot,
  getRuntimeMode,
  isLocalPersonalMode,
  isLocalTeamMode,
  isPersonalMode,
  parseRuntimeMode,
  requiresLogin,
  resetRuntimeModeForTests,
  resolveDataSource,
} from '@/shared/lib/local/runtimeMode';

function bootstrap(
  overrides: Partial<LocalAuthBootstrap> = {}
): LocalAuthBootstrap {
  return {
    mode: 'personal',
    require_login: false,
    authenticated: true,
    needs_setup: false,
    providers: [],
    allow_oauth_signup: false,
    ...overrides,
  };
}

describe('运行时模式单例', () => {
  beforeEach(() => {
    resetRuntimeModeForTests();
    configureDataSource('remote');
  });

  it('默认未初始化时 getRuntimeMode 抛错', () => {
    expect(() => getRuntimeMode()).toThrow(/not configured/i);
  });

  it('未初始化时 isPersonalMode 与 requiresLogin 都为假且不抛错', () => {
    expect(isPersonalMode()).toBe(false);
    expect(requiresLogin()).toBe(false);
  });

  it('配置 personal 后 isPersonalMode 为真且 requiresLogin 为假', () => {
    configureRuntimeMode('personal');
    expect(getRuntimeMode()).toBe('personal');
    expect(isPersonalMode()).toBe(true);
    expect(requiresLogin()).toBe(false);
  });

  it('配置 team 后 requiresLogin 为真', () => {
    configureRuntimeMode('team');
    expect(isPersonalMode()).toBe(false);
    expect(requiresLogin()).toBe(true);
  });

  it('未知 mode 值抛错', () => {
    expect(() => parseRuntimeMode('enterprise')).toThrow(/enterprise/);
    expect(() => applyBootstrap(bootstrap({ mode: 'enterprise' }))).toThrow();
  });
});

describe('模式与数据源的正交关系', () => {
  beforeEach(() => {
    resetRuntimeModeForTests();
    configureDataSource('remote');
  });

  it('team 模式无条件用本地数据源', () => {
    expect(resolveDataSource('team', true)).toBe('local');
    expect(resolveDataSource('team', false)).toBe('local');
  });

  it('personal 模式沿用构建期判断：配了云端基址就还是 remote', () => {
    expect(resolveDataSource('personal', true)).toBe('remote');
    expect(resolveDataSource('personal', false)).toBe('local');
  });

  it('applyBootstrap 同时设置数据源：team 走 local', () => {
    applyBootstrap(bootstrap({ mode: 'team', require_login: true }), {
      hasSharedApiBase: true,
    });
    expect(getRuntimeMode()).toBe('team');
    expect(getDataSourceMode()).toBe('local');
  });

  it('applyBootstrap 在 personal + 云端基址下保持 remote（云端构建零回退）', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: true });
    expect(getDataSourceMode()).toBe('remote');
  });

  it('applyBootstrap 在 personal + 无云端基址下走 local', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: false });
    expect(getDataSourceMode()).toBe('local');
  });

  it('applyBootstrap 存下快照供首屏使用', () => {
    expect(getBootstrapSnapshot()).toBeNull();
    applyBootstrap(
      bootstrap({ mode: 'team', require_login: true, needs_setup: true })
    );
    expect(getBootstrapSnapshot()?.needs_setup).toBe(true);
  });
});

describe('isLocalPersonalMode（个人版判定）', () => {
  beforeEach(() => {
    resetRuntimeModeForTests();
  });

  it('本地数据源 + personal 是个人版', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: false });
    expect(isLocalPersonalMode()).toBe(true);
  });

  it('本地数据源 + team 不是个人版', () => {
    applyBootstrap(bootstrap({ mode: 'team', require_login: true }));
    expect(isLocalPersonalMode()).toBe(false);
  });

  it('云端数据源 + personal 不是个人版（仍走云端 OAuth）', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: true });
    expect(isLocalPersonalMode()).toBe(false);
  });
});

describe('isLocalTeamMode（本机团队版判定）', () => {
  beforeEach(() => {
    resetRuntimeModeForTests();
  });

  it('本地数据源 + team 才是本机团队版', () => {
    applyBootstrap(bootstrap({ mode: 'team', require_login: true }));
    expect(isLocalTeamMode()).toBe(true);
  });

  it('个人版不是', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: false });
    expect(isLocalTeamMode()).toBe(false);
  });

  /**
   * 这一条是 `/api/admin/*`、工作区删除审批这些本机接口的门禁依据：
   * 云端构建的个人模式满足 `!isLocalPersonalMode()`，但它连不上本机后端，
   * 所以那两个判定**不是互补的**，不能用取反代替。
   */
  it('云端构建不是本机团队版，且与个人版判定不互补', () => {
    applyBootstrap(bootstrap(), { hasSharedApiBase: true });
    expect(isLocalTeamMode()).toBe(false);
    expect(isLocalPersonalMode()).toBe(false);
  });
});
