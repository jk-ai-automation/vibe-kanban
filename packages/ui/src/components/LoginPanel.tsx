import { useTranslation } from 'react-i18next';
import { SpinnerIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface LoginProviderOption {
  /** 后端 bootstrap 返回的提供方 id，例如 `feishu` / `lark` / `google`。 */
  id: string;
  label: string;
}

export interface LoginPanelProps {
  username: string;
  password: string;
  /** 已翻译好的整体错误文案；没有错误传 `null`。 */
  error: string | null;
  /** 字段级错误文案（已翻译）。 */
  usernameError: string | null;
  passwordError: string | null;
  isSubmitting: boolean;
  providers: LoginProviderOption[];
  onUsernameChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onSubmit: () => void;
  onProviderClick: (providerId: string) => void;
}

const inputClass = cn(
  'w-full h-cta px-base bg-panel rounded border text-base text-normal',
  'placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand'
);

/**
 * 本地账号密码登录页（无状态视图）。
 *
 * 四种登录失败（用户不存在 / 密码错误 / 账号停用 / 只有第三方登录）在后端
 * 是**不可区分**的 401，这里只按状态码给一句统一文案，不要按 message 分支。
 */
export function LoginPanel({
  username,
  password,
  error,
  usernameError,
  passwordError,
  isSubmitting,
  providers,
  onUsernameChange,
  onPasswordChange,
  onSubmit,
  onProviderClick,
}: LoginPanelProps) {
  const { t } = useTranslation();

  return (
    <div className="min-h-screen w-full flex items-center justify-center bg-primary text-normal p-double">
      <div className="w-full max-w-sm bg-secondary rounded border p-double">
        <h1 className="text-xl text-high font-medium mb-half">
          {t('localAuth.signInTitle')}
        </h1>
        <p className="text-base text-low mb-double">
          {t('localAuth.signInSubtitle')}
        </p>

        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
          className="flex flex-col gap-base"
        >
          <div className="flex flex-col gap-half">
            <label htmlFor="vk-login-username" className="text-base text-low">
              {t('localAuth.username')}
            </label>
            <input
              id="vk-login-username"
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
            <label htmlFor="vk-login-password" className="text-base text-low">
              {t('localAuth.password')}
            </label>
            <input
              id="vk-login-password"
              name="password"
              type="password"
              autoComplete="current-password"
              className={inputClass}
              value={password}
              onChange={(event) => onPasswordChange(event.target.value)}
              aria-invalid={passwordError !== null}
            />
            {passwordError && (
              <p className="text-sm text-error">{passwordError}</p>
            )}
          </div>

          {error && (
            <p role="alert" className="text-base text-error">
              {error}
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
            {isSubmitting ? t('localAuth.signingIn') : t('localAuth.signIn')}
          </button>
        </form>

        {providers.length > 0 && (
          <div className="mt-double flex flex-col gap-half">
            <p className="text-sm text-low text-center">
              {t('localAuth.orContinueWith')}
            </p>
            {providers.map((provider) => (
              <button
                key={provider.id}
                type="button"
                onClick={() => onProviderClick(provider.id)}
                className={cn(
                  'h-cta w-full rounded border bg-panel text-base text-normal',
                  'hover:text-high transition-colors',
                  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand'
                )}
              >
                {provider.label}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
