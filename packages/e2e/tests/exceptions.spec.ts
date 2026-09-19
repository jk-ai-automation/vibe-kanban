import { expect, test } from '../support/fixtures';
import {
  GATE_LABELS,
  STAGE_TIMEOUT,
  approveGate,
  kanbanProgress,
  openDetail,
  openProject,
  openWorkbench,
  sidebar,
  startRequirement,
  workbenchCard,
} from '../support/pages';

test('人工打回：必须写意见，同阶段重跑并留痕', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(5 * 60_000);
  const title = `打回重跑 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'confirm', title).filter({
      hasText: GATE_LABELS.requirement,
    })
  ).toBeVisible({ timeout: STAGE_TIMEOUT });

  const issue = await api.findIssueByTitle(project.projectId, title);
  await openDetail(page, project.projectId, issue.id);
  const gateBar = page.getByTestId('gate-bar');
  await expect(gateBar).toHaveAttribute('data-kind', 'human');

  await gateBar.getByRole('button', { name: '打回并说明' }).click();
  await expect(
    gateBar.getByRole('button', { name: '提交打回' })
  ).toBeDisabled();
  await gateBar.getByRole('textbox').fill('补充金额为 0 与负数的边界');
  await gateBar.getByRole('button', { name: '提交打回' }).click();

  const timeline = page.getByTestId('pipeline-timeline');
  await expect(timeline).toContainText('你打回了需求');
  await expect(timeline).toContainText('补充金额为 0 与负数的边界');
  await expect(timeline).toContainText('第 2 次', { timeout: 30_000 });
  await expect(gateBar).toHaveAttribute('data-kind', 'human', {
    timeout: STAGE_TIMEOUT,
  });

  const view = await api.issuePipeline(issue.id);
  const attempts = view!.stages
    .filter((stage) => stage.stage_key === 'requirement')
    .map((stage) => Number(stage.attempt));
  expect(Math.max(...attempts)).toBe(2);
});

test('[qa:always-fail-review] 评审轮次用尽后转人工', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(10 * 60_000);
  const title = `[qa:always-fail-review] 评审一直不过 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);

  await expect(
    workbenchCard(page, 'confirm', title).filter({ hasText: '需要你处理' })
  ).toBeVisible({ timeout: 6 * STAGE_TIMEOUT });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('failed');
  expect(view?.run.current_stage_key).toBe('review');
  const maxRounds = Number(
    view!.template.stages.find((stage) => stage.key === 'review')!.max_rounds
  );
  expect(
    view!.stages.filter((stage) => stage.stage_key === 'review')
  ).toHaveLength(maxRounds);

  await sidebar(page)
    .getByRole('button', { name: /需求流水线/ })
    .click();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'done,done,done,done,failed,pending,pending'
  );

  await openDetail(page, project.projectId, issue.id);
  await expect(page.getByTestId('gate-bar')).toHaveAttribute(
    'data-kind',
    'failed'
  );
  await expect(page.getByTestId('gate-bar')).toContainText(
    `第 ${maxRounds}/${maxRounds} 轮`
  );
});

test('[qa:test-fail-case-once] 用例问题退回用例设计，重新人工确认后交付', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(10 * 60_000);
  const title = `[qa:test-fail-case-once] 用例回流 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);
  // 开发 → 评审 → 测试（失败，归因 case）→ 回到用例设计，再次等人工
  await approveGate(page, title, GATE_LABELS.testDesign, 5 * STAGE_TIMEOUT);

  await expect(workbenchCard(page, 'delivered', title)).toBeVisible({
    timeout: 5 * STAGE_TIMEOUT,
  });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  const testDesignRuns = view!.stages.filter(
    (stage) => stage.stage_key === 'test_design'
  );
  expect(testDesignRuns).toHaveLength(2);
  const approvals = view!.decisions.filter(
    (decision) =>
      decision.decision === 'approve' &&
      testDesignRuns.some((stage) => stage.id === decision.stage_run_id)
  );
  expect(approvals).toHaveLength(2);
  expect(
    view!.stages.filter((stage) => stage.stage_key === 'test')
  ).toHaveLength(2);
});

test('暂停与继续：在人工关卡处暂停（关卡仍可决策），刷新仍是暂停，继续后回到关卡', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(4 * 60_000);
  const title = `暂停继续 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'confirm', title).filter({
      hasText: GATE_LABELS.requirement,
    })
  ).toBeVisible({ timeout: STAGE_TIMEOUT });

  // 契约 C5：waiting_gate 允许暂停。模拟器阶段只有 1～3 秒（C9），在运行中暂停
  // 时序不可控；停在人工关卡时暂停是确定的。
  const issue = await api.findIssueByTitle(project.projectId, title);
  await openDetail(page, project.projectId, issue.id);
  const gateBar = page.getByTestId('gate-bar');
  await expect(gateBar).toHaveAttribute('data-kind', 'human');
  await page.getByRole('button', { name: '暂停自动化' }).click();
  await expect(gateBar).toHaveAttribute('data-kind', 'paused');
  const resume = page.getByRole('button', { name: '继续', exact: true });
  await expect(resume).toBeVisible();
  // 契约 C12：暂停中关卡决策照常受理，关卡条仍给「确认 / 打回」，并说明已暂停
  await expect(gateBar).toContainText('自动化已暂停');
  await expect(
    gateBar.getByRole('button', { name: '确认并继续' })
  ).toBeVisible();
  await expect(
    gateBar.getByRole('button', { name: '打回并说明' })
  ).toBeVisible();

  await page.waitForTimeout(5_000);
  expect((await api.issuePipeline(issue.id))?.run.status).toBe('paused');

  await page.reload();
  await expect(gateBar).toHaveAttribute('data-kind', 'paused');
  // 工作台待确认栏里不再出现（pending 只含 waiting_gate 与 failed）
  await page.goto('/home');
  await expect(workbenchCard(page, 'confirm', title)).toHaveCount(0);

  await openDetail(page, project.projectId, issue.id);
  await page.getByRole('button', { name: '继续', exact: true }).click();
  await expect(gateBar).toHaveAttribute('data-kind', 'human', {
    timeout: 30_000,
  });
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('waiting_gate');
  expect(view?.run.current_stage_key).toBe('requirement');
});

test('刷新后状态恢复：工作台、看板、详情', async ({ page, api, project }) => {
  test.setTimeout(4 * 60_000);
  const title = `刷新恢复 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  const gateCard = workbenchCard(page, 'confirm', title).filter({
    hasText: GATE_LABELS.requirement,
  });
  await expect(gateCard).toBeVisible({ timeout: STAGE_TIMEOUT });

  await page.reload();
  await expect(gateCard).toBeVisible();

  const issue = await api.findIssueByTitle(project.projectId, title);
  await openProject(page, project);
  await page.reload();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'gate,pending,pending,pending,pending,pending,pending'
  );

  await openDetail(page, project.projectId, issue.id);
  await page.reload();
  await expect(page.getByTestId('gate-bar')).toHaveAttribute(
    'data-kind',
    'human'
  );
  await expect(page.getByTestId('pipeline-stepper')).toHaveAttribute(
    'data-current',
    'requirement'
  );
});
