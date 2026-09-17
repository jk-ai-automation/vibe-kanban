import { describe, expect, it } from 'vitest';
import { nextIssueId, siblingColumnIssueId } from './cardNavigation';

describe('nextIssueId（j/k 上下移动）', () => {
  const ids = ['a', 'b', 'c'];

  it('没有选中时 down 取第一个', () => {
    expect(nextIssueId(ids, null, 'down')).toBe('a');
  });

  it('没有选中时 up 取最后一个', () => {
    expect(nextIssueId(ids, null, 'up')).toBe('c');
  });

  it('正常往下走', () => {
    expect(nextIssueId(ids, 'a', 'down')).toBe('b');
  });

  it('正常往上走', () => {
    expect(nextIssueId(ids, 'c', 'up')).toBe('b');
  });

  it('末尾往下停在末尾（不回绕）', () => {
    expect(nextIssueId(ids, 'c', 'down')).toBe('c');
  });

  it('开头往上停在开头（不回绕）', () => {
    expect(nextIssueId(ids, 'a', 'up')).toBe('a');
  });

  it('空列表返回 null', () => {
    expect(nextIssueId([], null, 'down')).toBeNull();
    expect(nextIssueId([], 'a', 'down')).toBeNull();
    expect(nextIssueId(null, null, 'down')).toBeNull();
  });

  it('当前 id 已被筛掉时按「没有选中」处理', () => {
    expect(nextIssueId(ids, 'zzz', 'down')).toBe('a');
    expect(nextIssueId(ids, 'zzz', 'up')).toBe('c');
  });

  it('顺序跨列连续：orderedIssueIds 怎么排就怎么走', () => {
    // 第一列 a,b；第二列 c。从 b 往下直接跨到第二列的 c。
    expect(nextIssueId(['a', 'b', 'c'], 'b', 'down')).toBe('c');
  });
});

describe('siblingColumnIssueId（h/l 左右换列）', () => {
  const columns = [['a', 'b'], [], ['c']];

  it('往右跳过空列，落到下一个非空列的第一张', () => {
    expect(siblingColumnIssueId(columns, 'a', 'right')).toBe('c');
  });

  it('往左回到上一个非空列的第一张', () => {
    expect(siblingColumnIssueId(columns, 'c', 'left')).toBe('a');
  });

  it('右边没有非空列时停在原地', () => {
    expect(siblingColumnIssueId(columns, 'c', 'right')).toBe('c');
  });

  it('左边没有非空列时停在原地', () => {
    expect(siblingColumnIssueId(columns, 'b', 'left')).toBe('b');
  });

  it('没有选中时 right 取最左边的非空列第一张', () => {
    expect(siblingColumnIssueId(columns, null, 'right')).toBe('a');
  });

  it('没有选中时 left 取最右边的非空列第一张', () => {
    expect(siblingColumnIssueId(columns, null, 'left')).toBe('c');
  });

  it('全空 / 无列时返回 null', () => {
    expect(siblingColumnIssueId([], 'a', 'right')).toBeNull();
    expect(siblingColumnIssueId([[], []], null, 'right')).toBeNull();
    expect(siblingColumnIssueId(null, null, 'right')).toBeNull();
  });
});
