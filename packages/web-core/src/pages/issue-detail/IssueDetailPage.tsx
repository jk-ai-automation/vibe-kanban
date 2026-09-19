import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useRouter } from '@tanstack/react-router';
import type { IssueArtifactSummary } from 'shared/types';
import { PROJECT_ISSUES_SHAPE } from 'shared/remote-types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
import {
  useIssuePipeline,
  usePipelineRunAction,
} from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  type Translate,
} from '@/entities/pipeline/model/progress';
import { cellLabelKey } from '@/entities/pipeline/model/cardInfo';
import {
  pipelineStageLabelKey,
  pipelineStageTone,
} from '@/entities/pipeline/model/stages';
import { durationText } from '@/entities/pipeline/model/time';
import { IssuePanelTabBar } from '@vibe/ui/components/IssuePanelTabBar';
import { PipelineStageBadge } from '@vibe/ui/components/PipelineStageBadge';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { PipelineStepper } from '@vibe/ui/components/PipelineStepper';
import { PipelineTimeline } from '@vibe/ui/components/PipelineTimeline';
import { PrimaryButton } from '@vibe/ui/components/PrimaryButton';
import { Skeleton } from '@vibe/ui/components/Skeleton';
import { ArtifactSection } from './ArtifactSection';
import { GateBarContainer } from './GateBarContainer';
import { buildTimeline, timelineTextKey } from './timeline';
import {
  ISSUE_DETAIL_TABS,
  artifactKindLabelKey,
  defaultIssueDetailTab,
  issueDetailTabLabelKey,
  latestArtifactOfKind,
  latestArtifactsByKind,
  tabArtifactKinds,
  tabOfArtifactKind,
  type IssueDetailTab,
} from './issueDetailModel';

