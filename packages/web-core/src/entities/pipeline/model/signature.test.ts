import { describe, expect, it } from 'vitest';
import { pipelineSignature } from './signature';
import { makeRun, makeStage } from './__fixtures__/pipeline';

describe('pipelineSignature', () => {
  it('与数组顺序无关', () => {
    const runs = [makeRun({ id: 'a' }), makeRun({ id: 'b' })];
    const stages = [
      makeStage({ id: 's1', run_id: 'a' }),
      makeStage({ id: 's2', run_id: 'b' }),
    ];
    expect(pipelineSignature(runs, stages)).toBe(
      pipelineSignature([...runs].reverse(), [...stages].reverse())
    );
  });

  it('阶段状态或尝试次数变化时指纹变化', () => {
    const runs = [makeRun()];
    const before = pipelineSignature(runs, [makeStage()]);
    expect(pipelineSignature(runs, [makeStage({ status: 'passed' })])).not.toBe(
      before
    );
    expect(pipelineSignature(runs, [makeStage({ attempt: 2 })])).not.toBe(
      before
    );
  });

  it('不属于这些运行的阶段不计入', () => {
    const runs = [makeRun({ id: 'a' })];
    expect(pipelineSignature(runs, [makeStage({ run_id: 'zzz' })])).toBe(
      pipelineSignature(runs, [])
    );
  });

  it('没有运行时是空串', () => {
    expect(pipelineSignature([], [])).toBe('');
  });
});
