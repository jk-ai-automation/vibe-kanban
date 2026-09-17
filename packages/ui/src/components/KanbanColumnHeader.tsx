import { useTranslation } from 'react-i18next';
import { PlusIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface KanbanColumnHeaderProps {
  /** 状态列名。 */
  name: string;
  /** 状态列颜色（HSL 三元组字符串，跟 `project_statuses.color` 一致）。 */
  color: string;
  /** 列内卡片数。 */
  count: number;
  /** WIP 状态。`'over'` 时计数徽标变成 `text-error`。**只提示不阻止。** */
  wip: 'ok' | 'over';
  /** WIP 上限，只用于提示文案。 */
  wipLimit: number;
  /**
   * 流程阶段的 i18n key（`kanban.stage.*`）。
   * 传 `null` 就不显示阶段徽标——团队版所有列都会回落成同一个阶段，
   * 显示出来是一排一模一样的徽标，纯噪声。
   */
  stageLabelKey?: string | null;
  onAddIssue: () => void;
}

/**
 * 看板列头：阶段徽标 + 状态名 + 计数（含 WIP 提示） + 新建按钮。
 *
 * 文案在这里翻译，好让 `KanbanColumn` 保持「内部零 hook」。
 * 配色全走设计令牌，暗色模式自动跟随。
 */
export function KanbanColumnHeader({
  name,
  color,
  count,
  wip,
  wipLimit,
  stageLabelKey,
  onAddIssue,
}: KanbanColumnHeaderProps) {
  const { t } = useTranslation('common');
  const isOverWip = wip === 'over';
  const wipHint = t('kanban.wipHint', { limit: wipLimit });

  return (
    <div className="border-t sticky border-b top-0 z-20 flex shrink-0 items-center justify-between gap-2 p-base bg-secondary">
      <div className="flex min-w-0 items-center gap-2">
        <div
          className="h-2 w-2 rounded-full shrink-0"
          style={{ backgroundColor: `hsl(${color})` }}
        />
        <p className="m-0 truncate text-sm">{name}</p>
        <span
          title={isOverWip ? wipHint : undefined}
          aria-label={isOverWip ? wipHint : undefined}
          className={cn(
            'shrink-0 rounded-sm px-1 font-ibm-plex-mono text-sm',
            isOverWip ? 'bg-error/10 text-error' : 'text-low'
          )}
        >
          {count}
        </span>
        {stageLabelKey && (
          <span className="hidden shrink-0 rounded-sm bg-panel px-1 text-xs text-low lg:inline">
            {t(stageLabelKey)}
          </span>
        )}
      </div>
      <button
        type="button"
        onClick={onAddIssue}
        className="p-half rounded-sm text-low hover:text-normal hover:bg-secondary transition-colors shrink-0"
        aria-label={t('kanban.createNewIssue')}
      >
        <PlusIcon className="size-icon-xs" weight="bold" />
      </button>
    </div>
  );
}
