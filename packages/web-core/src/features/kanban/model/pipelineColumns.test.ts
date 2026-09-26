import { describe, expect, it } from 'vitest';
import {
  PIPELINE_STAGE_KEYS,
  boardStageOf,
} from '@/entities/pipeline/model/stages';
import {
  LEGACY_DEFAULT_STATUS_NAMES,
  pipelineColumnEmptyKey,
  pipelineColumnHintKey,
  pipelineColumnTitleKey,
} from './pipelineColumns';
import { STAGE_ORDER, isStageType } from './stageType';

describe('entities 与 features 的阶段类型一致', () => {
  it('流水线阶段映射出的列都是看板认识的阶段', () => {
    for (const key of PIPELINE_STAGE_KEYS) {
      expect(isStageType(boardStageOf(key))).toBe(true);
    }
  });

  it('默认列名表覆盖全部六个看板阶段', () => {
    expect(Object.keys(LEGACY_DEFAULT_STATUS_NAMES).sort()).toEqual(
      Object.keys(STAGE_ORDER).sort()
    );
  });
});

describe('pipelineColumnTitleKey', () => {
  it('没改过名的默认列显示流水线列名', () => {
    expect(pipelineColumnTitleKey('待规划', 'backlog')).toBe(
      'pipeline.column.backlog'
    );
    expect(pipelineColumnTitleKey(' 待开发 ', 'todo')).toBe(
      'pipeline.column.todo'
    );
    expect(pipelineColumnTitleKey('已完成', 'done')).toBe(
      'pipeline.column.done'
    );
  });

  it('用户改过名就返回 null（显示用户的名字）', () => {
    expect(pipelineColumnTitleKey('等上线', 'done')).toBeNull();
  });
});

describe('列提示与空列说明', () => {
  it('前两列是人工确认，交付列是自动，其它没有提示', () => {
    expect(pipelineColumnHintKey('backlog')).toBe('pipeline.column.humanGate');
    expect(pipelineColumnHintKey('todo')).toBe('pipeline.column.humanGate');
    expect(pipelineColumnHintKey('done')).toBe('pipeline.column.auto');
    expect(pipelineColumnHintKey('dev')).toBeNull();
  });

  it('每列都有「为什么空」', () => {
    expect(pipelineColumnEmptyKey('review')).toBe(
      'pipeline.columnEmpty.review'
    );
    expect(pipelineColumnEmptyKey('backlog')).toBe(
      'pipeline.columnEmpty.backlog'
    );
  });
});
