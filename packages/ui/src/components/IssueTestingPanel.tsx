import { useTranslation } from 'react-i18next';
import { FlaskIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface IssueTestingPanelProps {
  className?: string;
}

function TestingSlot({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="rounded-sm border border-dashed border-border p-base">
      <p className="m-0 text-sm font-medium text-normal">{title}</p>
      <p className="m-0 mt-half text-sm text-low">{hint}</p>
      {children}
    </div>
  );
}

/**
 * 需求详情的「测试」页（设计文档 §7.3）。
 *
 * **本期是占位面板**：atp 还没接进来，这里只把 `results.json` 将来要落的三个槽位
 * （通过率 / 失败清单 / 报告链接）先摆好，并明确写「尚未接入」。
 * 只写结构不接数据——接入时把每个槽位换成真实数据即可，外层不用改。
 */
export function IssueTestingPanel({ className }: IssueTestingPanelProps) {
  const { t } = useTranslation('common');

  return (
    <section
      className={cn('flex flex-col gap-base p-base', className)}
      aria-label={t('kanban.tabs.testing')}
    >
      <div className="flex items-center gap-half rounded-sm bg-panel px-base py-half">
        <FlaskIcon className="size-icon-sm shrink-0 text-low" weight="bold" />
        <div className="min-w-0">
          <p className="m-0 text-sm font-medium text-normal">
            {t('kanban.testing.notConnectedTitle')}
          </p>
          <p className="m-0 text-sm text-low">
            {t('kanban.testing.notConnectedHint')}
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-base sm:grid-cols-3">
        <TestingSlot
          title={t('kanban.testing.passRate')}
          hint={t('kanban.testing.passRateHint')}
        >
          <p className="m-0 mt-half font-ibm-plex-mono text-lg text-low">—</p>
        </TestingSlot>
        <TestingSlot
          title={t('kanban.testing.failures')}
          hint={t('kanban.testing.failuresHint')}
        >
          <p className="m-0 mt-half font-ibm-plex-mono text-lg text-low">—</p>
        </TestingSlot>
        <TestingSlot
          title={t('kanban.testing.report')}
          hint={t('kanban.testing.reportHint')}
        >
          <p className="m-0 mt-half font-ibm-plex-mono text-lg text-low">—</p>
        </TestingSlot>
      </div>
    </section>
  );
}
