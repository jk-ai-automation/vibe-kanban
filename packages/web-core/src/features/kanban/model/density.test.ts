import { describe, expect, it } from 'vitest';
import {
  DEFAULT_KANBAN_DENSITY,
  densityClasses,
  densityLabelKey,
  isKanbanDensity,
  KANBAN_DENSITY_STORAGE_KEY,
  normalizeDensity,
  toggleDensity,
} from './density';

describe('normalizeDensity', () => {
  it('认识两个合法值', () => {
    expect(normalizeDensity('comfortable')).toBe('comfortable');
    expect(normalizeDensity('compact')).toBe('compact');
  });

  it('非法值一律回落成舒适', () => {
    expect(normalizeDensity('cozy')).toBe('comfortable');
    expect(normalizeDensity(null)).toBe('comfortable');
    expect(normalizeDensity(undefined)).toBe('comfortable');
    expect(normalizeDensity(42)).toBe('comfortable');
    expect(normalizeDensity({})).toBe('comfortable');
  });

  it('默认值是舒适', () => {
    expect(DEFAULT_KANBAN_DENSITY).toBe('comfortable');
  });
});

describe('isKanbanDensity', () => {
  it('只认两个字面量', () => {
    expect(isKanbanDensity('comfortable')).toBe(true);
    expect(isKanbanDensity('compact')).toBe(true);
    expect(isKanbanDensity('COMPACT')).toBe(false);
    expect(isKanbanDensity('')).toBe(false);
  });
});

describe('densityClasses', () => {
  it('两套 class 不同', () => {
    const comfortable = densityClasses('comfortable');
    const compact = densityClasses('compact');
    expect(comfortable.card).not.toBe(compact.card);
    expect(comfortable.cards).not.toBe(compact.cards);
  });

  it('紧凑模式收起描述预览', () => {
    expect(densityClasses('compact').showDescription).toBe(false);
    expect(densityClasses('comfortable').showDescription).toBe(true);
  });

  it('非法值拿到舒适那一套', () => {
    expect(densityClasses('nonsense')).toEqual(densityClasses('comfortable'));
  });

  it('只用既有间距令牌，不硬编码像素', () => {
    for (const value of ['comfortable', 'compact']) {
      const classes = densityClasses(value);
      expect(classes.card).toMatch(/^p-(base|half)$/);
      expect(classes.cards).toMatch(/^gap-(base|half)$/);
    }
  });
});

describe('toggleDensity', () => {
  it('两种密度轮换', () => {
    expect(toggleDensity('comfortable')).toBe('compact');
    expect(toggleDensity('compact')).toBe('comfortable');
  });

  it('从非法值切换等价于从默认值切换', () => {
    expect(toggleDensity('nonsense')).toBe('compact');
  });
});

describe('densityLabelKey', () => {
  it('返回字面量 i18n key', () => {
    expect(densityLabelKey('comfortable')).toBe('kanban.density.comfortable');
    expect(densityLabelKey('compact')).toBe('kanban.density.compact');
  });
});

describe('KANBAN_DENSITY_STORAGE_KEY', () => {
  it('跟既有本地存储 key 同前缀', () => {
    expect(KANBAN_DENSITY_STORAGE_KEY).toBe('vk-kanban-density');
  });
});
