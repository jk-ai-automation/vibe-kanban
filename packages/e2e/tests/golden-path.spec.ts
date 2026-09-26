import { expect, test } from '../support/fixtures';
import {
  GATE_LABELS,
  STAGE_TIMEOUT,
  approveGate,
  kanbanProgress,
  openDetail,
  openWorkbench,
  sidebar,
  startRequirement,
  workbenchCard,
} from '../support/pages';

test('黄金路径：一句话 → 三道人工关卡 → 自动开发评审测试 → 交付', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(8 * 60_000);
  const title = `下单最小金额校验 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'running', title).or(
      workbenchCard(page, 'confirm', title)
    )
  ).toBeVisible({ timeout: 30_000 });

  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);

  await expect(workbenchCard(page, 'delivered', title)).toBeVisible({
    timeout: 5 * STAGE_TIMEOUT,
  });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('completed');

  // 看板：卡片在「交付」列，七格全绿
  await sidebar(page)
    .getByRole('button', { name: /需求流水线/ })
    .click();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'done,done,done,done,done,done,done'
  );
  await expect(
    page.locator('[data-testid="kanban-column-header"][data-stage="done"]')
  ).toContainText('1');

  // 详情：七段全勾、产出物页签、时间线三次确认、关卡条显示已交付
  await openDetail(page, project.projectId, issue.id);
  await expect(
    page.getByTestId('pipeline-stepper').locator('li[data-state="done"]')
  ).toHaveCount(7);
  await page.getByRole('tab', { name: '需求与验收标准' }).click();
  await expect(
    page.locator('[data-testid="artifact-section"][data-kind="requirement"]')
  ).toBeVisible();
  await page.getByRole('tab', { name: '用例' }).click();
  await expect(page.getByTestId('csv-table')).toBeVisible();
  await page.getByRole('tab', { name: '交付报告' }).click();
  await expect(
    page.locator(
      '[data-testid="artifact-section"][data-kind="delivery_report"]'
    )
  ).toBeVisible();
  const timeline = page.getByTestId('pipeline-timeline');
  await expect(timeline).toContainText('你确认了需求');
  await expect(timeline).toContainText('你确认了设计规格');
  await expect(timeline).toContainText('你确认了用例设计');
  await expect(page.getByTestId('gate-bar')).toHaveAttribute(
    'data-kind',
    'completed'
  );
});
