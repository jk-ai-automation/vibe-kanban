import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { StartPipelineRequest } from 'shared/types';
import {
  PIPELINE_API_PATHS,
  createIssueAndStartPipeline,
  decideGate,
  getIssuePipeline,
  listPendingPipelines,
  runPipelineAction,
} from './pipelineApi';
import {
  setLocalApiTransport,
  type LocalApiRequestOptions,
} from '@/shared/lib/localApiTransport';
import { resetRuntimeModeForTests } from '@/shared/lib/local/runtimeMode';

interface RecordedCall {
  path: string;
  init: LocalApiRequestOptions;
}

let calls: RecordedCall[] = [];
let responses: Response[] = [];

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
const envelope = (data: unknown) =>
  json({ success: true, data, error_data: null, message: null });

const PIPELINE_REQUEST = {
  repos: [{ repo_id: 'repo-1', target_branch: 'main' }],
  executor_config: { executor: 'CLAUDE_CODE' },
  template_key: null,
} as unknown as StartPipelineRequest;

const ISSUE_REQUEST = {
  id: 'i1',
  project_id: 'p1',
  status_id: 's1',
  title: '下单最小金额校验',
  description: null,
  priority: null,
  start_date: null,
  target_date: null,
  completed_at: null,
  sort_order: 0,
  parent_issue_id: null,
  parent_issue_sort_order: null,
  extension_metadata: {},
};

beforeEach(() => {
  calls = [];
  responses = [];
  resetRuntimeModeForTests();
  setLocalApiTransport({
    request: (path, init = {}) => {
      calls.push({ path, init });
      return Promise.resolve(
        responses.shift() ?? new Response(null, { status: 500 })
      );
    },
    openWebSocket: () => {
      throw new Error('not used');
    },
  });
});

afterEach(() => {
  setLocalApiTransport(null);
});

describe('PIPELINE_API_PATHS（契约 §2）', () => {
  it('路径逐字对上契约', () => {
    expect(PIPELINE_API_PATHS.issuePipeline('i1')).toBe(
      '/api/local/issues/i1/pipeline'
    );
    expect(PIPELINE_API_PATHS.artifact('a1')).toBe(
      '/api/local/pipeline/artifacts/a1'
    );
    expect(PIPELINE_API_PATHS.gate('s1')).toBe(
      '/api/local/pipeline/stage-runs/s1/gate'
    );
    expect(PIPELINE_API_PATHS.runAction('r1', 'resume')).toBe(
      '/api/local/pipeline/runs/r1/resume'
    );
    expect(PIPELINE_API_PATHS.pending('p 1')).toBe(
      '/api/local/pipeline/pending?project_id=p%201'
    );
    expect(PIPELINE_API_PATHS.pending(null)).toBe(
      '/api/local/pipeline/pending'
    );
  });
});

describe('信封解析', () => {
  it('没有运行时 data 为 null', async () => {
    responses.push(envelope(null));
    await expect(getIssuePipeline('i1')).resolves.toBeNull();
    expect(calls[0].path).toBe('/api/local/issues/i1/pipeline');
  });

  it('409 抛 PipelineApiError，带后端 message', async () => {
    responses.push(
      json(
        {
          success: false,
          data: null,
          error_data: null,
          message: '该阶段不在等待确认',
        },
        409
      )
    );
    await expect(
      decideGate('s1', { decision: 'approve', comment: null })
    ).rejects.toMatchObject({
      name: 'PipelineApiError',
      status: 409,
      message: '该阶段不在等待确认',
    });
  });

  it('HTTP 200 但 success=false 也算失败', async () => {
    responses.push(
      json({ success: false, data: null, error_data: null, message: '不行' })
    );
    await expect(listPendingPipelines('p1')).rejects.toMatchObject({
      status: 200,
      message: '不行',
    });
  });

  it('写操作是 POST，带 JSON 体', async () => {
    responses.push(envelope({ run: { id: 'r1' } }));
    await decideGate('s1', { decision: 'reject', comment: '缺边界' });
    expect(calls[0].init.method).toBe('POST');
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      decision: 'reject',
      comment: '缺边界',
    });
  });

  it('暂停 / 继续 / 取消都是无体 POST', async () => {
    responses.push(envelope({ run: { id: 'r1' } }));
    await runPipelineAction('r1', 'pause');
    expect(calls[0]).toMatchObject({
      path: '/api/local/pipeline/runs/r1/pause',
      init: { method: 'POST' },
    });
    expect(calls[0].init.body).toBeUndefined();
  });
});

describe('createIssueAndStartPipeline', () => {
  it('先建需求再启动流水线，需求 id 贯穿两步', async () => {
    responses.push(
      json({ txid: 0 }),
      envelope({ run: { id: 'r1', issue_id: 'i1' } })
    );
    await createIssueAndStartPipeline({
      issue: ISSUE_REQUEST,
      pipeline: PIPELINE_REQUEST,
    });
    expect(calls.map((c) => [c.init.method, c.path])).toEqual([
      ['POST', '/api/local/issues'],
      ['POST', '/api/local/issues/i1/pipeline'],
    ]);
    expect(JSON.parse(String(calls[0].init.body)).id).toBe('i1');
    expect(JSON.parse(String(calls[1].init.body))).toEqual(PIPELINE_REQUEST);
  });

  it('建需求失败就不启动流水线', async () => {
    responses.push(json({ message: '标题不能为空' }, 400));
    await expect(
      createIssueAndStartPipeline({
        issue: ISSUE_REQUEST,
        pipeline: PIPELINE_REQUEST,
      })
    ).rejects.toMatchObject({ status: 400, message: '标题不能为空' });
    expect(calls).toHaveLength(1);
  });
});

/**
 * 镜像后端契约测试 C13（`crates/server/src/routes/local_projects/mod.rs` 的
 * `流水线前端调用的路径都挂上了路由`）的解析方式：在源码里找每个
 * `/api/local/`，截到引号 / 反引号 / `?` / 空白为止，`${…}` 换成 `{id}`。
 * 这样不跑 cargo 也能知道后端测试会看到哪些路径。
 */
function pathsSeenByRustContractTest(source: string): string[] {
  const expanded = source.split('${LOCAL_API_PREFIX}').join('/api/local');
  const found: string[] = [];
  let rest = expanded;
  for (;;) {
    const at = rest.indexOf('/api/local/');
    if (at === -1) break;
    const tail = rest.slice(at);
    const match = /['"`?\s]/.exec(tail);
    const end = match ? match.index : tail.length;
    found.push(tail.slice(0, end).replace(/\$\{[^}]*\}/g, '{id}'));
    rest = tail.slice(end);
  }
  return found;
}

describe('后端路由契约测试（C13）能解析本文件', () => {
  it('扫到的每条路径都是后端路由表里登记的写法', async () => {
    const { readFileSync } = await import('node:fs');
    const source = readFileSync(
      new URL('./pipelineApi.ts', import.meta.url),
      'utf8'
    );
    const registered = new Set([
      '/api/local/issues',
      '/api/local/issues/{id}/pipeline',
      '/api/local/pipeline/artifacts/{id}',
      '/api/local/pipeline/stage-runs/{id}/gate',
      '/api/local/pipeline/runs/{id}/pause',
      '/api/local/pipeline/runs/{id}/resume',
      '/api/local/pipeline/runs/{id}/cancel',
      '/api/local/pipeline/pending',
    ]);
    const seen = pathsSeenByRustContractTest(source);
    expect(seen.filter((path) => !registered.has(path))).toEqual([]);
    expect(new Set(seen)).toEqual(registered);
  });
});
