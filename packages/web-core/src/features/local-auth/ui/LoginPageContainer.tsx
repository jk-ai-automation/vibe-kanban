import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { LoginPanel } from '@vibe/ui/components/LoginPanel';
import {
  InvalidCredentialsError,
  RateLimitedError,
  login,
} from '@/shared/lib/local/bootstrapApi';
import {
  isLoginFormValid,
  resolvePostLoginTarget,
  validateLoginForm,
  type LoginFormErrors,
} from '@/features/local-auth/model/loginForm';

interface LoginPageContainerProps {
  /** bootstrap 返回的已配齐凭据的第三方登录方式 id。 */
  providers: string[];
  /** 初始错误（例如 bootstrap 拉取失败）。 */
  initialError?: string | null;
}

/**
 * 登录页容器。
 *
 * 登录成功后**整页重载**到目标路径，而不是走前端路由：团队模式下未登录时
 * 很多集合（WebSocket、TanStack DB）已经带着 401 初始化过了，重载是最省心
 * 也最不容易留脏状态的收尾方式。
 */
export function LoginPageContainer({
  providers,
  initialError = null,
}: LoginPageContainerProps) {
  const { t } = useTranslation();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [fieldErrors, setFieldErrors] = useState<LoginFormErrors>({});
  const [error, setError] = useState<string | null>(initialError);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const providerOptions = useMemo(
    () =>
      providers.map((id) => ({
        id,
        label: t(`localAuth.providers.${id}`, { defaultValue: id }),
      })),
    [providers, t]
  );

  const handleSubmit = useCallback(async () => {
    const errors = validateLoginForm({ username, password });
    setFieldErrors(errors);
    if (!isLoginFormValid(errors)) return;

    setError(null);
    setIsSubmitting(true);
    try {
      await login({ username: username.trim(), password });
      const target = resolvePostLoginTarget(
        window.location.search,
        `${window.location.pathname}${window.location.search}`
      );
      window.location.assign(target);
    } catch (caught) {
      // 只按错误类型（即状态码）分支，绝不匹配后端 message 文本：
      // 四种登录失败在后端是刻意不可区分的。
      if (caught instanceof InvalidCredentialsError) {
        setError(t('localAuth.invalidCredentials'));
      } else if (caught instanceof RateLimitedError) {
        setError(t('localAuth.rateLimited'));
      } else {
        setError(t('localAuth.networkError'));
      }
      setIsSubmitting(false);
    }
  }, [password, t, username]);

  const handleProviderClick = useCallback((providerId: string) => {
    window.location.assign(`/api/local-auth/oauth/${providerId}/start`);
  }, []);

  return (
    <LoginPanel
      username={username}
      password={password}
      error={error}
      usernameError={
        fieldErrors.username ? t(`localAuth.${fieldErrors.username}`) : null
      }
      passwordError={
        fieldErrors.password ? t(`localAuth.${fieldErrors.password}`) : null
      }
      isSubmitting={isSubmitting}
      providers={providerOptions}
      onUsernameChange={setUsername}
      onPasswordChange={setPassword}
      onSubmit={() => void handleSubmit()}
      onProviderClick={handleProviderClick}
    />
  );
}
