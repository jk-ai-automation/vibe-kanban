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
 * JSX 从 KanbanContainer 原样搬出，行为不变。
 */
export function KanbanBoardView({
  columns,
  onDragEnd,
  ...columnProps
}: KanbanBoardViewProps) {
  return (
    <div className="flex-1 overflow-x-auto px-double">
      <KanbanProvider onDragEnd={onDragEnd}>
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
