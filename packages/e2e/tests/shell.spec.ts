import { expect, test } from '../support/fixtures';
import { gotoPath, openProject, sidebar } from '../support/pages';

test('个人版外壳：根路径进工作台，五个中文入口，无横幅、无外网徽标、无 Team/Personal 切换', async ({
  page,
  project,
}) => {
  await gotoPath(page, '/');
  await expect(page).toHaveURL(/\/home$/);

  const nav = sidebar(page);
  for (const name of ['工作台', '需求流水线', '测试中心', '文档', '设置']) {
    await expect(
      nav.getByRole('button', { name: new RegExp(name) })
    ).toBeVisible();
  }
  await expect(page.getByLabel(/Star on GitHub|Join our Discord/)).toHaveCount(
    0
  );

  await nav.getByRole('button', { name: /文档/ }).click();
  await expect(page.getByTestId('docs-empty')).toContainText(
    '规格与需求文档会出现在这里'
  );

  await openProject(page, project);
  await expect(
    page.getByText('Vibe Kanban Cloud is shutting down')
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: /^(Team|Personal)$/ })
  ).toHaveCount(0);
  await expect(page.getByTestId('kanban-column-header')).toHaveCount(6);
  await expect(
    page.locator('[data-testid="kanban-column-header"][data-stage="backlog"]')
  ).toContainText('需求');
  await expect(
    page.locator('[data-testid="kanban-column-header"][data-stage="backlog"]')
  ).toContainText('人工确认');
  await expect(page.getByRole('button', { name: '新建需求' })).toHaveCount(1);
});
