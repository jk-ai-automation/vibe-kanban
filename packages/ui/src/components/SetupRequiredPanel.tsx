import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';

export interface SetupRequiredPanelProps {
  /** 是否正在重新检测。 */
  isChecking: boolean;
  onRetry: () => void;
}

/**
 * 团队模式首启向导（最小可用版本）。
 *
 * 后端的 `/api/local-auth/setup` 还在实现中（计划任务 D4），所以这里**不猜
 * 请求体**，只把「为什么进不去、接下来怎么做、好了之后点哪里」讲清楚，
 * 避免白屏。D 批接口落地后把表单补进来即可，本组件的 props 不必变。
 */
export function SetupRequiredPanel({
  isChecking,
  onRetry,
}: SetupRequiredPanelProps) {
  const { t } = useTranslation();

  return (
    <div className="min-h-screen w-full flex items-center justify-center bg-primary text-normal p-double">
      <div className="w-full max-w-md bg-secondary rounded border p-double">
        <h1 className="text-xl text-high font-medium mb-half">
          {t('localAuth.setupTitle')}
        </h1>
        <p className="text-base text-normal mb-base">
          {t('localAuth.setupDescription')}
        </p>
        <p className="text-base text-low mb-double">
          {t('localAuth.setupPending')}
        </p>
        <button
          type="button"
          onClick={onRetry}
          disabled={isChecking}
          className={cn(
            'h-cta w-full rounded bg-brand text-on-brand text-base font-medium',
            'hover:bg-brand-hover transition-colors',
            'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
            'disabled:opacity-50 disabled:cursor-not-allowed'
          )}
        >
          {isChecking ? t('localAuth.checking') : t('localAuth.retry')}
        </button>
      </div>
    </div>
  );
}
