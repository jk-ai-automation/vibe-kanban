import { cn } from '../lib/cn';

export interface PipelineTimelineItem {
  id: string;
  actor: 'ai' | 'human';
  /** 已翻译：「AI」/「你」。 */
  actorLabel: string;
  /** 已翻译的事件文案。 */
  text: string;
  /** 摘要 / 错误 / 打回意见，原样展示。 */
  detail: string | null;
  /** 「14:03 · 设计规格 · 1 分 12 秒」。 */
  meta: string;
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
            {item.detail && (
              <p className="m-0 break-words text-sm text-low">{item.detail}</p>
            )}
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
