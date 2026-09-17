import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  CaretDownIcon,
  CheckIcon,
  CopyIcon,
  SpinnerIcon,
} from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './Dropdown';

export type MembersPanelRole = 'admin' | 'member';
export type MembersPanelStatus = 'active' | 'disabled';
export type MembersPanelInviteStatus = 'pending' | 'used' | 'expired';

export interface MembersPanelMember {
  id: string;
  username: string;
  displayName: string;
  email: string | null;
  role: MembersPanelRole;
  status: MembersPanelStatus;
  avatarColor: string;
  lastLoginAt: string | null;
  isFixedLocal: boolean;
  canChangeRole: boolean;
  /** 仅当 `status==='active'` 时有意义：能不能停用。 */
  canDisable: boolean;
  canResetPassword: boolean;
}

export interface MembersPanelInvite {
  id: string;
  role: MembersPanelRole;
  createdAt: string;
  expiresAt: string;
  status: MembersPanelInviteStatus;
  canRevoke: boolean;
}

export interface NewMemberFormValues {
  username: string;
  displayName: string;
  email: string;
  password: string;
  role: MembersPanelRole;
}

export interface NewMemberFormErrors {
  username?: string | null;
  displayName?: string | null;
  email?: string | null;
  password?: string | null;
}

export interface NewInviteFormValues {
  role: MembersPanelRole;
  expiresInDays: number;
}

export interface CreatedInvite {
  code: string;
  link: string;
}

export interface MembersPanelProps {
  members: MembersPanelMember[];
  invites: MembersPanelInvite[];
  invitesLoading: boolean;
  invitesError: string | null;

  addMemberOpen: boolean;
  onToggleAddMember: (open: boolean) => void;
  newMember: NewMemberFormValues;
  newMemberErrors: NewMemberFormErrors;
  newMemberSubmitting: boolean;
  newMemberError: string | null;
  onNewMemberChange: <K extends keyof NewMemberFormValues>(
    field: K,
    value: NewMemberFormValues[K]
  ) => void;
  onCreateMember: () => void;

  onChangeRole: (memberId: string, role: MembersPanelRole) => void;
  onToggleStatus: (memberId: string, nextStatus: MembersPanelStatus) => void;
  actionError: string | null;

  resetPasswordTargetId: string | null;
  resetPasswordValue: string;
  resetPasswordError: string | null;
  resetPasswordSubmitting: boolean;
  onStartResetPassword: (memberId: string) => void;
  onResetPasswordChange: (value: string) => void;
  onResetPasswordCancel: () => void;
  onResetPasswordSubmit: () => void;

  addInviteOpen: boolean;
  onToggleAddInvite: (open: boolean) => void;
  newInvite: NewInviteFormValues;
  newInviteError: string | null;
  newInviteSubmitting: boolean;
  onNewInviteChange: <K extends keyof NewInviteFormValues>(
    field: K,
    value: NewInviteFormValues[K]
  ) => void;
  onCreateInvite: () => void;
  createdInvite: CreatedInvite | null;
  onDismissCreatedInvite: () => void;
  onRevokeInvite: (inviteId: string) => void;
}

const inputClass = cn(
  'w-full h-cta px-base bg-panel rounded border text-base text-normal',
  'placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand'
);

const secondaryButtonClass = cn(
  'h-cta px-base rounded border bg-panel text-base text-normal',
  'hover:text-high transition-colors',
  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
  'disabled:opacity-50 disabled:cursor-not-allowed'
);

const primaryButtonClass = cn(
  'h-cta px-base rounded bg-brand text-on-brand text-base font-medium',
  'hover:bg-brand-hover transition-colors',
  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
  'disabled:opacity-50 disabled:cursor-not-allowed',
  'flex items-center justify-center gap-half'
);

