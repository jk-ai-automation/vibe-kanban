import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';
import { PrimaryButton } from './PrimaryButton';

export type PipelineGateBarKind =
  | 'human'
  | 'auto'
  | 'running'
  | 'paused'
  | 'failed'
  | 'completed'
  | 'cancelled';

export interface PipelineGateBarProps {
  kind: PipelineGateBarKind;
  /** 已翻译：关卡名 / 「自动关卡」/ 状态名。 */
  title: string;
  /** 已翻译：说明、判定条件与轮次。 */
  message: string;
  onConfirm: () => void;
  onRejectSubmit: (comment: string) => void;
  /** 打回意见草稿；null = 输入框收起。受控。 */
  rejectDraft: string | null;
  onRejectDraftChange: (value: string | null) => void;
  isBusy: boolean;
  error: string | null;
  /**
   * 是否显示「确认 / 打回」。默认只在 `kind === 'human'` 时显示；暂停中仍有
   * 待决人工关卡时（契约 C12：暂停时关卡决策照常受理）由容器传 true。
   */
  canDecide?: boolean;
}

/**
 * 底部固定关卡条（设计文档 §8.4）：等人工时显示「确认并继续」「打回并说明」；
 * 其它状态只显示判定条件与进度。打回必须写意见（契约 §2：reject 缺意见 400）。
 */
export function PipelineGateBar({
  kind,
  title,
  message,
  onConfirm,
  onRejectSubmit,
  rejectDraft,
  onRejectDraftChange,
  isBusy,
  error,
  canDecide,
}: PipelineGateBarProps) {
  const { t } = useTranslation('common');
  const isHuman = canDecide ?? kind === 'human';
  const trimmed = rejectDraft?.trim() ?? '';

  return (
    <div
      data-testid="gate-bar"
      data-kind={kind}
      className={cn(
        'flex shrink-0 flex-col gap-half border-t border-border px-double py-base',
        isHuman
          ? 'bg-brand/5'
          : kind === 'failed'
            ? 'bg-stage-failed/5'
            : 'bg-secondary'
      )}
    >
      <div className="flex flex-wrap items-center gap-base">
        <div className="min-w-0 flex-1">
          <p className="m-0 text-sm font-medium text-high">{title}</p>
          <p className="m-0 text-sm text-low">{message}</p>
        </div>
        {isHuman && rejectDraft === null && (
          <div className="flex items-center gap-half">
            <PrimaryButton
              variant="tertiary"
              value={t('pipeline.gate.reject')}
              onClick={() => onRejectDraftChange('')}
              disabled={isBusy}
            />
            <PrimaryButton
              value={t('pipeline.gate.confirm')}
              actionIcon={isBusy ? 'spinner' : undefined}
              onClick={onConfirm}
              disabled={isBusy}
            />
          </div>
        )}
      </div>
      {isHuman && rejectDraft !== null && (
        <div className="flex flex-col gap-half">
          <textarea
            aria-label={t('pipeline.gate.rejectPlaceholder')}
            placeholder={t('pipeline.gate.rejectPlaceholder')}
            value={rejectDraft}
            onChange={(event) => onRejectDraftChange(event.target.value)}
            rows={3}
            autoFocus
            className="w-full resize-y rounded-sm border border-border bg-primary px-base py-half text-sm text-normal placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand"
          />
          <div className="flex justify-end gap-half">
            <PrimaryButton
              variant="tertiary"
              value={t('pipeline.gate.cancel')}
              onClick={() => onRejectDraftChange(null)}
              disabled={isBusy}
            />
            <PrimaryButton
              value={t('pipeline.gate.rejectSubmit')}
              onClick={() => onRejectSubmit(trimmed)}
              disabled={isBusy || trimmed.length === 0}
            />
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="m-0 text-sm text-error">
          {error}
        </p>
      )}
    </div>
  );
}
