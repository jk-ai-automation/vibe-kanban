import type { LocalAuthBootstrap } from 'shared/types';
import {
  configureDataSource,
  isLocalMode,
  type DataSourceMode,
} from '@/shared/lib/local/dataSource';

/**
 * 运行时模式：由 `GET /api/local-auth/bootstrap` 决定，取代原先写死在
 * 构建期的 `VITE_VK_DATA_SOURCE`。
 *
 * **和数据源是两个正交概念**：
 * - `RuntimeMode` 说的是「这台后端要不要登录」（personal / team）。
 * - `DataSourceMode`（`dataSource.ts`）说的是「数据从哪来」：
 *   `'local'` = 本机 SQLite + `/api/*`；`'remote'` = 云端 ElectricSQL。
 *
 * 团队模式一定是本地数据源（云端 relay 在 team 模式下被后端关掉），
 * 但个人模式**既可能**是本地数据源（自托管、npx 本地跑），**也可能**是
 * 远端数据源（官方云端构建，`VITE_VK_SHARED_API_BASE` 已配置）。
 * 所以不能反过来由数据源推模式，也不能由模式无条件推数据源。
 */
export type RuntimeMode = 'personal' | 'team';

/** `applyBootstrap` 之后才有值；没配置就是 `null`（remote-web 永远是 `null`）。 */
let runtimeMode: RuntimeMode | null = null;

/** 启动时那一次 bootstrap 的快照，供会话上下文拿 `needs_setup` / `providers` 做首屏。 */
let bootstrapSnapshot: LocalAuthBootstrap | null = null;

export function configureRuntimeMode(mode: RuntimeMode): void {
  runtimeMode = mode;
}

/**
 * 取运行时模式。**未配置时抛错**，避免静默按 personal 跑把登录页吞掉。
 * 只在确定已经 `applyBootstrap` 过的代码路径里用（local-web 入口之后）。
 * 跨端共享代码请用 [`isPersonalMode`] / [`requiresLogin`]，它们不抛错。
 */
export function getRuntimeMode(): RuntimeMode {
  if (runtimeMode === null) {
    throw new Error(
      'Runtime mode is not configured. Call applyBootstrap() or configureRuntimeMode() first.'
    );
  }
  return runtimeMode;
}

export function isPersonalMode(): boolean {
  return runtimeMode === 'personal';
}

/** 团队模式才强制登录。未配置（remote-web / 测试）一律按「不强制」。 */
export function requiresLogin(): boolean {
  return runtimeMode === 'team';
}

/**
 * 「个人版」＝ 本地数据源 + 不需要登录。
 *
 * 只有这种组合才没有登录概念（`useAuth` 恒为已登录、用户菜单不显示登录 / 退出）。
 * 云端构建的个人模式（数据源 `remote`）仍然走云端 OAuth，不能被当成个人版。
 */
export function isLocalPersonalMode(): boolean {
  return isLocalMode() && !requiresLogin();
}

export function getBootstrapSnapshot(): LocalAuthBootstrap | null {
  return bootstrapSnapshot;
}

export function parseRuntimeMode(mode: string): RuntimeMode {
  if (mode === 'personal' || mode === 'team') return mode;
  throw new Error(`Unknown runtime mode from bootstrap: ${mode}`);
}

/**
 * 团队模式一定走本地数据源；个人模式沿用构建期的判断，保证既有云端构建零回退。
 */
export function resolveDataSource(
  mode: RuntimeMode,
  hasSharedApiBase: boolean
): DataSourceMode {
  if (mode === 'team') return 'local';
  return hasSharedApiBase ? 'remote' : 'local';
}

/** 把 bootstrap 响应落到两个单例上。返回解析出的模式，方便调用方打日志。 */
export function applyBootstrap(
  bootstrap: LocalAuthBootstrap,
  options: { hasSharedApiBase?: boolean } = {}
): RuntimeMode {
  const mode = parseRuntimeMode(bootstrap.mode);
  runtimeMode = mode;
  bootstrapSnapshot = bootstrap;
  configureDataSource(
    resolveDataSource(mode, options.hasSharedApiBase === true)
  );
  return mode;
}

/** 仅测试用。 */
export function resetRuntimeModeForTests(): void {
  runtimeMode = null;
  bootstrapSnapshot = null;
}
