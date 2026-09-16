/**
 * 看板列表密度（设计文档 §7.5：列表密度切换（舒适/紧凑）记在本地存储）。
 *
 * **刻意不进 `KanbanFilterState`**：密度是「看怎么显示」，不是筛选条件。
 * 塞进 `filtersToSearch` / `searchToFilters` 会破坏那对函数的往返一致性测试，
 * 也会让分享出去的链接带上跟内容无关的参数。
 * 这里也不写 URL，只走 localStorage（跟既有的移动端字号 `mobileFontScale` 同一套路）。
 */
export type KanbanDensity = 'comfortable' | 'compact';

export const DEFAULT_KANBAN_DENSITY: KanbanDensity = 'comfortable';

/** localStorage key。跟 `vk-mobile-font-scale` 同前缀。 */
export const KANBAN_DENSITY_STORAGE_KEY = 'vk-kanban-density';

export function isKanbanDensity(value: unknown): value is KanbanDensity {
  return value === 'comfortable' || value === 'compact';
}

/** 非法值（旧版本残留、用户手改 localStorage）一律回落成舒适，绝不抛错。 */
export function normalizeDensity(value: unknown): KanbanDensity {
  return isKanbanDensity(value) ? value : DEFAULT_KANBAN_DENSITY;
}

/**
 * 密度对应的 Tailwind class。
 *
 * 只用既有设计令牌的间距（`p-base` = 12px / `p-half` = 6px），
 * 不硬编码像素，暗色模式无关。
 */
export type DensityClasses = {
  /** 列内卡片之间的间距容器。 */
  cards: string;
  /** 单张卡片的内边距。 */
  card: string;
  /** 描述预览的行数上限（紧凑模式下收起）。 */
  showDescription: boolean;
};

const DENSITY_CLASSES: Record<KanbanDensity, DensityClasses> = {
  comfortable: {
    cards: 'gap-base',
    card: 'p-base',
    showDescription: true,
  },
  compact: {
    cards: 'gap-half',
    card: 'p-half',
    showDescription: false,
  },
};

export function densityClasses(value: unknown): DensityClasses {
  return DENSITY_CLASSES[normalizeDensity(value)];
}

/** 两种密度轮换，供工具栏上的单个按钮用。 */
export function toggleDensity(value: unknown): KanbanDensity {
  return normalizeDensity(value) === 'comfortable' ? 'compact' : 'comfortable';
}

const DENSITY_LABEL_KEYS: Record<KanbanDensity, string> = {
  comfortable: 'kanban.density.comfortable',
  compact: 'kanban.density.compact',
};

/** 密度的 i18n key（common 命名空间）。字面量映射表，方便未引用 key 检查脚本抓到。 */
export function densityLabelKey(value: unknown): string {
  return DENSITY_LABEL_KEYS[normalizeDensity(value)];
}
