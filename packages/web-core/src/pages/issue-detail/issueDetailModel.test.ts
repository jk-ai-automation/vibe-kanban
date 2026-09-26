import { describe, expect, it } from 'vitest';
import {
  ISSUE_DETAIL_TABS,
  artifactKindLabelKey,
  artifactRenderMode,
  defaultIssueDetailTab,
  gateBarState,
  isIssueDetailTab,
  issueDetailTabLabelKey,
  latestArtifactOfKind,
  latestArtifactsByKind,
  tabArtifactKinds,
  tabOfArtifactKind,
} from './issueDetailModel';
import {
  at,
  makeArtifact,
  makeRun,
  makeStage,
  makeView,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('页签', () => {
  it('六个页签，顺序固定', () => {
    expect(ISSUE_DETAIL_TABS).toEqual([
      'requirement',
      'spec',
      'cases',
      'code',
      'test',
      'delivery',
    ]);
    expect(issueDetailTabLabelKey('cases')).toBe('issueDetail.tab.cases');
    expect(isIssueDetailTab('code')).toBe(true);
    expect(isIssueDetailTab('overview')).toBe(false);
  });

  it('页签与产出物种类互相对应', () => {
    expect(tabArtifactKinds('spec')).toEqual(['spec', 'plan']);
    expect(tabArtifactKinds('cases')).toEqual(['test_cases', 'trace_matrix']);
    expect(tabOfArtifactKind('plan')).toBe('spec');
    expect(tabOfArtifactKind('review')).toBe('code');
    expect(tabOfArtifactKind('delivery_report')).toBe('delivery');
  });

  it('默认页签跟着当前阶段走', () => {
    expect(defaultIssueDetailTab(null)).toBe('requirement');
    expect(defaultIssueDetailTab('test_design')).toBe('cases');
    expect(defaultIssueDetailTab('review')).toBe('code');
    expect(defaultIssueDetailTab('deliver')).toBe('delivery');
  });
});

describe('产出物', () => {
  it('同一种类取版本最大的', () => {
    const artifacts = [
      makeArtifact({ id: 'v1', kind: 'spec', version: 1 }),
      makeArtifact({ id: 'v2', kind: 'spec', version: 2 }),
      makeArtifact({ id: 'req', kind: 'requirement' }),
    ];
    expect(latestArtifactOfKind(artifacts, 'spec')?.id).toBe('v2');
    expect(latestArtifactOfKind(artifacts, 'review')).toBeNull();
    expect(latestArtifactsByKind(artifacts).map((a) => a.id)).toEqual([
      'req',
      'v2',
    ]);
  });

  it('渲染方式', () => {
    expect(artifactRenderMode('test_cases')).toBe('csv');
    expect(artifactRenderMode('review')).toBe('review');
    expect(artifactRenderMode('test_report')).toBe('test_report');
    expect(artifactRenderMode('trace_matrix')).toBe('markdown');
    expect(artifactKindLabelKey('trace_matrix')).toBe(
      'issueDetail.artifactKind.trace_matrix'
    );
  });
});

describe('gateBarState', () => {
  it('没有流水线', () => {
    expect(gateBarState(null)).toEqual({ kind: 'none' });
  });

  it('人工关卡等待中', () => {
    const view = makeView({
      run: makeRun({ status: 'waiting_gate', current_stage_key: 'spec' }),
      stages: [
        makeStage({
          id: 'sp',
          stage_key: 'spec',
          status: 'waiting_gate',
          gate_kind: 'human',
        }),
      ],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'human',
      runId: 'run-1',
      stageRunId: 'sp',
      stageKey: 'spec',
      gateLabel: '设计规格确认',
    });
  });

  it('人工阶段还在跑（AI 在写需求）', () => {
    expect(gateBarState(makeView())).toEqual({
      kind: 'running',
      runId: 'run-1',
      stageKey: 'requirement',
    });
  });

  it('自动阶段：带轮次（此前 1 次失败 → 第 2 轮，同后端 count_failed）', () => {
    const view = makeView({
      run: makeRun({ current_stage_key: 'develop' }),
      stages: [
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
    });
    expect(gateBarState(view)).toEqual({
      kind: 'auto',
      runId: 'run-1',
      stageKey: 'develop',
      attempt: 2,
      maxRounds: 3,
      gateCondition: 'checks_passed',
    });
  });

  it('暂停 / 失败 / 完成 / 取消', () => {
    const kind = (status: string) =>
      gateBarState(
        makeView({ run: makeRun({ status, current_stage_key: 'review' }) })
      ).kind;
    expect(kind('paused')).toBe('paused');
    expect(kind('failed')).toBe('failed');
    expect(kind('completed')).toBe('completed');
    expect(kind('cancelled')).toBe('cancelled');
  });

  it('失败时带上轮次，便于显示「第 3/3 轮」（三次失败尝试都计入，同后端 count_failed）', () => {
    const view = makeView({
      run: makeRun({
        status: 'failed',
        current_stage_key: 'review',
        updated_at: at(9),
      }),
      stages: [1, 2, 3].map((attempt) =>
        makeStage({
          id: `rv${attempt}`,
          stage_key: 'review',
          attempt,
          status: 'failed',
          gate_kind: 'auto',
        })
      ),
    });
    expect(gateBarState(view)).toEqual({
      kind: 'failed',
      runId: 'run-1',
      stageKey: 'review',
      attempt: 3,
      maxRounds: 3,
      error: null,
    });
  });

  it('失败时带上当前阶段最新尝试的失败原因，关卡条才看得到真实原因', () => {
    const view = makeView({
      run: makeRun({
        status: 'failed',
        current_stage_key: 'develop',
        updated_at: at(9),
      }),
      stages: [
        makeStage({
          id: 'dv1',
          stage_key: 'develop',
          attempt: 1,
          status: 'failed',
          error: '旧一轮的原因',
        }),
        makeStage({
          id: 'dv2',
          stage_key: 'develop',
          attempt: 2,
          status: 'failed',
          error:
            'Claude Code 版本过旧，不支持当前模型\n编码智能体输出末尾：\nAPI Error: 400',
        }),
      ],
    });
    const state = gateBarState(view);
    expect(state.kind === 'failed' && state.error).toBe(
      'Claude Code 版本过旧，不支持当前模型\n编码智能体输出末尾：\nAPI Error: 400'
    );
  });
});

describe('gateBarState：暂停中的关卡（契约 C12）', () => {
  it('暂停时当前阶段在等人工确认：带上待决关卡，决策照常可做', () => {
    const view = makeView({
      run: makeRun({ status: 'paused', current_stage_key: 'spec' }),
      stages: [
        makeStage({
          id: 'sp',
          stage_key: 'spec',
          status: 'waiting_gate',
          gate_kind: 'human',
        }),
      ],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'paused',
      runId: 'run-1',
      stageKey: 'spec',
      pendingGate: { stageRunId: 'sp', gateLabel: '设计规格确认' },
    });
  });

  it('用户手动停止后的暂停：没有待决关卡', () => {
    const view = makeView({
      run: makeRun({ status: 'paused', current_stage_key: 'develop' }),
      stages: [
        makeStage({
          stage_key: 'develop',
          status: 'failed',
          gate_kind: 'auto',
          error: '用户手动停止',
        }),
      ],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'paused',
      runId: 'run-1',
      stageKey: 'develop',
      pendingGate: null,
    });
  });
});

describe('gateBarState：模板里的判定条件（契约 C10）', () => {
  it('自动阶段带上模板的 gate_condition，而不是按阶段 key 猜', () => {
    const template = {
      ...makeView().template,
      stages: makeView().template.stages.map((stage) =>
        stage.key === 'review'
          ? { ...stage, gate_condition: 'checks_passed' }
          : stage
      ),
    };
    const view = makeView({
      run: makeRun({ current_stage_key: 'review' }),
      stages: [makeStage({ stage_key: 'review', gate_kind: 'auto' })],
      template,
    });
    const state = gateBarState(view);
    expect(state.kind === 'auto' && state.gateCondition).toBe('checks_passed');
  });
});

describe('gateBarState：轮次不计「用户手动停止」', () => {
  it('轮次用尽失败时，之前的手动停止不占轮次', () => {
    const view = makeView({
      run: makeRun({ status: 'failed', current_stage_key: 'review' }),
      stages: [1, 2, 3, 4].map((attempt) =>
        makeStage({
          id: `r${attempt}`,
          stage_key: 'review',
          attempt,
          status: 'failed',
          gate_kind: 'auto',
          error: attempt === 2 ? '用户手动停止' : 'blocker',
        })
      ),
    });
    const state = gateBarState(view);
    expect(state.kind === 'failed' && state.attempt).toBe(3);
  });

  it('自动阶段续跑：扣掉手动停止', () => {
    const view = makeView({
      run: makeRun({ current_stage_key: 'develop' }),
      stages: [
        makeStage({
          id: 'd1',
          stage_key: 'develop',
          attempt: 1,
          status: 'failed',
          gate_kind: 'auto',
          error: '用户手动停止',
        }),
        makeStage({
          id: 'd2',
          stage_key: 'develop',
          attempt: 2,
          gate_kind: 'auto',
        }),
      ],
    });
    const state = gateBarState(view);
    expect(state.kind === 'auto' && state.attempt).toBe(1);
  });
});
