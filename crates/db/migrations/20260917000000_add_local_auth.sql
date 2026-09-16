-- 团队版本地账号体系（P3 第一批）：用户、会话、第三方身份、邀请码。
-- 约定一：所有表的 id 必须是第 0 列
-- （crates/services/src/services/events.rs 的 preupdate 钩子用 get_old_column_value(0) 取主键）。
-- 本期这四张表不接变更钩子，但列顺序仍按此约定写，免得以后接钩子时要改表。
-- 约定二：时间戳一律写 RFC3339（`...+00:00`），与 sqlx 绑定 DateTime<Utc> 的格式
-- （to_rfc3339_opts(AutoSi, false)）逐字一致。不要用 datetime('now','subsec')——
-- 它写出的是 "YYYY-MM-DD HH:MM:SS.SSS"，和 RFC3339 混在同一列里就没法按字符串比较，
-- 迁移 20260916010000 末尾正是在补这个坑。

CREATE TABLE local_users (
    id            BLOB PRIMARY KEY NOT NULL,
    username      TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    email         TEXT,
    -- 仅第三方登录的用户可以没有密码。
    password_hash TEXT,
    role          TEXT NOT NULL DEFAULT 'member',
    status        TEXT NOT NULL DEFAULT 'active',
    avatar_color  TEXT NOT NULL DEFAULT '#6366f1',
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00'),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00'),
    last_login_at TEXT
);
-- 大小写敏感（SQLite 默认 BINARY 排序规则）：「Alice 与 alice 是同一人」
-- 由应用层的 normalize_username 保证，库层只兜底防止完全重名。
CREATE UNIQUE INDEX idx_local_users_username ON local_users(username);
CREATE UNIQUE INDEX idx_local_users_email ON local_users(email) WHERE email IS NOT NULL;

CREATE TABLE local_sessions (
    id           BLOB PRIMARY KEY NOT NULL,
    user_id      BLOB NOT NULL REFERENCES local_users(id) ON DELETE CASCADE,
    -- 只存 SHA-256 十六进制串，明文令牌只发给浏览器。
    token_hash   TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00'),
    expires_at   TEXT NOT NULL,
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00'),
    user_agent   TEXT,
    ip           TEXT,
    revoked_at   TEXT
);
CREATE UNIQUE INDEX idx_local_sessions_token ON local_sessions(token_hash);
CREATE INDEX idx_local_sessions_user ON local_sessions(user_id);
CREATE INDEX idx_local_sessions_expires ON local_sessions(expires_at);

CREATE TABLE local_user_identities (
    id         BLOB PRIMARY KEY NOT NULL,
    user_id    BLOB NOT NULL REFERENCES local_users(id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    email      TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00')
);
CREATE UNIQUE INDEX idx_local_identities_provider_subject
    ON local_user_identities(provider, subject);
CREATE INDEX idx_local_identities_user ON local_user_identities(user_id);

CREATE TABLE local_invites (
    id         BLOB PRIMARY KEY NOT NULL,
    code_hash  TEXT NOT NULL,
    role       TEXT NOT NULL DEFAULT 'member',
    created_by BLOB REFERENCES local_users(id) ON DELETE SET NULL,
    expires_at TEXT NOT NULL,
    used_by    BLOB REFERENCES local_users(id) ON DELETE SET NULL,
    used_at    TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00')
);
CREATE UNIQUE INDEX idx_local_invites_code ON local_invites(code_hash);

-- 个人版固定本机用户：id 与 crates/db/src/models/local_project.rs 的
-- DEFAULT_USER_ID = Uuid::from_u128(2) 完全一致（sqlx 把 Uuid 编码成 16 字节大端
-- BLOB，即 as_bytes()），保证历史数据里的 creator_user_id / author_id 仍能对上
-- 一条真实的用户行。
-- 历史数据不改写：本迁移不 UPDATE issues / issue_comments 的任何字段。
-- password_hash 留空：个人版不登录；切到团队模式后由初始化向导设置密码。
INSERT INTO local_users (id, username, display_name, role, status)
VALUES (X'00000000000000000000000000000002', 'local', '本机', 'admin', 'active');
