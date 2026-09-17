/**
 * 设置对话框里哪些 section 该出现。
 *
 * 单独成一个纯函数模块（而不是塞进 `settingsRegistry.tsx`）是为了能被
 * vitest 收进来测——`vitest.config.ts` 只收 `src/**\/*.test.ts`，
 * `.tsx` 里的逻辑没法单独钉住。
 */

/**
 * 只在**本机团队版**下存在的 section。
 *
 * 判据必须是 `isLocalTeamMode()`，**不能**用 `!isLocalPersonalMode()`：
 * 云端构建的个人模式满足后者的取反，但它连不上本机后端的
 * `/api/local-auth/oauth/*`，那块界面在那里点了只会报错。
 */
export const LOCAL_TEAM_ONLY_SECTIONS = ['account'] as const;

export interface SettingsSectionVisibilityContext {
  /** `isLocalTeamMode()` 的结果。 */
  localTeamMode: boolean;
}

export function isSettingsSectionVisible(
  id: string,
  ctx: SettingsSectionVisibilityContext
): boolean {
  return (LOCAL_TEAM_ONLY_SECTIONS as readonly string[]).includes(id)
    ? ctx.localTeamMode
    : true;
}

export function filterVisibleSections<T extends { id: string }>(
  sections: readonly T[],
  ctx: SettingsSectionVisibilityContext
): T[] {
  return sections.filter((section) =>
    isSettingsSectionVisible(section.id, ctx)
  );
}
