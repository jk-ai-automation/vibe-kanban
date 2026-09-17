/**
 * 看板卡片键盘导航（设计文档 §7.5：`j/k` 上下移动）。
 *
 * 顺序取自 `KanbanContainer` 已经算好的 `orderedIssueIds`
 * （可见状态列按 sort_order 展平），所以「上下」天然跨列连续。
 */
export type NavDirection = 'up' | 'down';

/**
 * 求下一张要选中的卡片 id。
 *
 * - 列表为空 → `null`。
 * - 当前没有选中：`down` 取第一个，`up` 取最后一个。
 * - **到头就停住，不回绕**：末尾按 `down` 还是末尾，开头按 `up` 还是开头。
 * - 当前 id 不在列表里（卡片被筛掉/删掉）→ 按「没有选中」处理。
 */
export function nextIssueId(
  ids: readonly string[] | null | undefined,
  current: string | null | undefined,
  direction: NavDirection
): string | null {
  if (!ids || ids.length === 0) {
    return null;
  }

  const index = current ? ids.indexOf(current) : -1;

  if (index === -1) {
    return direction === 'down' ? ids[0] : ids[ids.length - 1];
  }

  const nextIndex = direction === 'down' ? index + 1 : index - 1;
  if (nextIndex < 0 || nextIndex >= ids.length) {
    return ids[index];
  }

  return ids[nextIndex];
}

/**
 * 左右移动：在「当前卡片所在列」的基础上换一列，落到目标列的第一张卡片。
 * 目标列为空时继续往同方向找，全都空就停在原地（返回 `current`）。
 *
 * `columns` 是按渲染顺序排好的每列 issue id 数组。
 */
export function siblingColumnIssueId(
  columns: readonly (readonly string[])[] | null | undefined,
  current: string | null | undefined,
  direction: 'left' | 'right'
): string | null {
  if (!columns || columns.length === 0) {
    return null;
  }

  const step = direction === 'right' ? 1 : -1;
  const columnIndex = current
    ? columns.findIndex((column) => column.includes(current))
    : -1;

  if (columnIndex === -1) {
    // 没有选中：从最靠近方向起点的那一列开始找第一张卡片。
    const ordered = direction === 'right' ? columns : [...columns].reverse();
    for (const column of ordered) {
      if (column.length > 0) return column[0];
    }
    return null;
  }

  for (let i = columnIndex + step; i >= 0 && i < columns.length; i += step) {
    if (columns[i].length > 0) {
      return columns[i][0];
    }
  }

  return current ?? null;
}
