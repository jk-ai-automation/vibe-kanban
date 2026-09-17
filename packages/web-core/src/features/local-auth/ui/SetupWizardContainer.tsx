import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import {
  SetupRequiredPanel,
  type SetupWizardStatus,
} from '@vibe/ui/components/SetupRequiredPanel';
import {
  LocalAuthRequestError,
  RateLimitedError,
  SetupTokenInvalidError,
  fetchSetupStatus,
  submitSetupAdmin,
} from '@/shared/lib/local/bootstrapApi';
import {
  extractSetupToken,
  isAlreadyInitializedMessage,
} from '@/features/local-auth/model/setupWizard';
import {
  isAccountFormValid,
  translateFieldError,
  validateAccountForm,
} from '@/features/local-auth/model/members';

interface SetupWizardContainerProps {
  /** 沿用 `SetupRequiredPanel` 原有含义：bootstrap 是否正在重新拉取。 */
  isChecking: boolean;
  onRetry: () => void;
}

const SETUP_STATUS_QUERY_KEY = ['local-auth', 'setup-status'] as const;

/**
 * 首启初始化向导的容器。
 *
 * `SetupRequiredPanel` 是纯视图（照 `LoginPanel` 的分层方式），本组件负责：
 * 从 URL 取令牌、查询令牌是否有效（不消费）、建号表单的字段/校验/提交。
 *
 * 建成后**整页重载**到 `/`，理由与 `LoginPageContainer` 一致：很多集合已经
 * 带着「未登录」状态初始化过了，重载比手工清理状态更省心。
 */
export function SetupWizardContainer({
  isChecking,
  onRetry,
}: SetupWizardContainerProps) {
  const { t } = useTranslation();

  const token = useMemo(() => extractSetupToken(window.location.search), []);

  const statusQuery = useQuery({
    queryKey: [...SETUP_STATUS_QUERY_KEY, token],
    queryFn: () => fetchSetupStatus(token as string),
    enabled: token !== null,
    retry: false,
    refetchOnWindowFocus: false,
  });

  const [username, setUsername] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [fieldErrors, setFieldErrors] = useState<{
    username: string | null;
    password: string | null;
    email: string | null;
  }>({ username: null, password: null, email: null });
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  // 提交时可能发现令牌其实已经失效/已经初始化——这两种覆盖 GET 查询的结果。
  const [submitTokenInvalid, setSubmitTokenInvalid] = useState(false);
  const [submitAlreadyInitialized, setSubmitAlreadyInitialized] =
    useState(false);

  const status: SetupWizardStatus = submitAlreadyInitialized
    ? 'already-initialized'
    : submitTokenInvalid
      ? 'invalid-token'
      : token === null
        ? 'no-token'
        : statusQuery.isPending
          ? 'checking-token'
          : statusQuery.isError
            ? 'invalid-token'
            : 'ready';

  const handleSubmit = useCallback(async () => {
    if (!token) return;
    const rawErrors = validateAccountForm({ username, password, email });
    setFieldErrors({
      username: translateFieldError(t, rawErrors.username),
      password: translateFieldError(t, rawErrors.password),
      email: translateFieldError(t, rawErrors.email),
    });
    if (!isAccountFormValid(rawErrors)) return;

    setSubmitError(null);
    setIsSubmitting(true);
    try {
      await submitSetupAdmin({
        token,
        username: username.trim(),
        display_name: displayName.trim() || username.trim(),
        password,
        email: email.trim().length > 0 ? email.trim() : null,
      });
      // 建成即登录：整页重载回首页，让新会话从头初始化。
      window.location.assign('/');
    } catch (error) {
      if (error instanceof SetupTokenInvalidError) {
        setSubmitTokenInvalid(true);
      } else if (error instanceof RateLimitedError) {
        setSubmitError(t('localAuth.rateLimited'));
      } else if (
        error instanceof LocalAuthRequestError &&
        error.status === 409 &&
        isAlreadyInitializedMessage(error.message)
      ) {
        setSubmitAlreadyInitialized(true);
      } else if (error instanceof LocalAuthRequestError) {
        // 400（弱密码/非法用户名）与 409（用户名已被占用）的后端文案是
        // 特意写给人看的，直接展示；令牌没被消费，改对可以直接重试。
        setSubmitError(error.message);
      } else {
        setSubmitError(t('localAuth.networkError'));
      }
      setIsSubmitting(false);
    }
  }, [displayName, email, password, t, token, username]);

  return (
    <SetupRequiredPanel
      isChecking={isChecking}
      onRetry={onRetry}
      status={status}
      username={username}
      displayName={displayName}
      email={email}
      password={password}
      onUsernameChange={setUsername}
      onDisplayNameChange={setDisplayName}
      onEmailChange={setEmail}
      onPasswordChange={setPassword}
      usernameError={fieldErrors.username}
      passwordError={fieldErrors.password}
      emailError={fieldErrors.email}
      isSubmitting={isSubmitting}
      submitError={submitError}
      onSubmit={() => void handleSubmit()}
    />
  );
}
