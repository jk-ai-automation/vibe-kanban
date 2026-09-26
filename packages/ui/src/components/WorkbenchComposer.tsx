import { useTranslation } from 'react-i18next';
import { ArrowRightIcon } from '@phosphor-icons/react';
import { PrimaryButton } from './PrimaryButton';

export interface WorkbenchOption {
  id: string;
  label: string;
}

export interface WorkbenchComposerProps {
  prompt: string;
  onPromptChange: (value: string) => void;
  repos: WorkbenchOption[];
  repoId: string | null;
  onRepoChange: (repoId: string) => void;
  branches: string[];
  branch: string | null;
  onBranchChange: (branch: string) => void;
  agents: WorkbenchOption[];
  agent: string | null;
  onAgentChange: (agent: string) => void;
  canSubmit: boolean;
  isSubmitting: boolean;
  /** 已翻译的前置提示（没有项目 / 没有仓库）。 */
  notice: string | null;
  /** 已翻译的错误。 */
  error: string | null;
  onSubmit: () => void;
}

const selectClassName =
  'max-w-[14rem] truncate rounded-sm border border-border bg-primary px-half py-half text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand disabled:opacity-50';

/**
 * 工作台顶部「描述需求」（设计文档 §8.2 + 草图①）：一段话 + 仓库 / 分支 /
 * 流程模板 / 智能体 + 开始（⌘↵）。受控、无状态。
 */
export function WorkbenchComposer({
  prompt,
  onPromptChange,
  repos,
  repoId,
  onRepoChange,
  branches,
  branch,
  onBranchChange,
  agents,
  agent,
  onAgentChange,
  canSubmit,
  isSubmitting,
  notice,
  error,
  onSubmit,
}: WorkbenchComposerProps) {
  const { t } = useTranslation('common');

  return (
    <section className="flex flex-col gap-half rounded-sm border border-border bg-secondary p-base">
      <textarea
        aria-label={t('workbench.composer.label')}
        placeholder={t('workbench.composer.placeholder')}
        value={prompt}
        onChange={(event) => onPromptChange(event.target.value)}
        onKeyDown={(event) => {
          if (
            (event.metaKey || event.ctrlKey) &&
            event.key === 'Enter' &&
            canSubmit
          ) {
            event.preventDefault();
            onSubmit();
          }
        }}
        rows={3}
        className="w-full resize-y rounded-sm bg-primary px-base py-half text-base text-high placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand"
      />
      <p className="m-0 text-sm text-low">{t('workbench.composer.example')}</p>
      <div className="flex flex-wrap items-center gap-base">
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.repo')}
          <select
            aria-label={t('workbench.composer.repo')}
            className={selectClassName}
            value={repoId ?? ''}
            onChange={(event) => onRepoChange(event.target.value)}
            disabled={repos.length === 0}
          >
            {repos.map((repo) => (
              <option key={repo.id} value={repo.id}>
                {repo.label}
              </option>
            ))}
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.branch')}
          <select
            aria-label={t('workbench.composer.branch')}
            className={selectClassName}
            value={branch ?? ''}
            onChange={(event) => onBranchChange(event.target.value)}
            disabled={branches.length === 0}
          >
            {branches.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.template')}
          <select
            aria-label={t('workbench.composer.template')}
            className={selectClassName}
            value="standard"
            disabled
          >
            <option value="standard">
              {t('workbench.composer.templateStandard')}
            </option>
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.agent')}
          <select
            aria-label={t('workbench.composer.agent')}
            className={selectClassName}
            value={agent ?? ''}
            onChange={(event) => onAgentChange(event.target.value)}
            disabled={agents.length === 0}
          >
            {agents.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </select>
        </span>
        <div className="ml-auto">
          <PrimaryButton
            value={
              isSubmitting
                ? t('workbench.composer.submitting')
                : t('workbench.composer.submit')
            }
            actionIcon={isSubmitting ? 'spinner' : ArrowRightIcon}
            onClick={onSubmit}
            disabled={!canSubmit}
          />
        </div>
      </div>
      {notice && <p className="m-0 text-sm text-low">{notice}</p>}
      {error && (
        <p role="alert" className="m-0 text-sm text-error">
          {error}
        </p>
      )}
    </section>
  );
}
