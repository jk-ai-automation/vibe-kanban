import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { SpinnerIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export type SetupWizardStatus =
  | 'checking-token'
  | 'no-token'
  | 'invalid-token'
  | 'already-initialized'
  | 'ready';

export interface SetupRequiredPanelProps {
  /** 是否正在重新检测 bootstrap（`needs_setup` 有没有变成 false）。 */
  isChecking: boolean;
  onRetry: () => void;

  status: SetupWizardStatus;

  username: string;
  displayName: string;
  email: string;
  password: string;
  onUsernameChange: (value: string) => void;
  onDisplayNameChange: (value: string) => void;
  onEmailChange: (value: string) => void;
  onPasswordChange: (value: string) => void;

  usernameError: string | null;
  passwordError: string | null;
  emailError: string | null;

  isSubmitting: boolean;
  submitError: string | null;
  onSubmit: () => void;
}

const inputClass = cn(
  'w-full h-cta px-base bg-panel rounded border text-base text-normal',
  'placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand'
);

const primaryButtonClass = cn(
  'h-cta w-full rounded bg-brand text-on-brand text-base font-medium',
  'hover:bg-brand-hover transition-colors',
  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
  'disabled:opacity-50 disabled:cursor-not-allowed',
  'flex items-center justify-center gap-half'
);

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen w-full flex items-center justify-center bg-primary text-normal p-double">
      <div className="w-full max-w-md bg-secondary rounded border p-double">
        {children}
      </div>
    </div>
  );
}

/**
 * 团队模式首启初始化向导。
 *
 * 令牌的三种失效情况（缺失 / 错误 / 过期 / 已用）后端统一返回 401，
 * 前端也统一按 `invalid-token` 展示同一句「链接已失效」，不细分——细分了
 * 也没有更可操作的建议，都是「找服务器管理员重启拿新链接」。
 * `already-initialized` 是唯一需要单独文案的失败：它不是「链接坏了」，
 * 是「已经有人用这张链接建过管理员了」，应该引导去登录页而不是重试。
 */
export function SetupRequiredPanel({
  isChecking,
  onRetry,
  status,
  username,
  displayName,
  email,
  password,
  onUsernameChange,
  onDisplayNameChange,
  onEmailChange,
  onPasswordChange,
  usernameError,
  passwordError,
  emailError,
  isSubmitting,
  submitError,
  onSubmit,
}: SetupRequiredPanelProps) {
  const { t } = useTranslation();

  const header = (
    <>
      <h1 className="text-xl text-high font-medium mb-half">
        {t('localAuth.setupTitle')}
      </h1>
      <p className="text-base text-normal mb-base">
        {t('localAuth.setupDescription')}
      </p>
    </>
  );

  if (status === 'no-token') {
    return (
      <Shell>
        {header}
        <p className="text-base text-low">{t('localAuth.setupNoToken')}</p>
      </Shell>
    );
  }

  if (status === 'checking-token') {
    return (
      <Shell>
        {header}
        <p className="text-base text-low">{t('localAuth.checking')}</p>
      </Shell>
    );
  }

  if (status === 'invalid-token') {
    return (
      <Shell>
        {header}
        <p className="text-base text-error mb-double">
          {t('localAuth.setupLinkExpired')}
        </p>
        <button
          type="button"
          onClick={onRetry}
          disabled={isChecking}
          className={primaryButtonClass}
        >
          {isChecking ? t('localAuth.checking') : t('localAuth.retry')}
        </button>
      </Shell>
    );
  }

  if (status === 'already-initialized') {
    return (
      <Shell>
        {header}
        <p className="text-base text-normal mb-double">
          {t('localAuth.setupAlreadyInitialized')}
        </p>
        <button
          type="button"
          onClick={onRetry}
          disabled={isChecking}
          className={primaryButtonClass}
        >
          {isChecking ? t('localAuth.checking') : t('localAuth.setupGoToLogin')}
        </button>
      </Shell>
    );
  }

  return (
    <Shell>
      {header}
      <p className="text-sm text-low mb-base">{t('localAuth.setupRoleNote')}</p>

      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="flex flex-col gap-base"
      >
        <div className="flex flex-col gap-half">
          <label htmlFor="vk-setup-username" className="text-base text-low">
            {t('localAuth.username')}
          </label>
          <input
            id="vk-setup-username"
            name="username"
            type="text"
            autoComplete="username"
            autoFocus
            className={inputClass}
            value={username}
            onChange={(event) => onUsernameChange(event.target.value)}
            aria-invalid={usernameError !== null}
          />
          {usernameError && (
            <p className="text-sm text-error">{usernameError}</p>
          )}
        </div>

        <div className="flex flex-col gap-half">
          <label htmlFor="vk-setup-password" className="text-base text-low">
            {t('localAuth.password')}
          </label>
          <input
            id="vk-setup-password"
            name="password"
            type="password"
            autoComplete="new-password"
            className={inputClass}
            value={password}
            onChange={(event) => onPasswordChange(event.target.value)}
            aria-invalid={passwordError !== null}
          />
          {passwordError && (
            <p className="text-sm text-error">{passwordError}</p>
          )}
        </div>

        <div className="flex flex-col gap-half">
          <label htmlFor="vk-setup-display-name" className="text-base text-low">
            {t('localAuth.setupDisplayName')}
          </label>
          <input
            id="vk-setup-display-name"
            name="displayName"
            type="text"
            className={inputClass}
            value={displayName}
            onChange={(event) => onDisplayNameChange(event.target.value)}
          />
        </div>

        <div className="flex flex-col gap-half">
          <label htmlFor="vk-setup-email" className="text-base text-low">
            {t('localAuth.setupEmail')}
          </label>
          <input
            id="vk-setup-email"
            name="email"
            type="text"
            className={inputClass}
            value={email}
            onChange={(event) => onEmailChange(event.target.value)}
            aria-invalid={emailError !== null}
          />
          {emailError && <p className="text-sm text-error">{emailError}</p>}
        </div>

        {submitError && (
          <p role="alert" className="text-base text-error">
            {submitError}
          </p>
        )}

        <button
          type="submit"
          disabled={isSubmitting}
          className={primaryButtonClass}
        >
          {isSubmitting && (
            <SpinnerIcon className="size-icon-xs animate-spin" />
          )}
          {isSubmitting
            ? t('localAuth.setupSubmitting')
            : t('localAuth.setupSubmit')}
        </button>
      </form>
    </Shell>
  );
}
