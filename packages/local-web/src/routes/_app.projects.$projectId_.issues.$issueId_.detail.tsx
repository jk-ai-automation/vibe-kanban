import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { IssueDetailPage } from '@/pages/issue-detail/IssueDetailPage';

function IssueDetailRouteComponent() {
  const { projectId, issueId } = Route.useParams();
  return <IssueDetailPage projectId={projectId} issueId={issueId} />;
}

/**
 * 需求详情全屏页（设计文档 §8.4）。`$projectId_` / `$issueId_` 表示不嵌套在
 * 看板路由下（否则会被 `LocalProjectKanban` 包住）。原右侧面板路由不动。
 */
export const Route = createFileRoute(
  '/_app/projects/$projectId_/issues/$issueId_/detail'
)({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: IssueDetailRouteComponent,
});
