import type { PipelineRun, PipelineStageRun } from 'shared/types';
import type { CreateIssueRequest } from 'shared/remote-types';
import { toMillis, toNumber } from '@/entities/pipeline/model/progress';
import { splitMessageToTitleDescription } from '@/shared/lib/string';

const byDesc =
  (value: (run: PipelineRun) => unknown) => (a: PipelineRun, b: PipelineRun) =>
    (toMillis(value(b)) ?? 0) - (toMillis(value(a)) ?? 0);

/** 「正在自动跑」：running + paused（等人工与失败走 pending 接口，进「需要你确认」）。 */
export function runningRuns(runs: readonly PipelineRun[]): PipelineRun[] {
  return runs
    .filter((run) => run.status === 'running' || run.status === 'paused')
    .sort(byDesc((run) => run.updated_at));
}

/** 「最近交付」：completed + cancelled，最近结束在前。 */
export function recentDelivered(
  runs: readonly PipelineRun[],
  limit = 5
): PipelineRun[] {
  return runs
    .filter((run) => run.status === 'completed' || run.status === 'cancelled')
    .sort(byDesc((run) => run.finished_at))
    .slice(0, limit);
}

export interface WeeklyStats {
  delivered: number;
  avgCycleMinutes: number | null;
  /** 百分比整数；本周没有交付为 null。 */
  firstPassRate: number | null;
}

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * 本周统计（草图：交付 7 · 平均周期 52 分 · 一次通过率 71%）。
 * 「一次通过」= 这次运行的所有阶段都只跑了一次。
 */
export function weeklyStats(
  runs: readonly PipelineRun[],
  stages: readonly PipelineStageRun[],
  now: number
): WeeklyStats {
  const delivered = runs.filter((run) => {
    const finished = toMillis(run.finished_at);
    return (
      run.status === 'completed' &&
      finished !== null &&
      now - finished <= WEEK_MS
    );
  });
  if (delivered.length === 0) {
    return { delivered: 0, avgCycleMinutes: null, firstPassRate: null };
  }
  const cycles = delivered.map(
    (run) =>
      ((toMillis(run.finished_at) ?? 0) - (toMillis(run.created_at) ?? 0)) /
      60_000
  );
  const firstPass = delivered.filter((run) =>
    stages
      .filter((stage) => stage.run_id === run.id)
      .every((stage) => (toNumber(stage.attempt) ?? 1) <= 1)
  ).length;
  return {
    delivered: delivered.length,
    avgCycleMinutes: Math.round(
      cycles.reduce((sum, c) => sum + c, 0) / cycles.length
    ),
    firstPassRate: Math.round((firstPass / delivered.length) * 100),
  };
}

function sameLocalDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}

/** 「今天自动完成 N 个阶段」。 */
export function stagesFinishedToday(
  stages: readonly PipelineStageRun[],
  now: number
): number {
  return stages.filter((stage) => {
    const finished = toMillis(stage.finished_at);
    return (
      stage.status === 'passed' &&
      finished !== null &&
      sameLocalDay(finished, now)
    );
  }).length;
}

export function greetingKey(hour: number): string {
  if (hour < 12) return 'workbench.greeting.morning';
  if (hour < 18) return 'workbench.greeting.afternoon';
  return 'workbench.greeting.evening';
}

/** 新需求落在哪一列：优先 `stage_type = backlog`，否则排序第一列。 */
export function pickBacklogStatusId(
  statuses: readonly { id: string; sort_order: number; stage_type?: unknown }[]
): string | null {
  const backlog = statuses.find((status) => status.stage_type === 'backlog');
  if (backlog) return backlog.id;
  const sorted = [...statuses].sort((a, b) => a.sort_order - b.sort_order);
  return sorted[0]?.id ?? null;
}

/** 工作台输入框 → 建需求请求体（第一行做标题，沿用 `splitMessageToTitleDescription`）。 */
export function buildCreateIssueRequest(args: {
  id: string;
  projectId: string;
  statusId: string;
  prompt: string;
}): CreateIssueRequest & { id: string } {
  const { title, description } = splitMessageToTitleDescription(args.prompt);
  return {
    id: args.id,
    project_id: args.projectId,
    status_id: args.statusId,
    title,
    description,
    priority: null,
    start_date: null,
    target_date: null,
    completed_at: null,
    sort_order: 0,
    parent_issue_id: null,
    parent_issue_sort_order: null,
    extension_metadata: {},
  };
}

/** 目标分支默认值：仓库默认目标分支 → 当前分支 → 第一个本地分支。 */
export function pickDefaultBranch(
  repo: { default_target_branch: string | null },
  branches: readonly { name: string; is_current: boolean; is_remote: boolean }[]
): string | null {
  const local = branches.filter((branch) => !branch.is_remote);
  if (
    repo.default_target_branch &&
    local.some((branch) => branch.name === repo.default_target_branch)
  ) {
    return repo.default_target_branch;
  }
  return (
    local.find((branch) => branch.is_current)?.name ?? local[0]?.name ?? null
  );
}

export function pickWorkbenchProjectId(
  projects: readonly { id: string }[],
  selectedProjectId: string | null
): string | null {
  if (selectedProjectId && projects.some((p) => p.id === selectedProjectId)) {
    return selectedProjectId;
  }
  return projects[0]?.id ?? null;
}

/**
 * 「需要你确认」卡片的等待起点（契约 C11）：阶段进入 `waiting_gate` /
 * `failed` 时后端写 `finished_at`；老数据没有时退回阶段开始时间。
 */
export function waitingSince(
  stage: Pick<PipelineStageRun, 'started_at' | 'finished_at'>
): number | null {
  return toMillis(stage.finished_at) ?? toMillis(stage.started_at);
}
