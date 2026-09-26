import { describe, expect, it } from 'vitest';
import {
  PIPELINE_STAGE_KEYS,
  autoConditionLabelKey,
  boardStageOf,
  fallbackGateLabelKey,
  isPipelineStageKey,
  pipelineStageLabelKey,
  pipelineStageTone,
} from './stages';

describe('PIPELINE_STAGE_KEYS', () => {
  it('是标准七阶段，顺序固定', () => {
    expect(PIPELINE_STAGE_KEYS).toEqual([
      'requirement',
      'spec',
      'test_design',
      'develop',
      'review',
      'test',
      'deliver',
    ]);
  });
});

describe('boardStageOf（设计文档 §6.4 阶段与看板列映射）', () => {
  it('规格与用例设计合并到同一列', () => {
    expect(boardStageOf('requirement')).toBe('backlog');
    expect(boardStageOf('spec')).toBe('todo');
    expect(boardStageOf('test_design')).toBe('todo');
    expect(boardStageOf('develop')).toBe('dev');
    expect(boardStageOf('review')).toBe('review');
    expect(boardStageOf('test')).toBe('test');
    expect(boardStageOf('deliver')).toBe('done');
  });
});

describe('isPipelineStageKey', () => {
  it('只认七个阶段', () => {
    expect(isPipelineStageKey('develop')).toBe(true);
    expect(isPipelineStageKey('dev')).toBe(false);
    expect(isPipelineStageKey(null)).toBe(false);
  });
});

describe('i18n key', () => {
  it('阶段名 key 是字面量', () => {
    expect(pipelineStageLabelKey('test_design')).toBe(
      'pipeline.stage.test_design'
    );
  });

  it('人工关卡有专名，其它阶段回落到阶段名', () => {
    expect(fallbackGateLabelKey('spec')).toBe('pipeline.gateLabel.spec');
    expect(fallbackGateLabelKey('develop')).toBe('pipeline.stage.develop');
  });

  it('按模板的 gate_condition（契约 C10）给判定条件文案', () => {
    expect(autoConditionLabelKey('checks_passed')).toBe(
      'pipeline.gate.autoCondition.checksPassed'
    );
    expect(autoConditionLabelKey('no_blocking_findings')).toBe(
      'pipeline.gate.autoCondition.noBlockingFindings'
    );
    expect(autoConditionLabelKey('all_cases_passed')).toBe(
      'pipeline.gate.autoCondition.allCasesPassed'
    );
    expect(autoConditionLabelKey('artifacts_present')).toBe(
      'pipeline.gate.autoCondition.artifactsPresent'
    );
  });

  it('人工关卡（null）或不认识的判定名没有文案', () => {
    expect(autoConditionLabelKey(null)).toBeNull();
    expect(autoConditionLabelKey(undefined)).toBeNull();
    expect(autoConditionLabelKey('custom_rule')).toBeNull();
    expect(autoConditionLabelKey('toString')).toBeNull();
  });
});

describe('pipelineStageTone（设计文档 §8.5 阶段语义色）', () => {
  it('灰=需求/规格/用例，蓝=开发，紫=评审，琥珀=测试，绿=交付', () => {
    expect(PIPELINE_STAGE_KEYS.map(pipelineStageTone)).toEqual([
      'neutral',
      'neutral',
      'neutral',
      'dev',
      'review',
      'test',
      'done',
    ]);
  });
});
