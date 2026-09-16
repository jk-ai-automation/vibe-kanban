-- 个人版需求链路（P2 第一期）本地存储。
-- 注意：遗留表 projects / tasks 已废弃（无人读写），本迁移不触碰它们；
-- 新项目表取名 local_projects 以避免与遗留表冲突。
-- 约定：所有参与变更钩子的表，id 必须是第 0 列
-- （crates/services/src/services/events.rs 的 preupdate 钩子用 get_old_column_value(0) 取主键）。

CREATE TABLE local_projects (
    id              BLOB PRIMARY KEY NOT NULL,
    organization_id BLOB NOT NULL,
    name            TEXT NOT NULL,
    color           TEXT NOT NULL DEFAULT '#6366f1',
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE project_statuses (
    id          BLOB PRIMARY KEY NOT NULL,
    project_id  BLOB NOT NULL REFERENCES local_projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    color       TEXT NOT NULL DEFAULT '#94a3b8',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    hidden      INTEGER NOT NULL DEFAULT 0,
    stage_type  TEXT NOT NULL DEFAULT 'todo',
    created_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);
CREATE INDEX idx_project_statuses_project ON project_statuses(project_id);

CREATE TABLE issues (
    id                      BLOB PRIMARY KEY NOT NULL,
    project_id              BLOB NOT NULL REFERENCES local_projects(id) ON DELETE CASCADE,
    issue_number            INTEGER NOT NULL,
    simple_id               TEXT NOT NULL,
    status_id               BLOB NOT NULL REFERENCES project_statuses(id),
    title                   TEXT NOT NULL,
    description             TEXT,
    priority                TEXT,
    start_date              TEXT,
    target_date             TEXT,
    completed_at            TEXT,
    sort_order              REAL NOT NULL DEFAULT 0,
    parent_issue_id         BLOB REFERENCES issues(id) ON DELETE SET NULL,
    parent_issue_sort_order REAL,
    extension_metadata      TEXT NOT NULL DEFAULT '{}',
    creator_user_id         BLOB,
    created_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    UNIQUE (project_id, issue_number)
);
CREATE INDEX idx_issues_project ON issues(project_id);
CREATE INDEX idx_issues_status ON issues(status_id);
CREATE INDEX idx_issues_parent ON issues(parent_issue_id);

CREATE TABLE project_tags (
    id         BLOB PRIMARY KEY NOT NULL,
    project_id BLOB NOT NULL REFERENCES local_projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT '#64748b'
);
CREATE INDEX idx_project_tags_project ON project_tags(project_id);

CREATE TABLE issue_tags (
    id       BLOB PRIMARY KEY NOT NULL,
    issue_id BLOB NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    tag_id   BLOB NOT NULL REFERENCES project_tags(id) ON DELETE CASCADE,
    UNIQUE (issue_id, tag_id)
);
CREATE INDEX idx_issue_tags_issue ON issue_tags(issue_id);

CREATE TABLE issue_comments (
    id         BLOB PRIMARY KEY NOT NULL,
    issue_id   BLOB NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_id  BLOB,
    parent_id  BLOB REFERENCES issue_comments(id) ON DELETE CASCADE,
    message    TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);
CREATE INDEX idx_issue_comments_issue ON issue_comments(issue_id);

ALTER TABLE workspaces ADD COLUMN issue_id BLOB REFERENCES issues(id) ON DELETE SET NULL;
CREATE INDEX idx_workspaces_issue_id ON workspaces(issue_id);
