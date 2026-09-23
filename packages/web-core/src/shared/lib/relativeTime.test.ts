import { describe, expect, it } from 'vitest';
import { relativeTimeAgo, relativeTimeShort } from '@vibe/ui/lib/relativeTime';

const now = new Date('2026-09-22T12:00:00Z');
const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe('relativeTimeAgo', () => {
  it('不到一分钟算刚刚', () => {
    expect(relativeTimeAgo(ago(30_000), now)).toEqual({
      key: 'relativeTime.justNow',
      params: { n: 0 },
    });
  });

  it('按分钟、小时、天、周递进', () => {
    expect(relativeTimeAgo(ago(5 * MIN), now)).toEqual({
      key: 'relativeTime.minutesAgo',
      params: { n: 5 },
    });
    expect(relativeTimeAgo(ago(2 * HOUR), now)).toEqual({
      key: 'relativeTime.hoursAgo',
      params: { n: 2 },
    });
    expect(relativeTimeAgo(ago(3 * DAY), now)).toEqual({
      key: 'relativeTime.daysAgo',
      params: { n: 3 },
    });
    expect(relativeTimeAgo(ago(15 * DAY), now)).toEqual({
      key: 'relativeTime.weeksAgo',
      params: { n: 2 },
    });
  });
});

describe('relativeTimeShort', () => {
  it('不到一分钟显示 now', () => {
    expect(relativeTimeShort(ago(10_000), now).key).toBe(
      'relativeTime.shortNow'
    );
  });

  it('取最大的非零单位', () => {
    expect(relativeTimeShort(ago(7 * MIN), now)).toEqual({
      key: 'relativeTime.shortMinutes',
      params: { n: 7 },
    });
    expect(relativeTimeShort(ago(5 * HOUR), now)).toEqual({
      key: 'relativeTime.shortHours',
      params: { n: 5 },
    });
    expect(relativeTimeShort(ago(40 * DAY), now)).toEqual({
      key: 'relativeTime.shortDays',
      params: { n: 40 },
    });
  });
});
