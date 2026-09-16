import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InviteAcceptPanel } from '@vibe/ui/components/InviteAcceptPanel';
import {
  LocalAuthRequestError,
  acceptInvite,
} from '@/shared/lib/local/bootstrapApi';
import { describeInviteAcceptError } from '@/features/local-auth/model/inviteAccept';
import {
  isAccountFormValid,
  translateFieldError,
  validateAccountForm,
} from '@/features/local-auth/model/members';

interface InviteAcceptContainerProps {
  /** 从 `/login?invite=<code>` 里取出的邀请码，非空字符串。 */
  code: string;
}

/**
 * 邀请注册页容器。
 *
 * 字段与校验规则跟管理员建号表单（`MembersPageContainer`）、首启向导
 * （`SetupWizardContainer`）共用同一套纯函数
 * （`features/local-auth/model/members.ts::validateAccountForm`），不另写
 * 一套；`displayName` 和这两处一样是自由文本，留空时用用户名兜底。
 *
 * 成功后**整页重载**回首页：注册即登录（后端已经下发会话 Cookie），
 * 理由与 `SetupWizardContainer`/`LoginPageContainer` 一致——团队模式下很多
 * 集合已经带着未登录状态初始化过了，重载比手工清理更省心。
 *
 * 失败后**不清空**已填字段：409（用户名或邮箱冲突）时邀请码没有被消费，
 * 用户改个用户名就能重试，保留输入是更好的体验。
 */
export function InviteAcceptContainer({ code }: InviteAcceptContainerProps) {
  const { t } = useTranslation();

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

  const handleSubmit = useCallback(async () => {
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
      await acceptInvite({
        code,
        username: username.trim(),
        display_name: displayName.trim() || username.trim(),
        password,
        email: email.trim().length > 0 ? email.trim() : null,
      });
      // 注册即登录：整页重载回首页，让新会话从头初始化。
      window.location.assign('/');
    } catch (error) {
      if (error instanceof LocalAuthRequestError) {
        const display = describeInviteAcceptError(error.status, error.message);
        setSubmitError(
          display.kind === 'message' ? display.text : t(display.key)
        );
      } else {
        setSubmitError(t('localAuth.networkError'));
      }
      setIsSubmitting(false);
    }
  }, [code, displayName, email, password, t, username]);

  const handleBackToLogin = useCallback(() => {
    // 丢掉 ?invite= 参数，回到普通登录页；整页导航即可，没有额外状态要清理。
    window.location.assign('/login');
  }, []);

  return (
    <InviteAcceptPanel
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
      onBackToLogin={handleBackToLogin}
    />
  );
}
