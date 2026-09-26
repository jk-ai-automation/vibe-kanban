import { describe, expect, it } from 'vitest';
import {
  MANUAL_STOP_ERROR,
  firstErrorLine,
  isManualStop,
  latestAttempts,
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  stageRound,
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

  it('自动阶段第 2 轮（此前有 1 次失败尝试，轮次 = 失败数 + 1，同后端 count_failed）', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const info = pipelineStatus(
      [
        makeStage({
          id: 'd1',
          stage_key: 'develop',
          attempt: 1,
          gate_kind: 'auto',
          status: 'failed',
        }),
        makeStage({
          id: 'd2',
          stage_key: 'develop',
          attempt: 2,
          gate_kind: 'auto',
        }),
      ],
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

describe('轮次不计「用户手动停止」（后端 MANUAL_STOP_ERROR 不计入 max_rounds）', () => {
  const develop = (attempt: number, extra: Record<string, unknown> = {}) =>
    makeStage({
      id: `d${attempt}`,
      stage_key: 'develop',
      gate_kind: 'auto',
      attempt,
      status: 'failed',
      ...extra,
    });

  it('常量与后端逐字一致', () => {
    expect(MANUAL_STOP_ERROR).toBe('用户手动停止');
  });

  it('第 2 次尝试是手动停止后的续跑：仍是第 1 轮，不显示轮次', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const stages = [
      develop(1, { error: MANUAL_STOP_ERROR }),
      develop(2, { status: 'running' }),
    ];
    const info = pipelineStatus(stages, STANDARD_TEMPLATE, run);
    expect(info.attempt).toBe(1);
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.running{"stage":"pipeline.stage.develop"}'
    );
  });

  it('失败一次 + 手动停止一次后的第 3 次尝试：第 2 轮', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const stages = [
      develop(1, { error: '进程退出码 1' }),
      develop(2, { error: MANUAL_STOP_ERROR }),
      develop(3, { status: 'running' }),
    ];
    expect(stageRound(stages, stages[2])).toBe(2);
    expect(pipelineStatus(stages, STANDARD_TEMPLATE, run).attempt).toBe(2);
  });

  it('只数本运行、本阶段、更早的失败尝试（手动停止不计）', () => {
    const current = develop(3, { status: 'running' });
    const stages = [
      develop(1, { run_id: 'other', error: MANUAL_STOP_ERROR }),
      makeStage({
        id: 'r1',
        stage_key: 'review',
        attempt: 1,
        status: 'failed',
        error: MANUAL_STOP_ERROR,
      }),
      current,
    ];
    expect(stageRound(stages, current)).toBe(1);
    expect(isManualStop(develop(1, { error: MANUAL_STOP_ERROR }))).toBe(true);
    expect(isManualStop(develop(1, { error: '别的原因' }))).toBe(false);
  });
});

describe('stageRound 与后端 count_failed 对齐', () => {
  const develop = (attempt: number, extra: Record<string, unknown> = {}) =>
    makeStage({
      id: `d${attempt}`,
      stage_key: 'develop',
      gate_kind: 'auto',
      attempt,
      status: 'failed',
      ...extra,
    });

  it('测试回流后开发重跑：之前开发尝试是 passed，不算失败，仍是第 1 轮', () => {
    const stages = [
      develop(1, { status: 'passed' }),
      develop(2, { status: 'running' }),
    ];
    expect(stageRound(stages, stages[1])).toBe(1);
  });

  it('被打回（rejected）不计入轮次', () => {
    const stages = [
      develop(1, { status: 'rejected' }),
      develop(2, { status: 'running' }),
    ];
    expect(stageRound(stages, stages[1])).toBe(1);
  });

  it('当前尝试本身失败：按计入后的失败数显示', () => {
    const stages = [develop(1), develop(2), develop(3)];
    expect(stageRound(stages, stages[2])).toBe(3);
  });
});

describe('firstErrorLine：一行的位置只放失败原因第一行', () => {
  it('多行失败原因取第一行（已知接口报错时就是那句中文说明）', () => {
    expect(
      firstErrorLine(
        'Claude Code 版本过旧，不支持当前模型：请升级平台钉住的命令行版本。\n' +
          '编码智能体 进程未成功结束（状态：失败，退出码 1）\n' +
          '编码智能体输出末尾（最多 50 行 / 4096 字节）：\n' +
          'API Error: 400 {"error":{"details":{"error_code":"claude_code_version_too_old"}}}'
      )
    ).toBe(
      'Claude Code 版本过旧，不支持当前模型：请升级平台钉住的命令行版本。'
    );
  });

  it('单行原样返回，空与 null 都当没有', () => {
    expect(firstErrorLine('用户手动停止')).toBe('用户手动停止');
    expect(firstErrorLine(null)).toBeNull();
    expect(firstErrorLine('')).toBeNull();
    expect(firstErrorLine('\n后面才有内容')).toBeNull();
  });
});
