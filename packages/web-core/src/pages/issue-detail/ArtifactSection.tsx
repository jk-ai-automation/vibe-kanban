import { useTranslation } from 'react-i18next';
import type { ArtifactKind, IssueArtifactSummary } from 'shared/types';
import { MarkdownPreview } from '@/shared/components/MarkdownPreview';
import { getResolvedTheme, useTheme } from '@/shared/hooks/useTheme';
import { useArtifact } from '@/entities/pipeline/model/hooks/usePipelineData';
import { CsvTableView } from '@vibe/ui/components/CsvTableView';
import { Skeleton } from '@vibe/ui/components/Skeleton';
import { artifactKindLabelKey, artifactRenderMode } from './issueDetailModel';
import {
  parseCsv,
  parseReview,
  parseTestReport,
  type ReviewSeverity,
  type TestCaseStatus,
} from './artifactParsers';

const SEVERITY_KEYS: Record<ReviewSeverity, string> = {
  blocker: 'issueDetail.severity.blocker',
  major: 'issueDetail.severity.major',
  minor: 'issueDetail.severity.minor',
  unknown: 'issueDetail.severity.unknown',
};

const ATTRIBUTION_KEYS: Record<'code' | 'case', string> = {
  code: 'issueDetail.attribution.code',
  case: 'issueDetail.attribution.case',
};

const CASE_STATUS_KEYS: Record<TestCaseStatus, string> = {
  passed: 'issueDetail.caseStatus.passed',
  failed: 'issueDetail.caseStatus.failed',
  unknown: 'issueDetail.caseStatus.unknown',
};

function RawContent({ content }: { content: string }) {
  const { t } = useTranslation('common');
  return (
    <div className="flex flex-col gap-half">
      <p className="m-0 text-sm text-low">{t('issueDetail.parseError')}</p>
      <pre className="m-0 overflow-x-auto rounded-sm bg-secondary p-base font-ibm-plex-mono text-xs text-normal">
        {content}
      </pre>
    </div>
  );
}

function ArtifactBody({
  kind,
  content,
  theme,
}: {
  kind: ArtifactKind;
  content: string;
  theme: 'light' | 'dark';
}) {
  const { t } = useTranslation('common');
  switch (artifactRenderMode(kind)) {
    case 'markdown':
      return (
        <MarkdownPreview content={content} theme={theme} className="text-sm" />
      );
    case 'csv': {
      const table = parseCsv(content);
      return (
        <CsvTableView
          header={table.header}
          rows={table.rows}
          emptyText={t('issueDetail.noArtifact')}
        />
      );
    }
    case 'review': {
      const result = parseReview(content);
      if (!result.ok) return <RawContent content={content} />;
      if (result.value.length === 0) {
        return (
          <p className="m-0 text-sm text-low">{t('issueDetail.reviewEmpty')}</p>
        );
      }
      return (
        <ul className="m-0 flex list-none flex-col gap-half p-0">
          {result.value.map((finding, index) => (
            <li
              key={index}
              className="rounded-sm border border-border p-half text-sm"
            >
              <span
                className={
                  finding.severity === 'blocker'
                    ? 'font-medium text-stage-failed'
                    : 'font-medium text-normal'
                }
              >
                {t(SEVERITY_KEYS[finding.severity])}
              </span>
              {finding.file && (
                <span className="ml-half font-ibm-plex-mono text-xs text-low">
                  {finding.line !== null
                    ? `${finding.file}:${finding.line}`
                    : finding.file}
                </span>
              )}
              <p className="m-0 text-normal">{finding.message}</p>
            </li>
          ))}
        </ul>
      );
    }
    case 'test_report': {
      const result = parseTestReport(content);
      if (!result.ok) return <RawContent content={content} />;
      const report = result.value;
      return (
        <div className="flex flex-col gap-half">
          <p className="m-0 text-sm text-normal">
            {t('issueDetail.testSummary', {
              passed: report.passed,
              failed: report.failed,
              total: report.total,
            })}
          </p>
          <CsvTableView
            header={[
              t('issueDetail.testCols.case'),
              t('issueDetail.testCols.status'),
              t('issueDetail.testCols.attribution'),
              t('issueDetail.testCols.message'),
            ]}
            rows={report.cases.map((testCase) => [
              testCase.id,
              t(CASE_STATUS_KEYS[testCase.status]),
              testCase.attribution
                ? t(ATTRIBUTION_KEYS[testCase.attribution])
                : '—',
              testCase.message,
            ])}
            emptyText={t('issueDetail.noArtifact')}
          />
        </div>
      );
    }
  }
}

/** 一个产出物（最新版本）：标题 + 版本 + 截断提示 + 按种类渲染的内容。 */
export function ArtifactSection({
  summary,
}: {
  summary: IssueArtifactSummary;
}) {
  const { t } = useTranslation('common');
  const { theme } = useTheme();
  const { data, isLoading, error } = useArtifact(summary.id);

  return (
    <section
      data-testid="artifact-section"
      data-kind={summary.kind}
      className="flex flex-col gap-half"
    >
      <header className="flex items-center gap-half">
        <h3 className="m-0 text-sm font-medium text-high">
          {t(artifactKindLabelKey(summary.kind))}
        </h3>
        <span className="font-ibm-plex-mono text-xs text-low">
          {t('issueDetail.version', { version: String(summary.version) })}
        </span>
      </header>
      {summary.truncated && (
        <p className="m-0 text-sm text-low">{t('issueDetail.truncated')}</p>
      )}
      {isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : error || !data ? (
        <p role="alert" className="m-0 text-sm text-error">
          {t('pipeline.gate.error', { message: error?.message ?? '' })}
        </p>
      ) : (
        <ArtifactBody
          kind={summary.kind}
          content={data.content}
          theme={getResolvedTheme(theme)}
        />
      )}
    </section>
  );
}
