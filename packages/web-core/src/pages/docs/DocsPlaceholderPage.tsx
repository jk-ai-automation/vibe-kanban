import { useTranslation } from 'react-i18next';
import { BookOpenIcon } from '@phosphor-icons/react';

/**
 * 「文档」导航项的空态占位（设计文档 §8.1）。
 * 下一期把各需求的 requirement / spec / plan 产出物归档到这里。
 */
export function DocsPlaceholderPage() {
  const { t } = useTranslation('common');

  return (
    <div className="h-full overflow-y-auto bg-primary px-double py-double">
      <div className="mx-auto flex max-w-3xl flex-col gap-base">
        <div className="flex items-center gap-half">
          <BookOpenIcon className="size-icon-base text-low" weight="bold" />
          <h2 className="m-0 text-xl font-medium text-high">
            {t('docs.title')}
          </h2>
        </div>
        <div
          data-testid="docs-empty"
          className="rounded-sm border border-dashed border-border p-double text-center"
        >
          <p className="m-0 text-base text-normal">{t('docs.emptyTitle')}</p>
          <p className="m-0 mt-half text-sm text-low">{t('docs.emptyHint')}</p>
        </div>
      </div>
    </div>
  );
}
