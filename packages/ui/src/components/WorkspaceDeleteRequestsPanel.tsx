import { useTranslation } from 'react-i18next';

export interface WorkspaceDeleteRequestRow {
  id: string;
  workspaceId: string;
  workspaceName: string | null;
  workspaceBranch: string;
  requesterUsername: string | null;
  reason: string | null;
  createdAt: string;
}

export interface WorkspaceDeleteRequestsPanelProps {
  requests: WorkspaceDeleteRequestRow[];
  isLoading: boolean;
  /** 已翻译好的错误文案；没有错误时传 `null`。 */
  error: string | null;
  /** 正在处理中的申请 id，按钮据此禁用，避免重复点触发并发批准。 */
  busyRequestId: string | null;
  onApprove: (requestId: string) => void;
  onReject: (requestId: string) => void;
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

/**
 * 成员管理页里的「删除申请」审批队列。
 *
 * 为什么放在成员管理页：申请可能来自任何需求下的任何工作区，管理员不该靠
 * 翻看板去发现它。成员管理页已经是团队治理的落点（成员、邀请码），而且
 * 那页本身只有管理员进得去，权限语义天然对上。
 *
 * 工作区卡片上另有一套就地批准/驳回，处理「管理员正看着这个需求」的常见情形。
 */
export function WorkspaceDeleteRequestsPanel({
  requests,
  isLoading,
  error,
  busyRequestId,
  onApprove,
  onReject,
}: WorkspaceDeleteRequestsPanelProps) {
  const { t } = useTranslation();

  return (
    <section className="flex flex-col gap-base">
      <header className="flex items-center gap-base">
        <h2 className="text-base font-medium text-high">
          {t('deleteRequests.title')}
        </h2>
        {requests.length > 0 && (
          <span className="inline-flex items-center rounded bg-brand/15 px-half py-0 text-sm font-medium text-brand">
            {requests.length}
          </span>
        )}
      </header>

      {error && <p className="text-sm text-error">{error}</p>}

      {isLoading ? (
        <p className="text-sm text-low">{t('deleteRequests.loading')}</p>
      ) : requests.length === 0 ? (
        <p className="text-sm text-low">{t('deleteRequests.empty')}</p>
      ) : (
        <div className="overflow-x-auto rounded border bg-panel">
          <table className="w-full min-w-[40rem] text-left">
            <thead>
              <tr className="border-b text-sm text-low">
                <th className="px-base py-half font-normal">
                  {t('deleteRequests.columnWorkspace')}
                </th>
                <th className="px-base py-half font-normal">
                  {t('deleteRequests.columnBranch')}
                </th>
                <th className="px-base py-half font-normal">
                  {t('deleteRequests.columnRequester')}
                </th>
                <th className="px-base py-half font-normal">
                  {t('deleteRequests.columnReason')}
                </th>
                <th className="px-base py-half font-normal">
                  {t('deleteRequests.columnCreatedAt')}
                </th>
                <th className="px-base py-half font-normal" />
              </tr>
            </thead>
            <tbody>
              {requests.map((request) => {
                const busy = busyRequestId === request.id;
                return (
                  <tr key={request.id} className="border-b last:border-b-0">
                    <td className="px-base py-half text-sm text-high">
                      {request.workspaceName ?? request.workspaceBranch}
                    </td>
                    <td className="px-base py-half text-sm text-low">
                      {request.workspaceBranch}
                    </td>
                    <td className="px-base py-half text-sm text-low">
                      {request.requesterUsername ??
                        t('deleteRequests.unknownRequester')}
                    </td>
                    <td className="px-base py-half text-sm text-low">
                      {request.reason ?? t('deleteRequests.noReason')}
                    </td>
                    <td className="px-base py-half text-sm text-low">
                      {formatDateTime(request.createdAt)}
                    </td>
                    <td className="px-base py-half">
                      <div className="flex flex-wrap items-center gap-half">
                        <button
                          type="button"
                          disabled={busy}
                          className="h-cta rounded border border-error/40 px-base text-sm text-error transition-colors hover:bg-error/10 disabled:opacity-50"
                          onClick={() => onApprove(request.id)}
                        >
                          {t('deleteRequests.approve')}
                        </button>
                        <button
                          type="button"
                          disabled={busy}
                          className="h-cta rounded border bg-panel px-base text-sm text-normal transition-colors hover:text-high disabled:opacity-50"
                          onClick={() => onReject(request.id)}
                        >
                          {t('deleteRequests.reject')}
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
