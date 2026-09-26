import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';
import type { BaseCodingAgent } from 'shared/types';
import { PROJECT_PROJECT_STATUSES_SHAPE } from 'shared/remote-types';
import { WorkbenchComposer } from '@vibe/ui/components/WorkbenchComposer';
import { repoApi } from '@/shared/lib/api';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useUserSystem } from '@/shared/hooks/useUserSystem';
import { useExecutorConfig } from '@/shared/hooks/useExecutorConfig';
import { toPrettyCase } from '@/shared/lib/string';
import { useStartPipeline } from '@/entities/pipeline/model/hooks/usePipelineData';
import { PipelineApiError } from '@/entities/pipeline/api/pipelineApi';
import {
  buildCreateIssueRequest,
  pickBacklogStatusId,
  pickDefaultBranch,
} from './workbenchModel';

/**
 * 工作台输入框容器：一句话 → 建需求 → 启动流水线（契约 §2 POST pipeline）。
 *
 * 不复用 `CreateChatBoxContainer` / `CreateModeRepoPickerBar`：它们绑死
 * `useCreateMode()` 草稿上下文并调用建工作区接口（计划 §2.5）。这里只复用
 * `useExecutorConfig`（纯 props 驱动）与 `repoApi`。
 */
export function WorkbenchComposerContainer({
  projectId,
}: {
  projectId: string | null;
}) {
  const { t } = useTranslation('common');
  const { profiles, config } = useUserSystem();
  const [prompt, setPrompt] = useState('');
  const [repoId, setRepoId] = useState<string | null>(null);
  const [branch, setBranch] = useState<string | null>(null);
  /**
   * 上次「开始」建好了需求但启动流水线失败：记下需求 id 与当时的描述。
   * 描述没改时再点「开始」只重试启动，不会再建一条重复需求。
   */
  const [orphanIssue, setOrphanIssue] = useState<{
    issueId: string;
    prompt: string;
  } | null>(null);

  const reposQuery = useQuery({
    queryKey: ['workbench', 'repos'],
    queryFn: () => repoApi.list(),
  });
  const repos = reposQuery.data ?? [];
  const effectiveRepoId =
    repoId && repos.some((repo) => repo.id === repoId)
      ? repoId
      : (repos[0]?.id ?? null);
  const repo = repos.find((item) => item.id === effectiveRepoId) ?? null;

  const branchesQuery = useQuery({
    queryKey: ['workbench', 'branches', effectiveRepoId],
    queryFn: () => repoApi.getBranches(effectiveRepoId as string),
    enabled: !!effectiveRepoId,
  });
  const localBranches = useMemo(
    () =>
      (branchesQuery.data ?? [])
        .filter((item) => !item.is_remote)
        .map((item) => item.name),
    [branchesQuery.data]
  );
  const effectiveBranch =
    branch && localBranches.includes(branch)
      ? branch
      : repo
        ? pickDefaultBranch(repo, branchesQuery.data ?? [])
        : null;

  const { executorConfig, effectiveExecutor, executorOptions, setExecutor } =
    useExecutorConfig({
      profiles,
      lastUsedConfig: null,
      scratchConfig: null,
      configExecutorProfile: config?.executor_profile,
    });

  const { data: statuses } = useShape(
    PROJECT_PROJECT_STATUSES_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: !!projectId }
  );
  const statusId = pickBacklogStatusId(statuses);
  const start = useStartPipeline();

  const canSubmit =
    !!projectId &&
    !!statusId &&
    !!effectiveRepoId &&
    !!effectiveBranch &&
    !!executorConfig &&
    prompt.trim().length > 0 &&
    !start.isPending;

  const handleSubmit = () => {
    if (
      !canSubmit ||
      !projectId ||
      !statusId ||
      !effectiveRepoId ||
      !effectiveBranch ||
      !executorConfig
    ) {
      return;
    }
    const reuseIssueId =
      orphanIssue && orphanIssue.prompt === prompt ? orphanIssue.issueId : null;
    const submittedPrompt = prompt;
    start.mutate(
      {
        issue: buildCreateIssueRequest({
          id: crypto.randomUUID(),
          projectId,
          statusId,
          prompt,
        }),
        pipeline: {
          repos: [{ repo_id: effectiveRepoId, target_branch: effectiveBranch }],
          executor_config: executorConfig,
          template_key: null,
        },
        reuseIssueId,
      },
      {
        onSuccess: () => {
          setPrompt('');
          setOrphanIssue(null);
        },
        onError: (error) => {
          if (
            error instanceof PipelineApiError &&
            error.issueCreated &&
            error.issueId
          ) {
            setOrphanIssue({ issueId: error.issueId, prompt: submittedPrompt });
          }
        },
      }
    );
  };

  // 失败原因原样显示（后端 message 是中文）；需求已建好时明确告诉用户，
  // 并说明再点「开始」只会重试启动。
  const startErrorText = start.error
    ? start.error instanceof PipelineApiError && start.error.issueCreated
      ? t('workbench.composer.startFailedAfterCreate', {
          message: start.error.message,
        })
      : t('workbench.composer.error', { message: start.error.message })
    : null;

  const notice = !projectId
    ? t('workbench.composer.noProject')
    : reposQuery.isSuccess && repos.length === 0
      ? t('workbench.composer.noRepo')
      : // profiles 为 null 是还没加载完；加载完仍拿不到执行器配置才提示，
        // 否则「开始」被禁用却不知道为什么。
        profiles !== null && !executorConfig
        ? t('workbench.composer.noAgent')
        : null;

  return (
    <WorkbenchComposer
      prompt={prompt}
      onPromptChange={setPrompt}
      repos={repos.map((item) => ({
        id: item.id,
        label: item.display_name || item.name,
      }))}
      repoId={effectiveRepoId}
      onRepoChange={(id) => {
        setRepoId(id);
        setBranch(null);
      }}
      branches={localBranches}
      branch={effectiveBranch}
      onBranchChange={setBranch}
      agents={executorOptions.map((option) => ({
        id: option,
        label: toPrettyCase(option),
      }))}
      agent={effectiveExecutor}
      onAgentChange={(id) => setExecutor(id as BaseCodingAgent)}
      canSubmit={canSubmit}
      isSubmitting={start.isPending}
      notice={notice}
      error={startErrorText}
      onSubmit={handleSubmit}
    />
  );
}
