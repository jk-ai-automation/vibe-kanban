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
import { SearchableTagDropdownContainer } from '@/shared/components/SearchableTagDropdownContainer';
import type { ResolvedRelationship } from '@/shared/lib/resolveRelationships';
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
 * `KanbanCardContent` 这些 UI 组件自己去 `useTranslation`。）
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
          color={status.color}
          count={count}
          wip={wipState(count, wipLimit)}
          wipLimit={wipLimit}
          stageLabelKey={showStageBadge ? stageLabelKey(stage) : null}
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

          return (
            <KanbanCard
              key={issue.id}
              id={issue.id}
              name={issue.title}
              index={index}
              className="group"
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
            onCreateIssue={
              emptyStateKind === 'empty'
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
