import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PipelineSkillInfo, PipelineStageKey } from 'shared/types';
import { getPipelineSkills } from '@/entities/pipeline/api/pipelineApi';

/** 来源 → i18n 键。锁文件里出现新来源时回落到来源名本身。 */
const SOURCE_KEYS: Record<string, string> = {
  platform: 'settings.pipelineSkills.source.platform',
  superpowers: 'settings.pipelineSkills.source.superpowers',
  atp: 'settings.pipelineSkills.source.atp',
};

export function PipelineSkillsSettingsSection() {
  const { t } = useTranslation('settings');
  const [skills, setSkills] = useState<PipelineSkillInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getPipelineSkills()
      .then((rows) => {
        if (!cancelled) setSkills(rows);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const stageLabel = (stage: PipelineStageKey | null) =>
    stage === null
      ? t('settings.pipelineSkills.noStage')
      : t(`pipeline.stage.${stage}`, { ns: 'common' });

  const sourceLabel = (source: string) => {
    const key = SOURCE_KEYS[source];
    return key ? t(key) : source;
  };

  return (
    <div className="space-y-4 pb-6">
      <p className="text-sm text-normal">
        {t('settings.pipelineSkills.description')}
      </p>
      <p className="text-xs text-low">{t('settings.pipelineSkills.readOnly')}</p>

      {error !== null && (
        <p className="text-sm text-error">
          {t('settings.pipelineSkills.loadError', { message: error })}
        </p>
      )}
      {error === null && skills === null && (
        <p className="text-sm text-low">
          {t('settings.pipelineSkills.loading')}
        </p>
      )}
      {skills !== null && skills.length === 0 && (
        <p className="text-sm text-low">{t('settings.pipelineSkills.empty')}</p>
      )}

      {skills !== null && skills.length > 0 && (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-low">
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.name')}
              </th>
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.source')}
              </th>
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.version')}
              </th>
              <th className="py-2 font-medium">
                {t('settings.pipelineSkills.columns.stage')}
              </th>
            </tr>
          </thead>
          <tbody>
            {skills.map((skill) => (
              <tr key={skill.name} className="border-b border-border/50">
                <td className="py-2 pr-3">
                  <code className="text-xs text-high">
                    {skill.qualified_name}
                  </code>
                </td>
                <td className="py-2 pr-3 text-normal">
                  {sourceLabel(skill.source)}
                  {skill.license !== null && (
                    <span className="ml-1 text-xs text-low">
                      ({skill.license})
                    </span>
                  )}
                </td>
                <td className="py-2 pr-3 text-normal">
                  {skill.source_version === null
                    ? '—'
                    : skill.source_version.slice(0, 12)}
                </td>
                <td className="py-2 text-normal">{stageLabel(skill.stage)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
