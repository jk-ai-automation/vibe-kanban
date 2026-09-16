'use client';

import type { MouseEvent, ReactNode } from 'react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import {
  CircleDashedIcon,
  DotsThreeIcon,
  PlusIcon,
} from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import { PriorityIcon, type PriorityLevel } from './PriorityIcon';
import { KanbanBadge } from './KanbanBadge';
import { KanbanAssignee, type KanbanAssigneeUser } from './KanbanAssignee';
import { RunningDots } from './RunningDots';
import { PrBadge, type PrBadgeStatus } from './PrBadge';
import {
  RelationshipBadge,
  type RelationshipDisplayType,
} from './RelationshipBadge';
import {
  KanbanStatusBadge,
  type KanbanStatusBadgeTone,
} from './KanbanStatusBadge';

/**
 * 卡片底部的状态徽标（设计文档 §7.2）。
 * 判定在 `@/features/kanban/model/cardBadges`，这里只拿 key 翻译后渲染。
 */
export interface KanbanCardStatusBadge {
  tone: KanbanStatusBadgeTone;
  labelKey: string;
  count: number;
}

export interface KanbanTag {
  id: string;
  name: string;
  color: string;
}

export interface KanbanRelationship {
  relationshipId: string;
  displayType: RelationshipDisplayType;
  relatedIssueDisplayId: string;
}

export interface KanbanPullRequest {
  id: string;
  number: number;
  url: string;
  status: PrBadgeStatus;
}

export interface TagEditRenderProps<TTag extends KanbanTag = KanbanTag> {
  allTags: TTag[];
  selectedTagIds: string[];
  onTagToggle: (tagId: string) => void;
  onCreateTag: (data: { name: string; color: string }) => string;
  trigger: ReactNode;
}

export interface TagEditProps<TTag extends KanbanTag = KanbanTag> {
  allTags: TTag[];
  selectedTagIds: string[];
  onTagToggle: (tagId: string) => void;
  onCreateTag: (data: { name: string; color: string }) => string;
  renderTagEditor?: (props: TagEditRenderProps<TTag>) => ReactNode;
}

const IMAGE_FILE_EXTENSION_REGEX =
  /\.(png|jpe?g|gif|webp|bmp|svg|avif|heic|heif)$/i;

function isImageLikeAttachmentName(name: string): boolean {
  const normalized = name.trim();
  if (!normalized) {
    return false;
  }

  return IMAGE_FILE_EXTENSION_REGEX.test(normalized);
}

