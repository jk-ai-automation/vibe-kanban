import { createCollection } from '@tanstack/react-db';
import type { Operation } from 'rfc6902';
import type { MutationDefinition, ShapeDefinition } from 'shared/remote-types';

import {
  type ElectricRow,
  getRowKey,
  extractFallbackRows,
  parseResponseError,
} from '@/shared/lib/electric/rows';
import {
  makeLocalApiRequest,
  openLocalApiWebSocket,
} from '@/shared/lib/localApiTransport';
import {
  resolveLocalMutationUrl,
  resolveLocalShapeEndpoint,
} from '@/shared/lib/local/localEndpoints';
import type { CollectionConfig, SyncError } from '@/shared/lib/electric/types';

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const;
/** WS 断线时的兜底轮询间隔。 */
const POLL_INTERVAL_MS = 15 * 1000;
/** WS 重连退避上限。 */
const MAX_RECONNECT_DELAY_MS = 8000;

export type LocalWrite =
  | { type: 'truncate' }
  | { type: 'insert'; value: ElectricRow }
  | { type: 'update'; value: ElectricRow }
  | { type: 'delete'; value: ElectricRow };

function unescapePointerSegment(segment: string): string {
  return segment.replace(/~1/g, '/').replace(/~0/g, '~');
}

/**
 * 把后端推来的 JSON Patch 转成集合写操作。
 * 只处理属于本表的路径：/<table> 整表替换，/<table>/<id> 单行增删改。
 */
export function patchToWrites(
  operations: Operation[],
  table: string
): LocalWrite[] {
  const writes: LocalWrite[] = [];
  const rootPath = `/${table}`;

  for (const operation of operations) {
    const path = operation.path;
    if (path !== rootPath && !path.startsWith(`${rootPath}/`)) {
      continue;
    }

    const isRoot = path === rootPath;
    const id = isRoot
      ? null
      : unescapePointerSegment(path.slice(rootPath.length + 1));

    if (isRoot) {
      if (operation.op !== 'replace' && operation.op !== 'add') continue;
      writes.push({ type: 'truncate' });
      const value = (operation as { value: unknown }).value;
      const rows =
        value && typeof value === 'object'
          ? Object.values(value as Record<string, ElectricRow>)
          : [];
      for (const row of rows) {
        writes.push({ type: 'insert', value: row });
      }
      continue;
    }

    if (operation.op === 'add') {
      writes.push({
        type: 'insert',
        value: (operation as { value: ElectricRow }).value,
      });
    } else if (operation.op === 'replace') {
      writes.push({
        type: 'update',
        value: (operation as { value: ElectricRow }).value,
      });
    } else if (operation.op === 'remove') {
      writes.push({ type: 'delete', value: { id } as ElectricRow });
    }
  }

  return writes;
}

type MutationFnParams = {
  transaction: {
    mutations: Array<{
      modified?: unknown;
      original?: unknown;
      key?: string;
      changes?: unknown;
    }>;
  };
};

export function buildLocalMutationHandlers(args: {
  name: string;
  url: string | null;
  request: (path: string, init?: RequestInit) => Promise<Response>;
  refresh: () => Promise<void>;
}) {
  const { name, url, request, refresh } = args;

  const ensureUrl = (): string => {
    if (!url) {
      throw new Error(`个人版不支持修改 ${name}`);
    }
    return url;
  };

  const send = async (path: string, init: RequestInit): Promise<void> => {
    const response = await request(path, init);
    if (!response.ok) {
      throw new Error(
        await parseResponseError(response, `Failed to write ${name}`)
      );
    }
  };

  return {
    onInsert: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      for (const mutation of transaction.mutations) {
        await send(base, {
          method: 'POST',
          headers: JSON_HEADERS,
          body: JSON.stringify(mutation.modified),
        });
      }
      await refresh();
    },

    onUpdate: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      if (transaction.mutations.length > 1) {
        // 拖拽排序：合并成一次 /bulk，由后端放进一个事务。
        const updates = transaction.mutations.map((mutation) => {
          if (!mutation.key) {
            throw new Error(`Failed to update ${name}: missing key`);
          }
          return {
            id: String(mutation.key),
            ...(mutation.changes as Record<string, unknown>),
          };
        });
        await send(`${base}/bulk`, {
          method: 'POST',
          headers: JSON_HEADERS,
          body: JSON.stringify({ updates }),
        });
      } else {
        const mutation = transaction.mutations[0];
        if (!mutation?.key) {
          throw new Error(`Failed to update ${name}: missing key`);
        }
        await send(`${base}/${encodeURIComponent(String(mutation.key))}`, {
          method: 'PATCH',
          headers: JSON_HEADERS,
          body: JSON.stringify(mutation.changes),
        });
      }
      await refresh();
    },

    onDelete: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      for (const mutation of transaction.mutations) {
        await send(`${base}/${encodeURIComponent(String(mutation.key))}`, {
          method: 'DELETE',
          headers: JSON_HEADERS,
        });
      }
      await refresh();
    },
  };
}

