import type { MouseEvent } from 'react';
import {
  KanbanBoard,
  KanbanCard,
  KanbanCards,
  KanbanHeader,
} from '@vibe/ui/components/KanbanBoard';
import { KanbanCardContent } from '@vibe/ui/components/KanbanCardContent';
import { KanbanColumnHeader } from '@vibe/ui/components/KanbanColumnHeader';
import { KanbanColumnEmptyState } from '@vibe/ui/components/KanbanColumnEmptyState';
import {
  IssueWorkspaceCard,
  type WorkspaceWithStats,
} from '@vibe/ui/components/IssueWorkspaceCard';
import { PipelineProgressBar } from '@vibe/ui/components/PipelineProgressBar';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { SearchableTagDropdownContainer } from '@/shared/components/SearchableTagDropdownContainer';
import type { ResolvedRelationship } from '@/shared/lib/resolveRelationships';
import type { PipelineCardInfo } from '@/entities/pipeline/model/cardInfo';
import type { OrganizationMemberWithProfile } from 'shared/types';
import type { Issue, IssueTag, PullRequest, Tag } from 'shared/remote-types';
import type { BoardColumn } from '../model/boardModel';
import { columnEmptyStateKind, wipState } from '../model/columnState';
import {
  buildPrBadge,
  buildTestBadge,
  buildWorkspaceBadge,
} from '../model/cardBadges';
import type { DensityClasses } from '../model/density';
import { stageLabelKey } from '../model/stageType';
import {
  pipelineColumnEmptyKey,
  pipelineColumnHintKey,
  pipelineColumnTitleKey,
} from '../model/pipelineColumns';
import { cn } from '@/shared/lib/utils';

export type KanbanColumnProps = {
  column: BoardColumn;
  issueMap: Record<string, Issue>;
  issueAssigneesMap: Record<string, OrganizationMemberWithProfile[]>;
  workspacesByIssueId: Map<string, WorkspaceWithStats[]>;
  tags: Tag[];
  selectedIssueId: string | null;
  selectedIssueIds: Set<string>;
  isMultiSelectActive: boolean;
  isMobile: boolean;
  /** WIP 上限。列头计数超过它就变色提示，**只提示不阻止**。 */
  wipLimit: number;
  /** 当前是否有生效的筛选，决定空列显示哪种引导。 */
  hasActiveFilters: boolean;
  /**
   * 是否显示泳道阶段徽标。团队版（remote 数据源）所有列都会回落成同一个阶段，
   * 由调用方用 `shouldShowStageBadges` 统一判断后传进来。
   */
  showStageBadge: boolean;
  /** 负责人头像只在团队版出现。 */
  showAssignees: boolean;
  /** 密度对应的 class 与开关。 */
  density: DensityClasses;
  /**
   * 个人版流水线看板（设计文档 §8.3）：列名换成流水线列名、不显示列内新建、
   * 空列说明原因。默认 false，团队版行为不变。
   */
  isPipelineBoard?: boolean;
  /** 个人版：需求 id → 卡片底部进度格与状态标签。没有流水线的需求不在表里。 */
  pipelineCards?: ReadonlyMap<string, PipelineCardInfo>;
  getPullRequestsForIssue: (issueId: string) => PullRequest[];
  getTagObjectsForIssue: (issueId: string) => Tag[];
  getTagsForIssue: (issueId: string) => IssueTag[];
  getResolvedRelationshipsForIssue: (issueId: string) => ResolvedRelationship[];
  onAddIssue: (statusId: string) => void;
  onClearFilters: () => void;
  onCardClick: (issueId: string, event?: MouseEvent) => void;
  onCardPriorityClick: (issueId: string) => void;
  onCardAssigneeClick: (issueId: string) => void;
  onCardMoreActionsClick: (issueId: string) => void;
  onCardTagToggle: (issueId: string, tagId: string) => void;
  onCreateTag: (data: { name: string; color: string }) => string;
  onOpenIssueWorkspace: (issueId: string, workspaceId: string) => void;
};

/**
 * 看板的单个泳道。纯展示：所有数据与回调走 props，**内部不用任何 hook**。
 * （需要翻译的文案都交给 `KanbanColumnHeader` / `KanbanColumnEmptyState` /
 * `KanbanCardContent` 这些 UI 组件自己去 `useTranslation`；流水线卡片信息
 * 由容器翻译好传进来。）
 */
