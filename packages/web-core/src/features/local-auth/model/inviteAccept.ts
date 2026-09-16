/**
 * 邀请注册页（`/login?invite=<code>`）的纯逻辑：从 URL 取邀请码、个人版早退、
 * 错误码到展示方式的映射。
 *
 * 表单字段（用户名/密码/邮箱）本身的校验**不在这个文件里**——直接复用
 * `members.ts::validateAccountForm`（管理员建号表单同一套纯函数），不另写
 * 一套，避免两处校验规则跑偏。
 */

/**
 * 邀请码从 URL 的 `?invite=` 取；成员管理页生成的链接就是
 * `<base>/login?invite=<code>`（见 `members.ts::buildInviteLink`）。
 */
export function extractInviteCode(search: string): string | null {
  const code = new URLSearchParams(search).get('invite');
  if (!code) return null;
  const trimmed = code.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * 个人版没有账号体系，`?invite=` 参数在个人版应被忽略（后端这条路由本来
 * 就对个人版返回 404）。
 *
 * 这一层判断是双保险：`App.tsx` 已经用 `requiresLogin()` 在结构上保证个人版
 * 走不到 `LocalSessionProvider` 这整棵子树，本函数不是唯一的防线。之所以
 * 还在这里再判一次，是为了让「个人版忽略邀请参数」这条约束能被单测直接
 * 覆盖，不必依赖组件树是否挂载对。
 */
export function resolveInviteCode(
  search: string,
  isPersonal: boolean
): string | null {
  if (isPersonal) return null;
  return extractInviteCode(search);
}

export type InviteAcceptErrorDisplay =
  | { kind: 'message'; text: string }
  | { kind: 'i18nKey'; key: string };

/**
 * 接受邀请失败后怎么展示。
 *
 * 400（邀请码不存在/已过期/已使用——三种共用同一句；也覆盖弱密码、非法
 * 用户名）、409（用户名或邮箱被占用）、429（限速，文案里含剩余秒数）都是
 * 后端**特意写给人看的**中文文案，直接展示、不再翻译、也不再细分——尤其
 * 是 400 的邀请码类失败，前端**不得**试图猜是哪一种具体原因（防枚举，
 * 与 `crates/server/src/routes/local_auth/invite_routes.rs` 的设计一致）。
 *
 * 404（个人版）理论上不会出现：前端在渲染这个入口之前已经用
 * `resolveInviteCode` 挡掉了个人版。真出现（例如运行时模式发生了竞态）
 * 就落到兜底文案。
 */
export function describeInviteAcceptError(
  status: number,
  message: string
): InviteAcceptErrorDisplay {
  if (status === 400 || status === 409 || status === 429) {
    return { kind: 'message', text: message };
  }
  return { kind: 'i18nKey', key: 'localAuth.networkError' };
}
