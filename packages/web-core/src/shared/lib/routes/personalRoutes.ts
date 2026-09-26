/**
 * 个人版的路由路径（计划 §1.2）。
 *
 * `KanbanContainer` / `KanbanIssuePanelContainer` 也被 remote-web 编译，
 * 那里没有这些路由，所以共享代码里用 `router.history.push(路径字符串)`
 * 而不是带类型的 `navigate({ to })`；只在 local-web 独有的代码里用后者。
 */
export const PERSONAL_ROUTES = {
  workbench: '/home',
  docs: '/docs',
  testing: '/testing',
  project: (projectId: string) => `/projects/${encodeURIComponent(projectId)}`,
  issueDetail: (projectId: string, issueId: string) =>
    `/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/detail`,
} as const;

/** 与 `@vibe/ui/components/PersonalSidebar` 的同名类型逐字一致。 */
export type PersonalNavKey =
  | 'workbench'
  | 'pipeline'
  | 'testing'
  | 'docs'
  | 'settings';

function matches(pathname: string, prefix: string): boolean {
  return pathname === prefix || pathname.startsWith(`${prefix}/`);
}

/** 当前路径高亮哪个导航项（设置是对话框，永远不高亮）。 */
export function personalNavActiveKey(pathname: string): PersonalNavKey | null {
  if (matches(pathname, '/home')) return 'workbench';
  if (matches(pathname, '/projects')) return 'pipeline';
  if (matches(pathname, '/testing')) return 'testing';
  if (matches(pathname, '/docs')) return 'docs';
  return null;
}
