import { useTranslation } from 'react-i18next';
import { SpinnerIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface InviteAcceptPanelProps {
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
  /** 已翻译好的整体错误文案（例如后端「邀请码无效或已过期」）；没有错误传 `null`。 */
  submitError: string | null;
  onSubmit: () => void;
  onBackToLogin: () => void;
}

const inputClass = cn(
  'w-full h-cta px-base bg-panel rounded border text-base text-normal',
  'placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand'
);

/**
 * 邀请注册表单（无状态视图）。
 *
 * 邀请码本身**没有**一个「先查一下有没有效」的接口（`POST /invites/accept`
 * 是唯一入口）——这是刻意的：一个能查询邀请码状态的接口本身就是枚举
 * 攻击面。所以这里不像 `SetupRequiredPanel` 那样有 checking/invalid-token
 * 等前置状态，直接给表单，失败原因等提交之后由后端一次性给。
 */
export function InviteAcceptPanel({
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
  onBackToLogin,
}: InviteAcceptPanelProps) {
  const { t } = useTranslation();

  return (
    <div className="min-h-screen w-full flex items-center justify-center bg-primary text-normal p-double">
      <div className="w-full max-w-sm bg-secondary rounded border p-double">
        <h1 className="text-xl text-high font-medium mb-half">
          {t('localAuth.inviteAcceptTitle')}
        </h1>
        <p className="text-base text-low mb-double">
          {t('localAuth.inviteAcceptSubtitle')}
        </p>

        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
          className="flex flex-col gap-base"
        >
          <div className="flex flex-col gap-half">
            <label htmlFor="vk-invite-username" className="text-base text-low">
              {t('localAuth.username')}
            </label>
            <input
              id="vk-invite-username"
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
            <label htmlFor="vk-invite-password" className="text-base text-low">
              {t('localAuth.password')}
            </label>
            <input
              id="vk-invite-password"
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
            <label
              htmlFor="vk-invite-display-name"
              className="text-base text-low"
            >
              {t('localAuth.setupDisplayName')}
            </label>
            <input
              id="vk-invite-display-name"
              name="displayName"
              type="text"
              className={inputClass}
              value={displayName}
              onChange={(event) => onDisplayNameChange(event.target.value)}
            />
          </div>

          <div className="flex flex-col gap-half">
            <label htmlFor="vk-invite-email" className="text-base text-low">
              {t('localAuth.setupEmail')}
            </label>
            <input
              id="vk-invite-email"
              name="email"
              type="text"
              autoComplete="email"
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
            className={cn(
              'h-cta w-full rounded bg-brand text-on-brand text-base font-medium',
              'hover:bg-brand-hover transition-colors',
              'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
              'disabled:opacity-50 disabled:cursor-not-allowed',
              'flex items-center justify-center gap-half'
            )}
          >
            {isSubmitting && (
              <SpinnerIcon className="size-icon-xs animate-spin" />
            )}
            {isSubmitting
              ? t('localAuth.inviteAcceptSubmitting')
              : t('localAuth.inviteAcceptSubmit')}
          </button>
        </form>

        <button
          type="button"
          onClick={onBackToLogin}
          className={cn(
            'mt-double w-full text-sm text-low text-center',
            'hover:text-high transition-colors',
            'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand rounded'
          )}
        >
          {t('localAuth.inviteAcceptBackToLogin')}
        </button>
      </div>
    </div>
  );
}
