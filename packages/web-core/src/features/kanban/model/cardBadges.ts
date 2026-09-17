import type { StageType } from './stageType';

/**
 * 看板卡片底部那一行状态徽标的判定（设计文档 §7.2）。
 *
 * 只产出「色调 + i18n key」，不产出文案也不产出 class——渲染方自己 `t(labelKey)`。
 * i18n key 一律写成**字面量映射表**，因为 `scripts/check-unused-i18n-keys.mjs`
 * 抓不到模板字面量拼出来的 key（会被当成「动态使用」而漏掉未引用检测）。
 */
export type BadgeTone =
  | 'running'
  | 'stopped'
  | 'failed'
  | 'open'
  | 'merged'
  | 'closed'
  | 'pending';

export type CardBadge = {
  tone: BadgeTone;
  labelKey: string;
  /** 同类对象的个数，> 1 时渲染方可以追加「+N」。 */
  count: number;
};

/** 判定工作区徽标只需要这两个字段，别把整个 `WorkspaceWithStats` 拖进纯函数。 */
export type WorkspaceBadgeInput = {
  isRunning?: boolean;
  latestProcessStatus?: 'running' | 'completed' | 'failed' | 'killed';
};

const WORKSPACE_LABEL_KEYS = {
  running: 'kanban.badge.workspaceRunning',
  stopped: 'kanban.badge.workspaceStopped',
  failed: 'kanban.badge.workspaceFailed',
} as const;

/**
 * 关联工作区的运行状态。
 *
 * 优先级 **failed > running > stopped**：一个需求下只要有工作区跑挂了，
 * 卡片上就要先让人看见失败，而不是被另一个正在跑的工作区盖住。
 * 没有工作区返回 `null`（不渲染徽标，不留空占位）。
 */
export function buildWorkspaceBadge(
  workspaces: readonly WorkspaceBadgeInput[] | null | undefined
): CardBadge | null {
  if (!workspaces || workspaces.length === 0) {
    return null;
  }

  const count = workspaces.length;

  if (workspaces.some((w) => w.latestProcessStatus === 'failed')) {
    return { tone: 'failed', labelKey: WORKSPACE_LABEL_KEYS.failed, count };
  }

  if (
    workspaces.some(
      (w) => w.isRunning === true || w.latestProcessStatus === 'running'
    )
  ) {
    return { tone: 'running', labelKey: WORKSPACE_LABEL_KEYS.running, count };
  }

  return { tone: 'stopped', labelKey: WORKSPACE_LABEL_KEYS.stopped, count };
}

/** 判定 PR 徽标只需要状态。`PullRequestStatus` 只有这三个值（shared/remote-types.ts）。 */
export type PrBadgeInput = { status: 'open' | 'merged' | 'closed' };

const PR_LABEL_KEYS = {
  open: 'kanban.badge.prOpen',
  merged: 'kanban.badge.prMerged',
  closed: 'kanban.badge.prClosed',
} as const;

/** 流程上越靠后的状态越应该露出来：merged > open > closed。 */
const PR_STAGE_RANK: Record<PrBadgeInput['status'], number> = {
  closed: 0,
  open: 1,
  merged: 2,
};

/**
 * 关联 PR 的状态。多个 PR 时取「最靠后的状态」（已合并 > 待评审 > 已关闭）。
 *
 * 注意：设计文档里写的「草稿」态在本仓库不存在——`PullRequestStatus`
 * 只有 `open / merged / closed` 三个值，所以这里没有 draft。
 */
export function buildPrBadge(
  prs: readonly PrBadgeInput[] | null | undefined
): CardBadge | null {
  if (!prs || prs.length === 0) {
    return null;
  }

  let best: PrBadgeInput['status'] = 'closed';
  for (const pr of prs) {
    const status = pr.status;
    if (status !== 'open' && status !== 'merged' && status !== 'closed') {
      continue;
    }
    if (PR_STAGE_RANK[status] > PR_STAGE_RANK[best]) {
      best = status;
    }
  }

  return { tone: best, labelKey: PR_LABEL_KEYS[best], count: prs.length };
}

/**
 * 测试结果徽标。本期是占位：atp 还没接进来，恒返回「尚未接入」。
 * 接入后这里换成读 `results.json` 的通过率即可，调用点不用改。
 */
export function buildTestBadge(): CardBadge {
  return {
    tone: 'pending',
    labelKey: 'kanban.badge.testNotConnected',
    count: 0,
  };
}

/**
 * 负责人头像只在团队版出现（设计文档 §7.5：团队版才有的元素在个人版隐藏，不留空占位）。
 * 传 `isLocalPersonalMode()` 的结果进来，保持纯函数。
 */
export function shouldShowAssignees(isPersonalMode: boolean): boolean {
  return !isPersonalMode;
}

/**
 * 泳道阶段徽标要不要显示。
 *
 * **团队版（remote 数据源）的 `ProjectStatus` 根本没有 `stage_type` 字段**
 * （由 remote crate 生成，不能加字段），`readStageType` 会把每一列都回落成
 * `'todo'`。这时候渲染出来就是六个一模一样的「待开发」徽标，纯噪声。
 * 所以规则是：**所有列的阶段都相同就不显示**——这条同时覆盖了
 * 「团队版全部回落」和「个人版真的只有一个阶段」两种情况。
 */
export function shouldShowStageBadges(
  stages: readonly StageType[] | null | undefined
): boolean {
  if (!stages || stages.length < 2) {
    return false;
  }
  const first = stages[0];
  return stages.some((stage) => stage !== first);
}
