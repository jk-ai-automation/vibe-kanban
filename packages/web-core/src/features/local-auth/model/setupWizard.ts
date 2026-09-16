/** 首启初始化向导的纯逻辑：从 URL 取令牌、识别「已初始化」这一句特定文案。 */

/** 令牌从 URL 的 `?setup=` 取；`main.rs` 打印的链接就是 `<base>/login?setup=<token>`。 */
export function extractSetupToken(search: string): string | null {
  const token = new URLSearchParams(search).get('setup');
  if (!token) return null;
  const trimmed = token.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * 与 `crates/server/src/routes/local_auth/setup.rs::ALREADY_INITIALIZED`
 * 逐字一致。
 *
 * `POST /api/local-auth/setup` 的 409 有两种含义不同的原因——「已经有人
 * 初始化过了」（该去登录页）和「用户名被占用」（该换个用户名重试）——
 * 状态码相同，只能靠这句**后端刻意保证唯一**的文案区分，这是本页面
 * 唯一一处「按 message 文本分支」，且是必要的、经过设计的例外。
 */
const ALREADY_INITIALIZED_MESSAGE = '已初始化';

export function isAlreadyInitializedMessage(
  message: string | null | undefined
): boolean {
  return message === ALREADY_INITIALIZED_MESSAGE;
}
