import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';

export interface PipelineTimelineItem {
  id: string;
  actor: 'ai' | 'human';
  /** 已翻译：「AI」/「你」。 */
  actorLabel: string;
  /** 已翻译的事件文案。 */
  text: string;
  /** 摘要 / 错误 / 打回意见，原样展示（失败原因会带进程输出末尾，可能有几十行）。 */
  detail: string | null;
  /** 「14:03 · 设计规格 · 1 分 12 秒」。 */
  meta: string;
}

/** 超过这个行数（或这个字数）就先折叠，免得一段失败日志把整条时间线顶下去。 */
const DETAIL_COLLAPSE_LINES = 3;
const DETAIL_COLLAPSE_CHARS = 240;

/**
 * 摘要 / 失败原因原文。保留换行（失败原因是「一句中文说明 + 状态 + 日志末尾」的多行文本），
 * 太长时先折叠成三行，给一个展开按钮。
 */
function TimelineDetail({ text }: { text: string }) {
  const { t } = useTranslation('common');
  const [expanded, setExpanded] = useState(false);
  const collapsible =
    text.split('\n').length > DETAIL_COLLAPSE_LINES ||
    text.length > DETAIL_COLLAPSE_CHARS;

  return (
    <div className="flex min-w-0 flex-col items-start gap-0.5">
      <p
        data-testid="timeline-detail"
        className={cn(
          'm-0 whitespace-pre-wrap break-words text-sm text-low',
          collapsible && !expanded && 'line-clamp-3'
        )}
      >
        {text}
      </p>
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="rounded-sm text-xs text-brand hover:underline focus:outline-none focus:ring-1 focus:ring-brand"
        >
          {t(
            expanded
              ? 'issueDetail.timeline.collapse'
              : 'issueDetail.timeline.expand'
          )}
        </button>
      )}
    </div>
  );
}

/** 右栏时间线（设计文档 §8.4）：AI 与人的每一步。 */
export function PipelineTimeline({
  items,
  emptyText,
}: {
  items: PipelineTimelineItem[];
  emptyText: string;
}) {
  if (items.length === 0) {
    return <p className="m-0 text-sm text-low">{emptyText}</p>;
  }
  return (
    <ol
      data-testid="pipeline-timeline"
      className="m-0 flex list-none flex-col gap-base p-0"
    >
      {items.map((item) => (
        <li
          key={item.id}
          data-testid="timeline-item"
          data-actor={item.actor}
          className="flex gap-half"
        >
          <span
            aria-hidden="true"
            className={cn(
              'mt-1 size-2 shrink-0 rounded-full',
              item.actor === 'human' ? 'bg-brand' : 'bg-stage-dev'
            )}
          />
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <p className="m-0 flex items-center gap-half text-sm text-normal">
              <span className="shrink-0 rounded-sm bg-panel px-1 text-xs text-low">
                {item.actorLabel}
              </span>
              <span className="min-w-0">{item.text}</span>
            </p>
            {item.detail && <TimelineDetail text={item.detail} />}
            <p
              data-testid="relative-time"
              className="m-0 font-ibm-plex-mono text-xs text-low"
            >
              {item.meta}
            </p>
          </div>
        </li>
      ))}
    </ol>
  );
}