type SyncParams = {
  collection: { isReady: () => boolean };
  begin: () => void;
  write: (message: {
    type: 'insert' | 'update' | 'delete';
    value: ElectricRow;
    metadata?: Record<string, unknown>;
  }) => void;
  commit: () => void;
  markReady: () => void;
  truncate: () => void;
};

function applyWrites(syncParams: SyncParams, writes: LocalWrite[]): void {
  if (writes.length === 0) return;
  syncParams.begin();
  for (const write of writes) {
    if (write.type === 'truncate') {
      syncParams.truncate();
    } else {
      syncParams.write({ type: write.type, value: write.value, metadata: {} });
    }
  }
  syncParams.commit();
  syncParams.markReady();
}

/**
 * 个人版集合：REST 全量快照打底，WS JSON Patch 做增量，写成功后立即重拉。
 */
export function createLocalShapeCollection<TRow extends ElectricRow>(
  collectionId: string,
  shape: ShapeDefinition<TRow>,
  params: Record<string, string>,
  config?: CollectionConfig,
  mutation?: MutationDefinition<unknown, unknown, unknown>
) {
  const endpoint = resolveLocalShapeEndpoint(shape, params);
  const reportError = (error: SyncError) => config?.onError?.(error);

  let refreshNow: () => Promise<void> = async () => {};

  const mutationHandlers = mutation
    ? buildLocalMutationHandlers({
        name: mutation.name,
        url: resolveLocalMutationUrl(mutation),
        request: (path, init) => makeLocalApiRequest(path, init),
        refresh: () => refreshNow(),
      })
    : {};

  const sync = (syncParams: SyncParams) => {
    if (endpoint.kind === 'empty') {
      applyWrites(syncParams, [{ type: 'truncate' }]);
      syncParams.markReady();
      return { cleanup: () => {}, loadSubset: () => true };
    }

    let cleanedUp = false;
    let socket: WebSocket | null = null;
    let reconnectAttempt = 0;
    let reconnectTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
    let inFlight: Promise<void> | null = null;

    const fetchSnapshot = async (): Promise<void> => {
      if (inFlight) return inFlight;
      inFlight = (async () => {
        try {
          const response = await makeLocalApiRequest(endpoint.path, {
            method: 'GET',
            cache: 'no-store',
          });
          if (!response.ok) {
            throw new Error(
              await parseResponseError(
                response,
                `Failed to fetch local ${shape.table}`
              )
            );
          }
          const rows = extractFallbackRows(await response.json(), shape.table);
          if (cleanedUp) return;
          applyWrites(syncParams, [
            { type: 'truncate' },
            ...rows.map((row) => ({ type: 'insert' as const, value: row })),
          ]);
        } catch (error) {
          reportError({
            message:
              error instanceof Error ? error.message : 'Local fetch failed',
          });
          if (!cleanedUp && !syncParams.collection.isReady()) {
            syncParams.markReady();
          }
        } finally {
          inFlight = null;
        }
      })();
      return inFlight;
    };

    refreshNow = fetchSnapshot;
    void fetchSnapshot();

    const connect = async () => {
      if (cleanedUp || !endpoint.wsPath) return;
      try {
        socket = await openLocalApiWebSocket(endpoint.wsPath);
      } catch {
        scheduleReconnect();
        return;
      }

      socket.onmessage = (event) => {
        if (cleanedUp) return;
        try {
          const message = JSON.parse(String(event.data)) as {
            JsonPatch?: Operation[];
          };
          if (!message.JsonPatch) return;
          applyWrites(syncParams, patchToWrites(message.JsonPatch, shape.table));
        } catch {
          // 收到无法解析的帧时忽略，下一次快照会纠正状态
        }
      };
      socket.onopen = () => {
        reconnectAttempt = 0;
      };
      socket.onclose = () => {
        if (!cleanedUp) scheduleReconnect();
      };
      socket.onerror = () => {
        socket?.close();
      };
    };

    function scheduleReconnect() {
      if (cleanedUp || reconnectTimer) return;
      const delay = Math.min(
        MAX_RECONNECT_DELAY_MS,
        1000 * Math.pow(2, reconnectAttempt)
      );
      reconnectAttempt += 1;
      reconnectTimer = globalThis.setTimeout(() => {
        reconnectTimer = null;
        void fetchSnapshot();
        void connect();
      }, delay);
    }

    void connect();

    // WS 覆盖不到的表（例如评论）靠轮询兜底。
    const pollId = endpoint.wsPath
      ? null
      : globalThis.setInterval(() => void fetchSnapshot(), POLL_INTERVAL_MS);

    return {
      cleanup: () => {
        cleanedUp = true;
        if (pollId) globalThis.clearInterval(pollId);
        if (reconnectTimer) globalThis.clearTimeout(reconnectTimer);
        socket?.close();
      },
      loadSubset: () => true,
    };
  };

  return createCollection({
    id: collectionId,
    getKey: (item: ElectricRow) => getRowKey(item),
    sync: { sync },
    ...mutationHandlers,
  } as never) as unknown as ReturnType<typeof createCollection> & {
    __rowType?: TRow;
  };
}
