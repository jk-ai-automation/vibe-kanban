import { describe, expect, it } from 'vitest';

import {
  extractFallbackRows,
  getRowKey,
  parseResponseError,
} from '@/shared/lib/electric/rows';

describe('getRowKey', () => {
  it('优先使用 id', () => {
    expect(getRowKey({ id: 'abc', issue_id: 'x', tag_id: 'y' })).toBe('abc');
  });

  it('没有 id 时按字母序拼接所有 *_id 字段', () => {
    expect(getRowKey({ tag_id: 'tag', issue_id: 'issue' })).toBe('issue-tag');
  });

  it('id 为空字符串时回退到复合键', () => {
    expect(getRowKey({ id: '', issue_id: 'issue', tag_id: 'tag' })).toBe(
      'issue-tag'
    );
  });
});

describe('extractFallbackRows', () => {
  it('按表名取出数组', () => {
    expect(extractFallbackRows({ issues: [{ id: '1' }] }, 'issues')).toEqual([
      { id: '1' },
    ]);
  });

  it('空数组是合法结果', () => {
    expect(extractFallbackRows({ issues: [] }, 'issues')).toEqual([]);
  });

  it('响应不是对象时抛错', () => {
    expect(() => extractFallbackRows(null, 'issues')).toThrow(/not an object/);
  });

  it('缺少目标数组时抛错并带上表名', () => {
    expect(() => extractFallbackRows({ other: [] }, 'issues')).toThrow(
      /issues/
    );
  });
});

describe('parseResponseError', () => {
  it('优先读 message 字段', async () => {
    const response = new Response(JSON.stringify({ message: '坏了' }), {
      status: 400,
    });
    await expect(parseResponseError(response, '默认')).resolves.toBe('坏了');
  });

  it('响应不是 JSON 时回退到默认文案', async () => {
    const response = new Response('<html>', { status: 500 });
    await expect(parseResponseError(response, '默认')).resolves.toBe('默认');
  });
});
