import { useEffect, useMemo, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { GateDecisionRequest } from 'shared/types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import {
  PIPELINE_RUNS_SHAPE,
  PIPELINE_STAGE_RUNS_SHAPE,
} from '../../api/pipelineShapes';
import {
  createIssueAndStartPipeline,
  decideGate,
  getArtifact,
  getIssuePipeline,
  listPendingPipelines,
  runPipelineAction,
  type PipelineRunAction,
} from '../../api/pipelineApi';
import { pipelineSignature } from '../signature';

export const pipelineQueryKeys = {
  all: ['pipeline'] as const,
  issue: (issueId: string) => ['pipeline', 'issue', issueId] as const,
  artifact: (artifactId: string) =>
    ['pipeline', 'artifact', artifactId] as const,
  pendingAll: ['pipeline', 'pending'] as const,
  pending: (projectId: string | null) =>
    ['pipeline', 'pending', projectId ?? 'all'] as const,
};

/** 个人版且有项目时才开集合（团队版一律不开，保证行为不变）。 */
function collectionEnabled(projectId: string | null): boolean {
  return isLocalPersonalMode() && !!projectId;
}

export function usePipelineRuns(projectId: string | null) {
  const { data, isLoading } = useShape(
    PIPELINE_RUNS_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: collectionEnabled(projectId) }
  );
  return { runs: data, isLoading };
}

export function usePipelineStageRuns(projectId: string | null) {
  const { data, isLoading } = useShape(
    PIPELINE_STAGE_RUNS_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: collectionEnabled(projectId) }
  );
  return { stages: data, isLoading };
}

/** 指纹变化（首次除外）时失效给定查询。 */
function useInvalidateOnChange(
  signature: string,
  queryKey: readonly unknown[] | null
) {
  const queryClient = useQueryClient();
  const previous = useRef<string | null>(null);
  const keyString = queryKey ? JSON.stringify(queryKey) : null;
  useEffect(() => {
    if (
      keyString &&
      previous.current !== null &&
      previous.current !== signature
    ) {
      void queryClient.invalidateQueries({
        queryKey: JSON.parse(keyString) as unknown[],
      });
    }
    previous.current = signature;
  }, [keyString, queryClient, signature]);
}

/**
 * 单个需求的流水线全貌。产出物与决策不推送（契约 §3），所以集合里
 * 这条需求的运行 / 阶段一有变化就重拉。
 */
export function useIssuePipeline(
  projectId: string | null,
  issueId: string | null
) {
  const query = useQuery({
    queryKey: pipelineQueryKeys.issue(issueId ?? ''),
    queryFn: () => getIssuePipeline(issueId as string),
    enabled: !!issueId,
  });
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const signature = useMemo(
    () =>
      pipelineSignature(
        runs.filter((run) => run.issue_id === issueId),
        stages
      ),
    [runs, stages, issueId]
  );
  useInvalidateOnChange(
    signature,
    issueId ? pipelineQueryKeys.issue(issueId) : null
  );
  return query;
}

/** 工作台「需要你确认」（等人工 + 失败）。 */
export function usePendingPipelines(projectId: string | null) {
  const query = useQuery({
    queryKey: pipelineQueryKeys.pending(projectId),
    queryFn: () => listPendingPipelines(projectId),
    enabled: collectionEnabled(projectId),
  });
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const signature = useMemo(
    () => pipelineSignature(runs, stages),
    [runs, stages]
  );
  useInvalidateOnChange(
    signature,
    projectId ? pipelineQueryKeys.pending(projectId) : null
  );
  return query;
}

/** 产出物按版本不可变，拉一次就够。 */
export function useArtifact(artifactId: string | null) {
  return useQuery({
    queryKey: pipelineQueryKeys.artifact(artifactId ?? ''),
    queryFn: () => getArtifact(artifactId as string),
    enabled: !!artifactId,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useGateDecision() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      issueId: string;
      stageRunId: string;
      request: GateDecisionRequest;
    }) => decideGate(args.stageRunId, args.request),
    onSuccess: (view, args) => {
      queryClient.setQueryData(pipelineQueryKeys.issue(args.issueId), view);
      void queryClient.invalidateQueries({
        queryKey: pipelineQueryKeys.pendingAll,
      });
    },
  });
}

export function usePipelineRunAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      issueId: string;
      runId: string;
      action: PipelineRunAction;
    }) => runPipelineAction(args.runId, args.action),
    onSuccess: (view, args) => {
      queryClient.setQueryData(pipelineQueryKeys.issue(args.issueId), view);
      void queryClient.invalidateQueries({
        queryKey: pipelineQueryKeys.pendingAll,
      });
    },
  });
}

export function useStartPipeline() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createIssueAndStartPipeline,
    onSuccess: (view) => {
      queryClient.setQueryData(
        pipelineQueryKeys.issue(view.run.issue_id),
        view
      );
      void queryClient.invalidateQueries({
        queryKey: pipelineQueryKeys.pendingAll,
      });
    },
  });
}
