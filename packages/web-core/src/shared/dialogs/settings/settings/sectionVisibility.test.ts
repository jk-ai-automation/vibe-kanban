import { describe, expect, it } from 'vitest';
import {
  LOCAL_TEAM_ONLY_SECTIONS,
  filterVisibleSections,
  isSettingsSectionVisible,
} from '@/shared/dialogs/settings/settings/sectionVisibility';

const 全部 = [
  { id: 'general' },
  { id: 'repos' },
  { id: 'agents' },
  { id: 'mcp' },
  { id: 'organizations' },
  { id: 'remote-projects' },
  { id: 'relay' },
  { id: 'account' },
];

describe('isSettingsSectionVisible', () => {
  it('本机团队版下账号设置出现', () => {
    expect(isSettingsSectionVisible('account', { localTeamMode: true })).toBe(
      true
    );
  });

  /** 个人版零回退：这块界面必须整个消失，而不是显示出来再报错。 */
  it('不是本机团队版时账号设置完全不出现', () => {
    expect(isSettingsSectionVisible('account', { localTeamMode: false })).toBe(
      false
    );
  });

  it('其余 section 不受影响', () => {
    for (const { id } of 全部.filter((s) => s.id !== 'account')) {
      expect(isSettingsSectionVisible(id, { localTeamMode: false })).toBe(true);
      expect(isSettingsSectionVisible(id, { localTeamMode: true })).toBe(true);
    }
  });

  it('只有 account 在本机团队版白名单里', () => {
    expect([...LOCAL_TEAM_ONLY_SECTIONS]).toEqual(['account']);
  });
});

describe('filterVisibleSections', () => {
  it('个人版下过滤掉账号设置，其它一个不少', () => {
    const visible = filterVisibleSections(全部, { localTeamMode: false });
    expect(visible.map((s) => s.id)).toEqual([
      'general',
      'repos',
      'agents',
      'mcp',
      'organizations',
      'remote-projects',
      'relay',
    ]);
  });

  it('团队版下全都在，且顺序不变', () => {
    const visible = filterVisibleSections(全部, { localTeamMode: true });
    expect(visible.map((s) => s.id)).toEqual(全部.map((s) => s.id));
  });
});
