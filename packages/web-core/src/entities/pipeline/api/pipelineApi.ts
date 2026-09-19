import type {
  ApiResponse,
  GateDecisionRequest,
  IssueArtifact,
  IssuePipelineView,
  PendingPipelineItem,
  StartPipelineRequest,
} from 'shared/types';
import type { CreateIssueRequest } from 'shared/remote-types';
import { makeLocalApiRequest } from '@/shared/lib/localApiTransport';
import { parseEnvelopeError } from '@/shared/lib/local/apiEnvelope';

export type PipelineRunAction = 'pause' | 'resume' | 'cancel';

/**
 * 暂停 / 继续 / 取消各写一条完整路径，不拼 `${action}`：后端路由契约测试
 * （C13）按源码里的字面路径比对，每个 `${…}` 都会被当成 `{id}`。
 */
const RUN_ACTION_PATHS: Record<PipelineRunAction, (runId: string) => string> = {
  pause: (runId) => `/api/local/pipeline/runs/${runId}/pause`,
  resume: (runId) => `/api/local/pipeline/runs/${runId}/resume`,
  cancel: (runId) => `/api/local/pipeline/runs/${runId}/cancel`,
};

/**
 * 契约 §2 的全部路径。改名先改契约。
 *
 * 本文件被后端路由契约测试扫描（C13）：路径必须写成完整字面量，
 * 参数只用 `${…}` 占位，查询串以 `?` 开头；注释里不要写别的本地接口路径。
 */
export const PIPELINE_API_PATHS = {
  issues: '/api/local/issues',
  issuePipeline: (issueId: string) => `/api/local/issues/${issueId}/pipeline`,
  artifact: (artifactId: string) =>
    `/api/local/pipeline/artifacts/${artifactId}`,
  gate: (stageRunId: string) =>
    `/api/local/pipeline/stage-runs/${stageRunId}/gate`,
  runAction: (runId: string, action: PipelineRunAction) =>
    RUN_ACTION_PATHS[action](runId),
  pending: (projectId: string | null) =>
    projectId
      ? `/api/local/pipeline/pending?project_id=${encodeURIComponent(projectId)}`
      : '/api/local/pipeline/pending',
} as const;

export class PipelineApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = 'PipelineApiError';
  }
}

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const;

function post(body?: unknown): RequestInit {
  return body === undefined
    ? { method: 'POST' }
    : { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) };
}

/**
 * 走 `ApiResponse<T>` 信封的请求。
 *
 * 和 `adminApi.ts` 的 `requestLocalEnvelope` 同样的错误语义（400/404/409 的
 * message 可以直接展示），但**不钉死 `hostScope: 'none'`**：流水线数据和看板
 * 集合一样跟随当前 host。
 */
async function requestEnvelope<T>(
  path: string,
  init: RequestInit = {}
): Promise<T> {
  const response = await makeLocalApiRequest(path, init);
  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new PipelineApiError(
      response.status,
      message ?? `${path} failed with status ${response.status}`
    );
  }
  let envelope: ApiResponse<T> | null = null;
  try {
    envelope = (await response.json()) as ApiResponse<T>;
  } catch {
    envelope = null;
  }
  if (!envelope || !envelope.success) {
    throw new PipelineApiError(
      response.status,
      envelope?.message ?? `${path} returned an unsuccessful response`
    );
  }
  return envelope.data as T;
}

export function getIssuePipeline(
  issueId: string
): Promise<IssuePipelineView | null> {
  return requestEnvelope(PIPELINE_API_PATHS.issuePipeline(issueId));
}

export function startPipeline(
  issueId: string,
  payload: StartPipelineRequest
): Promise<IssuePipelineView> {
  return requestEnvelope(
    PIPELINE_API_PATHS.issuePipeline(issueId),
    post(payload)
  );
}

export function getArtifact(artifactId: string): Promise<IssueArtifact> {
  return requestEnvelope(PIPELINE_API_PATHS.artifact(artifactId));
}

export function decideGate(
  stageRunId: string,
  payload: GateDecisionRequest
): Promise<IssuePipelineView> {
  return requestEnvelope(PIPELINE_API_PATHS.gate(stageRunId), post(payload));
}

export function runPipelineAction(
  runId: string,
  action: PipelineRunAction
): Promise<IssuePipelineView> {
  return requestEnvelope(PIPELINE_API_PATHS.runAction(runId, action), post());
}

export function listPendingPipelines(
  projectId: string | null
): Promise<PendingPipelineItem[]> {
  return requestEnvelope(PIPELINE_API_PATHS.pending(projectId));
}

/**
 * 建需求。本地写接口返回 `{ txid }` 而不是信封
 * （`crates/server/src/routes/local_projects/mod.rs`），只看状态码。
 */
export async function createIssue(payload: CreateIssueRequest): Promise<void> {
  const response = await makeLocalApiRequest(
    PIPELINE_API_PATHS.issues,
    post(payload)
  );
  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new PipelineApiError(
      response.status,
      message ?? `create issue failed with status ${response.status}`
    );
  }
}

/** 工作台「开始」：先建需求（前端生成 id），再启动流水线。 */
export async function createIssueAndStartPipeline(args: {
  issue: CreateIssueRequest & { id: string };
  pipeline: StartPipelineRequest;
}): Promise<IssuePipelineView> {
  await createIssue(args.issue);
  return startPipeline(args.issue.id, args.pipeline);
}