function Avatar({
  displayName,
  color,
}: {
  displayName: string;
  color: string;
}) {
  const initial =
    Array.from(displayName.trim() || '?')[0]?.toUpperCase() ?? '?';
  return (
    <span
      className="flex size-icon-lg shrink-0 items-center justify-center rounded-full text-xs font-medium text-white"
      style={{ backgroundColor: color }}
      aria-hidden="true"
    >
      {initial}
    </span>
  );
}

function RoleBadge({
  role,
  t,
}: {
  role: MembersPanelRole;
  t: (key: string) => string;
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center rounded px-half py-0 text-sm font-medium',
        role === 'admin' ? 'bg-brand/15 text-brand' : 'bg-secondary text-low'
      )}
    >
      {role === 'admin'
        ? t('localAuth.members.roleAdmin')
        : t('localAuth.members.roleMember')}
    </span>
  );
}

function StatusBadge({
  status,
  t,
}: {
  status: MembersPanelStatus;
  t: (key: string) => string;
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center rounded px-half py-0 text-sm font-medium',
        status === 'active'
          ? 'bg-success/15 text-success'
          : 'bg-secondary text-low'
      )}
    >
      {status === 'active'
        ? t('localAuth.members.statusActive')
        : t('localAuth.members.statusDisabled')}
    </span>
  );
}

function InviteStatusBadge({
  status,
  t,
}: {
  status: MembersPanelInviteStatus;
  t: (key: string) => string;
}) {
  const key =
    status === 'pending'
      ? 'statusPending'
      : status === 'used'
        ? 'statusUsed'
        : 'statusExpired';
  return (
    <span
      className={cn(
        'inline-flex items-center rounded px-half py-0 text-sm font-medium',
        status === 'pending'
          ? 'bg-success/15 text-success'
          : 'bg-secondary text-low'
      )}
    >
      {t(`localAuth.invites.${key}`)}
    </span>
  );
}

function formatDateTime(value: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleString();
}

/**
 * 成员管理界面（无状态视图）。
 *
 * 所有表单字段状态与校验都由容器（`MembersPageContainer`）驱动——包括
 * 「添加成员」「重置密码」「生成邀请码」三个小表单——这个组件只负责渲染与
 * 转发事件，和 `LoginPanel` 同样的分层方式。角色下拉与停用/启用按钮的
 * 可用性（`canChangeRole` / `canDisable`）由容器算好直接传下来，`local`
 * 固定账号那一行永远收到 `false`。
 */