export function KanbanColumn({
  column,
  issueMap,
  issueAssigneesMap,
  workspacesByIssueId,
  tags,
  selectedIssueId,
  selectedIssueIds,
  isMultiSelectActive,
  isMobile,
  wipLimit,
  hasActiveFilters,
  showStageBadge,
  showAssignees,
  density,
  isPipelineBoard = false,
  pipelineCards,
  getPullRequestsForIssue,
  getTagObjectsForIssue,
  getTagsForIssue,
  getResolvedRelationshipsForIssue,
  onAddIssue,
  onClearFilters,
  onCardClick,
  onCardPriorityClick,
  onCardAssigneeClick,
  onCardMoreActionsClick,
  onCardTagToggle,
  onCreateTag,
  onOpenIssueWorkspace,
}: KanbanColumnProps) {
  const { status, stage, issueIds, count } = column;
  const emptyStateKind = columnEmptyStateKind(count, hasActiveFilters);
  const isCompact = !density.showDescription;

  return (
    <KanbanBoard>
      <KanbanHeader>
        <KanbanColumnHeader
          name={status.name}
          nameKey={
            isPipelineBoard ? pipelineColumnTitleKey(status.name, stage) : null
          }
          color={status.color}
          count={count}
          wip={wipState(count, wipLimit)}
          wipLimit={wipLimit}
          stageLabelKey={showStageBadge ? stageLabelKey(stage) : null}
          hintKey={isPipelineBoard ? pipelineColumnHintKey(stage) : null}
          showAddButton={!isPipelineBoard}
          dataStage={stage}
          onAddIssue={() => onAddIssue(status.id)}
        />
      </KanbanHeader>
      <KanbanCards id={status.id}>
        {issueIds.map((issueId, index) => {
          const issue = issueMap[issueId];
          if (!issue) return null;
          const issueWorkspaces = workspacesByIssueId.get(issue.id) ?? [];
          const workspaceIdsShownOnCard = new Set(
            issueWorkspaces.map((workspace) => workspace.id)
          );
          const allIssuePullRequests = getPullRequestsForIssue(issue.id);
          const issueCardPullRequests = allIssuePullRequests.filter((pr) => {
            if (!pr.workspace_id) {
              return true;
            }

            // 已经在工作区卡片里露出的 PR，不在需求层重复渲染。
            return !workspaceIdsShownOnCard.has(pr.workspace_id);
          });
          const pipelineCard = pipelineCards?.get(issue.id) ?? null;

          return (
            <KanbanCard
              key={issue.id}
              id={issue.id}
              name={issue.title}
              index={index}
              className={cn(
                'group',
                // 失败红边框：颜色 + 文字 + 进度格三重编码（设计文档 §8.5）
                pipelineCard?.isFailed && 'ring-1 ring-stage-failed'
              )}
              onClick={(e) => onCardClick(issue.id, e)}
              isOpen={selectedIssueId === issue.id}
              isMobile={isMobile}
              isSelected={selectedIssueIds.has(issue.id)}
              dragDisabled={isMultiSelectActive}
            >
              <KanbanCardContent
                displayId={issue.simple_id}
                title={issue.title}
                description={issue.description}
                priority={issue.priority}
                tags={getTagObjectsForIssue(issue.id)}
                assignees={issueAssigneesMap[issue.id] ?? []}
                pullRequests={issueCardPullRequests}
                relationships={getResolvedRelationshipsForIssue(issue.id)}
                isSubIssue={!!issue.parent_issue_id}
                isMobile={isMobile}
                workspaceBadge={buildWorkspaceBadge(issueWorkspaces)}
                // 紧凑模式下把逐个 PR 链接收成一个汇总徽标；舒适模式保留可点链接。
                prBadge={isCompact ? buildPrBadge(issueCardPullRequests) : null}
                testBadge={buildTestBadge()}
                showAssignees={showAssignees}
                showDescription={density.showDescription}
                onPriorityClick={(e) => {
                  e.stopPropagation();
                  onCardPriorityClick(issue.id);
                }}
                onAssigneeClick={(e) => {
                  e.stopPropagation();
                  onCardAssigneeClick(issue.id);
                }}
                onMoreActionsClick={() => onCardMoreActionsClick(issue.id)}
                tagEditProps={{
                  allTags: tags,
                  selectedTagIds: getTagsForIssue(issue.id).map(
                    (it) => it.tag_id
                  ),
                  onTagToggle: (tagId) => onCardTagToggle(issue.id, tagId),
                  onCreateTag,
                  renderTagEditor: ({
                    allTags,
                    selectedTagIds,
                    onTagToggle,
                    onCreateTag: onCreateTagFromEditor,
                    trigger,
                  }) => (
                    <SearchableTagDropdownContainer
                      tags={allTags}
                      selectedTagIds={selectedTagIds}
                      onTagToggle={onTagToggle}
                      onCreateTag={onCreateTagFromEditor}
                      disabled={false}
                      contentClassName=""
                      trigger={trigger}
                    />
                  ),
                }}
              />
              {pipelineCard && (
                <div
                  data-testid="kanban-card-pipeline"
                  data-issue-id={issue.id}
                  className="mt-half flex flex-col gap-half"
                >
                  <div className="flex items-center">
                    <PipelineStatusTag
                      tone={pipelineCard.tone}
                      label={pipelineCard.statusText}
                    />
                  </div>
                  <PipelineProgressBar
                    cells={pipelineCard.cells}
                    ariaLabel={pipelineCard.progressLabel}
                  />
                </div>
              )}
              {issueWorkspaces.length > 0 && (
                <div className={cn('mt-base flex flex-col', density.cards)}>
                  {issueWorkspaces.map((workspace) => (
                    <IssueWorkspaceCard
                      key={workspace.id}
                      workspace={workspace}
                      onClick={
                        workspace.localWorkspaceId
                          ? () =>
                              onOpenIssueWorkspace(
                                issue.id,
                                workspace.localWorkspaceId!
                              )
                          : undefined
                      }
                      showOwner={false}
                      showStatusBadge={false}
                      showNoPrText={false}
                    />
                  ))}
                </div>
              )}
            </KanbanCard>
          );
        })}
        {emptyStateKind !== 'none' && (
          <KanbanColumnEmptyState
            kind={emptyStateKind}
            hintKey={
              isPipelineBoard && emptyStateKind === 'empty'
                ? pipelineColumnEmptyKey(stage)
                : undefined
            }
            onCreateIssue={
              emptyStateKind === 'empty' && !isPipelineBoard
                ? () => onAddIssue(status.id)
                : undefined
            }
            onClearFilters={
              emptyStateKind === 'filtered' ? onClearFilters : undefined
            }
          />
        )}
      </KanbanCards>
    </KanbanBoard>
  );
}
