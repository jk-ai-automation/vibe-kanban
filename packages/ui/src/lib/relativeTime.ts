/**
 * 相对时间的 i18n 描述：`t(spec.key, spec.params)` 渲染。
 * 英文文案与改造前的硬编码字符串一致（"just now" / "5m ago" / "3d" …），
 * 团队版英文界面外观不变；其它语言走各自 locale。
 */
export interface RelativeTimeSpec {
  key: string;
  params: { n: number };
}

function diffParts(dateString: string, now: Date) {
  const diffMs = now.getTime() - new Date(dateString).getTime();
  const minutes = Math.floor(diffMs / (1000 * 60));
  const hours = Math.floor(diffMs / (1000 * 60 * 60));
  const days = Math.floor(diffMs / (1000 * 60 * 60 * 24));
  return { minutes, hours, days, weeks: Math.floor(days / 7) };
}

/** 长格式："just now" / "5m ago" / "2h ago" / "3d ago" / "1w ago"。 */
export function relativeTimeAgo(
  dateString: string,
  now: Date = new Date()
): RelativeTimeSpec {
  const { minutes, hours, days, weeks } = diffParts(dateString, now);
  if (minutes < 1) return { key: 'relativeTime.justNow', params: { n: 0 } };
  if (minutes < 60)
    return { key: 'relativeTime.minutesAgo', params: { n: minutes } };
  if (hours < 24) return { key: 'relativeTime.hoursAgo', params: { n: hours } };
  if (days < 7) return { key: 'relativeTime.daysAgo', params: { n: days } };
  return { key: 'relativeTime.weeksAgo', params: { n: weeks } };
}

/** 短格式："now" / "5m" / "2h" / "3d"。 */
export function relativeTimeShort(
  dateString: string,
  now: Date = new Date()
): RelativeTimeSpec {
  const { minutes, hours, days } = diffParts(dateString, now);
  if (days > 0) return { key: 'relativeTime.shortDays', params: { n: days } };
  if (hours > 0)
    return { key: 'relativeTime.shortHours', params: { n: hours } };
  if (minutes > 0)
    return { key: 'relativeTime.shortMinutes', params: { n: minutes } };
  return { key: 'relativeTime.shortNow', params: { n: 0 } };
}
