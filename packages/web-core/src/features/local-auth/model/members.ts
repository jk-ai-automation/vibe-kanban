/**
 * 成员管理页的纯逻辑：排序、权限判定、表单校验、邀请状态打标。
 *
 * **前端的判定只是 UX**：真正的拦截在后端（`crates/server/src/routes/admin/*`），
 * 这里算错了顶多是按钮该灰没灰，不会绕过权限——但算法要尽量跟后端一致，
 * 否则会出现「按钮能点，点了却 403/409」的糟糕体验。
 */

/** 迁移写入的本机固定用户。团队模式下 MCP / 本机进程依赖它，界面上要单独标出来。 */
export const FIXED_LOCAL_USERNAME = 'local';

export type MemberRole = 'admin' | 'member';
export type MemberStatus = 'active' | 'disabled';

export interface ActorLike {
  id: string;
  role: string;
}

export interface MemberLike {
  id: string;
  username: string;
  role: string;
  status: string;
}

export function isFixedLocalUser(member: { username: string }): boolean {
  return member.username === FIXED_LOCAL_USERNAME;
}

/**
 * 「可登录的管理员」＝ active + admin + 不是本机固定用户。
 *
 * 这是后端 `count_login_capable_admins_except` 判据的前端镜像：后端按
 * 「有没有密码凭据」判断，前端拿不到这个字段（`AdminUserInfo` 不含
 * `password_hash`），但建号 / 邀请两条路径都**强制要求密码**，所以
 * 「不是 `local`」在前端语境下等价于「有凭据」。
 */
export function isLoginCapableAdmin(member: MemberLike): boolean {
  return (
    member.role === 'admin' &&
    member.status === 'active' &&
    !isFixedLocalUser(member)
  );
}

export function countLoginCapableAdminsExcept(
  members: MemberLike[],
  excludeId: string
): number {
  return members.filter(
    (member) => member.id !== excludeId && isLoginCapableAdmin(member)
  ).length;
}

/** 按角色（admin 在前）再按用户名排序，供列表展示。 */
export function sortMembers<T extends { role: string; username: string }>(
  members: T[]
): T[] {
  return [...members].sort((a, b) => {
    if (a.role !== b.role) return a.role === 'admin' ? -1 : 1;
    return a.username.localeCompare(b.username);
  });
}

export function canChangeRole(
  actor: ActorLike,
  target: MemberLike,
  members: MemberLike[]
): boolean {
  if (actor.role !== 'admin') return false;
  if (actor.id === target.id) return false;
  if (isFixedLocalUser(target)) return false;
  if (
    isLoginCapableAdmin(target) &&
    countLoginCapableAdminsExcept(members, target.id) === 0
  ) {
    return false;
  }
  return true;
}

/** 是否可以把 `target` 停用。启用（反方向）没有这些限制，调用方不必查这个函数。 */
export function canDisable(
  actor: ActorLike,
  target: MemberLike,
  members: MemberLike[]
): boolean {
  if (actor.id === target.id) return false;
  if (isFixedLocalUser(target)) return false;
  if (
    isLoginCapableAdmin(target) &&
    countLoginCapableAdminsExcept(members, target.id) === 0
  ) {
    return false;
  }
  return true;
}

// --------------------------------------------------------------- 表单校验

/** 与 `crates/db/src/models/local_user.rs::validate_username` 完全一致。 */
const USERNAME_PATTERN = /^[a-z0-9._-]+$/;
export const MAX_USERNAME_LEN = 64;
export const MIN_PASSWORD_BYTES = 8;
export const MAX_PASSWORD_BYTES = 1024;
const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const MAX_EMAIL_LEN = 254;

export function normalizeUsername(raw: string): string {
  return raw.trim().toLowerCase();
}

export function validateUsername(raw: string): string | undefined {
  const normalized = normalizeUsername(raw);
  if (normalized.length === 0) return 'errors.usernameRequired';
  if (normalized.length > MAX_USERNAME_LEN) return 'errors.usernameInvalid';
  if (!USERNAME_PATTERN.test(normalized)) return 'errors.usernameInvalid';
  return undefined;
}

/**
 * 密码长度按**字节**算，与后端 `services::local_auth::password`（Rust
 * `String::len()`，UTF-8 字节数）严格对齐——emoji / 中文密码按字符数算会
 * 在边界上和后端判断不一致。
 */
function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

export function validatePassword(raw: string): string | undefined {
  const bytes = utf8ByteLength(raw);
  if (bytes < MIN_PASSWORD_BYTES) return 'errors.passwordTooShort';
  if (bytes > MAX_PASSWORD_BYTES) return 'errors.passwordTooLong';
  return undefined;
}

/** 邮箱是可选字段：空串永远合法。 */
export function validateEmailOptional(raw: string): string | undefined {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return undefined;
  if (trimmed.length > MAX_EMAIL_LEN) return 'errors.emailInvalid';
  if (!EMAIL_PATTERN.test(trimmed)) return 'errors.emailInvalid';
  return undefined;
}

export interface AccountFormValues {
  username: string;
  password: string;
  email: string;
}

export interface AccountFormErrors {
  username?: string;
  password?: string;
  email?: string;
}

/** 建号（管理员建号 / 首启向导）共用的账号三件套校验：用户名 + 密码 + 邮箱。 */
export function validateAccountForm(
  values: AccountFormValues
): AccountFormErrors {
  const errors: AccountFormErrors = {};
  const usernameError = validateUsername(values.username);
  if (usernameError) errors.username = usernameError;
  const passwordError = validatePassword(values.password);
  if (passwordError) errors.password = passwordError;
  const emailError = validateEmailOptional(values.email);
  if (emailError) errors.email = emailError;
  return errors;
}

