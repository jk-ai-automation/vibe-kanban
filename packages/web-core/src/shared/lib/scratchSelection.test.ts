import { describe, expect, it } from 'vitest';
import { resolveScratchSelection } from './scratchSelection';

describe('resolveScratchSelection（scratch 拉回来时怎么处理选中的组织 / 项目）', () => {
  it('①服务端有值、会话里的项目是回落写入的：用服务端值，且不回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: 'org-server',
        serverProjectId: 'project-server',
        sessionOrgId: 'org-first',
        sessionProjectId: 'project-first',
        sessionProjectSource: 'fallback',
      })
    ).toEqual({
      orgId: 'org-server',
      projectId: 'project-server',
      shouldSave: false,
    });
  });

  it('②用户显式选择 / 深链进入某个项目：保留会话值并回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: 'org-server',
        serverProjectId: 'project-server',
        sessionOrgId: 'org-session',
        sessionProjectId: 'project-session',
        sessionProjectSource: 'user',
      })
    ).toEqual({
      // 组织一律以服务端为准：会话里的组织都是自动挑的第一个，不是用户选的
      orgId: 'org-server',
      projectId: 'project-session',
      shouldSave: true,
    });
  });

  it('③服务端没有项目、用户有选择：用会话值并回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: null,
        serverProjectId: null,
        sessionOrgId: null,
        sessionProjectId: 'project-session',
        sessionProjectSource: 'user',
      })
    ).toEqual({
      orgId: null,
      projectId: 'project-session',
      shouldSave: true,
    });
  });

  it('用户选的和服务端存的一样：保留但不必回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: 'org-server',
        serverProjectId: 'project-same',
        sessionOrgId: null,
        sessionProjectId: 'project-same',
        sessionProjectSource: 'user',
      })
    ).toEqual({
      orgId: 'org-server',
      projectId: 'project-same',
      shouldSave: false,
    });
  });

  it('会话里没有项目：一律用服务端值，不回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: 'org-server',
        serverProjectId: 'project-server',
        sessionOrgId: 'org-first',
        sessionProjectId: null,
        sessionProjectSource: 'user',
      })
    ).toEqual({
      orgId: 'org-server',
      projectId: 'project-server',
      shouldSave: false,
    });
  });

  it('服务端与会话都没有项目：保持空，不回存', () => {
    expect(
      resolveScratchSelection({
        serverOrgId: null,
        serverProjectId: null,
        sessionOrgId: null,
        sessionProjectId: null,
        sessionProjectSource: 'fallback',
      })
    ).toEqual({ orgId: null, projectId: null, shouldSave: false });
  });
});
