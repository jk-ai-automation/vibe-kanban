/**
 * 从失败响应里取出 `ApiResponse.message`，供**明确允许展示后端文案**的调用方
 * （管理接口、首启向导）使用。
 *
 * 和 `bootstrapApi.ts` 里的 `readEnvelope` 刻意不同：那个用于登录等「四种
 * 失败必须保持不可区分」的场景，特意丢掉 message；这里相反，message 本身
 * 就是要展示给用户看的内容（“不能停用自己”“用户名已被占用”……）。
 */
export interface EnvelopeError {
  status: number;
  message: string | null;
}

export async function parseEnvelopeError(
  response: Response
): Promise<EnvelopeError> {
  let message: string | null = null;
  try {
    const body: unknown = await response.json();
    if (
      body &&
      typeof body === 'object' &&
      'message' in body &&
      typeof (body as { message: unknown }).message === 'string'
    ) {
      message = (body as { message: string }).message;
    }
  } catch {
    message = null;
  }
  return { status: response.status, message };
}
