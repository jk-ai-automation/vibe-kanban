export interface CsvTable {
  header: string[];
  rows: string[][];
}

const BOM = '﻿';

/** RFC 4180 CSV 解析（atp 旧 CSV 用例，契约 §4 `test-cases.csv`）。 */
export function parseCsv(text: string): CsvTable {
  const input = text.startsWith(BOM) ? text.slice(1) : text;
  const records: string[][] = [];
  let record: string[] = [];
  let field = '';
  let inQuotes = false;
  let i = 0;

  while (i < input.length) {
    const ch = input[i];
    if (inQuotes) {
      if (ch === '"') {
        if (input[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i += 1;
        continue;
      }
      field += ch;
      i += 1;
      continue;
    }
    if (ch === '"' && field === '') {
      inQuotes = true;
      i += 1;
      continue;
    }
    if (ch === ',') {
      record.push(field);
      field = '';
      i += 1;
      continue;
    }
    if (ch === '\r' || ch === '\n') {
      record.push(field);
      records.push(record);
      record = [];
      field = '';
      i += ch === '\r' && input[i + 1] === '\n' ? 2 : 1;
      continue;
    }
    field += ch;
    i += 1;
  }
  if (field !== '' || record.length > 0) {
    record.push(field);
    records.push(record);
  }

  const nonEmpty = records.filter((r) => !(r.length === 1 && r[0] === ''));
  const [header = [], ...rows] = nonEmpty;
  return { header, rows };
}

export type ParseResult<T> = { ok: true; value: T } | { ok: false };

function parseJson(content: string): unknown {
  try {
    return JSON.parse(content);
  } catch {
    return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * 以下读取器都是宽松的：智能体写的 JSON 字段类型经常不规范（后端对非判定
 * 字段同样宽松），类型不符时给 null，绝不抛错。
 */

/** 字符串原样；有限数字转成字符串（如 `"id": 7`）；其它为 null。 */
const text = (value: unknown): string | null => {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  return null;
};

/** 有限数字；数字字符串（如 `"3"`）也认；其它为 null。 */
const num = (value: unknown): number | null => {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
};

export type ReviewSeverity = 'blocker' | 'major' | 'minor' | 'unknown';

export interface ReviewFinding {
  severity: ReviewSeverity;
  file: string | null;
  /** 行号；智能体可能写成区间字符串（如 `"12-15"`），原样保留。 */
  line: number | string | null;
  message: string;
}

/** 与后端判定一致：只认小写的三种严重度（`gates.rs` 按 `== "blocker"` 判定）。 */
function normalizeSeverity(value: unknown): ReviewSeverity {
  return value === 'blocker' || value === 'major' || value === 'minor'
    ? value
    : 'unknown';
}

function normalizeLine(value: unknown): number | string | null {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value === 'string') return value.trim() === '' ? null : value;
  return null;
}

/** 评审结果（契约 §4 `review.json`）。 */
export function parseReview(content: string): ParseResult<ReviewFinding[]> {
  const data = parseJson(content);
  if (!isRecord(data) || !Array.isArray(data.findings)) return { ok: false };
  return {
    ok: true,
    value: data.findings.filter(isRecord).map((finding) => ({
      severity: normalizeSeverity(finding.severity),
      file: text(finding.file),
      line: normalizeLine(finding.line),
      message: typeof finding.message === 'string' ? finding.message : '',
    })),
  };
}

export type TestCaseStatus = 'passed' | 'failed' | 'unknown';
export type TestAttribution = 'code' | 'case' | null;

export interface TestCaseResult {
  id: string;
  status: TestCaseStatus;
  attribution: TestAttribution;
  message: string;
}

export interface TestReport {
  total: number;
  passed: number;
  failed: number;
  cases: TestCaseResult[];
}

/** 测试报告（契约 §4 `test-report.json`）。 */
export function parseTestReport(content: string): ParseResult<TestReport> {
  const data = parseJson(content);
  if (!isRecord(data)) return { ok: false };
  const hasCases = Array.isArray(data.cases);
  if (!hasCases && num(data.total) === null) return { ok: false };

  const rawCases: unknown[] = hasCases ? (data.cases as unknown[]) : [];
  const cases: TestCaseResult[] = rawCases.filter(isRecord).map((c) => ({
    id: text(c.id) ?? '',
    status:
      c.status === 'passed' || c.status === 'failed' ? c.status : 'unknown',
    attribution:
      c.attribution === 'code' || c.attribution === 'case'
        ? c.attribution
        : null,
    message: typeof c.message === 'string' ? c.message : '',
  }));
  return {
    ok: true,
    value: {
      total: num(data.total) ?? cases.length,
      passed:
        num(data.passed) ?? cases.filter((c) => c.status === 'passed').length,
      failed:
        num(data.failed) ?? cases.filter((c) => c.status === 'failed').length,
      cases,
    },
  };
}
