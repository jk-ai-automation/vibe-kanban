/**
 * 工作区详情顶部的「需求 → 工作区 → PR」路径条（设计文档 §7.4）。
 *
 * 纯函数：只决定有几段、每段显示什么、哪一段可点。
 * 渲染在 `@vibe/ui/components/WorkspaceBreadcrumb`。
 */
export type WorkspaceBreadcrumbSegmentKind = 'issue' | 'workspace' | 'pr';

export type WorkspaceBreadcrumbSegment = {
  kind: WorkspaceBreadcrumbSegmentKind;
  /** 直接可显示的文本（`simple_id` / 分支名 / `#12`）。 */
  label: string;
  /** 这一段有没有可跳转的目标。没有就渲染成静态文本。 */
  actionable: boolean;
  /** PR 段才有：外链地址。 */
  href?: string;
};

export type WorkspaceBreadcrumbInput = {
  /** 关联需求的 `simple_id`。没有关联需求时传 `null`。 */
  issueSimpleId?: string | null;
  /** 需求面板能不能打开（有 projectId + issueId 才行）。 */
  canOpenIssue?: boolean;
  /** 工作区名或分支名。 */
  workspaceLabel?: string | null;
  canOpenWorkspace?: boolean;
  /** 关联 PR。多个时取编号最大的那个（最新创建的）。 */
  pullRequests?: readonly { number: number; url: string }[] | null;
};

function normalizeLabel(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * 拼出路径条。
 *
 * - 无关联需求 → 只有「工作区 → PR」。
 * - 无 PR → 只有「需求 → 工作区」。
 * - 全都没有 → 空数组（调用方不渲染路径条）。
 */
export function buildWorkspaceBreadcrumb(
  input: WorkspaceBreadcrumbInput
): WorkspaceBreadcrumbSegment[] {
  const segments: WorkspaceBreadcrumbSegment[] = [];

  const issueLabel = normalizeLabel(input.issueSimpleId);
  if (issueLabel) {
    segments.push({
      kind: 'issue',
      label: issueLabel,
      actionable: input.canOpenIssue !== false,
    });
  }

  const workspaceLabel = normalizeLabel(input.workspaceLabel);
  if (workspaceLabel) {
    segments.push({
      kind: 'workspace',
      label: workspaceLabel,
      actionable: input.canOpenWorkspace !== false,
    });
  }

  const pr = pickLatestPullRequest(input.pullRequests);
  if (pr) {
    segments.push({
      kind: 'pr',
      label: `#${pr.number}`,
      actionable: true,
      href: pr.url,
    });
  }

  return segments;
}

/** 多个 PR 时取编号最大的（最新那个）。编号相同按先出现的算。 */
export function pickLatestPullRequest(
  prs: readonly { number: number; url: string }[] | null | undefined
): { number: number; url: string } | null {
  if (!prs || prs.length === 0) {
    return null;
  }

  let best: { number: number; url: string } | null = null;
  for (const pr of prs) {
    if (!Number.isFinite(pr.number)) continue;
    if (best === null || pr.number > best.number) {
      best = pr;
    }
  }
  return best;
}
