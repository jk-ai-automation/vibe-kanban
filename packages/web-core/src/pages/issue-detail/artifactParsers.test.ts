import { describe, expect, it } from 'vitest';
import {
  formatDuration,
  parseCsv,
  parseReview,
  parseTestReport,
} from './artifactParsers';

/** 旧报告没有 U4 新字段时，用例上这三项都是空。 */
const NEW_EMPTY = { durationS: null, module: null, source: null };

describe('parseCsv', () => {
  it('第一行是表头', () => {
    expect(parseCsv('用例编号,标题\nTC-1,下单\nTC-2,撤单\n')).toEqual({
      header: ['用例编号', '标题'],
      rows: [
        ['TC-1', '下单'],
        ['TC-2', '撤单'],
      ],
    });
  });

  it('引号内的逗号、换行与转义引号', () => {
    const csv = 'a,b\n"x, y","第一行\n第二行"\n"say ""hi""",z';
    expect(parseCsv(csv).rows).toEqual([
      ['x, y', '第一行\n第二行'],
      ['say "hi"', 'z'],
    ]);
  });

  it('CRLF 与 BOM', () => {
    expect(parseCsv('\uFEFFa,b\r\n1,2\r\n')).toEqual({
      header: ['a', 'b'],
      rows: [['1', '2']],
    });
  });

  it('空字段保留，空行跳过', () => {
    expect(parseCsv('a,b,c\n1,,3\n\n')).toEqual({
      header: ['a', 'b', 'c'],
      rows: [['1', '', '3']],
    });
  });

  it('空输入', () => {
    expect(parseCsv('')).toEqual({ header: [], rows: [] });
  });
});

describe('parseReview', () => {
  it('解析契约里的最小结构', () => {
    const result = parseReview(
      '{"findings":[{"severity":"blocker","file":"src/a.rs","line":3,"message":"未处理错误"}]}'
    );
    expect(result).toEqual({
      ok: true,
      value: [
        {
          severity: 'blocker',
          file: 'src/a.rs',
          line: 3,
          message: '未处理错误',
        },
      ],
    });
  });

  it('不认识的严重度归为 unknown，缺字段给默认值', () => {
    const result = parseReview('{"findings":[{"severity":"nit"}]}');
    expect(result).toEqual({
      ok: true,
      value: [{ severity: 'unknown', file: null, line: null, message: '' }],
    });
  });

  it('不是 JSON 或结构不对', () => {
    expect(parseReview('oops')).toEqual({ ok: false });
    expect(parseReview('{"items":[]}')).toEqual({ ok: false });
  });
});

describe('parseTestReport', () => {
  it('解析契约里的最小结构', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 2,
        passed: 1,
        failed: 1,
        cases: [
          {
            id: 'kline_021',
            status: 'failed',
            attribution: 'code',
            message: 'next_cursor 应为空',
          },
          { id: 'kline_020', status: 'passed', attribution: null, message: '' },
        ],
      })
    );
    expect(result).toEqual({
      ok: true,
      value: {
        total: 2,
        passed: 1,
        failed: 1,
        reportPath: null,
        cases: [
          {
            ...NEW_EMPTY,
            id: 'kline_021',
            status: 'failed',
            attribution: 'code',
            message: 'next_cursor 应为空',
          },
          {
            ...NEW_EMPTY,
            id: 'kline_020',
            status: 'passed',
            attribution: null,
            message: '',
          },
        ],
      },
    });
  });

  it('缺计数时按用例现算', () => {
    const result = parseTestReport(
      '{"cases":[{"id":"a","status":"passed"},{"id":"b","status":"failed","attribution":"case"}]}'
    );
    expect(result.ok && result.value).toMatchObject({
      total: 2,
      passed: 1,
      failed: 1,
    });
  });

  it('结构不对', () => {
    expect(parseTestReport('[]')).toEqual({ ok: false });
  });
});

describe('宽松解析：字段类型不符不崩溃（后端对非判定字段同样宽松）', () => {
  it('评审：file 是数字、line 是区间字符串、message 不是字符串', () => {
    const result = parseReview(
      '{"findings":[{"severity":"major","file":3,"line":"12-15","message":["x"]},{"severity":1},"oops",null]}'
    );
    expect(result).toEqual({
      ok: true,
      value: [
        { severity: 'major', file: '3', line: '12-15', message: '' },
        { severity: 'unknown', file: null, line: null, message: '' },
      ],
    });
  });

  it('评审：空白 line 与非有限数字当作没有', () => {
    const result = parseReview(
      '{"findings":[{"severity":"minor","line":"  "},{"severity":"minor","line":1e999}]}'
    );
    expect(result.ok && result.value.map((f) => f.line)).toEqual([null, null]);
  });

  it('测试报告：id 是数字、message 是对象、attribution 不是字符串、计数是字符串', () => {
    const result = parseTestReport(
      '{"total":"2","passed":1,"failed":1,"cases":[{"id":7,"status":"failed","attribution":1,"message":{"a":1}},{"status":"skipped"}]}'
    );
    expect(result).toEqual({
      ok: true,
      value: {
        total: 2,
        passed: 1,
        failed: 1,
        reportPath: null,
        cases: [
          {
            ...NEW_EMPTY,
            id: '7',
            status: 'failed',
            attribution: null,
            message: '',
          },
          {
            ...NEW_EMPTY,
            id: '',
            status: 'unknown',
            attribution: null,
            message: '',
          },
        ],
      },
    });
  });

  it('测试报告：cases 缺失时当作空列表，计数照读', () => {
    expect(parseTestReport('{"total":3,"passed":3,"failed":0}')).toEqual({
      ok: true,
      value: { total: 3, passed: 3, failed: 0, cases: [], reportPath: null },
    });
  });
});

describe('parseTestReport（U4 新字段）', () => {
  it('读出耗时、模块、来源与报告路径', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 2,
        passed: 1,
        failed: 1,
        report_path: 'atp/test_results/test_report_1.html',
        cases: [
          {
            id: 'vk_gen_001',
            status: 'passed',
            attribution: null,
            message: '',
            duration_s: 0.42,
            module: 'vk_gen',
            source: 'atp',
          },
          {
            id: 'tests::add',
            status: 'failed',
            attribution: 'code',
            message: '断言失败',
            source: 'repo',
          },
        ],
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBe('atp/test_results/test_report_1.html');
    expect(result.value.cases[0]).toMatchObject({
      durationS: 0.42,
      module: 'vk_gen',
      source: 'atp',
    });
    expect(result.value.cases[1]).toMatchObject({
      durationS: null,
      module: null,
      source: 'repo',
    });
  });

  it('旧报告没有新字段时全部为空', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 1,
        passed: 1,
        failed: 0,
        cases: [{ id: 'a', status: 'passed', attribution: null, message: '' }],
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBeNull();
    expect(result.value.cases[0]).toMatchObject({
      durationS: null,
      module: null,
      source: null,
    });
  });

  it('来源不是 atp 或 repo 时当作未知，耗时为负或非数字时为空', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 1,
        cases: [
          {
            id: 'a',
            status: 'passed',
            source: 'other',
            duration_s: -1,
            module: 3,
          },
        ],
        report_path: 42,
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBeNull();
    expect(result.value.cases[0]).toMatchObject({
      durationS: null,
      module: '3',
      source: null,
    });
  });
});

describe('formatDuration', () => {
  it('按量级取单位', () => {
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(0.0421)).toBe('42 ms');
    expect(formatDuration(1.234)).toBe('1.23 s');
    expect(formatDuration(75)).toBe('1 min 15 s');
  });
});
