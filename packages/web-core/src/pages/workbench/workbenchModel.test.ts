import { describe, expect, it } from 'vitest';
import {
  buildCreateIssueRequest,
  greetingKey,
  pickBacklogStatusId,
  pickDefaultBranch,
  pickWorkbenchProjectId,
  recentDelivered,
  runningRuns,
  stagesFinishedToday,
  waitingSince,
  weeklyStats,
} from './workbenchModel';
import {
  T0_MS,
  at,
  makeRun,
  makeStage,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('三栏分桶', () => {
  const runs = [
    makeRun({ id: 'a', status: 'running', updated_at: at(1) }),
    makeRun({ id: 'b', status: 'paused', updated_at: at(5) }),
    makeRun({ id: 'c', status: 'waiting_gate' }),
    makeRun({ id: 'd', status: 'completed', finished_at: at(10) }),
    makeRun({ id: 'e', status: 'cancelled', finished_at: at(20) }),
    makeRun({ id: 'f', status: 'failed' }),
  ];

  it('正在自动跑 = running + paused，最近更新在前', () => {
    expect(runningRuns(runs).map((r) => r.id)).toEqual(['b', 'a']);
  });

  it('最近交付 = completed + cancelled，最近结束在前，可限条数', () => {
    expect(recentDelivered(runs).map((r) => r.id)).toEqual(['e', 'd']);
    expect(recentDelivered(runs, 1).map((r) => r.id)).toEqual(['e']);
  });
});

describe('weeklyStats', () => {
  const now = T0_MS + 60 * 60_000;

  it('只算最近 7 天完成的；平均周期按分钟取整；一次通过率', () => {
    const runs = [
      makeRun({
        id: 'r1',
        status: 'completed',
        created_at: at(0),
        finished_at: at(40),
      }),
      makeRun({
        id: 'r2',
        status: 'completed',
        created_at: at(0),
        finished_at: at(60),
      }),
      makeRun({
        id: 'old',
        status: 'completed',
        created_at: at(-20_000),
        finished_at: at(-19_000),
      }),
    ];
    const stages = [
      makeStage({ run_id: 'r1', attempt: 1, status: 'passed' }),
      makeStage({
        id: 's2',
        run_id: 'r2',
        stage_key: 'review',
        attempt: 2,
        status: 'passed',
      }),
    ];
    expect(weeklyStats(runs, stages, now)).toEqual({
      delivered: 2,
      avgCycleMinutes: 50,
      firstPassRate: 50,
    });
  });

  it('本周没有交付', () => {
    expect(weeklyStats([], [], now)).toEqual({
      delivered: 0,
      avgCycleMinutes: null,
      firstPassRate: null,
    });
  });
});

describe('stagesFinishedToday', () => {
  it('只数今天（本地日期）通过的阶段', () => {
    const now = new Date(2026, 8, 18, 15, 0).getTime();
    const today = new Date(2026, 8, 18, 9, 0).toISOString();
    const yesterday = new Date(2026, 8, 17, 23, 0).toISOString();
    const stages = [
      makeStage({ id: 'a', status: 'passed', finished_at: today }),
      makeStage({ id: 'b', status: 'passed', finished_at: yesterday }),
      makeStage({ id: 'c', status: 'failed', finished_at: today }),
    ];
    expect(stagesFinishedToday(stages, now)).toBe(1);
  });
});

describe('greetingKey', () => {
  it('上午 / 下午 / 晚上', () => {
    expect(greetingKey(9)).toBe('workbench.greeting.morning');
    expect(greetingKey(14)).toBe('workbench.greeting.afternoon');
    expect(greetingKey(20)).toBe('workbench.greeting.evening');
  });
});

describe('建需求', () => {
  it('优先挑 stage_type = backlog 的列', () => {
    const statuses = [
      { id: 'todo', sort_order: 0, stage_type: 'todo' },
      { id: 'backlog', sort_order: 1, stage_type: 'backlog' },
    ];
    expect(pickBacklogStatusId(statuses)).toBe('backlog');
  });

  it('没有 stage_type 时取排序第一列；没有列返回 null', () => {
    expect(
      pickBacklogStatusId([
        { id: 'b', sort_order: 2 },
        { id: 'a', sort_order: 1 },
      ])
    ).toBe('a');
    expect(pickBacklogStatusId([])).toBeNull();
  });

  it('请求体：第一行做标题，其余做描述，id 用传入的', () => {
    expect(
      buildCreateIssueRequest({
        id: 'i1',
        projectId: 'p1',
        statusId: 's1',
        prompt: '  下单最小金额校验\n低于 5 USDT 返回 400  ',
      })
    ).toEqual({
      id: 'i1',
      project_id: 'p1',
      status_id: 's1',
      title: '下单最小金额校验',
      description: '低于 5 USDT 返回 400',
      priority: null,
      start_date: null,
      target_date: null,
      completed_at: null,
      sort_order: 0,
      parent_issue_id: null,
      parent_issue_sort_order: null,
      extension_metadata: {},
    });
  });
});

describe('pickDefaultBranch', () => {
  const branches = [
    { name: 'origin/main', is_current: false, is_remote: true },
    { name: 'dev', is_current: true, is_remote: false },
    { name: 'main', is_current: false, is_remote: false },
  ];

  it('仓库设置了默认目标分支且存在：用它', () => {
    expect(pickDefaultBranch({ default_target_branch: 'main' }, branches)).toBe(
      'main'
    );
  });

  it('否则用当前分支；再否则第一个本地分支；都没有返回 null', () => {
    expect(pickDefaultBranch({ default_target_branch: 'nope' }, branches)).toBe(
      'dev'
    );
    expect(
      pickDefaultBranch({ default_target_branch: null }, [
        { name: 'x', is_current: false, is_remote: false },
      ])
    ).toBe('x');
    expect(pickDefaultBranch({ default_target_branch: null }, [])).toBeNull();
  });
});

describe('pickWorkbenchProjectId', () => {
  it('选中的项目还在就用它，否则第一个', () => {
    const projects = [{ id: 'a' }, { id: 'b' }];
    expect(pickWorkbenchProjectId(projects, 'b')).toBe('b');
    expect(pickWorkbenchProjectId(projects, 'gone')).toBe('a');
    expect(pickWorkbenchProjectId([], null)).toBeNull();
  });
});

describe('waitingSince（契约 C11：等待起点 = 阶段 finished_at）', () => {
  it('等人工 / 失败的阶段用 finished_at', () => {
    expect(
      waitingSince(
        makeStage({
          status: 'waiting_gate',
          started_at: at(0),
          finished_at: at(4),
        })
      )
    ).toBe(Date.parse(at(4)));
  });

  it('老数据没有 finished_at 时退回开始时间；都没有为 null', () => {
    expect(
      waitingSince(makeStage({ started_at: at(1), finished_at: null }))
    ).toBe(Date.parse(at(1)));
    expect(
      waitingSince(makeStage({ started_at: null, finished_at: 'bad' }))
    ).toBeNull();
  });
});
