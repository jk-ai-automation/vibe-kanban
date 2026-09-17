-- 工作区删除的管理员审批流程：删除申请表。
-- 设计见 docs/superpowers/specs/2026-09-17-workspace-delete-approval-design.md。
--
-- 约定一：id 必须是第 0 列（crates/services/src/services/events.rs 的 preupdate
-- 钩子用 get_old_column_value(0) 取主键）。本表暂不接变更钩子，列序仍照此写。
-- 约定二：时间戳一律 RFC3339（`...+00:00`），与 sqlx 绑定 DateTime<Utc> 的格式
-- 逐字一致，保证同一列里能按字符串比较。

CREATE TABLE workspace_delete_requests (
    id                   BLOB PRIMARY KEY NOT NULL,
    -- ON DELETE CASCADE：申请是「请求删除某个工作区」，工作区没了申请就失去指向。
    -- 用 SET NULL 会留下 workspace_id IS NULL 的僵尸 pending，管理员在队列里看到
    -- 一条点不开的申请。级联还顺带把两个竞态做成机械保证：
    --   1. 管理员绕过审批直接删了工作区 → 申请当场消失 → 再去批准得到 404，
    --      既不 panic 也不会误删同 id 的别的东西；
    --   2. 批准的最后一步就是删工作区 → 级联带走这条申请 →
    --      'approved' 永远不会作为持久状态留在库里（设计规则 3）。
    workspace_id         BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- ON DELETE SET NULL：账号注销不该连带抹掉一条待处理申请
    -- （那等于「删号即绕过审批队列」）。与 workspaces.created_by_user_id、
    -- local_invites.created_by 的约定一致。
    requested_by_user_id BLOB REFERENCES local_users(id) ON DELETE SET NULL,
    -- 'pending' / 'rejected' / 'withdrawn'。'approved' 只在批准事务内部短暂存在，
    -- 正常路径下从不落盘（见上面 CASCADE 的注释）。
    status               TEXT NOT NULL DEFAULT 'pending',
    reason               TEXT,
    decided_by_user_id   BLOB REFERENCES local_users(id) ON DELETE SET NULL,
    decided_at           TEXT,
    decision_note        TEXT,
    created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00'),
    updated_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00')
);

-- 「同一工作区同时只能有一条待处理申请」由**数据库**保证，不靠「先查后写」。
-- 条件唯一索引：并发两个申请打进来，第二个拿到 SQLITE_CONSTRAINT_UNIQUE，
-- 库里仍然只有一条 pending。已处理（rejected / withdrawn）的行不占用这个名额。
CREATE UNIQUE INDEX idx_workspace_delete_requests_one_pending
    ON workspace_delete_requests (workspace_id)
    WHERE status = 'pending';

-- 审批队列按 status 过滤后按时间排序；申请人查「我的申请」按申请人过滤。
CREATE INDEX idx_workspace_delete_requests_status ON workspace_delete_requests (status);
CREATE INDEX idx_workspace_delete_requests_requester
    ON workspace_delete_requests (requested_by_user_id);
