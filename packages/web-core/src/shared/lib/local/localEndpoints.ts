import type { MutationDefinition, ShapeDefinition } from 'shared/remote-types';

const LOCAL_API_PREFIX = '/api/local';
const ISSUE_STREAM_PATH = '/api/issues/streams/ws';

export type LocalShapeEndpoint =
  | { kind: 'rest'; path: string; wsPath: string | null }
  | { kind: 'empty'; table: string };

/** 项目维度的需求流覆盖的表：它们共用 /api/issues/streams/ws。 */
const ISSUE_STREAM_TABLES = new Set([
  'issues',
  'project_statuses',
  'issue_comments',
  // 流水线状态与需求走同一条流（契约 §3）
  'pipeline_runs',
  'pipeline_stage_runs',
]);

/** 前端表名 → 本地 REST 资源段。 */
const REST_RESOURCE: Record<string, string> = {
  projects: 'projects',
  project_statuses: 'project_statuses',
  issues: 'issues',
  tags: 'tags',
  issue_tags: 'issue_tags',
  issue_comments: 'issue_comments',
  workspaces: 'workspaces',
  pull_requests: 'pull_requests',
  pipeline_runs: 'pipeline_runs',
  pipeline_stage_runs: 'pipeline_stage_runs',
};

/** 每个表在个人版里允许的过滤参数。其他参数一律丢弃。 */
const ALLOWED_PARAM: Record<string, string | null> = {
  projects: null,
  project_statuses: 'project_id',
  issues: 'project_id',
  tags: 'project_id',
  issue_tags: 'project_id',
  issue_comments: 'issue_id',
  workspaces: 'project_id',
  pull_requests: 'project_id',
  pipeline_runs: 'project_id',
  pipeline_stage_runs: 'project_id',
};

/** mutation.name → 本地写接口。null 表示个人版不支持该写操作。 */
const MUTATION_URL: Record<string, string | null> = {
  Project: `${LOCAL_API_PREFIX}/projects`,
  ProjectStatus: `${LOCAL_API_PREFIX}/project_statuses`,
  Issue: `${LOCAL_API_PREFIX}/issues`,
  Tag: `${LOCAL_API_PREFIX}/tags`,
  IssueTag: `${LOCAL_API_PREFIX}/issue_tags`,
  IssueComment: `${LOCAL_API_PREFIX}/issue_comments`,
  IssueAssignee: null,
  IssueFollower: null,
  IssueRelationship: null,
  IssueCommentReaction: null,
  PullRequestIssue: null,
  Notification: null,
};

export function resolveLocalShapeEndpoint(
  shape: ShapeDefinition<unknown>,
  params: Record<string, string>
): LocalShapeEndpoint {
  const resource = REST_RESOURCE[shape.table];
  if (!resource) {
    return { kind: 'empty', table: shape.table };
  }

  const paramName = ALLOWED_PARAM[shape.table] ?? null;
  if (paramName === null) {
    return {
      kind: 'rest',
      path: `${LOCAL_API_PREFIX}/${resource}`,
      wsPath: null,
    };
  }

  const value = params[paramName];
  if (!value) {
    // 例如 USER_WORKSPACES_SHAPE 只带 owner_user_id：个人版没有这个维度。
    return { kind: 'empty', table: shape.table };
  }

  const encoded = encodeURIComponent(value);
  const query = `${paramName}=${encoded}`;
  const wsPath =
    ISSUE_STREAM_TABLES.has(shape.table) && paramName === 'project_id'
      ? `${ISSUE_STREAM_PATH}?${query}`
      : null;

  return {
    kind: 'rest',
    path: `${LOCAL_API_PREFIX}/${resource}?${query}`,
    wsPath,
  };
}

export function resolveLocalMutationUrl(
  mutation: MutationDefinition<unknown, unknown, unknown>
): string | null {
  return MUTATION_URL[mutation.name] ?? null;
}
