import {
  GearIcon,
  GitBranchIcon,
  BuildingsIcon,
  CloudIcon,
  CpuIcon,
  PlugIcon,
  BroadcastIcon,
  UserCircleIcon,
} from '@phosphor-icons/react';
import type { Icon } from '@phosphor-icons/react';
import { GeneralSettingsSection } from './GeneralSettingsSection';
import { ReposSettingsSection } from './ReposSettingsSection';
import { OrganizationsSettingsSection } from './OrganizationsSettingsSection';
import { RemoteProjectsSettingsSection } from './RemoteProjectsSettingsSection';
import { AgentsSettingsSection } from './AgentsSettingsSection';
import { McpSettingsSection } from './McpSettingsSection';
import { RelaySettingsSectionContent } from './RelaySettingsSection';
import { AccountSettingsSection } from './AccountSettingsSection';
import {
  filterVisibleSections,
  type SettingsSectionVisibilityContext,
} from './sectionVisibility';

export type { SettingsSectionVisibilityContext };

export type SettingsSectionType =
  | 'general'
  | 'repos'
  | 'organizations'
  | 'remote-projects'
  | 'agents'
  | 'mcp'
  | 'relay'
  | 'account';

export type SettingsSectionGroup = 'host' | 'universal';

export type SettingsSectionInitialState = {
  general: undefined;
  repos: { repoId?: string } | undefined;
  organizations: { organizationId?: string } | undefined;
  'remote-projects':
    | { organizationId?: string; projectId?: string }
    | undefined;
  agents: { executor?: string; variant?: string } | undefined;
  mcp: undefined;
  relay: { hostId?: string } | undefined;
  account: undefined;
};

export interface SettingsSectionDefinition {
  id: SettingsSectionType;
  icon: Icon;
  group: SettingsSectionGroup;
}

export const SETTINGS_SECTION_DEFINITIONS: SettingsSectionDefinition[] = [
  { id: 'general', icon: GearIcon, group: 'host' },
  { id: 'repos', icon: GitBranchIcon, group: 'host' },
  { id: 'agents', icon: CpuIcon, group: 'host' },
  { id: 'mcp', icon: PlugIcon, group: 'host' },
  { id: 'organizations', icon: BuildingsIcon, group: 'universal' },
  { id: 'remote-projects', icon: CloudIcon, group: 'universal' },
  { id: 'relay', icon: BroadcastIcon, group: 'universal' },
  // 只在本机团队版下出现，见 `sectionVisibility.ts`。
  { id: 'account', icon: UserCircleIcon, group: 'universal' },
];

/**
 * 某一组里**当前形态下真的该出现**的 section。
 *
 * 导航栏与「初始 section 合不合法」都必须走这一个函数：只在导航栏过滤，
 * 个人版下一个 `initialSection: 'account'` 就能把那块界面直接打开。
 */
export function visibleSettingsSections(
  group: SettingsSectionGroup,
  ctx: SettingsSectionVisibilityContext
): SettingsSectionDefinition[] {
  return filterVisibleSections(
    SETTINGS_SECTION_DEFINITIONS.filter((section) => section.group === group),
    ctx
  );
}

export function isHostSpecificSettingsSection(
  type: SettingsSectionType
): boolean {
  return (
    SETTINGS_SECTION_DEFINITIONS.find((section) => section.id === type)
      ?.group === 'host'
  );
}

export function renderSettingsSection(
  type: SettingsSectionType,
  initialState?: SettingsSectionInitialState[SettingsSectionType],
  onClose?: () => void
) {
  switch (type) {
    case 'general':
      return <GeneralSettingsSection />;
    case 'repos':
      return (
        <ReposSettingsSection
          initialState={initialState as SettingsSectionInitialState['repos']}
        />
      );
    case 'organizations':
      return <OrganizationsSettingsSection />;
    case 'remote-projects':
      return (
        <RemoteProjectsSettingsSection
          initialState={
            initialState as SettingsSectionInitialState['remote-projects']
          }
        />
      );
    case 'agents':
      return <AgentsSettingsSection />;
    case 'mcp':
      return <McpSettingsSection />;
    case 'relay':
      return (
        <RelaySettingsSectionContent
          initialState={initialState as SettingsSectionInitialState['relay']}
          onClose={onClose}
        />
      );
    case 'account':
      return <AccountSettingsSection />;
    default:
      return <GeneralSettingsSection />;
  }
}
