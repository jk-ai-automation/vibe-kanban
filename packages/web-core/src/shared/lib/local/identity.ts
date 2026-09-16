import { MemberRole } from 'shared/types';
import type { ListOrganizationsResponse } from 'shared/types';

/**
 * 个人版固定身份。取值必须与
 * crates/db/src/models/local_project.rs 的
 * DEFAULT_ORGANIZATION_ID / DEFAULT_USER_ID 完全一致。
 */
export const LOCAL_ORGANIZATION_ID = '00000000-0000-0000-0000-000000000001';
export const LOCAL_USER_ID = '00000000-0000-0000-0000-000000000002';

export const LOCAL_ORGANIZATION = {
  id: LOCAL_ORGANIZATION_ID,
  name: '本机',
  slug: 'local',
} as const;

export const LOCAL_USER = {
  id: LOCAL_USER_ID,
  user_id: LOCAL_USER_ID,
  name: '本机',
  username: 'local',
  email: null,
  avatar_url: null,
} as const;

/**
 * 个人版的固定组织列表。字段以 shared/types.ts 的 ListOrganizationsResponse 为准；
 * 若类型检查报缺字段，按类型补齐，不要用 as unknown 绕过。
 *
 * 偏差说明：计划文档中的示例字段（`role: 'owner'`）与当前
 * `OrganizationWithRole` 类型不符——实际字段是 `user_role: MemberRole`
 * （`MemberRole` 只有 `ADMIN` / `MEMBER`，没有 `owner`），且还多出
 * `is_personal` 与 `issue_prefix` 两个字段。按现有类型补齐：
 * `user_role: MemberRole.ADMIN`（单人本机场景下等价于所有者权限）、
 * `is_personal: true`、`issue_prefix: 'ISS'`（与远端默认前缀一致，
 * 本地简号生成走 `local_project.rs` 的项目级前缀，不依赖这个字段）。
 */
export const LOCAL_ORGANIZATIONS_RESPONSE: ListOrganizationsResponse = {
  organizations: [
    {
      id: LOCAL_ORGANIZATION_ID,
      name: '本机',
      slug: 'local',
      is_personal: true,
      issue_prefix: 'ISS',
      user_role: MemberRole.ADMIN,
      created_at: '1970-01-01T00:00:00Z',
      updated_at: '1970-01-01T00:00:00Z',
    },
  ],
};
