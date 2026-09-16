import { describe, expect, it } from 'vitest';
import {
  columnEmptyStateKind,
  DEFAULT_WIP_LIMIT,
  wipState,
} from './columnState';

describe('DEFAULT_WIP_LIMIT', () => {
  it('默认上限是 5（设计文档 §7.2）', () => {
    expect(DEFAULT_WIP_LIMIT).toBe(5);
  });
});

describe('wipState（列头 WIP 提示）', () => {
  it('低于上限是 ok', () => {
    expect(wipState(3, 5)).toBe('ok');
  });

  it('恰好等于上限仍是 ok（「超过」才提示）', () => {
    expect(wipState(5, 5)).toBe('ok');
  });

  it('超过上限是 over', () => {
    expect(wipState(6, 5)).toBe('over');
  });

  it('不传上限时用 DEFAULT_WIP_LIMIT', () => {
    expect(wipState(5)).toBe('ok');
    expect(wipState(6)).toBe('over');
  });

  it('上限为 0 或负数时关闭提示', () => {
    expect(wipState(100, 0)).toBe('ok');
    expect(wipState(100, -1)).toBe('ok');
  });

  it('非有限数不抛错，一律回落成 ok', () => {
    expect(wipState(Number.NaN, 5)).toBe('ok');
    expect(wipState(6, Number.NaN)).toBe('ok');
    expect(wipState(Number.POSITIVE_INFINITY, 5)).toBe('ok');
  });

  it('空列是 ok', () => {
    expect(wipState(0, 5)).toBe('ok');
  });
});

describe('columnEmptyStateKind（空列引导）', () => {
  it('列里有卡片时不显示空态', () => {
    expect(columnEmptyStateKind(1, false)).toBe('none');
    expect(columnEmptyStateKind(1, true)).toBe('none');
  });

  it('列为空且无筛选 → 新建引导', () => {
    expect(columnEmptyStateKind(0, false)).toBe('empty');
  });

  it('列为空且有筛选 → 筛选无结果', () => {
    expect(columnEmptyStateKind(0, true)).toBe('filtered');
  });

  it('非有限数当成「有卡片」，绝不误显示空态', () => {
    expect(columnEmptyStateKind(Number.NaN, false)).toBe('none');
  });
});
