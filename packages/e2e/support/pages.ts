import { expect, type Locator, type Page } from '@playwright/test';
import type { E2eProject } from './fixtures';

/** 三道人工关卡名（§4 V3：以模板 `gate_label` 为准；与 zh-Hans `pipeline.gateLabel.*` 一致）。 */
export const GATE_LABELS = {
  requirement: '需求确认',
  spec: '设计规格确认',
  testDesign: '用例设计确认',
} as const;

/**
 * 单个阶段的等待上限。模拟器流水线模式每行日志 0.1 秒（契约 C9），阶段本身
 * 1～3 秒；再加工作区创建、引擎调度与推送，CI 机器慢时也够用。
 */
export const STAGE_TIMEOUT = 90_000;

/**
 * 首屏渲染上限。vite dev 第一次进某条路由要现场编译成百上千个模块，
 * 机器忙时要好几分钟，所以导航一律只等 `domcontentloaded`，再用这个
 * 超时等待页面里的关键元素出现。
 */
export const FIRST_PAINT_TIMEOUT = 180_000;

/** 只等 DOM，不等 load：vite dev 的懒加载模块会把 load 拖很久。 */
export async function gotoPath(page: Page, path: string): Promise<void> {
  await page.goto(path, { waitUntil: 'domcontentloaded' });
}

export function sidebar(page: Page): Locator {
  return page.getByTestId('personal-sidebar');
}

export async function openProject(
  page: Page,
  project: E2eProject
): Promise<void> {
  await gotoPath(page, `/projects/${project.projectId}`);
  await expect(page.getByTestId('kanban-column-header').first()).toBeVisible({
    timeout: FIRST_PAINT_TIMEOUT,
  });
}

/** 先进看板（外壳把它记为当前项目），再点「工作台」。 */
export async function openWorkbench(
  page: Page,
  project: E2eProject
): Promise<void> {
  await openProject(page, project);
  await sidebar(page)
    .getByRole('button', { name: /工作台/ })
    .click();
  await expect(page).toHaveURL(/\/home$/, { timeout: FIRST_PAINT_TIMEOUT });
  await expect(page.getByRole('textbox', { name: '描述一个需求' })).toBeVisible(
    { timeout: FIRST_PAINT_TIMEOUT }
  );
}

export async function startRequirement(
  page: Page,
  project: E2eProject,
  text: string
): Promise<void> {
  await page
    .getByRole('combobox', { name: '仓库' })
    .selectOption({ label: project.repoName });
  await expect(page.getByRole('combobox', { name: '分支' })).toHaveValue(
    'main'
  );
  const input = page.getByRole('textbox', { name: '描述一个需求' });
  await input.fill(text);
  await page.getByRole('button', { name: '开始' }).click();
  // 成功后输入框清空（WorkbenchComposerContainer 的 onSuccess）
  await expect(input).toHaveValue('', { timeout: 30_000 });
}

export function workbenchCard(
  page: Page,
  column: 'confirm' | 'running' | 'delivered',
  title: string
): Locator {
  return page
    .getByTestId(`workbench-column-${column}`)
    .getByTestId('workbench-card')
    .filter({ hasText: title });
}

/** 在工作台「需要你确认」里确认指定关卡。 */
export async function approveGate(
  page: Page,
  title: string,
  gateLabel: string,
  timeout = STAGE_TIMEOUT
): Promise<void> {
  const card = workbenchCard(page, 'confirm', title).filter({
    hasText: gateLabel,
  });
  await expect(card).toBeVisible({ timeout });
  await card.getByRole('button', { name: '确认并继续' }).click();
  await expect(card).toHaveCount(0, { timeout: 30_000 });
}

export async function openDetail(
  page: Page,
  projectId: string,
  issueId: string
): Promise<void> {
  await gotoPath(page, `/projects/${projectId}/issues/${issueId}/detail`);
  await expect(page.getByTestId('pipeline-stepper')).toBeVisible({
    timeout: FIRST_PAINT_TIMEOUT,
  });
}

export function kanbanProgress(page: Page, issueId: string): Locator {
  return page
    .locator(`[data-testid="kanban-card-pipeline"][data-issue-id="${issueId}"]`)
    .getByTestId('pipeline-progress');
}
