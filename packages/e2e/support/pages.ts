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

export function sidebar(page: Page): Locator {
  return page.getByTestId('personal-sidebar');
}

export async function openProject(
  page: Page,
  project: E2eProject
): Promise<void> {
  await page.goto(`/projects/${project.projectId}`);
  await expect(page.getByTestId('kanban-column-header').first()).toBeVisible();
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
  await expect(page).toHaveURL(/\/home$/);
  await expect(
    page.getByRole('textbox', { name: '描述一个需求' })
  ).toBeVisible();
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
  await page.goto(`/projects/${projectId}/issues/${issueId}/detail`);
  await expect(page.getByTestId('pipeline-stepper')).toBeVisible();
}

export function kanbanProgress(page: Page, issueId: string): Locator {
  return page
    .locator(`[data-testid="kanban-card-pipeline"][data-issue-id="${issueId}"]`)
    .getByTestId('pipeline-progress');
}
