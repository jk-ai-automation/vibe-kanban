import { beforeEach, describe, expect, it } from 'vitest';
import {
  ISSUE_COMMENTS_SHAPE,
  ISSUE_MUTATION,
  PROJECTS_SHAPE,
  PROJECT_ISSUES_SHAPE,
  PROJECT_ISSUE_ASSIGNEES_SHAPE,
  PROJECT_PROJECT_STATUSES_SHAPE,
  PROJECT_TAGS_SHAPE,
  PROJECT_WORKSPACES_SHAPE,
  PULL_REQUEST_ISSUE_MUTATION,
  USER_WORKSPACES_SHAPE,
} from 'shared/remote-types';

import {
  configureDataSource,
  getDataSourceMode,
  isLocalMode,
} from '@/shared/lib/local/dataSource';
import {
  resolveLocalMutationUrl,
  resolveLocalShapeEndpoint,
} from '@/shared/lib/local/localEndpoints';
import {
  PIPELINE_RUNS_SHAPE,
  PIPELINE_STAGE_RUNS_SHAPE,
} from '@/entities/pipeline/api/pipelineShapes';
import { patchToWrites } from '@/shared/lib/local/localCollections';

describe('数据源开关', () => {
  beforeEach(() => {
    configureDataSource('remote');
  });

  it('默认是 remote，保证团队版行为不变', () => {
    expect(getDataSourceMode()).toBe('remote');
    expect(isLocalMode()).toBe(false);
  });

  it('切到 local 后 isLocalMode 为真', () => {
    configureDataSource('local');
    expect(getDataSourceMode()).toBe('local');
    expect(isLocalMode()).toBe(true);
  });
});

describe('resolveLocalShapeEndpoint', () => {
  it('项目集合不带查询参数', () => {
    expect(resolveLocalShapeEndpoint(PROJECTS_SHAPE, {})).toEqual({
      kind: 'rest',
      path: '/api/local/projects',
      wsPath: null,
    });
  });

  it('项目维度集合带上 project_id', () => {
    const endpoint = resolveLocalShapeEndpoint(PROJECT_ISSUES_SHAPE, {
      project_id: 'p1',
    });
    expect(endpoint).toEqual({
      kind: 'rest',
      path: '/api/local/issues?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('状态列也订阅同一个需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_PROJECT_STATUSES_SHAPE, {
        project_id: 'p1',
      })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/project_statuses?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('标签映射到 /api/local/tags', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_TAGS_SHAPE, { project_id: 'p1' }).path
    ).toBe('/api/local/tags?project_id=p1');
  });

  it('评论按 issue_id 过滤', () => {
    expect(
      resolveLocalShapeEndpoint(ISSUE_COMMENTS_SHAPE, { issue_id: 'i1' }).path
    ).toBe('/api/local/issue_comments?issue_id=i1');
  });

  it('项目维度工作区走投影接口', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_WORKSPACES_SHAPE, { project_id: 'p1' })
        .path
    ).toBe('/api/local/workspaces?project_id=p1');
  });

  it('个人版没有跨项目的用户工作区集合，返回空集合', () => {
    expect(
      resolveLocalShapeEndpoint(USER_WORKSPACES_SHAPE, { owner_user_id: 'u1' })
    ).toEqual({ kind: 'empty', table: 'workspaces' });
  });

  it('未实现的集合返回空集合而不是抛错', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_ISSUE_ASSIGNEES_SHAPE, {
        project_id: 'p1',
      })
    ).toEqual({ kind: 'empty', table: 'issue_assignees' });
  });

  it('参数值被 URL 编码，防止拼接出越权路径', () => {
    const endpoint = resolveLocalShapeEndpoint(PROJECT_ISSUES_SHAPE, {
      project_id: '../../etc/passwd',
    });
    expect(endpoint).toEqual({
      kind: 'rest',
      path: '/api/local/issues?project_id=..%2F..%2Fetc%2Fpasswd',
      wsPath: '/api/issues/streams/ws?project_id=..%2F..%2Fetc%2Fpasswd',
    });
  });
});

describe('resolveLocalMutationUrl', () => {
  it('需求写接口指向 /api/local/issues', () => {
    expect(resolveLocalMutationUrl(ISSUE_MUTATION)).toBe('/api/local/issues');
  });

  it('个人版不支持的写操作返回 null', () => {
    expect(resolveLocalMutationUrl(PULL_REQUEST_ISSUE_MUTATION)).toBeNull();
  });
});

describe('流水线集合（契约 §3）', () => {
  it('pipeline_runs：REST 快照 + 订阅需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PIPELINE_RUNS_SHAPE, { project_id: 'p1' })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/pipeline_runs?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('pipeline_stage_runs：同一条需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PIPELINE_STAGE_RUNS_SHAPE, { project_id: 'p1' })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/pipeline_stage_runs?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('缺 project_id 时是空集合', () => {
    expect(resolveLocalShapeEndpoint(PIPELINE_RUNS_SHAPE, {})).toEqual({
      kind: 'empty',
      table: 'pipeline_runs',
    });
  });

  it('推送路径 /pipeline_runs/{id} 能落成集合写操作', () => {
    expect(
      patchToWrites(
        [{ op: 'replace', path: '/pipeline_runs/r1', value: { id: 'r1' } }],
        'pipeline_runs'
      )
    ).toEqual([{ type: 'update', value: { id: 'r1' } }]);
  });
});
