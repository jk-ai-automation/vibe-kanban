import type { MouseEvent } from 'react';
import { PlusIcon } from '@phosphor-icons/react';
import {
  KanbanBoard,
  KanbanCard,
  KanbanCards,
  KanbanHeader,
} from '@vibe/ui/components/KanbanBoard';
import { KanbanCardContent } from '@vibe/ui/components/KanbanCardContent';
import {
  IssueWorkspaceCard,
  type WorkspaceWithStats,
} from '@vibe/ui/components/IssueWorkspaceCard';
import { SearchableTagDropdownContainer } from '@/shared/components/SearchableTagDropdownContainer';
import type { ResolvedRelationship } from '@/shared/lib/resolveRelationships';
import type { OrganizationMemberWithProfile } from 'shared/types';
import type { Issue, IssueTag, PullRequest, Tag } from 'shared/remote-types';
import type { BoardColumn } from '../model/boardModel';

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
  getPullRequestsForIssue: (issueId: string) => PullRequest[];
  getTagObjectsForIssue: (issueId: string) => Tag[];
  getTagsForIssue: (issueId: string) => IssueTag[];
  getResolvedRelationshipsForIssue: (issueId: string) => ResolvedRelationship[];
  onAddIssue: (statusId: string) => void;
  onCardClick: (issueId: string, event?: MouseEvent) => void;
  onCardPriorityClick: (issueId: string) => void;
  onCardAssigneeClick: (issueId: string) => void;
  onCardMoreActionsClick: (issueId: string) => void;
  onCardTagToggle: (issueId: string, tagId: string) => void;
  onCreateTag: (data: { name: string; color: string }) => string;
  onOpenIssueWorkspace: (issueId: string, workspaceId: string) => void;
};

/**
 * 看板的单个泳道。纯展示：所有数据与回调走 props，内部不用任何 hook。
 * JSX 从 KanbanContainer 原样搬出，行为不变。
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
  getPullRequestsForIssue,
  getTagObjectsForIssue,
  getTagsForIssue,
  getResolvedRelationshipsForIssue,
  onAddIssue,
  onCardClick,
  onCardPriorityClick,
  onCardAssigneeClick,
  onCardMoreActionsClick,
  onCardTagToggle,
  onCreateTag,
  onOpenIssueWorkspace,
}: KanbanColumnProps) {
  const { status, issueIds } = column;

  return (
    <KanbanBoard>
      <KanbanHeader>
        <div className="border-t sticky border-b top-0 z-20 flex shrink-0 items-center justify-between gap-2 p-base bg-secondary">
          <div className="flex items-center gap-2">
            <div
              className="h-2 w-2 rounded-full shrink-0"
              style={{ backgroundColor: `hsl(${status.color})` }}
            />
            <p className="m-0 text-sm">{status.name}</p>
          </div>
          <button
            type="button"
            onClick={() => onAddIssue(status.id)}
            className="p-half rounded-sm text-low hover:text-normal hover:bg-secondary transition-colors"
            aria-label="Add task"
          >
            <PlusIcon className="size-icon-xs" weight="bold" />
          </button>
        </div>
      </KanbanHeader>
      <KanbanCards id={status.id}>
        {issueIds.map((issueId, index) => {
          const issue = issueMap[issueId];
          if (!issue) return null;
          const issueWorkspaces = workspacesByIssueId.get(issue.id) ?? [];
          const workspaceIdsShownOnCard = new Set(
            issueWorkspaces.map((workspace) => workspace.id)
          );
          const issueCardPullRequests = getPullRequestsForIssue(
            issue.id
          ).filter((pr) => {
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
                <div className="mt-base flex flex-col gap-half">
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
      </KanbanCards>
    </KanbanBoard>
  );
}
