export type ElectricRow = Record<string, unknown> & { [key: string]: unknown };

/**
 * 集合的主键：优先 id，否则按字母序拼接所有 *_id 字段。
 * 与 TanStack DB 的 getKey 约定保持一致。
 */
export function getRowKey(item: Record<string, unknown>): string {
  if ('id' in item && item.id) {
    return String(item.id);
  }

  return Object.entries(item)
    .filter(([key]) => key.endsWith('_id'))
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([, value]) => String(value))
    .join('-');
}

/**
 * 快照响应必须形如 { "<表名>": [ ...行 ] }。
 * 本地接口（/api/local/*）与云端后备接口（/v1/fallback/*）共用这一约定。
 */
export function extractFallbackRows(
  payload: unknown,
  table: string
): Array<ElectricRow> {
  if (!payload || typeof payload !== 'object') {
    throw new Error(`Fallback response for "${table}" is not an object`);
  }

  const rows = (payload as Record<string, unknown>)[table];
  if (!Array.isArray(rows)) {
    throw new Error(`Fallback response missing "${table}" array`);
  }

  return rows as Array<ElectricRow>;
}

export async function parseResponseError(
  response: Response,
  fallbackMessage: string
): Promise<string> {
  try {
    const body = (await response.json()) as {
      message?: string;
      error?: string;
    };
    return body.message || body.error || fallbackMessage;
  } catch {
    return fallbackMessage;
  }
}
