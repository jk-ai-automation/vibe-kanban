import { useCallback, useState, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import type {
  AdminResetPasswordRequest,
  CreateInviteRequest,
  CreateLocalUserRequest,
  UpdateLocalUserRequest,
} from 'shared/types';
import {
  MembersPanel,
  type CreatedInvite,
  type MembersPanelInvite,
  type MembersPanelMember,
  type MembersPanelRole,
  type MembersPanelStatus,
  type NewInviteFormValues,
  type NewMemberFormErrors,
  type NewMemberFormValues,
} from '@vibe/ui/components/MembersPanel';
import {
  AdminApiError,
  createInvite,
  createMember,
  deleteInvite,
  listInvites,
  listMembers,
  resetMemberPassword,
  updateMember,
} from '@/shared/lib/local/adminApi';
import {
  buildInviteLink,
  canChangeRole,
  canDisable,
  canRevokeInvite,
  describeAdminError,
  DEFAULT_INVITE_TTL_DAYS,
  getInviteStatus,
  isAccountFormValid,
  isFixedLocalUser,
  isInviteFormValid,
  sortInvitesByCreatedAtDesc,
  sortMembers,
  translateFieldError,
  validateAccountForm,
  validateInviteForm,
  validateResetPasswordForm,
  type MemberLike,
} from '@/features/local-auth/model/members';
import { useLocalSession } from '@/shared/hooks/auth/useLocalSession';

const MEMBERS_QUERY_KEY = ['admin', 'users'] as const;
const INVITES_QUERY_KEY = ['admin', 'invites'] as const;

const EMPTY_NEW_MEMBER: NewMemberFormValues = {
  username: '',
  displayName: '',
  email: '',
  password: '',
  role: 'member',
};

const EMPTY_NEW_MEMBER_ERRORS: NewMemberFormErrors = {};

const DEFAULT_NEW_INVITE: NewInviteFormValues = {
  role: 'member',
  expiresInDays: DEFAULT_INVITE_TTL_DAYS,
};

function displayAdminError(
  t: (key: string) => string,
  error: unknown,
  fallbackKey: string
): string {
  if (error instanceof AdminApiError) {
    const display = describeAdminError(error);
    return display.kind === 'message' ? display.text : t(display.key);
  }
  return t(fallbackKey);
}

function FullPageMessage({ children }: { children: ReactNode }) {
  return (
    <div className="h-full w-full flex items-center justify-center bg-primary text-normal p-double">
      {children}
    </div>
  );
}

/**
 * 403：当前用户不是 admin。**不能只靠隐藏入口**——直接访问 `/members` 的
 * member、或者已经打开这页之后被另一位管理员降权，都要落到这里而不是崩溃。
 */
function ForbiddenMessage({
  t,
  onBackHome,
}: {
  t: (key: string) => string;
  onBackHome: () => void;
}) {
  return (
    <FullPageMessage>
      <div className="w-full max-w-md bg-secondary rounded border p-double text-center">
        <h1 className="text-xl text-high font-medium mb-half">
          {t('localAuth.members.forbiddenTitle')}
        </h1>
        <p className="text-base text-low mb-double">
          {t('localAuth.members.forbiddenDescription')}
        </p>
        <button
          type="button"
          className="h-cta px-base rounded bg-brand text-on-brand text-base font-medium hover:bg-brand-hover transition-colors"
          onClick={onBackHome}
        >
          {t('localAuth.members.backHome')}
        </button>
      </div>
    </FullPageMessage>
  );
}

/**
 * 成员管理页容器。
 *
 * 这页只在 `LocalSessionProvider` 已经确认登录之后才会挂载，但**后端才是
 * 权威**：如果当前用户已经不是 admin（比如被另一位管理员降权，会话还没
 * 过期），`/api/admin/*` 一律 403。这里在请求列表之前就先按本地会话的
 * `role` 拦一道，避免对着一个注定 403 的接口发一堆请求；请求本身仍然
 * 走真实的 admin 接口,所以哪怕本地判断出错，后端也不会放行。
 */
export function MembersPageContainer() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const localSession = useLocalSession();
  const isAdmin = localSession?.user.role === 'admin';

  const [addMemberOpen, setAddMemberOpen] = useState(false);
  const [newMember, setNewMember] = useState(EMPTY_NEW_MEMBER);
  const [newMemberErrors, setNewMemberErrors] = useState(
    EMPTY_NEW_MEMBER_ERRORS
  );
  const [newMemberError, setNewMemberError] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);

  const [resetPasswordTargetId, setResetPasswordTargetId] = useState<
    string | null
  >(null);
  const [resetPasswordValue, setResetPasswordValue] = useState('');
  const [resetPasswordError, setResetPasswordError] = useState<string | null>(
    null
  );

  const [addInviteOpen, setAddInviteOpen] = useState(false);
  const [newInvite, setNewInvite] = useState(DEFAULT_NEW_INVITE);
  const [newInviteError, setNewInviteError] = useState<string | null>(null);
  const [createdInvite, setCreatedInvite] = useState<CreatedInvite | null>(
    null
  );

  const membersQuery = useQuery({
    queryKey: MEMBERS_QUERY_KEY,
    queryFn: listMembers,
    enabled: isAdmin,
    retry: false,
  });
  const invitesQuery = useQuery({
    queryKey: INVITES_QUERY_KEY,
    queryFn: listInvites,
    enabled: isAdmin,
    retry: false,
  });

  const invalidateMembers = useCallback(
    () => queryClient.invalidateQueries({ queryKey: MEMBERS_QUERY_KEY }),
    [queryClient]
  );
  const invalidateInvites = useCallback(
    () => queryClient.invalidateQueries({ queryKey: INVITES_QUERY_KEY }),
    [queryClient]
  );

  const createMemberMutation = useMutation({
    mutationFn: (payload: CreateLocalUserRequest) => createMember(payload),
    onSuccess: () => {
      setAddMemberOpen(false);
      setNewMember(EMPTY_NEW_MEMBER);
      setNewMemberErrors(EMPTY_NEW_MEMBER_ERRORS);
      setNewMemberError(null);
      void invalidateMembers();
    },
    onError: (error: unknown) => {
      setNewMemberError(displayAdminError(t, error, 'localAuth.networkError'));
    },
  });

  const changeRoleMutation = useMutation({
    mutationFn: ({ id, role }: { id: string; role: MembersPanelRole }) =>
      updateMember(id, {
        role,
        status: null,
        display_name: null,
      } satisfies UpdateLocalUserRequest),
    onSuccess: () => {
      setActionError(null);
      void invalidateMembers();
    },
    onError: (error: unknown) => {
      setActionError(displayAdminError(t, error, 'localAuth.networkError'));
    },
  });

  const toggleStatusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: MembersPanelStatus }) =>
      updateMember(id, {
        status,
        role: null,
        display_name: null,
      } satisfies UpdateLocalUserRequest),
    onSuccess: () => {
      setActionError(null);
      void invalidateMembers();
    },
    onError: (error: unknown) => {
      setActionError(displayAdminError(t, error, 'localAuth.networkError'));
    },
  });

  const resetPasswordMutation = useMutation({
    mutationFn: ({
      id,
      payload,
    }: {
      id: string;
      payload: AdminResetPasswordRequest;
    }) => resetMemberPassword(id, payload),
    onSuccess: () => {
      setResetPasswordTargetId(null);
      setResetPasswordValue('');
      setResetPasswordError(null);
    },
    onError: (error: unknown) => {
      setResetPasswordError(
        displayAdminError(t, error, 'localAuth.networkError')
      );
    },
  });

  const createInviteMutation = useMutation({
    mutationFn: (payload: CreateInviteRequest) => createInvite(payload),
    onSuccess: (result) => {
      setAddInviteOpen(false);
      setNewInvite(DEFAULT_NEW_INVITE);
      setNewInviteError(null);
      setCreatedInvite({
        code: result.code,
        link: buildInviteLink(window.location.origin, result.code),
      });
      void invalidateInvites();
    },
    onError: (error: unknown) => {
      setNewInviteError(displayAdminError(t, error, 'localAuth.networkError'));
    },
  });

  const deleteInviteMutation = useMutation({
    mutationFn: (id: string) => deleteInvite(id),
    onSuccess: () => {
      setActionError(null);
      void invalidateInvites();
    },
    onError: (error: unknown) => {
      setActionError(displayAdminError(t, error, 'localAuth.networkError'));
    },
  });

  const handleNewMemberChange = useCallback(
    <K extends keyof NewMemberFormValues>(
      field: K,
      value: NewMemberFormValues[K]
    ) => {
      setNewMember((prev) => ({ ...prev, [field]: value }));
    },
    []
  );

  const handleCreateMember = useCallback(() => {
    const rawErrors = validateAccountForm({
      username: newMember.username,
      password: newMember.password,
      email: newMember.email,
    });
    setNewMemberErrors({
      username: translateFieldError(t, rawErrors.username),
      password: translateFieldError(t, rawErrors.password),
      email: translateFieldError(t, rawErrors.email),
    });
    if (!isAccountFormValid(rawErrors)) return;

    setNewMemberError(null);
    const trimmedUsername = newMember.username.trim();
    const trimmedDisplayName = newMember.displayName.trim();
    const trimmedEmail = newMember.email.trim();
    createMemberMutation.mutate({
      username: trimmedUsername,
      display_name: trimmedDisplayName || trimmedUsername,
      email: trimmedEmail.length > 0 ? trimmedEmail : null,
      password: newMember.password,
      role: newMember.role,
    });
  }, [createMemberMutation, newMember, t]);

  const handleNewInviteChange = useCallback(
    <K extends keyof NewInviteFormValues>(
      field: K,
      value: NewInviteFormValues[K]
    ) => {
      setNewInvite((prev) => ({ ...prev, [field]: value }));
    },
    []
  );

  const handleCreateInvite = useCallback(() => {
    const errors = validateInviteForm(newInvite);
    if (!isInviteFormValid(errors)) {
      setNewInviteError(translateFieldError(t, errors.expiresInDays));
      return;
    }
    setNewInviteError(null);
    createInviteMutation.mutate({
      role: newInvite.role,
      // ts-rs 把 Rust i64 映射成 TS `bigint`；仓库里一律用普通 number 做
      // 类型转换（见 `useCreateAttachments.ts`），避免 `JSON.stringify`
      // 遇到真正的 BigInt 抛错。
      expires_in_days: newInvite.expiresInDays as unknown as bigint,
    });
  }, [createInviteMutation, newInvite, t]);

  const handleStartResetPassword = useCallback((memberId: string) => {
    setResetPasswordTargetId(memberId);
    setResetPasswordValue('');
    setResetPasswordError(null);
  }, []);

  const handleResetPasswordSubmit = useCallback(() => {
    if (!resetPasswordTargetId) return;
    const errors = validateResetPasswordForm(resetPasswordValue);
    if (errors.password) {
      setResetPasswordError(translateFieldError(t, errors.password));
      return;
    }
    setResetPasswordError(null);
    resetPasswordMutation.mutate({
      id: resetPasswordTargetId,
      payload: { new_password: resetPasswordValue },
    });
  }, [resetPasswordMutation, resetPasswordTargetId, resetPasswordValue, t]);

  if (!localSession) {
    // 只是防御性分支：这个页面只会在会话确认存在之后挂载。
    return null;
  }

  if (!isAdmin) {
    return <ForbiddenMessage t={t} onBackHome={() => navigate({ to: '/' })} />;
  }

  if (membersQuery.isPending) {
    return <FullPageMessage>{t('localAuth.loading')}</FullPageMessage>;
  }

  if (membersQuery.isError) {
    if (
      membersQuery.error instanceof AdminApiError &&
      membersQuery.error.status === 403
    ) {
      return (
        <ForbiddenMessage t={t} onBackHome={() => navigate({ to: '/' })} />
      );
    }
    return (
      <FullPageMessage>
        <div className="w-full max-w-md bg-secondary rounded border p-double text-center flex flex-col gap-base items-center">
          <p className="text-base text-error">
            {t('localAuth.members.loadError')}
          </p>
          <button
            type="button"
            className="h-cta px-base rounded border bg-panel text-base text-normal hover:text-high transition-colors"
            onClick={() => void membersQuery.refetch()}
          >
            {t('localAuth.members.retry')}
          </button>
        </div>
      </FullPageMessage>
    );
  }

  const actor = { id: localSession.user.id, role: localSession.user.role };
  const rawMembers: MemberLike[] = membersQuery.data.users.map((user) => ({
    id: user.id,
    username: user.username,
    role: user.role,
    status: user.status,
  }));

  const members: MembersPanelMember[] = sortMembers(
    membersQuery.data.users.map((user) => ({
      id: user.id,
      username: user.username,
      displayName: user.display_name,
      email: user.email,
      role: user.role as MembersPanelRole,
      status: user.status as MembersPanelStatus,
      avatarColor: user.avatar_color,
      lastLoginAt: user.last_login_at,
      isFixedLocal: isFixedLocalUser(user),
      canChangeRole: canChangeRole(actor, user, rawMembers),
      canDisable: canDisable(actor, user, rawMembers),
      // 后端对重置密码没有额外限制（不含自己、不含 local），前端不必挡。
      canResetPassword: true,
    }))
  );

  const invites: MembersPanelInvite[] = invitesQuery.data
    ? sortInvitesByCreatedAtDesc(invitesQuery.data.invites).map((invite) => ({
        id: invite.id,
        role: invite.role as MembersPanelRole,
        createdAt: invite.created_at,
        expiresAt: invite.expires_at,
        status: getInviteStatus(invite),
        canRevoke: canRevokeInvite(invite),
      }))
    : [];

  return (
    <MembersPanel
      members={members}
      invites={invites}
      invitesLoading={invitesQuery.isPending}
      invitesError={
        invitesQuery.isError ? t('localAuth.invites.loadError') : null
      }
      addMemberOpen={addMemberOpen}
      onToggleAddMember={(open) => {
        setAddMemberOpen(open);
        if (!open) {
          setNewMember(EMPTY_NEW_MEMBER);
          setNewMemberErrors(EMPTY_NEW_MEMBER_ERRORS);
          setNewMemberError(null);
        }
      }}
      newMember={newMember}
      newMemberErrors={newMemberErrors}
      newMemberSubmitting={createMemberMutation.isPending}
      newMemberError={newMemberError}
      onNewMemberChange={handleNewMemberChange}
      onCreateMember={handleCreateMember}
      onChangeRole={(memberId, role) =>
        changeRoleMutation.mutate({ id: memberId, role })
      }
      onToggleStatus={(memberId, nextStatus) =>
        toggleStatusMutation.mutate({ id: memberId, status: nextStatus })
      }
      actionError={actionError}
      resetPasswordTargetId={resetPasswordTargetId}
      resetPasswordValue={resetPasswordValue}
      resetPasswordError={resetPasswordError}
      resetPasswordSubmitting={resetPasswordMutation.isPending}
      onStartResetPassword={handleStartResetPassword}
      onResetPasswordChange={setResetPasswordValue}
      onResetPasswordCancel={() => {
        setResetPasswordTargetId(null);
        setResetPasswordValue('');
        setResetPasswordError(null);
      }}
      onResetPasswordSubmit={handleResetPasswordSubmit}
      addInviteOpen={addInviteOpen}
      onToggleAddInvite={(open) => {
        setAddInviteOpen(open);
        if (!open) {
          setNewInvite(DEFAULT_NEW_INVITE);
          setNewInviteError(null);
        }
      }}
      newInvite={newInvite}
      newInviteError={newInviteError}
      newInviteSubmitting={createInviteMutation.isPending}
      onNewInviteChange={handleNewInviteChange}
      onCreateInvite={handleCreateInvite}
      createdInvite={createdInvite}
      onDismissCreatedInvite={() => setCreatedInvite(null)}
      onRevokeInvite={(inviteId) => deleteInviteMutation.mutate(inviteId)}
    />
  );
}
