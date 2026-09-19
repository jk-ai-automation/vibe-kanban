/** 需要翻译的文本：key + 参数，渲染方 `t(spec.key, spec.params)`。 */
export interface TextSpec {
  key: string;
  params: Record<string, number>;
}

/** 耗时（设计文档 §8.4 时间线「耗时」）。 */
export function durationText(ms: number): TextSpec {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) {
    return { key: 'pipeline.duration.seconds', params: { s: totalSeconds } };
  }
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return {
      key: 'pipeline.duration.minutesSeconds',
      params: { m: totalMinutes, s: totalSeconds % 60 },
    };
  }
  return {
    key: 'pipeline.duration.hoursMinutes',
    params: { h: Math.floor(totalMinutes / 60), m: totalMinutes % 60 },
  };
}

/**
 * 相对时间。现有 `shared/lib/date.ts:17 formatRelativeTime` 写死英文，
 * 这里换成 i18n key。
 */
export function relativeTimeText(at: number, now: number): TextSpec {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return { key: 'pipeline.time.justNow', params: {} };
  if (minutes < 60)
    return { key: 'pipeline.time.minutesAgo', params: { n: minutes } };
  const hours = Math.floor(minutes / 60);
  if (hours < 24)
    return { key: 'pipeline.time.hoursAgo', params: { n: hours } };
  return {
    key: 'pipeline.time.daysAgo',
    params: { n: Math.floor(hours / 24) },
  };
}
