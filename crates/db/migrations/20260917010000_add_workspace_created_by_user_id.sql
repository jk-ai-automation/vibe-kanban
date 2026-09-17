-- 团队版权限修复：本地 workspaces 表缺真实创建者列，导致投影层只能返回
-- 「当前请求者自己的 id」当作 owner_user_id——对谁都成立，等于谁都能删别人的
-- 工作区。这里补一列记录真正的创建者。
--
-- 列顺序：ALTER TABLE ADD COLUMN 总是把新列追加在表尾，不会改变 id 仍是
-- 第 0 列这件事（crates/services/src/services/events.rs 的 preupdate 钩子
-- 用 get_old_column_value(0) 取主键，workspaces 在钩子白名单里）。
--
-- 列不带 DEFAULT：SQLite 不允许给带 REFERENCES 的新增列同时指定非 NULL 的
-- DEFAULT（"Cannot add a REFERENCES column with non-NULL default value"）。
-- 所以这里分两步——先加一列允许为空的外键列，再用 UPDATE 显式回填。
--
-- 回填：把回填值直接写成 DEFAULT_USER_ID 的字面量（与
-- crates/db/src/models/local_project.rs 的 Uuid::from_u128(2) 完全一致，
-- 也是 20260917000000_add_local_auth.sql 里插入的那条本机用户行的主键）。
-- 只回填 IS NULL 的行：新迁移刚加完列时所有行都是 NULL，回填一次即可；
-- 之后任何显式写入 created_by_user_id 的行（新建工作区、或本迁移已跑过的
-- 老库）都不会再被这条 UPDATE 命中，保证重复执行迁移不会覆盖真实创建者。
--
-- ON DELETE SET NULL（不用 CASCADE）：工作区本身是有价值的工作记录（分支、
-- 执行历史），创建者账号被删除不应该连带清空这些记录——与同一张表的
-- issue_id 外键、以及 local_invites.created_by 的约定一致。
ALTER TABLE workspaces
    ADD COLUMN created_by_user_id BLOB REFERENCES local_users(id) ON DELETE SET NULL;

UPDATE workspaces
SET created_by_user_id = X'00000000000000000000000000000002'
WHERE created_by_user_id IS NULL;