function formatKanbanDescriptionPreview(
  markdown: string,
  options: {
    codeBlockLabel: string;
    imageLabel: string;
    imageWithNameLabel: (name: string) => string;
    fileLabel: string;
    fileWithNameLabel: (name: string) => string;
  }
): string {
  return markdown
    .replace(/```[\s\S]*?```/g, options.codeBlockLabel)
    .replace(
      /!\[([^\]]*)\]\(([^)]+)\)/g,
      (_match, altText: string, url: string) => {
        const normalizedAlt = altText.trim();
        const normalizedUrl = url.trim();
        const isImageAttachment =
          normalizedUrl.startsWith('attachment://') &&
          isImageLikeAttachmentName(normalizedAlt);

        if (isImageAttachment) {
          return normalizedAlt
            ? options.imageWithNameLabel(normalizedAlt)
            : options.imageLabel;
        }

        return normalizedAlt
          ? options.fileWithNameLabel(normalizedAlt)
          : options.fileLabel;
      }
    )
    .replace(
      /(?<!!)\[([^\]]*)\]\((attachment:\/\/[^)]+|\.vibe-attachments\/[^)]+)\)/g,
      (_match, label: string) => {
        const normalizedLabel = label.trim();
        return normalizedLabel
          ? options.fileWithNameLabel(normalizedLabel)
          : options.fileLabel;
      }
    )
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '$1')
    .replace(/^#{1,6}\s+/gm, '')
    .replace(/^\s*>\s?/gm, '')
    .replace(/^\s*([-*+]|\d+\.)\s+/gm, '')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/__([^_]+)__/g, '$1')
    .replace(/\*([^*]+)\*/g, '$1')
    .replace(/_([^_]+)_/g, '$1')
    .replace(/~~([^~]+)~~/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

export type KanbanCardContentProps<TTag extends KanbanTag = KanbanTag> = {
  displayId: string;
  title: string;
  description?: string | null;
  priority: PriorityLevel | null;
  tags: KanbanTag[];
  assignees: KanbanAssigneeUser[];
  pullRequests?: KanbanPullRequest[];
  relationships?: KanbanRelationship[];
  isSubIssue?: boolean;
  isLoading?: boolean;
  className?: string;
  onPriorityClick?: (e: MouseEvent) => void;
  onAssigneeClick?: (e: MouseEvent) => void;
  onMoreActionsClick?: () => void;
  tagEditProps?: TagEditProps<TTag>;
  isMobile?: boolean;

  // ---- 以下都是可选、都有默认值，保证现有调用点（列表视图等）不破 ----
  /** 关联工作区的运行状态徽标。`null` / 不传就不渲染。 */
  workspaceBadge?: KanbanCardStatusBadge | null;
  /** PR 状态徽标。`null` / 不传就退回原来的 `pullRequests` 逐个渲染。 */
  prBadge?: KanbanCardStatusBadge | null;
  /** 测试结果徽标（本期占位）。 */
  testBadge?: KanbanCardStatusBadge | null;
  /** 负责人头像。个人版传 `false`，**不留空占位**。 */
  showAssignees?: boolean;
  /** 紧凑密度下收起描述预览。 */
  showDescription?: boolean;
};

export function KanbanCardContent<TTag extends KanbanTag = KanbanTag>({
  displayId,
  title,
  description,
  priority,
  tags,
  assignees,
  pullRequests = [],
  relationships = [],
  isSubIssue,
  isLoading = false,
  className,
  onPriorityClick,
  onAssigneeClick,
  onMoreActionsClick,
  tagEditProps,
  isMobile,
  workspaceBadge = null,
  prBadge = null,
  testBadge = null,
  showAssignees = true,
  showDescription = true,
}: KanbanCardContentProps<TTag>) {
  const { t } = useTranslation('common');
  const previewDescription = useMemo(() => {
    if (!description) {
      return null;
    }

    const formatted = formatKanbanDescriptionPreview(description, {
      codeBlockLabel: t('kanban.previewCodeBlock'),
      imageLabel: t('kanban.previewImage'),
      imageWithNameLabel: (name: string) =>
        t('kanban.previewImageWithName', { name }),
      fileLabel: t('kanban.previewFile'),
      fileWithNameLabel: (name: string) =>
        t('kanban.previewFileWithName', { name }),
    });
    return formatted.length > 0 ? formatted : null;
  }, [description, t]);

  const tagsDisplay = (
    <>
      {tags.slice(0, 2).map((tag) => (
        <KanbanBadge key={tag.id} name={tag.name} color={tag.color} />
      ))}
      {tags.length > 2 && (
        <span className="text-sm text-low">+{tags.length - 2}</span>
      )}
      {tagEditProps && tags.length === 0 && (
        <PlusIcon className="size-icon-xs text-low" weight="bold" />
      )}
    </>
  );
  const tagEditorTrigger = (
    <button
      type="button"
      onClick={(e) => e.stopPropagation()}
      className="flex items-center gap-half cursor-pointer hover:bg-secondary rounded-sm transition-colors"
    >
      {tagsDisplay}
    </button>
  );

  return (
    <div className={cn('flex flex-col gap-half min-w-0', className)}>
      {/* Row 1: Task ID + sub-issue indicator + loading dots + more actions */}
      <div className="flex items-center justify-between gap-half">
        <div className="flex items-center gap-half min-w-0">
          {isSubIssue && (
            <span className="text-sm text-low">
              {t('kanban.subIssueIndicator')}
            </span>
          )}
          <span className="font-ibm-plex-mono text-sm text-low truncate">
            {displayId}
          </span>
          {isLoading && <RunningDots />}
        </div>
        {onMoreActionsClick && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onMoreActionsClick();
            }}
            onMouseDown={(e) => e.stopPropagation()}
            className={cn(
              'p-half -m-half rounded-sm text-low hover:text-normal hover:bg-secondary shrink-0',
              isMobile
                ? ''
                : 'invisible opacity-0 group-hover:visible group-hover:opacity-100',
              'transition-[opacity,color,background-color]'
            )}
            aria-label="More actions"
            title="More actions"
          >
            <DotsThreeIcon className="size-icon-xs" weight="bold" />
          </button>
        )}
      </div>

      {/* Row 2: Title */}
      <span className="text-base text-normal truncate">{title}</span>

      {/* Row 3: Description (optional, truncated; 紧凑密度下收起) */}
      {showDescription && previewDescription && (
        <p
          className={cn(
            'text-sm text-low m-0',
            isMobile
              ? 'leading-tight line-clamp-2'
              : 'leading-relaxed line-clamp-4'
          )}
        >
          {previewDescription}
        </p>
      )}

      {/* Row 4: Priority + Assignee */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-half min-w-0">
          {onPriorityClick ? (
            <button
              type="button"
              onClick={onPriorityClick}
              onMouseDown={(e) => e.stopPropagation()}
              className="flex items-center cursor-pointer hover:bg-secondary rounded-sm transition-colors"
            >
              <PriorityIcon priority={priority} />
              {!priority && (
                <CircleDashedIcon
                  className="size-icon-xs text-low"
                  weight="bold"
                />
              )}
            </button>
          ) : (
            <PriorityIcon priority={priority} />
          )}
        </div>
        {/* 负责人头像只在团队版出现；个人版 showAssignees=false，整块不渲染，不留空占位 */}
        {showAssignees &&
          (onAssigneeClick ? (
            <button
              type="button"
              onClick={onAssigneeClick}
              onMouseDown={(e) => e.stopPropagation()}
              className="cursor-pointer hover:bg-secondary rounded-sm transition-colors"
            >
              <KanbanAssignee assignees={assignees} />
            </button>
          ) : (
            <KanbanAssignee assignees={assignees} />
          ))}
      </div>

      {/* Row 5: 工作区 → PR → 测试 → 标签 → 关系（独立一行，避免溢出）
          前三个是设计文档 §7.2 的状态徽标，都可选；不传时这一行退回改造前的内容。 */}
      {(tags.length > 0 ||
        tagEditProps ||
        pullRequests.length > 0 ||
        relationships.length > 0 ||
        workspaceBadge ||
        prBadge ||
        testBadge) && (
        <div className="flex items-center gap-half flex-wrap min-w-0">
          {workspaceBadge && (
            <KanbanStatusBadge
              tone={workspaceBadge.tone}
              label={t(workspaceBadge.labelKey)}
              count={workspaceBadge.count}
            />
          )}
          {prBadge && (
            <KanbanStatusBadge
              tone={prBadge.tone}
              label={t(prBadge.labelKey)}
              count={prBadge.count}
            />
          )}
          {testBadge && (
            <KanbanStatusBadge
              tone={testBadge.tone}
              label={t(testBadge.labelKey)}
            />
          )}
          {tagEditProps ? (
            (tagEditProps.renderTagEditor?.({
              allTags: tagEditProps.allTags,
              selectedTagIds: tagEditProps.selectedTagIds,
              onTagToggle: tagEditProps.onTagToggle,
              onCreateTag: tagEditProps.onCreateTag,
              trigger: tagEditorTrigger,
            }) ?? tagEditorTrigger)
          ) : (
            <>
              {tags.slice(0, 2).map((tag) => (
                <KanbanBadge key={tag.id} name={tag.name} color={tag.color} />
              ))}
              {tags.length > 2 && (
                <span className="text-sm text-low">+{tags.length - 2}</span>
              )}
            </>
          )}
          {/* 汇总徽标已经替代了逐个 PR 链接，两者只显示一种，避免重复 */}
          {!prBadge && (
            <>
              {pullRequests.slice(0, 2).map((pr) => (
                <PrBadge
                  key={pr.id}
                  number={pr.number}
                  url={pr.url}
                  status={pr.status}
                />
              ))}
              {pullRequests.length > 2 && (
                <span className="text-sm text-low">
                  +{pullRequests.length - 2}
                </span>
              )}
            </>
          )}
          {relationships.slice(0, 2).map((rel) => (
            <RelationshipBadge
              key={rel.relationshipId}
              displayType={rel.displayType}
              relatedIssueDisplayId={rel.relatedIssueDisplayId}
              compact
            />
          ))}
          {relationships.length > 2 && (
            <span className="text-sm text-low">
              +{relationships.length - 2}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
