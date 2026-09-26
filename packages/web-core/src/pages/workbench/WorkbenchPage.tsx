import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useRouter } from '@tanstack/react-router';
import type { PendingPipelineItem, PipelineRun } from 'shared/types';
import { PROJECTS_SHAPE, PROJECT_ISSUES_SHAPE } from 'shared/remote-types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useUiPreferencesStore } from '@/shared/stores/useUiPreferencesStore';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
import {
  useGateDecision,
  usePendingPipelines,
  usePipelineRuns,
  usePipelineStageRuns,
} from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  buildPipelineCardInfo,
  groupStagesByRun,
} from '@/entities/pipeline/model/cardInfo';
import {
  firstErrorLine,
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  type Translate,
} from '@/entities/pipeline/model/progress';
import {
  pipelineStageLabelKey,
  pipelineStageTone,
} from '@/entities/pipeline/model/stages';
import { relativeTimeText } from '@/entities/pipeline/model/time';
import { PipelineProgressBar } from '@vibe/ui/components/PipelineProgressBar';
import { PipelineStageBadge } from '@vibe/ui/components/PipelineStageBadge';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { PrimaryButton } from '@vibe/ui/components/PrimaryButton';
import { WorkbenchCard } from '@vibe/ui/components/WorkbenchCard';
import { WorkbenchColumn } from '@vibe/ui/components/WorkbenchColumn';
import { WorkbenchComposerContainer } from './WorkbenchComposerContainer';
import {
  greetingKey,
  pickWorkbenchProjectId,
  recentDelivered,
  runningRuns,
  stagesFinishedToday,
  waitingSince,
  weeklyStats,
} from './workbenchModel';

/** 每分钟刷新一次「现在」，让相对时间与问候语跟着走。 */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/**
 * 工作台（设计文档 §8.2 + 草图①）：顶部描述需求；三栏只回答三件事——
 * 什么在等我、什么在自动跑、最近交付了什么。
 */
