import { cn } from '../lib/cn';

export interface IssuePanelTabBarItem<TTab extends string = string> {
  id: TTab;
  label: string;
}

export interface IssuePanelTabBarProps<TTab extends string = string> {
  tabs: IssuePanelTabBarItem<TTab>[];
  activeTab: TTab;
  onTabChange: (tab: TTab) => void;
  className?: string;
}

/**
 * 需求详情三段式的标签条（设计文档 §7.3）。
 *
 * 纯展示、受控。只有一个标签页时不渲染（新建模式下只有「概况」）。
 * 选中态用 `brand` 下划线 + `text-high`，未选中 `text-low`，都是设计令牌，
 * 暗色模式自动跟随。
 */
export function IssuePanelTabBar<TTab extends string = string>({
  tabs,
  activeTab,
  onTabChange,
  className,
}: IssuePanelTabBarProps<TTab>) {
  if (tabs.length < 2) {
    return null;
  }

  return (
    <div
      role="tablist"
      className={cn(
        'flex shrink-0 items-center gap-base border-b px-base',
        className
      )}
    >
      {tabs.map((tab) => {
        const isActive = tab.id === activeTab;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={isActive}
            onClick={() => onTabChange(tab.id)}
            className={cn(
              'relative -mb-px border-b-2 py-half text-sm transition-colors',
              'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
              isActive
                ? 'border-brand text-high'
                : 'border-transparent text-low hover:text-normal'
            )}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
