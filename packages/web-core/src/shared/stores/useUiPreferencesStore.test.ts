import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  useUiPreferencesStore,
  waitForScratchHydration,
} from './useUiPreferencesStore';

afterEach(() => {
  vi.useRealTimers();
  useUiPreferencesStore.setState({
    scratchHydrated: false,
    selectedProjectId: null,
    selectedProjectSource: 'fallback',
  });
});

describe('setSelectedProjectId 的来源标记', () => {
  it('默认是自动回落；显式传 user 才算用户选择', () => {
    useUiPreferencesStore.getState().setSelectedProjectId('p1');
    expect(useUiPreferencesStore.getState().selectedProjectSource).toBe(
      'fallback'
    );

    useUiPreferencesStore.getState().setSelectedProjectId('p2', 'user');
    expect(useUiPreferencesStore.getState().selectedProjectId).toBe('p2');
    expect(useUiPreferencesStore.getState().selectedProjectSource).toBe('user');
  });
});

describe('waitForScratchHydration', () => {
  it('已经拉回来时立即返回', async () => {
    useUiPreferencesStore.setState({ scratchHydrated: true });
    await expect(waitForScratchHydration(50)).resolves.toBe(true);
  });

  it('拉回来后才返回', async () => {
    const pending = waitForScratchHydration(1000);
    useUiPreferencesStore.getState().setScratchHydrated(true);
    await expect(pending).resolves.toBe(true);
  });

  it('超时后返回 false，不把首屏卡住', async () => {
    vi.useFakeTimers();
    const pending = waitForScratchHydration(3000);
    await vi.advanceTimersByTimeAsync(3000);
    await expect(pending).resolves.toBe(false);
  });
});