export function WorkbenchPage() {
  const { t } = useTranslation('common');
  const translate: Translate = (key, params) => String(t(key, params));
  const router = useRouter();
  const now = useNow(60_000);
  const isPersonal = isLocalPersonalMode();

  const selectedProjectId = useUiPreferencesStore((s) => s.selectedProjectId);
  const { data: projects } = useShape(
    PROJECTS_SHAPE,
    {},
    { enabled: isPersonal }
  );
  const projectId = pickWorkbenchProjectId(projects, selectedProjectId);

  const { data: issues } = useShape(
    PROJECT_ISSUES_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: isPersonal && !!projectId }
  );
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const { data: pending = [] } = usePendingPipelines(projectId);
  const gate = useGateDecision();

  const issueById = useMemo(
    () => new Map(issues.map((issue) => [issue.id, issue])),
    [issues]
  );
  const stagesByRun = useMemo(() => groupStagesByRun(stages), [stages]);
  const running = useMemo(() => runningRuns(runs), [runs]);
  const delivered = useMemo(() => recentDelivered(runs), [runs]);
  const weekly = useMemo(
    () => weeklyStats(runs, stages, now),
    [runs, stages, now]
  );
  const finishedToday = useMemo(
    () => stagesFinishedToday(stages, now),
    [stages, now]
  );

  if (!isPersonal) return null;

  const openIssue = (issueId: string) => {
    if (projectId)
      router.history.push(PERSONAL_ROUTES.issueDetail(projectId, issueId));
  };
  const timeText = (value: unknown): string | null => {
    const at = typeof value === 'number' ? value : toMillis(value);
    if (at === null) return null;
    const spec = relativeTimeText(at, now);
    return t(spec.key, spec.params);
  };
  const stageBadge = (key: PipelineRun['current_stage_key']) => (
    <PipelineStageBadge
      tone={pipelineStageTone(key)}
      label={t(pipelineStageLabelKey(key))}
    />
  );

  const renderPending = (item: PendingPipelineItem) => {
    const info = pipelineStatus([item.stage_run], null, item.run);
    const isHumanGate =
      item.stage_run.status === 'waiting_gate' &&
      item.stage_run.gate_kind === 'human';
    return (
      <WorkbenchCard
        key={item.run.id}
        issueId={item.run.issue_id}
        simpleId={item.issue_simple_id}
        title={item.issue_title}
        stageBadge={stageBadge(item.stage_run.stage_key)}
        statusTag={
          <PipelineStatusTag
            tone={info.tone}
            label={pipelineStatusText(info, translate)}
          />
        }
        timeText={timeText(waitingSince(item.stage_run))}
        // 失败原因带着进程输出末尾，卡片上只放第一行（需求详情的时间线有全文）。
        meta={firstErrorLine(item.stage_run.error) ?? item.stage_run.summary}
        highlight={info.tone === 'failed' ? 'failed' : 'gate'}
        openLabel={t('workbench.card.open')}
        onOpen={() => openIssue(item.run.issue_id)}
        actions={
          isHumanGate ? (
            <PrimaryButton
              value={t('pipeline.gate.confirm')}
              disabled={gate.isPending}
              onClick={() =>
                gate.mutate({
                  issueId: item.run.issue_id,
                  stageRunId: item.stage_run.id,
                  request: { decision: 'approve', comment: null },
                })
              }
            />
          ) : null
        }
      />
    );
  };

  const renderRun = (run: PipelineRun, withProgress: boolean) => {
    const issue = issueById.get(run.issue_id);
    const card = buildPipelineCardInfo(
      run,
      stagesByRun.get(run.id) ?? [],
      translate
    );
    return (
      <WorkbenchCard
        key={run.id}
        issueId={run.issue_id}
        simpleId={issue?.simple_id ?? ''}
        title={issue?.title ?? ''}
        stageBadge={withProgress ? stageBadge(run.current_stage_key) : null}
        statusTag={
          <PipelineStatusTag tone={card.tone} label={card.statusText} />
        }
        timeText={timeText(withProgress ? run.updated_at : run.finished_at)}
        progress={
          withProgress ? (
            <PipelineProgressBar
              cells={card.cells}
              ariaLabel={card.progressLabel}
            />
          ) : null
        }
        openLabel={t('workbench.card.open')}
        onOpen={() => openIssue(run.issue_id)}
      />
    );
  };

  return (
    <div className="h-full overflow-y-auto bg-primary">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-double px-double py-double">
        <header
          data-testid="workbench-greeting"
          className="flex flex-col gap-half"
        >
          <h1 className="m-0 text-xl font-medium text-high">
            {t(greetingKey(new Date(now).getHours()))}
          </h1>
          <p className="m-0 text-sm text-low">
            {t('workbench.summary', {
              stages: finishedToday,
              pending: pending.length,
            })}
          </p>
        </header>

        <WorkbenchComposerContainer projectId={projectId} />

        {gate.error && (
          <p role="alert" className="m-0 text-sm text-error">
            {t('pipeline.gate.error', { message: gate.error.message })}
          </p>
        )}

        <div className="grid grid-cols-1 gap-double lg:grid-cols-3">
          <WorkbenchColumn
            testId="workbench-column-confirm"
            title={t('workbench.columns.confirm')}
            count={pending.length}
            emptyText={t('workbench.empty.confirm')}
          >
            {pending.map(renderPending)}
          </WorkbenchColumn>
          <WorkbenchColumn
            testId="workbench-column-running"
            title={t('workbench.columns.running')}
            count={running.length}
            emptyText={t('workbench.empty.running')}
          >
            {running.map((run) => renderRun(run, true))}
          </WorkbenchColumn>
          <WorkbenchColumn
            testId="workbench-column-delivered"
            title={t('workbench.columns.delivered')}
            count={delivered.length}
            emptyText={t('workbench.empty.delivered')}
            footer={
              <p className="m-0 text-sm text-low">
                {weekly.delivered > 0
                  ? t('workbench.weekly', {
                      delivered: weekly.delivered,
                      avg: weekly.avgCycleMinutes ?? 0,
                      rate: weekly.firstPassRate ?? 0,
                    })
                  : t('workbench.weeklyEmpty')}
              </p>
            }
          >
            {delivered.map((run) => renderRun(run, false))}
          </WorkbenchColumn>
        </div>
      </div>
    </div>
  );
}
