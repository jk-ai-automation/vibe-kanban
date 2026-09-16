/**
 * 需求详情三段式（设计文档 §7.3）：概况 / 开发 / 测试。
 *
 * 纯函数层：决定「有哪些页」「默认落在哪页」「非法值怎么回落」。
 * 渲染在 `@vibe/ui/components/KanbanIssuePanel`，状态在
 * `KanbanIssuePanelContainer` 里（`useState`）。
 */
export type IssuePanelTab = 'overview' | 'development' | 'testing';

export const ISSUE_PANEL_TAB_ORDER: IssuePanelTab[] = [
  'overview',
  'development',
  'testing',
];

export const FALLBACK_ISSUE_PANEL_TAB: IssuePanelTab = 'overview';

export function isIssuePanelTab(value: unknown): value is IssuePanelTab {
  return value === 'overview' || value === 'development' || value === 'testing';
}

/**
 * 当前模式下可见的标签页。
 *
 * 新建模式只有「概况」：需求还没落库，开发（工作区）与测试都无从谈起，
 * 而且 `KanbanIssuePanel` 里的 `renderWorkspacesSection` 等本来就被
 * `!isCreateMode && issueId` 挡着。
 */
export function buildIssuePanelTabs(params: {
  mode: 'create' | 'edit';
}): IssuePanelTab[] {
  return params.mode === 'create' ? ['overview'] : [...ISSUE_PANEL_TAB_ORDER];
}

/**
 * 默认落在哪一页：已经有工作区就直接进「开发」，否则「概况」。
 * 新建模式永远是「概况」。
 */
export function defaultIssuePanelTab(params: {
  mode: 'create' | 'edit';
  hasWorkspaces: boolean;
}): IssuePanelTab {
  if (params.mode === 'create') {
    return FALLBACK_ISSUE_PANEL_TAB;
  }
  return params.hasWorkspaces ? 'development' : FALLBACK_ISSUE_PANEL_TAB;
}

/**
 * 把任意输入（URL query、localStorage、上一次的选择）收敛成合法标签页。
 * 不在当前可见清单里的值一律回落到「概况」，**绝不抛错**。
 */
export function normalizeIssuePanelTab(
  value: unknown,
  available: readonly IssuePanelTab[] = ISSUE_PANEL_TAB_ORDER
): IssuePanelTab {
  if (isIssuePanelTab(value) && available.includes(value)) {
    return value;
  }
  return FALLBACK_ISSUE_PANEL_TAB;
}

const ISSUE_PANEL_TAB_LABEL_KEYS: Record<IssuePanelTab, string> = {
  overview: 'kanban.tabs.overview',
  development: 'kanban.tabs.development',
  testing: 'kanban.tabs.testing',
};

/** 标签页的 i18n key（common 命名空间）。字面量映射表，别用模板字面量拼。 */
export function issuePanelTabLabelKey(tab: IssuePanelTab): string {
  return ISSUE_PANEL_TAB_LABEL_KEYS[tab];
}
