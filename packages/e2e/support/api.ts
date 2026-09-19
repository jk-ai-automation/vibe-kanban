import { randomUUID } from 'node:crypto';
import { BACKEND_URL } from './env';

/** 个人版固定组织（`crates/db/src/models/local_project.rs:9`，`Uuid::from_u128(1)`）。 */
export const PERSONAL_ORGANIZATION_ID = '00000000-0000-0000-0000-000000000001';

interface Envelope<T> {
  success: boolean;
  data: T | null;
  message: string | null;
}

/** 只声明测试用到的字段；完整类型见 shared/types.ts（契约 §1）。 */
export interface PipelineViewLike {
  run: {
    id: string;
    issue_id: string;
    status: string;
    current_stage_key: string;
  };
  stages: {
    id: string;
    run_id: string;
    stage_key: string;
    attempt: number;
    status: string;
    gate_kind: string;
  }[];
  decisions: {
    id: string;
    stage_run_id: string;
    decision: string;
    comment: string | null;
  }[];
  artifacts: { id: string; kind: string; version: number }[];
  template: {
    stages: { key: string; gate_label: string | null; max_rounds: number }[];
  };
}

export interface IssueLike {
  id: string;
  title: string;
  simple_id: string;
  status_id: string;
}

/**
 * 直连后端（不经 vite 代理）。Node 的 fetch 不带 Origin 头，
 * `origin.rs:48-50` 放行；个人版没有 CSRF。
 */
export class VkApi {
  constructor(private readonly base: string = BACKEND_URL) {}

  private request(
    method: string,
    path: string,
    body?: unknown
  ): Promise<Response> {
    return fetch(`${this.base}${path}`, {
      method,
      headers:
        body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  }

  /** `ApiResponse<T>` 信封接口。 */
  async envelope<T>(method: string, path: string, body?: unknown): Promise<T> {
    const response = await this.request(method, path, body);
    const text = await response.text();
    let parsed: Envelope<T>;
    try {
      parsed = JSON.parse(text) as Envelope<T>;
    } catch {
      throw new Error(
        `${method} ${path} → ${response.status}: ${text.slice(0, 300)}`
      );
    }
    if (!response.ok || !parsed.success) {
      throw new Error(
        `${method} ${path} → ${response.status}: ${parsed.message ?? text.slice(0, 300)}`
      );
    }
    return parsed.data as T;
  }

  /** 本地项目接口：快照返回 `{ <表名>: [...] }`，写操作返回 `{ txid }`。 */
  async json<T>(method: string, path: string, body?: unknown): Promise<T> {
    const response = await this.request(method, path, body);
    if (!response.ok) {
      throw new Error(
        `${method} ${path} → ${response.status}: ${(await response.text()).slice(0, 300)}`
      );
    }
    return (await response.json()) as T;
  }

  /** 跳过首启引导、项目指南、版本说明；固定简体中文、主题跟随系统（视觉回归靠 colorScheme 切换）。 */
  async prepareConfig(): Promise<void> {
    const info = await this.envelope<{ config: Record<string, unknown> }>(
      'GET',
      '/api/info'
    );
    const config = info.config;
    const showcases =
      (config.showcases as { seen_features?: string[] } | undefined) ?? {};
    const seen = new Set(showcases.seen_features ?? []);
    seen.add('projects-guide');
    await this.envelope('PUT', '/api/config', {
      ...config,
      disclaimer_acknowledged: true,
      onboarding_acknowledged: true,
      remote_onboarding_acknowledged: true,
      analytics_enabled: false,
      show_release_notes: false,
      language: 'ZH_HANS',
      theme: 'SYSTEM',
      showcases: { ...showcases, seen_features: [...seen] },
    });
  }

  async defaultExecutor(): Promise<string> {
    const info = await this.envelope<{
      config: { executor_profile: { executor: string } };
    }>('GET', '/api/info');
    return info.config.executor_profile.executor;
  }

  registerRepo(path: string, displayName: string) {
    return this.envelope<{ id: string; name: string; display_name: string }>(
      'POST',
      '/api/repos',
      {
        path,
        display_name: displayName,
      }
    );
  }

  async createProject(name: string): Promise<{ id: string; name: string }> {
    const id = randomUUID();
    await this.json('POST', '/api/local/projects', {
      id,
      organization_id: PERSONAL_ORGANIZATION_ID,
      name,
      color: '25 82% 54%',
    });
    return { id, name };
  }

  async listIssues(projectId: string): Promise<IssueLike[]> {
    const payload = await this.json<{ issues: IssueLike[] }>(
      'GET',
      `/api/local/issues?project_id=${projectId}`
    );
    return payload.issues;
  }

  async findIssueByTitle(projectId: string, title: string): Promise<IssueLike> {
    const issue = (await this.listIssues(projectId)).find(
      (item) => item.title === title
    );
    if (!issue) throw new Error(`找不到需求：${title}`);
    return issue;
  }

  issuePipeline(issueId: string): Promise<PipelineViewLike | null> {
    return this.envelope('GET', `/api/local/issues/${issueId}/pipeline`);
  }

  /** 不经界面直接建需求并启动流水线（视觉回归准备数据用）。 */
  async startPipelineDirect(args: {
    projectId: string;
    repoId: string;
    title: string;
  }): Promise<string> {
    const { project_statuses: statuses } = await this.json<{
      project_statuses: {
        id: string;
        sort_order: number;
        stage_type?: string;
      }[];
    }>('GET', `/api/local/project_statuses?project_id=${args.projectId}`);
    const backlog =
      statuses.find((status) => status.stage_type === 'backlog') ??
      [...statuses].sort((a, b) => a.sort_order - b.sort_order)[0];
    const issueId = randomUUID();
    await this.json('POST', '/api/local/issues', {
      id: issueId,
      project_id: args.projectId,
      status_id: backlog.id,
      title: args.title,
      description: null,
      priority: null,
      start_date: null,
      target_date: null,
      completed_at: null,
      sort_order: 0,
      parent_issue_id: null,
      parent_issue_sort_order: null,
      extension_metadata: {},
    });
    await this.envelope('POST', `/api/local/issues/${issueId}/pipeline`, {
      repos: [{ repo_id: args.repoId, target_branch: 'main' }],
      executor_config: { executor: await this.defaultExecutor() },
      template_key: null,
    });
    return issueId;
  }

  /** 轮询直到条件满足（引擎推进是异步的）。 */
  async waitForPipeline(
    issueId: string,
    predicate: (view: PipelineViewLike) => boolean,
    timeoutMs = 120_000
  ): Promise<PipelineViewLike> {
    const deadline = Date.now() + timeoutMs;
    let last: PipelineViewLike | null = null;
    while (Date.now() < deadline) {
      last = await this.issuePipeline(issueId);
      if (last && predicate(last)) return last;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    throw new Error(
      `等待流水线超时，最后状态：${JSON.stringify(last?.run ?? null)}`
    );
  }
}
