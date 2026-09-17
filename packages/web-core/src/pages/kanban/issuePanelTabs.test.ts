import { describe, expect, it } from 'vitest';
import {
  buildIssuePanelTabs,
  defaultIssuePanelTab,
  FALLBACK_ISSUE_PANEL_TAB,
  isIssuePanelTab,
  issuePanelTabLabelKey,
  normalizeIssuePanelTab,
} from './issuePanelTabs';

describe('buildIssuePanelTabs', () => {
  it('编辑模式是三段式', () => {
    expect(buildIssuePanelTabs({ mode: 'edit' })).toEqual([
      'overview',
      'development',
      'testing',
    ]);
  });

  it('新建模式只有概况', () => {
    expect(buildIssuePanelTabs({ mode: 'create' })).toEqual(['overview']);
  });

  it('返回的是副本，改它不影响后续调用', () => {
    const tabs = buildIssuePanelTabs({ mode: 'edit' });
    tabs.pop();
    expect(buildIssuePanelTabs({ mode: 'edit' })).toHaveLength(3);
  });
});

describe('defaultIssuePanelTab', () => {
  it('有工作区时默认进开发页', () => {
    expect(defaultIssuePanelTab({ mode: 'edit', hasWorkspaces: true })).toBe(
      'development'
    );
  });

  it('没有工作区时默认进概况', () => {
    expect(defaultIssuePanelTab({ mode: 'edit', hasWorkspaces: false })).toBe(
      'overview'
    );
  });

  it('新建模式永远是概况', () => {
    expect(defaultIssuePanelTab({ mode: 'create', hasWorkspaces: true })).toBe(
      'overview'
    );
  });
});

describe('normalizeIssuePanelTab', () => {
  it('合法值原样返回', () => {
    expect(normalizeIssuePanelTab('testing')).toBe('testing');
  });

  it('非法值回落到概况', () => {
    expect(normalizeIssuePanelTab('nonsense')).toBe('overview');
    expect(normalizeIssuePanelTab(undefined)).toBe('overview');
    expect(normalizeIssuePanelTab(null)).toBe('overview');
    expect(normalizeIssuePanelTab(3)).toBe('overview');
  });

  it('不在可见清单里的值也回落（新建模式点不到开发页）', () => {
    expect(normalizeIssuePanelTab('development', ['overview'])).toBe(
      'overview'
    );
  });

  it('回落值就是 FALLBACK_ISSUE_PANEL_TAB', () => {
    expect(FALLBACK_ISSUE_PANEL_TAB).toBe('overview');
  });
});

describe('isIssuePanelTab', () => {
  it('只认三个字面量', () => {
    expect(isIssuePanelTab('overview')).toBe(true);
    expect(isIssuePanelTab('development')).toBe(true);
    expect(isIssuePanelTab('testing')).toBe(true);
    expect(isIssuePanelTab('Overview')).toBe(false);
  });
});

describe('issuePanelTabLabelKey', () => {
  it('返回字面量 i18n key', () => {
    expect(issuePanelTabLabelKey('overview')).toBe('kanban.tabs.overview');
    expect(issuePanelTabLabelKey('development')).toBe(
      'kanban.tabs.development'
    );
    expect(issuePanelTabLabelKey('testing')).toBe('kanban.tabs.testing');
  });
});
