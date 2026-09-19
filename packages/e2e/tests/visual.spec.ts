import { expect, test, type Page } from '@playwright/test';
import { VkApi } from '../support/api';
import { createProjectWithRepo, type E2eProject } from '../support/fixtures';

/** 会随时间或运行次数变化的区域：相对时间、问候语与今日统计、版本号、需求编号。 */
function masks(page: Page) {
  return [
    page.getByTestId('relative-time'),
    page.getByTestId('workbench-greeting'),
    page.getByTestId('nav-footer'),
    page.getByTestId('issue-simple-id'),
  ];
}

test.describe('@visual 视觉回归：工作台 / 看板 / 详情 × 亮暗 × 1280/1440', () => {
  test.skip(
    process.platform !== 'linux',
    '基线只在 Linux（CI）上生成与比对：字体渲染跨平台不同（计划 §3 决策 11）'
  );
  test.describe.configure({ mode: 'serial' });

  let project: E2eProject;
  let issueId: string;

  test.beforeAll(async () => {
    test.setTimeout(3 * 60_000);
    const api = new VkApi();
    project = await createProjectWithRepo(api, '视觉回归');
    issueId = await api.startPipelineDirect({
      projectId: project.projectId,
      repoId: project.repoId,
      title: '视觉回归样例需求',
    });
    await api.waitForPipeline(
      issueId,
      (view) =>
        view.run.status === 'waiting_gate' &&
        view.run.current_stage_key === 'requirement',
      90_000
    );
  });

  for (const width of [1280, 1440]) {
    for (const scheme of ['light', 'dark'] as const) {
      test(`三页截图 ${scheme} ${width}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: scheme });

        await page.goto(`/projects/${project.projectId}`);
        await expect(
          page.locator(
            `[data-testid="kanban-card-pipeline"][data-issue-id="${issueId}"]`
          )
        ).toBeVisible();
        await expect(page).toHaveScreenshot(`kanban-${scheme}-${width}.png`, {
          mask: masks(page),
        });

        await page
          .getByTestId('personal-sidebar')
          .getByRole('button', { name: /工作台/ })
          .click();
        await expect(
          page
            .getByTestId('workbench-column-confirm')
            .getByTestId('workbench-card')
        ).toHaveCount(1);
        await expect(page).toHaveScreenshot(
          `workbench-${scheme}-${width}.png`,
          {
            mask: masks(page),
          }
        );

        await page.goto(
          `/projects/${project.projectId}/issues/${issueId}/detail`
        );
        await expect(page.getByTestId('gate-bar')).toHaveAttribute(
          'data-kind',
          'human'
        );
        await expect(
          page.getByTestId('artifact-section').first()
        ).toBeVisible();
        await expect(page).toHaveScreenshot(`detail-${scheme}-${width}.png`, {
          mask: masks(page),
        });
      });
    }
  }
});
