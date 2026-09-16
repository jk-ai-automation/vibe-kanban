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
