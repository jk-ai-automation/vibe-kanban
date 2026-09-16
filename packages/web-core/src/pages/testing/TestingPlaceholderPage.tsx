import { useTranslation } from 'react-i18next';
import { FlaskIcon } from '@phosphor-icons/react';
import { IssueTestingPanel } from '@vibe/ui/components/IssueTestingPanel';

/**
 * 「测试」导航项的占位页（设计文档 §7.1：测试(占位，下一期接 atp)）。
 *
 * 本期只说明「尚未接入」并把 atp `results.json` 将来要落的三个槽位摆出来，
 * 不接任何数据。复用需求详情测试页的同一块占位面板，接入时只改一处。
 */
export function TestingPlaceholderPage() {
  const { t } = useTranslation('common');

  return (
    <div className="h-full overflow-y-auto bg-primary px-double py-double">
      <div className="mx-auto flex max-w-3xl flex-col gap-base">
        <div className="flex items-center gap-half">
          <FlaskIcon className="size-icon-base text-low" weight="bold" />
          <h2 className="m-0 text-2xl font-medium text-high">
            {t('kanban.tabs.testing')}
          </h2>
        </div>
        <p className="m-0 text-sm text-low">
          {t('kanban.testing.notConnectedHint')}
        </p>
        <IssueTestingPanel className="px-0" />
      </div>
    </div>
  );
}
