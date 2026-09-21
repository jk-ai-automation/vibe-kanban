/**
 * scratch（服务端保存的界面偏好）拉回来时，怎么处理「选中的组织 / 项目」。
 *
 * 背景：scratch 走 WebSocket，通常比首屏晚到。在它到达之前，store 里的选择
 * 可能来自两类写入：
 *
 * - **回落**（`fallback`）：自动挑的第一个组织 / 项目（`SharedAppLayout` 的
 *   组织自动选中、`getFirstProjectDestination` 的兜底、组织 store 的本地镜像）。
 *   这类值不是用户的意思，绝不能盖掉服务端存的「上次选中」。
 * - **用户意图**（`user`）：用户显式切换项目，或直接用链接进了某个项目
 *   （`SharedAppLayout` 会把路由里的项目写进来）。这类值应当保留，并且要回存
 *   到服务端——它是在 hydration 之前写的，不会触发「store 变化 → 保存」的订阅。
 *
 * 组织一律以服务端为准：目前所有 hydration 之前的组织写入都是回落或本地镜像，
 * 用户显式切换组织发生在 hydration 之后，会走正常的保存订阅。
 */
export type SelectionSource = 'fallback' | 'user';

export interface ScratchSelectionInput {
  serverOrgId: string | null;
  serverProjectId: string | null;
  sessionOrgId: string | null;
  sessionProjectId: string | null;
  sessionProjectSource: SelectionSource;
}

export interface ScratchSelectionResult {
  orgId: string | null;
  projectId: string | null;
  /** 需要把保留下来的会话选择回存到服务端。 */
  shouldSave: boolean;
}

export function resolveScratchSelection(
  input: ScratchSelectionInput
): ScratchSelectionResult {
  const keepSessionProject =
    input.sessionProjectSource === 'user' && input.sessionProjectId !== null;

  const projectId = keepSessionProject
    ? input.sessionProjectId
    : input.serverProjectId;

  return {
    orgId: input.serverOrgId,
    projectId,
    shouldSave: keepSessionProject && projectId !== input.serverProjectId,
  };
}
