import { describe, expect, it } from 'vitest';
import {
  latestAttempts,
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  toNumber,
  type Translate,
} from './progress';
import {
  STANDARD_TEMPLATE,
  at,
  makeRun,
  makeStage,
} from './__fixtures__/pipeline';

const states = (cells: { state: string }[]) => cells.map((cell) => cell.state);

/** 假翻译：key + 参数原样拼出来，断言时一眼看出用了哪个 key。 */
const t: Translate = (key, params) =>
  params ? `${key}${JSON.stringify(params)}` : key;

describe('toMillis / toNumber', () => {
  it('兼容字符串、Date、bigint、空值与非法值', () => {
    expect(toMillis('2026-09-18T02:00:00.000Z')).toBe(
      Date.parse('2026-09-18T02:00:00.000Z')
    );
    expect(toMillis(new Date(5))).toBe(5);
    expect(toMillis(null)).toBeNull();
    expect(toMillis('not a date')).toBeNull();
    expect(toNumber(3)).toBe(3);
    expect(toNumber(BigInt(4))).toBe(4);
    expect(toNumber(undefined)).toBeNull();
  });
});

describe('latestAttempts', () => {
  it('取 attempt 最大的一次', () => {
    const latest = latestAttempts([
      makeStage({ id: 'a', stage_key: 'develop', attempt: 1 }),
      makeStage({ id: 'b', stage_key: 'develop', attempt: 2 }),
    ]);
    expect(latest.get('develop')?.id).toBe('b');
  });

  it('attempt 相同取 started_at 更晚的', () => {
    const latest = latestAttempts([
      makeStage({ id: 'a', stage_key: 'spec', attempt: 1, started_at: at(5) }),
      makeStage({ id: 'b', stage_key: 'spec', attempt: 1, started_at: at(3) }),
    ]);
    expect(latest.get('spec')?.id).toBe('a');
  });
});

describe('pipelineProgress', () => {
  it('刚启动：第一格蓝，其余未开始', () => {
    const run = makeRun();
    expect(states(pipelineProgress([makeStage()], null, run))).toEqual([
      'running',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('停在规格确认：第一格绿、第二格橙', () => {
    const run = makeRun({ status: 'waiting_gate', current_stage_key: 'spec' });
    const stages = [
      makeStage({ id: 's1', status: 'passed', finished_at: at(1) }),
      makeStage({ id: 's2', stage_key: 'spec', status: 'waiting_gate' }),
    ];
    expect(states(pipelineProgress(stages, STANDARD_TEMPLATE, run))).toEqual([
      'done',
      'gate',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('测试归因代码缺陷回到开发：后面的评审、测试重新算未开始', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const stages = [
      makeStage({
        id: 'd1',
        stage_key: 'develop',
        attempt: 1,
        status: 'passed',
      }),
      makeStage({
        id: 'r1',
        stage_key: 'review',
        attempt: 1,
        status: 'passed',
      }),
      makeStage({ id: 't1', stage_key: 'test', attempt: 1, status: 'failed' }),
      makeStage({
        id: 'd2',
        stage_key: 'develop',
        attempt: 2,
        status: 'running',
      }),
    ];
    expect(states(pipelineProgress(stages, null, run))).toEqual([
      'done',
      'done',
      'done',
      'running',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('评审轮次用尽：第五格红', () => {
    const run = makeRun({ status: 'failed', current_stage_key: 'review' });
    expect(states(pipelineProgress([], null, run))[4]).toBe('failed');
  });

  it('完成：七格全绿', () => {
    const run = makeRun({ status: 'completed', current_stage_key: 'deliver' });
    expect(new Set(states(pipelineProgress([], null, run)))).toEqual(
      new Set(['done'])
    );
  });

  it('暂停与取消各有自己的格子状态', () => {
    const paused = makeRun({ status: 'paused', current_stage_key: 'develop' });
    const cancelled = makeRun({
      status: 'cancelled',
      current_stage_key: 'test',
    });
    expect(states(pipelineProgress([], null, paused))[3]).toBe('paused');
    expect(states(pipelineProgress([], null, cancelled))[5]).toBe('cancelled');
  });

  it('运行中但当前阶段最新尝试在等关卡：显示橙', () => {
    const run = makeRun({ status: 'running' });
    const stages = [makeStage({ status: 'waiting_gate' })];
    expect(states(pipelineProgress(stages, null, run))[0]).toBe('gate');
  });

  it('只看本运行的阶段记录', () => {
    const run = makeRun({ id: 'run-2' });
    const other = makeStage({ run_id: 'run-1', status: 'waiting_gate' });
    expect(states(pipelineProgress([other], null, run))[0]).toBe('running');
  });

  it('模板顺序优先于默认顺序', () => {
    const template = {
      ...STANDARD_TEMPLATE,
      stages: STANDARD_TEMPLATE.stages.slice(0, 2),
    };
    const cells = pipelineProgress([], template, makeRun());
    expect(cells.map((cell) => cell.key)).toEqual(['requirement', 'spec']);
  });
});

describe('pipelineStatus + pipelineStatusText', () => {
  it('等人工：用模板里的关卡名', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = pipelineStatus(
      [makeStage({ status: 'waiting_gate' })],
      STANDARD_TEMPLATE,
      run
    );
    expect(info.tone).toBe('gate');
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.waitingGate{"gate":"需求确认"}'
    );
  });

  it('等人工但没有模板：回落到内置关卡名 key', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = pipelineStatus([], null, run);
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.waitingGate{"gate":"pipeline.gateLabel.requirement"}'
    );
  });

  it('自动阶段第 2 轮：显示轮次', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const info = pipelineStatus(
      [makeStage({ stage_key: 'develop', attempt: 2, gate_kind: 'auto' })],
      STANDARD_TEMPLATE,
      run
    );
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.runningRound{"stage":"pipeline.stage.develop","attempt":2,"max":3}'
    );
  });

  it('人工阶段被打回后重跑不显示轮次（打回不计入失败轮次）', () => {
    const info = pipelineStatus(
      [makeStage({ attempt: 2 })],
      STANDARD_TEMPLATE,
      makeRun()
    );
    expect(info.maxRounds).toBeNull();
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.running{"stage":"pipeline.stage.requirement"}'
    );
  });

  it('第 1 轮不显示轮次', () => {
    const info = pipelineStatus([makeStage()], STANDARD_TEMPLATE, makeRun());
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.running{"stage":"pipeline.stage.requirement"}'
    );
  });

  it('失败、暂停、完成、取消各用自己的 key', () => {
    const text = (status: string) =>
      pipelineStatusText(pipelineStatus([], null, makeRun({ status })), t);
    expect(text('failed')).toBe('pipeline.status.failed');
    expect(text('paused')).toBe('pipeline.status.paused');
    expect(text('completed')).toBe('pipeline.status.completed');
    expect(text('cancelled')).toBe('pipeline.status.cancelled');
  });
});

describe('用户手动停止（契约 C5：尝试 failed、运行 paused）', () => {
  it('当前格显示暂停而不是失败，文案是已暂停', () => {
    const run = makeRun({ status: 'paused', current_stage_key: 'develop' });
    const stages = [
      makeStage({
        stage_key: 'develop',
        gate_kind: 'auto',
        status: 'failed',
        error: '用户手动停止',
      }),
    ];
    expect(states(pipelineProgress(stages, STANDARD_TEMPLATE, run))[3]).toBe(
      'paused'
    );
    const info = pipelineStatus(stages, STANDARD_TEMPLATE, run);
    expect(info.tone).toBe('paused');
    expect(pipelineStatusText(info, t)).toBe('pipeline.status.paused');
  });
});
