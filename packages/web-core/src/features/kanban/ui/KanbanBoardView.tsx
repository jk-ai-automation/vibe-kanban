import {
  KanbanProvider,
  type DropResult,
} from '@vibe/ui/components/KanbanBoard';
import { KanbanColumn, type KanbanColumnProps } from './KanbanColumn';
import type { BoardColumn } from '../model/boardModel';

type ColumnRenderProps = Omit<KanbanColumnProps, 'column'>;

export type KanbanBoardViewProps = ColumnRenderProps & {
  columns: BoardColumn[];
  onDragEnd: (result: DropResult) => void;
};

/**
 * 看板渲染区：拖拽容器 + 逐列渲染。纯展示，内部不用任何 hook。
 *
 * 把列数传给 `KanbanProvider`，让网格按宽度自适应——1280px 窄屏下六列也不会
 * 撑出横向滚动条（设计文档 §11.7）。
 */
export function KanbanBoardView({
  columns,
  onDragEnd,
  ...columnProps
}: KanbanBoardViewProps) {
  return (
    <div className="flex-1 overflow-x-auto px-double">
      <KanbanProvider onDragEnd={onDragEnd} columnCount={columns.length}>
        {columns.map((column) => (
          <KanbanColumn
            key={column.status.id}
            column={column}
            {...columnProps}
          />
        ))}
      </KanbanProvider>
    </div>
  );
}
