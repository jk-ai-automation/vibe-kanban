-- 给已有项目补一条「测试中」状态列（stage_type = 'test'），插在「待评审」与「已完成」之间。
--
-- 为什么要自动补：
--   1. ProjectStatuses::create 把新建状态列的 stage_type 硬编码成 'todo'
--      （local_project_status.rs 的 INSERT），用户自己无论如何都造不出 stage_type='test' 的列。
--      不在这里补，老项目就永远拿不到测试阶段，看板的流程泳道会缺一段。
--   2. 这是纯追加：不改任何 issue 的 status_id，不删任何行，不改任何列的名称/颜色/隐藏状态。
--      唯一被写的既有字段是 sort_order，而且是整体 +1 平移，所有既有列的**相对顺序完全不变**。
--   3. 幂等：已经有 'test' 列的项目整条跳过（WHERE NOT EXISTS），重复执行不会插第二条。
--
-- 插入位置的选取（按优先级）：
--   a. 有 review 列  -> 最后一条 review 的 sort_order + 1
--   b. 没有 review 但有 done 列 -> 第一条 done 的 sort_order（把 done 及其后整体后移）
--   c. 两者都没有   -> 追加到末尾（MAX(sort_order) + 1）
--   d. 一列都没有（异常数据） -> 0
--
-- 用临时表先把每个项目的目标位置定下来，再平移、再插入。
-- 不能在 INSERT 里重算位置：分支 b 平移之后 MIN(done.sort_order) 会变，重算会错位。

CREATE TABLE _test_stage_backfill (
    project_id BLOB PRIMARY KEY NOT NULL,
    target     INTEGER NOT NULL
);

INSERT INTO _test_stage_backfill (project_id, target)
SELECT p.id,
       COALESCE(
           (SELECT MAX(r.sort_order) + 1 FROM project_statuses r
             WHERE r.project_id = p.id AND r.stage_type = 'review'),
           (SELECT MIN(d.sort_order) FROM project_statuses d
             WHERE d.project_id = p.id AND d.stage_type = 'done'),
           (SELECT MAX(a.sort_order) + 1 FROM project_statuses a
             WHERE a.project_id = p.id),
           0)
FROM local_projects p
WHERE NOT EXISTS (
    SELECT 1 FROM project_statuses s
     WHERE s.project_id = p.id AND s.stage_type = 'test'
);

-- 腾位置：目标位置及其之后的列整体后移一格，保持相对顺序。
UPDATE project_statuses
SET sort_order = sort_order + 1
WHERE EXISTS (
    SELECT 1 FROM _test_stage_backfill b
     WHERE b.project_id = project_statuses.project_id
       AND project_statuses.sort_order >= b.target
);

-- 插入。id 用 randomblob(16)：project_statuses.id 是 BLOB，sqlx 的 Uuid 解码走
-- Uuid::from_slice(16 字节)，随机 16 字节能正常解码（本迁移配套的单测会验证）。
-- created_at 显式写成 RFC3339，与 LocalProjects::create 走 sqlx 绑定写入的格式一致，
-- 避免 `ORDER BY sort_order, created_at` 在 sort_order 并列时混用两种格式排错。
INSERT INTO project_statuses
    (id, project_id, name, color, sort_order, hidden, stage_type, created_at)
SELECT randomblob(16), b.project_id, '测试中', '#06b6d4', b.target, 0, 'test',
       strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM _test_stage_backfill b;

DROP TABLE _test_stage_backfill;
