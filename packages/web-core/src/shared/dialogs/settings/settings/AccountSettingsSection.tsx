import { useCallback, useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { SpinnerIcon } from '@phosphor-icons/react';

import {
  LocalAuthRequestError,
  fetchOAuthBindings,
  startOAuthBind,
  unbindOAuth,
} from '@/shared/lib/local/bootstrapApi';
import {
  buildBindingRows,
  describeBindingError,
  isOAuthBoundRedirect,
  providerLabelKey,
  stripOAuthQueryFlag,
  type OAuthBindingRow,
} from '@/features/local-auth/model/bindings';
import { isLocalTeamMode } from '@/shared/lib/local/runtimeMode';
import { cn } from '@/shared/lib/utils';
import { SettingsCard } from './SettingsComponents';

export const OAUTH_BINDINGS_QUERY_KEY = ['local-auth', 'oauth-bindings'];

const buttonClass = cn(
  'h-cta px-base rounded border bg-panel text-base text-normal',
  'hover:text-high transition-colors',
  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
  'disabled:opacity-50 disabled:cursor-not-allowed'
);

function formatDateTime(value: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleString();
}

/**
 * 绑定成功后后端固定回跳 `/?oauth=bound`（`oauth_routes.rs` 的 `BIND_REDIRECT`）。
 *
 * 判定放在 `useState` 的惰性初始值里而不是 effect 里：地址栏只读一次，
 * 之后 effect 把参数摘掉，刷新页面就不会再提示一遍。
 */
function useBoundRedirectNotice(): boolean {
  const [justBound] = useState(
    () =>
      typeof window !== 'undefined' &&
      isOAuthBoundRedirect(window.location.search)
  );

  useEffect(() => {
    if (!justBound || typeof window === 'undefined') return;
    const rest = stripOAuthQueryFlag(window.location.search);
    window.history.replaceState(
      null,
      '',
      `${window.location.pathname}${rest}${window.location.hash}`
    );
  }, [justBound]);

  return justBound;
}

function BindingRow({
  row,
  busy,
  onBind,
  onUnbind,
}: {
  row: OAuthBindingRow;
  busy: boolean;
  onBind: (provider: string) => void;
  onUnbind: (provider: string) => void;
}) {
  const { t } = useTranslation();
  const labelKey = providerLabelKey(row.provider);
  // 后端将来多一个提供方时显示它的 id，而不是一片空白。
  const label = labelKey ? t(labelKey) : row.provider;

  return (
    <div className="flex flex-wrap items-center justify-between gap-base rounded border bg-panel p-base">
      <div className="min-w-0 flex flex-col gap-half">
        <span className="text-base font-medium text-high">{label}</span>
        {row.bound ? (
          <span className="text-sm text-low break-all">
            {t('localAuth.bindings.boundAt', {
              time: formatDateTime(row.boundAt),
            })}
            {row.email ? ` · ${row.email}` : ''}
          </span>
        ) : (
          <span className="text-sm text-low">
            {t('localAuth.bindings.notBound')}
          </span>
        )}
        {row.isLastLoginMethod && (
          <span className="text-sm text-low">
            {t('localAuth.bindings.lastLoginMethodHint')}
          </span>
        )}
        {row.bound && !row.canBind && (
          <span className="text-sm text-low">
            {t('localAuth.bindings.providerUnavailable')}
          </span>
        )}
      </div>

      <div className="shrink-0">
        {row.bound ? (
          <button
            type="button"
            className={buttonClass}
            disabled={busy || !row.canUnbind}
            title={
              row.isLastLoginMethod
                ? t('localAuth.bindings.lastLoginMethodHint')
                : undefined
            }
            onClick={() => onUnbind(row.provider)}
          >
            {t('localAuth.bindings.unbind')}
          </button>
        ) : (
          <button
            type="button"
            className={buttonClass}
            disabled={busy || !row.canBind}
            onClick={() => onBind(row.provider)}
          >
            {t('localAuth.bindings.bind')}
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * 账号设置 → 第三方账号。
 *
 * 整块只在**本机团队版**下存在（`settingsRegistry` 里已经按
 * `isLocalTeamMode()` 把这个 section 摘掉了）；这里再判一次是纵深防御：
 * 万一哪天有人直接 `SettingsDialog.show({ initialSection: 'account' })`，
 * 也不会在个人版里冒出一块点了必报错的界面。
 */
export function AccountSettingsSection() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const enabled = isLocalTeamMode();
  const [actionError, setActionError] = useState<string | null>(null);

  const bindings = useQuery({
    queryKey: OAUTH_BINDINGS_QUERY_KEY,
    queryFn: fetchOAuthBindings,
    enabled,
  });

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: OAUTH_BINDINGS_QUERY_KEY });
  }, [queryClient]);

  // 回跳是一次整页导航，查询缓存本来就是空的；这里只负责提示。
  const justBound = useBoundRedirectNotice();

  const bind = useMutation({
    mutationFn: startOAuthBind,
    onMutate: () => setActionError(null),
    onSuccess: (authorizeUrl) => {
      // 后端刻意不回 302（`fetch` 会跟着跳到提供方域名，拿回来的是一个
      // 不能用的跨域响应），所以由这里发起顶层导航。
      window.location.assign(authorizeUrl);
    },
    onError: (error: unknown) => {
      const status = error instanceof LocalAuthRequestError ? error.status : 0;
      setActionError(t(describeBindingError(status)));
    },
  });

  const unbind = useMutation({
    mutationFn: unbindOAuth,
    onMutate: () => setActionError(null),
    onSuccess: refresh,
    onError: (error: unknown) => {
      const status = error instanceof LocalAuthRequestError ? error.status : 0;
      setActionError(t(describeBindingError(status)));
      // 失败原因可能是「这条已经不在了」，把列表拉回真实状态。
      refresh();
    },
  });

  if (!enabled) return null;

  const busy = bind.isPending || unbind.isPending;
  const rows = bindings.data ? buildBindingRows(bindings.data) : [];

  return (
    <SettingsCard
      title={t('localAuth.bindings.sectionTitle')}
      description={t('localAuth.bindings.sectionDescription')}
    >
      {justBound && (
        <p role="status" className="text-base text-success">
          {t('localAuth.bindings.bindSuccess')}
        </p>
      )}
      {actionError && (
        <p role="alert" className="text-base text-error">
          {actionError}
        </p>
      )}

      {bindings.isPending ? (
        <p className="flex items-center gap-half text-base text-low">
          <SpinnerIcon className="size-icon-sm animate-spin" />
          {t('localAuth.bindings.loading')}
        </p>
      ) : bindings.isError ? (
        <div className="flex flex-wrap items-center gap-base">
          <p className="text-base text-error">
            {t('localAuth.bindings.loadError')}
          </p>
          <button
            type="button"
            className={buttonClass}
            onClick={() => void bindings.refetch()}
          >
            {t('localAuth.bindings.retry')}
          </button>
        </div>
      ) : rows.length === 0 ? (
        <p className="text-base text-low">
          {t('localAuth.bindings.noProviders')}
        </p>
      ) : (
        <div className="flex flex-col gap-base">
          {bindings.data && !bindings.data.has_password && (
            <p className="text-sm text-low">
              {t('localAuth.bindings.noPasswordHint')}
            </p>
          )}
          {rows.map((row) => (
            <BindingRow
              key={row.provider}
              row={row}
              busy={busy}
              onBind={(provider) => bind.mutate(provider)}
              onUnbind={(provider) => unbind.mutate(provider)}
            />
          ))}
        </div>
      )}
    </SettingsCard>
  );
}