function DetailSkeleton({ label }: { label: string }) {
  return (
    <div
      role="status"
      aria-label={label}
      className="flex h-full flex-col gap-base bg-primary p-double"
    >
      <Skeleton className="h-6 w-1/3" />
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}

/**
 * 需求详情全屏页（设计文档 §8.4 + 草图③）：顶部面包屑 / 标题 / 阶段徽标 /
 * 暂停；七段步进条；左边六个页签的产出物；右边时间线、产出物列表、用量；
 * 底部关卡条。原右侧面板路由不动。
 */
export function IssueDetailPage({
  projectId,
  issueId,
}: {
  projectId: string;
  issueId: string;
}) {
  const { t } = useTranslation('common');
  const translate: Translate = (key, params) => String(t(key, params));
  const router = useRouter();
  const appNavigation = useAppNavigation();
  const { data: issues, isLoading: issuesLoading } = useShape(
    PROJECT_ISSUES_SHAPE,
    {
      project_id: projectId,
    }
  );
  const issue = issues.find((item) => item.id === issueId) ?? null;
  const pipelineQuery = useIssuePipeline(projectId, issueId);
  const view = pipelineQuery.data ?? null;
  const runAction = usePipelineRunAction();
  const [selectedTab, setSelectedTab] = useState<IssueDetailTab | null>(null);

  const timelineItems = useMemo(() => {
    if (!view) return [];
    return buildTimeline(
      view.stages.filter((stage) => stage.run_id === view.run.id),
      view.decisions
    ).map((event) => {
      const stage = t(pipelineStageLabelKey(event.stageKey));
      const clock = new Date(event.at).toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
      });
      const duration =
        event.durationMs !== null ? durationText(event.durationMs) : null;
      return {
        id: event.id,
        actor: event.actor,
        actorLabel: t(
          event.actor === 'ai'
            ? 'issueDetail.timeline.actorAi'
            : 'issueDetail.timeline.actorHuman'
        ),
        text: t(timelineTextKey(event.kind), { stage, attempt: event.attempt }),
        detail: event.text,
        meta: [clock, stage, duration ? t(duration.key, duration.params) : null]
          .filter(Boolean)
          .join(' · '),
      };
    });
  }, [t, view]);

  if (issuesLoading || pipelineQuery.isLoading) {
    return <DetailSkeleton label={t('issueDetail.loading')} />;
  }
  if (!issue) {
    return (
      <div className="flex h-full items-center justify-center bg-primary p-double">
        <p className="m-0 text-sm text-low">{t('issueDetail.notFound')}</p>
      </div>
    );
  }

  const activeTab =
    selectedTab ?? defaultIssueDetailTab(view?.run.current_stage_key ?? null);
  const status = view
    ? pipelineStatus(view.stages, view.template, view.run)
    : null;
  const canPause =
    view?.run.status === 'running' || view?.run.status === 'waiting_gate';
  const canResume =
    view?.run.status === 'paused' || view?.run.status === 'failed';
  const tabArtifacts = view
    ? tabArtifactKinds(activeTab)
        .map((kind) => latestArtifactOfKind(view.artifacts, kind))
        .filter(
          (artifact): artifact is IssueArtifactSummary => artifact !== null
        )
    : [];

  return (
    <div className="flex h-full min-h-0 flex-col bg-primary">
      <header className="flex shrink-0 flex-col gap-base border-b border-border px-double py-base">
        <div className="flex flex-wrap items-center gap-half">
          <button
            type="button"
            onClick={() =>
              router.history.push(PERSONAL_ROUTES.project(projectId))
            }
            className="text-sm text-low hover:text-normal"
          >
            {t('issueDetail.breadcrumb')}
          </button>
          <span aria-hidden="true" className="text-sm text-low">
            ›
          </span>
          <span
            data-testid="issue-simple-id"
            className="font-ibm-plex-mono text-sm text-low"
          >
            {issue.simple_id}
          </span>
          <h1 className="m-0 min-w-0 truncate text-lg font-medium text-high">
            {issue.title}
          </h1>
          {view && (
            <PipelineStageBadge
              tone={pipelineStageTone(view.run.current_stage_key)}
              label={t(pipelineStageLabelKey(view.run.current_stage_key))}
            />
          )}
          {status && (
            <PipelineStatusTag
              tone={status.tone}
              label={pipelineStatusText(status, translate)}
            />
          )}
          <div className="ml-auto flex items-center gap-half">
            {view && canPause && (
              <PrimaryButton
                variant="tertiary"
                value={t('pipeline.gate.pause')}
                disabled={runAction.isPending}
                onClick={() =>
                  runAction.mutate({
                    issueId,
                    runId: view.run.id,
                    action: 'pause',
                  })
                }
              />
            )}
            {view && canResume && (
              <PrimaryButton
                value={t('pipeline.gate.resume')}
                disabled={runAction.isPending}
                onClick={() =>
                  runAction.mutate({
                    issueId,
                    runId: view.run.id,
                    action: 'resume',
                  })
                }
              />
            )}
          </div>
        </div>
        {runAction.error && (
          <p role="alert" className="m-0 text-sm text-error">
            {t('pipeline.gate.error', { message: runAction.error.message })}
          </p>
        )}
        {view && (
          <PipelineStepper
            currentKey={view.run.current_stage_key}
            steps={pipelineProgress(view.stages, view.template, view.run).map(
              (cell) => ({
                key: cell.key,
                label: t(pipelineStageLabelKey(cell.key)),
                state: cell.state,
                stateLabel: t(cellLabelKey(cell.state)),
              })
            )}
          />
        )}
      </header>

      {!view ? (
        <div className="flex flex-1 items-center justify-center p-double">
          <p className="m-0 text-sm text-low">{t('issueDetail.noPipeline')}</p>
        </div>
      ) : (
        <>
          <div className="grid min-h-0 flex-1 grid-cols-1 xl:grid-cols-[minmax(0,1fr)_320px]">
            <main className="flex min-h-0 flex-col">
              <IssuePanelTabBar
                className="px-double"
                tabs={ISSUE_DETAIL_TABS.map((tab) => ({
                  id: tab,
                  label: t(issueDetailTabLabelKey(tab)),
                }))}
                activeTab={activeTab}
                onTabChange={setSelectedTab}
              />
              <div className="flex min-h-0 flex-1 flex-col gap-double overflow-y-auto px-double py-base">
                {activeTab === 'code' &&
                  (view.run.workspace_id ? (
                    <div>
                      <PrimaryButton
                        variant="tertiary"
                        value={t('issueDetail.openWorkspace')}
                        onClick={() =>
                          appNavigation.goToWorkspace(
                            view.run.workspace_id as string
                          )
                        }
                      />
                    </div>
                  ) : (
                    <p className="m-0 text-sm text-low">
                      {t('issueDetail.noWorkspace')}
                    </p>
                  ))}
                {tabArtifacts.length === 0 ? (
                  <p className="m-0 text-sm text-low">
                    {t('issueDetail.noArtifact')}
                  </p>
                ) : (
                  tabArtifacts.map((summary) => (
                    <ArtifactSection key={summary.id} summary={summary} />
                  ))
                )}
              </div>
            </main>
            <aside className="flex min-h-0 flex-col gap-double overflow-y-auto border-l border-border bg-secondary p-base">
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">
                  {t('issueDetail.side.timeline')}
                </h2>
                <PipelineTimeline
                  items={timelineItems}
                  emptyText={t('issueDetail.timeline.empty')}
                />
              </section>
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">
                  {t('issueDetail.side.artifacts')}
                </h2>
                <ul className="m-0 flex list-none flex-col gap-1 p-0">
                  {latestArtifactsByKind(view.artifacts).map((artifact) => (
                    <li key={artifact.id}>
                      <button
                        type="button"
                        onClick={() =>
                          setSelectedTab(tabOfArtifactKind(artifact.kind))
                        }
                        className="flex w-full items-center justify-between gap-half rounded-sm px-half py-half text-left text-sm text-normal hover:bg-primary"
                      >
                        <span className="truncate">
                          {t(artifactKindLabelKey(artifact.kind))}
                        </span>
                        <span className="font-ibm-plex-mono text-xs text-low">
                          {t('issueDetail.version', {
                            version: String(artifact.version),
                          })}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">
                  {t('issueDetail.side.usage')}
                </h2>
                <p className="m-0 text-sm text-low">
                  {t('issueDetail.side.usageNotReady')}
                </p>
              </section>
            </aside>
          </div>
          <GateBarContainer
            key={`${view.run.id}:${view.run.status}:${view.run.current_stage_key}`}
            issueId={issueId}
            view={view}
          />
        </>
      )}
    </div>
  );
}
