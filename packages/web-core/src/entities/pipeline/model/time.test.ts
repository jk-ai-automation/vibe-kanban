import { describe, expect, it } from 'vitest';
import { durationText, relativeTimeText } from './time';

describe('durationText', () => {
  it('不足一分钟按秒', () => {
    expect(durationText(12_400)).toEqual({
      key: 'pipeline.duration.seconds',
      params: { s: 12 },
    });
  });
  it('一小时内按分秒', () => {
    expect(durationText(72_000)).toEqual({
      key: 'pipeline.duration.minutesSeconds',
      params: { m: 1, s: 12 },
    });
  });
  it('超过一小时按时分', () => {
    expect(durationText(3_780_000)).toEqual({
      key: 'pipeline.duration.hoursMinutes',
      params: { h: 1, m: 3 },
    });
  });
  it('负数当 0', () => {
    expect(durationText(-5).params).toEqual({ s: 0 });
  });
});

describe('relativeTimeText', () => {
  const now = Date.parse('2026-09-18T10:00:00Z');
  it('一分钟内是刚刚', () => {
    expect(relativeTimeText(now - 30_000, now).key).toBe(
      'pipeline.time.justNow'
    );
  });
  it('分钟 / 小时 / 天', () => {
    expect(relativeTimeText(now - 3 * 60_000, now)).toEqual({
      key: 'pipeline.time.minutesAgo',
      params: { n: 3 },
    });
    expect(relativeTimeText(now - 5 * 3_600_000, now)).toEqual({
      key: 'pipeline.time.hoursAgo',
      params: { n: 5 },
    });
    expect(relativeTimeText(now - 2 * 86_400_000, now)).toEqual({
      key: 'pipeline.time.daysAgo',
      params: { n: 2 },
    });
  });
  it('未来时间当刚刚', () => {
    expect(relativeTimeText(now + 60_000, now).key).toBe(
      'pipeline.time.justNow'
    );
  });
});
