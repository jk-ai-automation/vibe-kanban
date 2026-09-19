import { describe, expect, it } from 'vitest';
import {
  buildPipelineCardInfo,
  cellLabelKey,
  groupStagesByRun,
  latestRunByIssue,
} from './cardInfo';
import type { Translate } from './progress';
import { at, makeRun, makeStage } from './__fixtures__/pipeline';

const t: Translate = (key, params) =>
  params ? `${key}${JSON.stringify(params)}` : key;

describe('latestRunByIssue', () => {
  it('同一需求多次运行取最新创建的那次', () => {
    const map = latestRunByIssue([
      makeRun({ id: 'old', created_at: at(0) }),
      makeRun({ id: 'new', created_at: at(10) }),
      makeRun({ id: 'other', issue_id: 'issue-2' }),
    ]);
    expect(map.get('issue-1')?.id).toBe('new');
    expect(map.get('issue-2')?.id).toBe('other');
  });
});

describe('groupStagesByRun', () => {
  it('按 run_id 分组', () => {
    const groups = groupStagesByRun([
      makeStage({ id: 'a', run_id: 'r1' }),
      makeStage({ id: 'b', run_id: 'r2' }),
      makeStage({ id: 'c', run_id: 'r1' }),
    ]);
    expect(groups.get('r1')?.map((s) => s.id)).toEqual(['a', 'c']);
    expect(groups.get('r2')?.map((s) => s.id)).toEqual(['b']);
  });
});

describe('buildPipelineCardInfo', () => {
  it('七格 + 状态文案 + 无障碍标签', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = buildPipelineCardInfo(
      run,
      [makeStage({ status: 'waiting_gate' })],
      t
    );
    expect(info.runId).toBe('run-1');
    expect(info.cells).toHaveLength(7);
    expect(info.cells[0]).toEqual({
      key: 'requirement',
      state: 'gate',
      label: 'pipeline.stage.requirement: pipeline.cell.gate',
    });
    expect(info.tone).toBe('gate');
    expect(info.statusText).toBe(
      'pipeline.status.waitingGate{"gate":"pipeline.gateLabel.requirement"}'
    );
    expect(info.progressLabel.startsWith('pipeline.progressLabel')).toBe(true);
    expect(info.isFailed).toBe(false);
  });

  it('失败时 isFailed 为真（卡片加红边框）', () => {
    const run = makeRun({ status: 'failed', current_stage_key: 'review' });
    expect(buildPipelineCardInfo(run, [], t).isFailed).toBe(true);
  });

  it('格子状态的 key 是字面量', () => {
    expect(cellLabelKey('pending')).toBe('pipeline.cell.pending');
  });
});