export function isAccountFormValid(errors: AccountFormErrors): boolean {
  return Object.keys(errors).length === 0;
}

export interface EditMemberFormValues {
  displayName: string;
  email: string;
}

export interface EditMemberFormErrors {
  displayName?: string;
  email?: string;
}

export function validateEditMemberForm(
  values: EditMemberFormValues
): EditMemberFormErrors {
  const errors: EditMemberFormErrors = {};
  if (values.displayName.trim().length === 0) {
    errors.displayName = 'errors.displayNameRequired';
  }
  const emailError = validateEmailOptional(values.email);
  if (emailError) errors.email = emailError;
  return errors;
}

export function isEditMemberFormValid(errors: EditMemberFormErrors): boolean {
  return Object.keys(errors).length === 0;
}

export function validateResetPasswordForm(password: string): {
  password?: string;
} {
  const error = validatePassword(password);
  return error ? { password: error } : {};
}

// ----------------------------------------------------------------- 邀请码

export const MIN_INVITE_TTL_DAYS = 1;
/** 必须与 `crates/server/src/routes/admin/invites.rs::MAX_INVITE_TTL_DAYS` 一致。 */
export const MAX_INVITE_TTL_DAYS = 30;
/** 必须与 `crates/db/src/models/local_auth.rs::DEFAULT_INVITE_TTL_DAYS` 一致。 */
export const DEFAULT_INVITE_TTL_DAYS = 7;

export interface InviteFormValues {
  role: MemberRole;
  expiresInDays: number;
}

export interface InviteFormErrors {
  expiresInDays?: string;
}

export function validateInviteForm(values: InviteFormValues): InviteFormErrors {
  const errors: InviteFormErrors = {};
  if (
    !Number.isInteger(values.expiresInDays) ||
    values.expiresInDays < MIN_INVITE_TTL_DAYS ||
    values.expiresInDays > MAX_INVITE_TTL_DAYS
  ) {
    errors.expiresInDays = 'errors.inviteTtlInvalid';
  }
  return errors;
}

export function isInviteFormValid(errors: InviteFormErrors): boolean {
  return Object.keys(errors).length === 0;
}

export type InviteStatus = 'pending' | 'used' | 'expired';

export interface InviteLike {
  used_at: string | null;
  expires_at: string;
}

export function getInviteStatus(
  invite: InviteLike,
  now: Date = new Date()
): InviteStatus {
  if (invite.used_at) return 'used';
  if (new Date(invite.expires_at).getTime() <= now.getTime()) {
    return 'expired';
  }
  return 'pending';
}

/** 已使用的邀请没有「作废」的意义：邀请码已经消费掉了。 */
export function canRevokeInvite(invite: InviteLike): boolean {
  return invite.used_at === null;
}

/** 按创建时间倒序排列（新邀请在前）。 */
export function sortInvitesByCreatedAtDesc<T extends { created_at: string }>(
  invites: T[]
): T[] {
  return [...invites].sort(
    (a, b) =>
      new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
  );
}

/**
 * 邀请链接。与后端 `routes/local_auth/setup.rs::setup_link` 同样的拼接方式
 * （去掉 base 末尾的斜杠，避免拼出 `//login`），但走 `/login?invite=` 而不是
 * `?setup=`。
 */
export function buildInviteLink(origin: string, code: string): string {
  return `${origin.replace(/\/+$/, '')}/login?invite=${code}`;
}

/**
 * 把表单校验函数返回的相对 key（`'errors.xxx'`）翻成界面文案。
 *
 * **故意写成穷举 switch + 字面量 `t('localAuth.errors.xxx')`**，不用
 * `t(`localAuth.${key}`)` 这种模板字符串拼接——`check-unused-i18n-keys.mjs`
 * 对模板字面量一律判定为「动态使用」，会让真正没人引用的 key 也检测不出来。
 * 穷举字面量能让脚本按字面量老老实实地抓引用。
 */
export function translateFieldError(
  t: (key: string) => string,
  key: string | undefined
): string | null {
  switch (key) {
    case undefined:
      return null;
    case 'errors.usernameRequired':
      return t('localAuth.errors.usernameRequired');
    case 'errors.usernameInvalid':
      return t('localAuth.errors.usernameInvalid');
    case 'errors.passwordTooShort':
      return t('localAuth.errors.passwordTooShort');
    case 'errors.passwordTooLong':
      return t('localAuth.errors.passwordTooLong');
    case 'errors.emailInvalid':
      return t('localAuth.errors.emailInvalid');
    case 'errors.displayNameRequired':
      return t('localAuth.errors.displayNameRequired');
    case 'errors.inviteTtlInvalid':
      return t('localAuth.errors.inviteTtlInvalid');
    default:
      return key;
  }
}

// ------------------------------------------------------------- 错误展示

export interface AdminOperationError {
  status: number;
  message: string;
}

export type AdminErrorDisplay =
  | { kind: 'message'; text: string }
  | { kind: 'i18nKey'; key: string };

/**
 * 管理接口的错误怎么展示。
 *
 * 400 / 403 / 404 / 409 的后端文案是**特意写给人看的**（“不能停用自己”
 * “至少保留一个可用管理员”“用户名已被占用”……），可以直接展示，不必翻译成
 * 通用文案——这一点和登录页刻意模糊四种失败原因**正好相反**。
 * 其余状态码（网络错误、5xx、意外的 401）给一句通用兜底文案。
 */
export function describeAdminError(
  error: AdminOperationError
): AdminErrorDisplay {
  if (
    error.status === 400 ||
    error.status === 403 ||
    error.status === 404 ||
    error.status === 409
  ) {
    return { kind: 'message', text: error.message };
  }
  return { kind: 'i18nKey', key: 'localAuth.networkError' };
}