export function MembersPanel({
  members,
  invites,
  invitesLoading,
  invitesError,
  addMemberOpen,
  onToggleAddMember,
  newMember,
  newMemberErrors,
  newMemberSubmitting,
  newMemberError,
  onNewMemberChange,
  onCreateMember,
  onChangeRole,
  onToggleStatus,
  actionError,
  resetPasswordTargetId,
  resetPasswordValue,
  resetPasswordError,
  resetPasswordSubmitting,
  onStartResetPassword,
  onResetPasswordChange,
  onResetPasswordCancel,
  onResetPasswordSubmit,
  addInviteOpen,
  onToggleAddInvite,
  newInvite,
  newInviteError,
  newInviteSubmitting,
  onNewInviteChange,
  onCreateInvite,
  createdInvite,
  onDismissCreatedInvite,
  onRevokeInvite,
}: MembersPanelProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopyLink = () => {
    if (!createdInvite) return;
    navigator.clipboard
      .writeText(createdInvite.link)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => {
        /* 剪贴板不可用时用户仍能手动选中文本复制，静默失败即可。 */
      });
  };

  return (
    <div className="h-full overflow-auto bg-primary text-normal">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-double p-double">
        <header>
          <h1 className="text-xl font-medium text-high">
            {t('localAuth.members.pageTitle')}
          </h1>
        </header>

        {actionError && (
          <p role="alert" className="text-base text-error">
            {actionError}
          </p>
        )}

        {/* ------------------------------------------------------ 成员 */}
        <section className="flex flex-col gap-base rounded border bg-secondary p-double">
          <div className="flex items-center justify-between gap-base">
            <h2 className="text-base font-medium text-high">
              {t('localAuth.members.sectionMembers')}
            </h2>
            <button
              type="button"
              className={secondaryButtonClass}
              onClick={() => onToggleAddMember(!addMemberOpen)}
            >
              {t('localAuth.members.addMember')}
            </button>
          </div>

          {addMemberOpen && (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                onCreateMember();
              }}
              className="flex flex-col gap-base rounded border bg-panel p-base"
            >
              <div className="grid grid-cols-1 gap-base sm:grid-cols-2">
                <div className="flex flex-col gap-half">
                  <input
                    className={inputClass}
                    placeholder={t('localAuth.members.usernamePlaceholder')}
                    value={newMember.username}
                    autoComplete="off"
                    onChange={(event) =>
                      onNewMemberChange('username', event.target.value)
                    }
                    aria-invalid={Boolean(newMemberErrors.username)}
                  />
                  {newMemberErrors.username && (
                    <p className="text-sm text-error">
                      {newMemberErrors.username}
                    </p>
                  )}
                </div>
                <div className="flex flex-col gap-half">
                  <input
                    className={inputClass}
                    placeholder={t('localAuth.members.displayNamePlaceholder')}
                    value={newMember.displayName}
                    onChange={(event) =>
                      onNewMemberChange('displayName', event.target.value)
                    }
                    aria-invalid={Boolean(newMemberErrors.displayName)}
                  />
                  {newMemberErrors.displayName && (
                    <p className="text-sm text-error">
                      {newMemberErrors.displayName}
                    </p>
                  )}
                </div>
                <div className="flex flex-col gap-half">
                  <input
                    className={inputClass}
                    placeholder={t('localAuth.members.emailPlaceholder')}
                    value={newMember.email}
                    type="email"
                    onChange={(event) =>
                      onNewMemberChange('email', event.target.value)
                    }
                    aria-invalid={Boolean(newMemberErrors.email)}
                  />
                  {newMemberErrors.email && (
                    <p className="text-sm text-error">
                      {newMemberErrors.email}
                    </p>
                  )}
                </div>
                <div className="flex flex-col gap-half">
                  <input
                    className={inputClass}
                    placeholder={t('localAuth.members.passwordPlaceholder')}
                    value={newMember.password}
                    type="password"
                    autoComplete="new-password"
                    onChange={(event) =>
                      onNewMemberChange('password', event.target.value)
                    }
                    aria-invalid={Boolean(newMemberErrors.password)}
                  />
                  {newMemberErrors.password && (
                    <p className="text-sm text-error">
                      {newMemberErrors.password}
                    </p>
                  )}
                </div>
              </div>

              <div className="flex items-center gap-base">
                <span className="text-sm text-low">
                  {t('localAuth.members.columnRole')}
                </span>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button type="button" className={secondaryButtonClass}>
                      {newMember.role === 'admin'
                        ? t('localAuth.members.roleAdmin')
                        : t('localAuth.members.roleMember')}
                      <CaretDownIcon
                        className="ml-half size-icon-2xs"
                        weight="bold"
                      />
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent>
                    <DropdownMenuItem
                      onClick={() => onNewMemberChange('role', 'member')}
                    >
                      {t('localAuth.members.roleMember')}
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => onNewMemberChange('role', 'admin')}
                    >
                      {t('localAuth.members.roleAdmin')}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>

              {newMemberError && (
                <p role="alert" className="text-base text-error">
                  {newMemberError}
                </p>
              )}

              <div className="flex justify-end gap-half">
                <button
                  type="button"
                  className={secondaryButtonClass}
                  onClick={() => onToggleAddMember(false)}
                  disabled={newMemberSubmitting}
                >
                  {t('localAuth.members.cancel')}
                </button>
                <button
                  type="submit"
                  className={primaryButtonClass}
                  disabled={newMemberSubmitting}
                >
                  {newMemberSubmitting && (
                    <SpinnerIcon className="size-icon-xs animate-spin" />
                  )}
                  {newMemberSubmitting
                    ? t('localAuth.members.creating')
                    : t('localAuth.members.create')}
                </button>
              </div>
            </form>
          )}

          <div className="overflow-x-auto">
            <table className="w-full text-left text-base">
              <thead>
                <tr className="border-b text-sm text-low">
                  <th className="py-half pr-base font-medium">
                    {t('localAuth.members.columnUser')}
                  </th>
                  <th className="py-half pr-base font-medium">
                    {t('localAuth.members.columnEmail')}
                  </th>
                  <th className="py-half pr-base font-medium">
                    {t('localAuth.members.columnRole')}
                  </th>
                  <th className="py-half pr-base font-medium">
                    {t('localAuth.members.columnStatus')}
                  </th>
                  <th className="py-half pr-base font-medium">
                    {t('localAuth.members.columnLastLogin')}
                  </th>
                  <th className="py-half font-medium">
                    {t('localAuth.members.columnActions')}
                  </th>
                </tr>
              </thead>
              <tbody>
                {members.length === 0 && (
                  <tr>
                    <td colSpan={6} className="py-double text-center text-low">
                      {t('localAuth.members.empty')}
                    </td>
                  </tr>
                )}
                {members.map((member) => (
                  <tr key={member.id} className="border-b last:border-b-0">
                    <td className="py-half pr-base">
                      <div className="flex items-center gap-half">
                        <Avatar
                          displayName={member.displayName}
                          color={member.avatarColor}
                        />
                        <div className="min-w-0">
                          <div className="flex items-center gap-half">
                            <span className="truncate font-medium text-high">
                              {member.displayName}
                            </span>
                            {member.isFixedLocal && (
                              <span
                                className="inline-flex items-center rounded bg-secondary px-half py-0 text-sm text-low"
                                title={t('localAuth.members.fixedLocalHint')}
                              >
                                {t('localAuth.members.fixedLocalBadge')}
                              </span>
                            )}
                          </div>
                          <div className="truncate text-sm text-low">
                            @{member.username}
                          </div>
                        </div>
                      </div>
                    </td>
                    <td className="py-half pr-base text-sm text-low">
                      {member.email ?? ''}
                    </td>
                    <td className="py-half pr-base">
                      {member.canChangeRole ? (
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <button
                              type="button"
                              className={secondaryButtonClass}
                            >
                              {member.role === 'admin'
                                ? t('localAuth.members.roleAdmin')
                                : t('localAuth.members.roleMember')}
                              <CaretDownIcon
                                className="ml-half size-icon-2xs"
                                weight="bold"
                              />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent>
                            <DropdownMenuItem
                              icon={
                                member.role === 'member' ? CheckIcon : undefined
                              }
                              onClick={() => onChangeRole(member.id, 'member')}
                            >
                              {t('localAuth.members.roleMember')}
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              icon={
                                member.role === 'admin' ? CheckIcon : undefined
                              }
                              onClick={() => onChangeRole(member.id, 'admin')}
                            >
                              {t('localAuth.members.roleAdmin')}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      ) : (
                        <RoleBadge role={member.role} t={t} />
                      )}
                    </td>
                    <td className="py-half pr-base">
                      <StatusBadge status={member.status} t={t} />
                    </td>
                    <td className="py-half pr-base text-sm text-low">
                      {member.lastLoginAt
                        ? formatDateTime(member.lastLoginAt)
                        : t('localAuth.members.never')}
                    </td>
                    <td className="py-half">
                      <div className="flex flex-wrap items-center gap-half">
                        {member.status === 'active' ? (
                          <button
                            type="button"
                            className={secondaryButtonClass}
                            disabled={!member.canDisable}
                            title={
                              member.isFixedLocal
                                ? t('localAuth.members.fixedLocalHint')
                                : undefined
                            }
                            onClick={() =>
                              onToggleStatus(member.id, 'disabled')
                            }
                          >
                            {t('localAuth.members.disable')}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className={secondaryButtonClass}
                            disabled={member.isFixedLocal}
                            onClick={() => onToggleStatus(member.id, 'active')}
                          >
                            {t('localAuth.members.enable')}
                          </button>
                        )}
                        {member.canResetPassword && (
                          <button
                            type="button"
                            className={secondaryButtonClass}
                            onClick={() => onStartResetPassword(member.id)}
                          >
                            {t('localAuth.members.resetPassword')}
                          </button>
                        )}
                      </div>
                      {resetPasswordTargetId === member.id && (
                        <form
                          onSubmit={(event) => {
                            event.preventDefault();
                            onResetPasswordSubmit();
                          }}
                          className="mt-half flex flex-col gap-half rounded border bg-panel p-half"
                        >
                          <p className="text-sm text-low">
                            {t('localAuth.members.resetPasswordHint')}
                          </p>
                          <input
                            className={inputClass}
                            type="password"
                            autoFocus
                            autoComplete="new-password"
                            placeholder={t('localAuth.members.newPassword')}
                            value={resetPasswordValue}
                            onChange={(event) =>
                              onResetPasswordChange(event.target.value)
                            }
                            aria-invalid={Boolean(resetPasswordError)}
                          />
                          {resetPasswordError && (
                            <p className="text-sm text-error">
                              {resetPasswordError}
                            </p>
                          )}
                          <div className="flex justify-end gap-half">
                            <button
                              type="button"
                              className={secondaryButtonClass}
                              onClick={onResetPasswordCancel}
                              disabled={resetPasswordSubmitting}
                            >
                              {t('localAuth.members.cancel')}
                            </button>
                            <button
                              type="submit"
                              className={primaryButtonClass}
                              disabled={resetPasswordSubmitting}
                            >
                              {resetPasswordSubmitting && (
                                <SpinnerIcon className="size-icon-xs animate-spin" />
                              )}
                              {t('localAuth.members.resetPasswordSubmit')}
                            </button>
                          </div>
                        </form>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        {/* ------------------------------------------------------ 邀请码 */}
        <section className="flex flex-col gap-base rounded border bg-secondary p-double">
          <div className="flex items-center justify-between gap-base">
            <h2 className="text-base font-medium text-high">
              {t('localAuth.members.sectionInvites')}
            </h2>
            <button
              type="button"
              className={secondaryButtonClass}
              onClick={() => onToggleAddInvite(!addInviteOpen)}
            >
              {t('localAuth.invites.addInvite')}
            </button>
          </div>

          {createdInvite && (
            <div className="flex flex-col gap-half rounded border border-brand bg-panel p-base">
              <p className="text-base text-normal">
                {t('localAuth.invites.linkReady')}
              </p>
              <div className="flex flex-wrap items-center gap-half">
                <code className="flex-1 break-all rounded bg-secondary px-half py-half text-sm text-high">
                  {createdInvite.link}
                </code>
                <button
                  type="button"
                  className={secondaryButtonClass}
                  onClick={handleCopyLink}
                >
                  <CopyIcon className="size-icon-xs" weight="bold" />
                  {copied
                    ? t('localAuth.invites.copied')
                    : t('localAuth.invites.copy')}
                </button>
              </div>
              <div className="flex justify-end">
                <button
                  type="button"
                  className={secondaryButtonClass}
                  onClick={onDismissCreatedInvite}
                >
                  {t('localAuth.invites.dismiss')}
                </button>
              </div>
            </div>
          )}

          {addInviteOpen && (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                onCreateInvite();
              }}
              className="flex flex-col gap-base rounded border bg-panel p-base sm:flex-row sm:items-end"
            >
              <div className="flex flex-col gap-half">
                <span className="text-sm text-low">
                  {t('localAuth.invites.role')}
                </span>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button type="button" className={secondaryButtonClass}>
                      {newInvite.role === 'admin'
                        ? t('localAuth.members.roleAdmin')
                        : t('localAuth.members.roleMember')}
                      <CaretDownIcon
                        className="ml-half size-icon-2xs"
                        weight="bold"
                      />
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent>
                    <DropdownMenuItem
                      onClick={() => onNewInviteChange('role', 'member')}
                    >
                      {t('localAuth.members.roleMember')}
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => onNewInviteChange('role', 'admin')}
                    >
                      {t('localAuth.members.roleAdmin')}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
              <div className="flex flex-col gap-half">
                <span className="text-sm text-low">
                  {t('localAuth.invites.expiresInDays')}
                </span>
                <input
                  className={cn(inputClass, 'w-24')}
                  type="number"
                  min={1}
                  max={30}
                  value={newInvite.expiresInDays}
                  onChange={(event) =>
                    onNewInviteChange(
                      'expiresInDays',
                      Number(event.target.value)
                    )
                  }
                />
              </div>
              {newInviteError && (
                <p role="alert" className="text-base text-error">
                  {newInviteError}
                </p>
              )}
              <div className="flex gap-half sm:ml-auto">
                <button
                  type="button"
                  className={secondaryButtonClass}
                  onClick={() => onToggleAddInvite(false)}
                  disabled={newInviteSubmitting}
                >
                  {t('localAuth.members.cancel')}
                </button>
                <button
                  type="submit"
                  className={primaryButtonClass}
                  disabled={newInviteSubmitting}
                >
                  {newInviteSubmitting && (
                    <SpinnerIcon className="size-icon-xs animate-spin" />
                  )}
                  {newInviteSubmitting
                    ? t('localAuth.invites.creating')
                    : t('localAuth.invites.create')}
                </button>
              </div>
            </form>
          )}

          {invitesError && (
            <p role="alert" className="text-base text-error">
              {invitesError}
            </p>
          )}

          {invitesLoading ? (
            <p className="text-base text-low">{t('localAuth.loading')}</p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-base">
                <thead>
                  <tr className="border-b text-sm text-low">
                    <th className="py-half pr-base font-medium">
                      {t('localAuth.invites.columnRole')}
                    </th>
                    <th className="py-half pr-base font-medium">
                      {t('localAuth.invites.columnCreatedAt')}
                    </th>
                    <th className="py-half pr-base font-medium">
                      {t('localAuth.invites.columnExpiresAt')}
                    </th>
                    <th className="py-half pr-base font-medium">
                      {t('localAuth.invites.columnStatus')}
                    </th>
                    <th className="py-half font-medium">
                      {t('localAuth.invites.columnActions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {invites.length === 0 && (
                    <tr>
                      <td
                        colSpan={5}
                        className="py-double text-center text-low"
                      >
                        {t('localAuth.invites.empty')}
                      </td>
                    </tr>
                  )}
                  {invites.map((invite) => (
                    <tr key={invite.id} className="border-b last:border-b-0">
                      <td className="py-half pr-base">
                        <RoleBadge role={invite.role} t={t} />
                      </td>
                      <td className="py-half pr-base text-sm text-low">
                        {formatDateTime(invite.createdAt)}
                      </td>
                      <td className="py-half pr-base text-sm text-low">
                        {formatDateTime(invite.expiresAt)}
                      </td>
                      <td className="py-half pr-base">
                        <InviteStatusBadge status={invite.status} t={t} />
                      </td>
                      <td className="py-half">
                        {invite.canRevoke && (
                          <button
                            type="button"
                            className={secondaryButtonClass}
                            onClick={() => onRevokeInvite(invite.id)}
                          >
                            {t('localAuth.invites.revoke')}
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
