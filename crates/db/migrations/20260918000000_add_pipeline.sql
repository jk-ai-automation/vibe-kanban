-- 交付流水线（U2）。设计：docs/superpowers/specs/2026-09-18-personal-pipeline-design.md §5；
-- 契约：docs/superpowers/plans/2026-09-18-pipeline-contract.md §1。
--
-- 约定：
--   * 四张新表的 id 一律是第 0 列：变更钩子在 DELETE 时用 get_old_column_value(0) 取 id
--     （crates/services/src/services/events.rs 的 preupdate 分支）。
--   * 时间列不设 DEFAULT，由 Rust 侧显式写入（sqlx 绑定 DateTime<Utc>），避免两种时间格式混排。
--   * template_json / template_warning / executor_config / feedback 是引擎内部列，不在对外类型里。
--   * session_id / execution_process_id / decided_by 不加外键：它们只是留痕，
--     工作区或账号删除后记录仍要保留。

CREATE TABLE pipeline_runs (
    id                BLOB PRIMARY KEY NOT NULL,
    issue_id          BLOB NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    project_id        BLOB NOT NULL REFERENCES local_projects(id) ON DELETE CASCADE,
    workspace_id      BLOB REFERENCES workspaces(id) ON DELETE SET NULL,
    template_key      TEXT NOT NULL,
    template_version  INTEGER NOT NULL,
    template_json     TEXT NOT NULL,
    template_warning  TEXT,
    executor_config   TEXT NOT NULL,
    status            TEXT NOT NULL
        CHECK (status IN ('running', 'waiting_gate', 'paused', 'failed', 'completed', 'cancelled')),
    current_stage_key TEXT NOT NULL
        CHECK (current_stage_key IN ('requirement', 'spec', 'test_design', 'develop', 'review', 'test', 'deliver')),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    finished_at       TEXT
);

CREATE INDEX idx_pipeline_runs_issue ON pipeline_runs (issue_id, created_at);
CREATE INDEX idx_pipeline_runs_project_status ON pipeline_runs (project_id, status);
-- 同一需求同一时刻最多一条未结束的运行（failed 可以继续，算未结束）。
CREATE UNIQUE INDEX idx_pipeline_runs_one_active_per_issue
    ON pipeline_runs (issue_id)
    WHERE status NOT IN ('completed', 'cancelled');

CREATE TABLE pipeline_stage_runs (
    id                   BLOB PRIMARY KEY NOT NULL,
    run_id               BLOB NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    project_id           BLOB NOT NULL REFERENCES local_projects(id) ON DELETE CASCADE,
    stage_key            TEXT NOT NULL
        CHECK (stage_key IN ('requirement', 'spec', 'test_design', 'develop', 'review', 'test', 'deliver')),
    attempt              INTEGER NOT NULL CHECK (attempt >= 1),
    status               TEXT NOT NULL
        CHECK (status IN ('pending', 'running', 'waiting_gate', 'passed', 'rejected', 'failed', 'skipped')),
    gate_kind            TEXT NOT NULL CHECK (gate_kind IN ('human', 'auto', 'none')),
    session_id           BLOB,
    execution_process_id BLOB,
    feedback             TEXT,
    started_at           TEXT,
    finished_at          TEXT,
    summary              TEXT,
    error                TEXT,
    UNIQUE (run_id, stage_key, attempt)
);

CREATE INDEX idx_pipeline_stage_runs_run ON pipeline_stage_runs (run_id);
CREATE INDEX idx_pipeline_stage_runs_project ON pipeline_stage_runs (project_id);
CREATE INDEX idx_pipeline_stage_runs_session_status ON pipeline_stage_runs (session_id, status);

CREATE TABLE pipeline_gate_decisions (
    id           BLOB PRIMARY KEY NOT NULL,
    stage_run_id BLOB NOT NULL REFERENCES pipeline_stage_runs(id) ON DELETE CASCADE,
    decision     TEXT NOT NULL CHECK (decision IN ('approve', 'reject')),
    comment      TEXT,
    decided_by   BLOB,
    decided_at   TEXT NOT NULL
);

CREATE INDEX idx_pipeline_gate_decisions_stage_run ON pipeline_gate_decisions (stage_run_id);

CREATE TABLE issue_artifacts (
    id           BLOB PRIMARY KEY NOT NULL,
    issue_id     BLOB NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    stage_run_id BLOB NOT NULL REFERENCES pipeline_stage_runs(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL
        CHECK (kind IN ('requirement', 'spec', 'plan', 'test_cases', 'trace_matrix', 'review', 'test_report', 'delivery_report')),
    rel_path     TEXT NOT NULL,
    content      TEXT NOT NULL,
    truncated    INTEGER NOT NULL DEFAULT 0,
    version      INTEGER NOT NULL CHECK (version >= 1),
    created_at   TEXT NOT NULL,
    UNIQUE (issue_id, kind, version)
);

CREATE INDEX idx_issue_artifacts_stage_run ON issue_artifacts (stage_run_id);

-- execution_processes.run_reason 的 CHECK 加入 'pipelinestep'
-- （ExecutionProcessRunReason::PipelineStep，sqlx rename_all = "lowercase"）。
-- 写法照抄 20260203000000_add_archive_script_to_repos.sql：SQLite 不能改 CHECK，
-- 只能加新列 → 拷数据 → 删索引 → 删旧列 → 改名 → 重建索引。

-- 1. Add the replacement column with the wider CHECK
ALTER TABLE execution_processes
  ADD COLUMN run_reason_new TEXT NOT NULL DEFAULT 'setupscript'
    CHECK (run_reason_new IN ('setupscript',
                               'cleanupscript',
                               'archivescript',
                               'codingagent',
                               'devserver',
                               'pipelinestep'));

-- 2. Copy existing values across
UPDATE execution_processes
  SET run_reason_new = run_reason;

-- 3. Drop any indexes that reference run_reason
DROP INDEX IF EXISTS idx_execution_processes_run_reason;
DROP INDEX IF EXISTS idx_execution_processes_session_status_run_reason;
DROP INDEX IF EXISTS idx_execution_processes_session_run_reason_created;

-- 4. Remove the old column
ALTER TABLE execution_processes DROP COLUMN run_reason;

-- 5. Rename the new column back to the canonical name
ALTER TABLE execution_processes
  RENAME COLUMN run_reason_new TO run_reason;

-- 6. Re-create all indexes
CREATE INDEX idx_execution_processes_run_reason
        ON execution_processes(run_reason);

CREATE INDEX idx_execution_processes_session_status_run_reason
        ON execution_processes (session_id, status, run_reason);

CREATE INDEX idx_execution_processes_session_run_reason_created
        ON execution_processes (session_id, run_reason, created_at DESC);
