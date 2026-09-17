import { describe, expect, it } from 'vitest';
import {
  buildWorkspaceBreadcrumb,
  pickLatestPullRequest,
} from './workspaceBreadcrumb';

const 全量输入 = {
  issueSimpleId: 'VK-12',
  workspaceLabel: 'feat/login',
  pullRequests: [{ number: 7, url: 'https://example.com/pr/7' }],
};

describe('buildWorkspaceBreadcrumb（需求 → 工作区 → PR）', () => {
  it('三样齐全时是三段', () => {
    const segments = buildWorkspaceBreadcrumb(全量输入);
    expect(segments.map((s) => s.kind)).toEqual(['issue', 'workspace', 'pr']);
    expect(segments.map((s) => s.label)).toEqual(['VK-12', 'feat/login', '#7']);
  });

  it('无关联需求时只有两段', () => {
    const segments = buildWorkspaceBreadcrumb({
      ...全量输入,
      issueSimpleId: null,
    });
    expect(segments.map((s) => s.kind)).toEqual(['workspace', 'pr']);
  });

  it('无 PR 时只有两段', () => {
    const segments = buildWorkspaceBreadcrumb({
      ...全量输入,
      pullRequests: [],
    });
    expect(segments.map((s) => s.kind)).toEqual(['issue', 'workspace']);
  });

  it('全都没有时是空数组', () => {
    expect(buildWorkspaceBreadcrumb({})).toEqual([]);
  });

  it('空字符串 / 纯空格当成缺失', () => {
    const segments = buildWorkspaceBreadcrumb({
      issueSimpleId: '   ',
      workspaceLabel: '',
    });
    expect(segments).toEqual([]);
  });

  it('标签两端空格被裁掉', () => {
    const segments = buildWorkspaceBreadcrumb({
      workspaceLabel: '  feat/x  ',
    });
    expect(segments[0].label).toBe('feat/x');
  });

  it('PR 段带外链且总是可点', () => {
    const pr = buildWorkspaceBreadcrumb(全量输入)[2];
    expect(pr.actionable).toBe(true);
    expect(pr.href).toBe('https://example.com/pr/7');
  });

  it('canOpenIssue 为 false 时需求段不可点', () => {
    const segments = buildWorkspaceBreadcrumb({
      ...全量输入,
      canOpenIssue: false,
    });
    expect(segments[0].actionable).toBe(false);
  });

  it('canOpenWorkspace 为 false 时工作区段不可点', () => {
    const segments = buildWorkspaceBreadcrumb({
      ...全量输入,
      canOpenWorkspace: false,
    });
    expect(segments[1].actionable).toBe(false);
  });

  it('默认可点（不传 canOpen* 时）', () => {
    const segments = buildWorkspaceBreadcrumb(全量输入);
    expect(segments[0].actionable).toBe(true);
    expect(segments[1].actionable).toBe(true);
  });

  it('多个 PR 时取编号最大的', () => {
    const segments = buildWorkspaceBreadcrumb({
      ...全量输入,
      pullRequests: [
        { number: 3, url: 'https://example.com/pr/3' },
        { number: 9, url: 'https://example.com/pr/9' },
        { number: 5, url: 'https://example.com/pr/5' },
      ],
    });
    expect(segments[2].label).toBe('#9');
    expect(segments[2].href).toBe('https://example.com/pr/9');
  });
});

describe('pickLatestPullRequest', () => {
  it('空输入返回 null', () => {
    expect(pickLatestPullRequest([])).toBeNull();
    expect(pickLatestPullRequest(null)).toBeNull();
    expect(pickLatestPullRequest(undefined)).toBeNull();
  });

  it('忽略编号非数字的项', () => {
    const prs = [
      { number: Number.NaN, url: 'a' },
      { number: 2, url: 'b' },
    ];
    expect(pickLatestPullRequest(prs)?.url).toBe('b');
  });

  it('编号相同取先出现的', () => {
    const prs = [
      { number: 4, url: 'first' },
      { number: 4, url: 'second' },
    ];
    expect(pickLatestPullRequest(prs)?.url).toBe('first');
  });
});
