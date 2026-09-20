import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  BookOpenIcon,
  CaretDownIcon,
  FlaskIcon,
  GearIcon,
  HouseIcon,
  KanbanIcon,
  PlusIcon,
  type Icon,
} from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './Dropdown';

/** 与 `web-core/src/shared/lib/routes/personalRoutes.ts` 的同名类型逐字一致。 */
export type PersonalNavKey =
  | 'workbench'
  | 'pipeline'
  | 'testing'
  | 'docs'
  | 'settings';

export interface PersonalSidebarProject {
  id: string;
  name: string;
  /** HSL 三元组字符串，与 `projects.color` 一致。 */
  color: string;
}

export interface PersonalSidebarProps {
  projects: PersonalSidebarProject[];
  activeProjectId: string | null;
  onProjectSelect: (projectId: string) => void;
  onCreateProject: () => void;
  activeKey: PersonalNavKey | null;
  onNavigate: (key: PersonalNavKey) => void;
  /** 「工作台」旁的待处理数（等人工 + 失败）。 */
  pendingCount: number;
  appVersion: string | null;
  /** 用户菜单等底部附加内容。 */
  userSlot?: ReactNode;
  /**
   * 根节点的 `data-testid`。移动端抽屉里的那份要传一个不同的值：
   * 抽屉内容即使收起也留在 DOM 里（`MobileDrawer` 始终挂载 children），
   * 两份用同一个 testid 会让端到端定位器命中两个元素。
   */
  testId?: string;
  className?: string;
}

const NAV_ITEMS: { key: PersonalNavKey; labelKey: string; icon: Icon }[] = [
  { key: 'workbench', labelKey: 'nav.workbench', icon: HouseIcon },
  { key: 'pipeline', labelKey: 'nav.pipeline', icon: KanbanIcon },
  { key: 'testing', labelKey: 'nav.testing', icon: FlaskIcon },
  { key: 'docs', labelKey: 'nav.docs', icon: BookOpenIcon },
  { key: 'settings', labelKey: 'nav.settings', icon: GearIcon },
];

/**
 * 个人版左侧栏（设计文档 §8.1 + 草图①左栏）：顶部项目下拉（颜色块 + 全名），
 * 五个中文入口，底部版本号。无状态：数据与回调全部走 props。
 *
 * 选中态用 `bg-primary text-high`，**不用品牌橙**——设计文档 §8.5：
 * 橙只用于主操作与「等你处理」（这里的待处理计数）。
 */
export function PersonalSidebar({
  projects,
  activeProjectId,
  onProjectSelect,
  onCreateProject,
  activeKey,
  onNavigate,
  pendingCount,
  appVersion,
  userSlot,
  testId = 'personal-sidebar',
  className,
}: PersonalSidebarProps) {
  const { t } = useTranslation('common');
  const activeProject =
    projects.find((project) => project.id === activeProjectId) ?? null;

  return (
    <div
      data-testid={testId}
      className={cn(
        'flex h-full min-h-0 w-52 flex-col gap-base overflow-y-auto',
        'border-r border-border bg-secondary p-base',
        className
      )}
    >
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={t('nav.projectSwitcher')}
            className={cn(
              'flex w-full items-center gap-half rounded-sm px-half py-half',
              'text-left text-sm text-high hover:bg-primary',
              'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand'
            )}
          >
            <span
              aria-hidden="true"
              className="size-3 shrink-0 rounded-sm bg-panel"
              style={
                activeProject
                  ? { backgroundColor: `hsl(${activeProject.color})` }
                  : undefined
              }
            />
            <span className="min-w-0 flex-1 truncate">
              {activeProject?.name ?? t('nav.noProject')}
            </span>
            <CaretDownIcon
              className="size-icon-xs shrink-0 text-low"
              weight="bold"
            />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {projects.map((project) => (
            <DropdownMenuItem
              key={project.id}
              onSelect={() => onProjectSelect(project.id)}
            >
              <span className="flex min-w-0 items-center gap-half">
                <span
                  aria-hidden="true"
                  className="size-3 shrink-0 rounded-sm"
                  style={{ backgroundColor: `hsl(${project.color})` }}
                />
                <span className="truncate">{project.name}</span>
              </span>
            </DropdownMenuItem>
          ))}
          {projects.length > 0 && <DropdownMenuSeparator />}
          <DropdownMenuItem icon={PlusIcon} onSelect={onCreateProject}>
            {t('nav.createProject')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {NAV_ITEMS.map((item) => {
          const isActive = item.key === activeKey;
          return (
            <li key={item.key}>
              <button
                type="button"
                onClick={() => onNavigate(item.key)}
                aria-current={isActive ? 'page' : undefined}
                className={cn(
                  'flex w-full items-center gap-half rounded-sm px-half py-half',
                  'text-sm transition-colors',
                  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
                  isActive
                    ? 'bg-primary font-medium text-high'
                    : 'text-normal hover:bg-primary'
                )}
              >
                <item.icon
                  className={cn(
                    'size-icon-sm shrink-0',
                    isActive ? 'text-high' : 'text-low'
                  )}
                  weight="bold"
                />
                <span className="flex-1 truncate text-left">
                  {t(item.labelKey)}
                </span>
                {item.key === 'workbench' && pendingCount > 0 && (
                  <span className="rounded-sm bg-brand px-1 font-ibm-plex-mono text-xs text-on-brand">
                    {pendingCount}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>

      <div className="mt-auto flex flex-col gap-half">
        {userSlot}
        {appVersion && (
          <p data-testid="nav-footer" className="m-0 truncate text-xs text-low">
            {t('nav.footer', { version: appVersion })}
          </p>
        )}
      </div>
    </div>
  );
}
