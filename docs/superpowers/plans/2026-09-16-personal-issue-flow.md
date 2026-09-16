# 个人版需求链路 实施计划（P2 第一期）

> **给智能体执行者：** 必需子技能：使用 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 按任务逐条实施本计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 在不依赖 Docker、Postgres、ElectricSQL 与云端的前提下，让单人在本机打通「建需求 → 看板推进 → 从需求创建工作区并启动编码智能体 → 查看变更 → 合并 → 需求自动完成」。

**架构：** 本地 SQLite 新建与云端同构的需求存储（`local_projects` / `project_statuses` / `issues` / `project_tags` / `issue_tags` / `issue_comments`），`crates/server` 新增 `/api/local/*` REST 接口（路径与 JSON 与云端 `/v1/*` 同构），复用既有的 SQLite 变更钩子 + WebSocket JSON Patch 做实时推送；前端在 `createShapeCollection` 这一个接缝上加「数据源开关」，个人版走「REST 快照 + WS 增量 + 写后刷新」，团队版保持 ElectricSQL 现状。

**技术栈：** Rust（axum 0.8 / sqlx 0.8 SQLite / tokio / ts-rs）、TypeScript + React 18、TanStack DB、Vitest（本计划新引入到 `packages/web-core`）、pnpm 10 workspace。

---

## 阅读顺序（执行前必读）

1. 设计规格：`docs/superpowers/specs/2026-09-16-personal-issue-flow-design.md`
2. 上级方案第 6 节：`docs/superpowers/specs/2026-09-15-vibe-kanban-optimization-roadmap.md`
3. 仓库约定：`CLAUDE.md`、`packages/local-web/AGENTS.md`、`docs/AGENTS.md`

**本期不改动 `crates/remote`**（它被 `Cargo.toml` 的 `exclude` 排除在默认 workspace 之外，只在 `pnpm run backend:check` 中单独编译）。因此**不得修改 `crates/api-types` 中已有类型的字段**——任何新增字段都会连带改到远端实现与 `shared/remote-types.ts`。本计划所有「本地专有字段」都放在 `crates/db` 的本地结构体里。

---

## 关键约定（跨任务共享，务必保持一致）

| 名称 | 取值 | 定义位置 |
|---|---|---|
| 固定组织 ID | `00000000-0000-0000-0000-000000000001` | `db::models::local_project::DEFAULT_ORGANIZATION_ID`（`Uuid::from_u128(1)`） |
| 固定用户 ID | `00000000-0000-0000-0000-000000000002` | `db::models::local_project::DEFAULT_USER_ID`（`Uuid::from_u128(2)`） |
| 本地接口前缀 | `/api/local` | `crates/server/src/routes/local_projects/mod.rs` |
| 快照响应 | `{ "<前端表名>": [ ...行 ] }` | `routes/local_projects/mod.rs::snapshot()` |
| 写响应 | `{ "txid": 0 }` | `routes/local_projects/mod.rs::txid()` |
| 阶段枚举 | `backlog` / `todo` / `dev` / `review` / `done` | `db::models::local_project_status::StageType` |
| 需求 WS | `/api/issues/streams/ws?project_id=<uuid>` | `crates/server/src/routes/issues.rs` |
| 数据源开关 | `'local'` / `'remote'` | `packages/web-core/src/shared/lib/local/dataSource.ts` |

**数据库表名 ↔ 前端表名映射**（前端 `ShapeDefinition.table` 决定快照响应的 key，与 SQLite 表名不必相同）：

| SQLite 表 | 前端表名（响应 key） |
|---|---|
| `local_projects` | `projects` |
| `project_statuses` | `project_statuses` |
| `issues` | `issues` |
| `project_tags` | `tags` |
| `issue_tags` | `issue_tags` |
| `issue_comments` | `issue_comments` |
| `workspaces`（投影） | `workspaces` |
| `pull_requests`（投影） | `pull_requests` |

---

## 文件结构

### 新建（后端）

| 文件 | 职责 |
|---|---|
| `crates/db/migrations/20260916000000_add_local_issue_tracking.sql` | 6 张新表 + `workspaces.issue_id` |
| `crates/db/src/test_support.rs` | 临时 SQLite 测试库工厂 `TestDb` |
| `crates/db/src/models/local_project.rs` | `LocalProjects` 增删改查 + 默认状态列 + `simple_id` 前缀 |
| `crates/db/src/models/local_project_status.rs` | `LocalProjectStatus` 行类型、`StageType`、`ProjectStatuses` |
| `crates/db/src/models/issue.rs` | `Issues`：编号分配、字段级原子更新、批量事务、搜索分页、阶段流转 |
| `crates/db/src/models/issue_side.rs` | `ProjectTags` / `IssueTags` / `IssueComments`（三个小实体同生共死，放一起） |
| `crates/server/src/routes/local_projects/mod.rs` | `/api/local` 路由树 + `snapshot()` / `txid()` 辅助 |
| `crates/server/src/routes/local_projects/projects.rs` | 项目路由 |
| `crates/server/src/routes/local_projects/statuses.rs` | 状态列路由（含 `/bulk`） |
| `crates/server/src/routes/local_projects/issues.rs` | 需求路由（含 `/bulk`、`/search`） |
| `crates/server/src/routes/local_projects/side.rs` | 标签、需求标签、评论路由 |
| `crates/server/src/routes/local_projects/projections.rs` | `workspaces` / `pull_requests` 投影 + 空集合 |
| `crates/server/src/routes/issues.rs` | `/api/issues/streams/ws` |

### 修改（后端）

| 文件 | 改动 |
|---|---|
| `crates/db/Cargo.toml` | 新增 `tempfile`、`thiserror` 已有 |
| `crates/db/src/lib.rs` | `pub mod test_support;` + `DBService::new_at_path` |
| `crates/db/src/models/mod.rs` | 注册 4 个新模块 |
| `crates/db/src/models/workspace.rs` | `Workspace.issue_id` 字段 + 5 处 SELECT 列表 + `set_issue_id` |
| `crates/services/src/services/events/types.rs` | `HookTables` / `RecordTypes` 新增 3 张表 |
| `crates/services/src/services/events/patches.rs` | `issue_patch` / `project_status_patch` / `issue_comment_patch` |
| `crates/services/src/services/events.rs` | 前置删除钩子 + 更新钩子新增分支 |
| `crates/services/src/services/events/streams.rs` | `stream_issues_raw(project_id)` |
| `crates/server/src/routes/mod.rs` | 挂载 `local_projects` 与 `issues` |
| `crates/server/src/routes/workspaces/create.rs` | `linked_issue` 本地分支 |
| `crates/server/src/routes/workspaces/pr.rs` | 建 PR → `review` |
| `crates/server/src/routes/workspaces/git.rs` | 本地合并 → `done` |
| `crates/db/src/models/pull_request.rs` | `update_status` 合并态 → `done` |

### 新建（前端）

| 文件 | 职责 |
|---|---|
| `packages/web-core/vitest.config.ts` | Vitest 配置（别名与 `src/**/*.test.ts`） |
| `packages/web-core/src/shared/lib/electric/rows.ts` | 从 `collections.ts` 抽出的纯函数：`getRowKey` / `extractFallbackRows` / `parseResponseError` |
| `packages/web-core/src/shared/lib/electric/rows.test.ts` | 上述纯函数的单测 |
| `packages/web-core/src/shared/lib/local/dataSource.ts` | 数据源运行时开关 |
| `packages/web-core/src/shared/lib/local/identity.ts` | 固定组织 / 用户常量与对象 |
| `packages/web-core/src/shared/lib/local/localEndpoints.ts` | shape/mutation → 本地 REST 路径映射 |
| `packages/web-core/src/shared/lib/local/localEndpoints.test.ts` | 映射单测 |
| `packages/web-core/src/shared/lib/local/localCollections.ts` | 本地同步（快照 + WS 增量）与写后刷新 |
| `packages/web-core/src/shared/lib/local/localCollections.test.ts` | 快照解析、patch 增量、写后刷新单测 |

### 修改（前端）

| 文件 | 改动 |
|---|---|
| `packages/web-core/package.json` | `vitest` 依赖 + `test` 脚本 |
| `package.json`（根） | `web-core:test` 脚本并接入 `check` |
| `packages/web-core/src/shared/lib/electric/collections.ts` | 抽出纯函数、个人版分支 |
| `packages/web-core/src/shared/lib/remoteApi.ts` | `bulkUpdateIssues` 等在个人版走本地 |
| `packages/local-web/src/app/entry/Bootstrap.tsx` | `configureDataSource` 注入 |
| `packages/web-core/src/shared/hooks/useUserOrganizations.ts` | 个人版返回固定组织 |
| `packages/web-core/src/shared/hooks/auth/useAuth.ts` | 个人版视为已登录 |
| `packages/web-core/src/pages/kanban/ProjectKanban.tsx` | 还原 + 个人版旁路 |
| `packages/web-core/src/pages/kanban/LocalProjectKanban.tsx` | 还原 |
| `packages/web-core/src/pages/root/RootRedirectPage.tsx` | 个人版进项目 |

---

## 任务 0：前置环境准备（不产生提交）

**Files:** 无

- [ ] **步骤 1：确认 sqlx 命令行可用**

运行：

```bash
cd /Users/admin/work/github/vibe-kanban && cargo sqlx --version
```

预期：输出 `sqlx-cli-sqlx x.y.z`。若输出 `error: no such command: sqlx`，执行下一步。

- [ ] **步骤 2：安装 sqlx-cli（仅当上一步失败）**

```bash
cargo install sqlx-cli --no-default-features --features sqlite,rustls --locked
```

预期：最后一行 `Installed package sqlx-cli ...`。再次运行 `cargo sqlx --version` 应成功。

- [ ] **步骤 3：确认基线干净**

```bash
cd /Users/admin/work/github/vibe-kanban && git status --short && cargo check -p db
```

预期：`git status --short` 无输出；`cargo check -p db` 以 `Finished` 结束。

- [ ] **步骤 4：记录本地开发启动方式（后续验收步骤复用）**

```bash
export FRONTEND_PORT=$(node scripts/setup-dev-environment.js frontend)
export BACKEND_PORT=$(node scripts/setup-dev-environment.js backend)
export PREVIEW_PROXY_PORT=$(node scripts/setup-dev-environment.js preview_proxy)
export VK_ALLOWED_ORIGINS="http://localhost:$FRONTEND_PORT"
cargo run --bin server   # 另一个终端：pnpm run local-web:dev
```

说明：本机未安装 `cargo-watch`，所以**不要**用 `pnpm run dev` / `pnpm run backend:dev:watch`，改用上面的 `cargo run --bin server`。

---

## 任务 1：数据库迁移与测试库基座

**Files:**
- Create: `crates/db/migrations/20260916000000_add_local_issue_tracking.sql`
- Create: `crates/db/src/test_support.rs`
- Modify: `crates/db/Cargo.toml`
- Modify: `crates/db/src/lib.rs`
- Test: `crates/db/src/test_support.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：写迁移文件**

创建 `crates/db/migrations/20260916000000_add_local_issue_tracking.sql`：

```sql
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
```

- [ ] **步骤 2：给 db crate 加 tempfile 依赖**

在 `crates/db/Cargo.toml` 的 `[dependencies]` 末尾追加：

```toml
tempfile = "3"
```

（放在 `[dependencies]` 而非 `[dev-dependencies]`，因为 `crates/server` 的测试也要用 `db::test_support`。）

- [ ] **步骤 3：在 `crates/db/src/lib.rs` 增加 `new_at_path` 与模块声明**

在 `pub mod models;` 下一行加：

```rust
pub mod test_support;
```

在 `impl DBService {` 内、`pub async fn new_migration_pool` 之前插入：

```rust
    /// 在指定路径创建并迁移一个独立数据库。
    /// 供测试与离线工具使用，不读取 asset_dir()，因此不会污染开发数据。
    pub async fn new_at_path(path: &std::path::Path) -> Result<DBService, Error> {
        let database_url = format!("sqlite://{}", path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&database_url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePool::connect_with(options).await?;
        run_migrations(&pool).await?;
        Ok(DBService { pool })
    }
```

- [ ] **步骤 4：写失败的测试**

创建 `crates/db/src/test_support.rs`：

```rust
//! 测试用的一次性 SQLite 数据库。
//! 每个 TestDb 拥有自己的临时目录，Drop 时自动删除。

use sqlx::SqlitePool;
use tempfile::TempDir;

use crate::DBService;

pub struct TestDb {
    pub db: DBService,
    _dir: TempDir,
}

impl TestDb {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = DBService::new_at_path(&dir.path().join("test.sqlite"))
            .await
            .expect("初始化测试数据库失败");
        Self { db, _dir: dir }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.db.pool
    }
}

#[cfg(test)]
mod tests {
    use super::TestDb;

    /// 列出某张表的列名，顺序与 SQLite 存储顺序一致。
    async fn column_names(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
        // table 来自测试常量，不接受外部输入；PRAGMA 不支持绑定参数。
        let sql = format!("PRAGMA table_info({table})");
        let rows: Vec<(i64, String)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .expect("读取表结构失败")
            .into_iter()
            .map(|r: (i64, String, String, i64, Option<String>, i64)| (r.0, r.1))
            .collect();
        rows.into_iter().map(|(_, name)| name).collect()
    }

    #[tokio::test]
    async fn 迁移后新表存在且_id_是第零列() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();

        for table in [
            "local_projects",
            "project_statuses",
            "issues",
            "project_tags",
            "issue_tags",
            "issue_comments",
        ] {
            let columns = column_names(pool, table).await;
            assert!(!columns.is_empty(), "表 {table} 不存在");
            assert_eq!(columns[0], "id", "表 {table} 的 id 必须是第 0 列");
        }
    }

    #[tokio::test]
    async fn workspaces_新增_issue_id_列且外键为_set_null() {
        let test_db = TestDb::new().await;
        let columns = column_names(test_db.pool(), "workspaces").await;
        assert!(
            columns.contains(&"issue_id".to_string()),
            "workspaces 缺少 issue_id 列，实际列：{columns:?}"
        );

        let fks: Vec<(String, String)> = sqlx::query_as(
            "SELECT \"table\", on_delete FROM pragma_foreign_key_list('workspaces')",
        )
        .fetch_all(test_db.pool())
        .await
        .expect("读取外键失败");

        assert!(
            fks.iter()
                .any(|(table, on_delete)| table == "issues" && on_delete == "SET NULL"),
            "workspaces.issue_id 必须是 ON DELETE SET NULL，实际外键：{fks:?}"
        );
    }

    #[tokio::test]
    async fn 遗留表未被删除() {
        let test_db = TestDb::new().await;
        let legacy = column_names(test_db.pool(), "projects").await;
        assert!(!legacy.is_empty(), "遗留 projects 表不应被本迁移删除");
    }
}
```

- [ ] **步骤 5：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db test_support 2>&1 | tail -20
```

预期：编译失败或 `迁移后新表存在且_id_是第零列` 失败（迁移尚未被 sqlx 缓存 / 表不存在）。

- [ ] **步骤 6：刷新 sqlx 离线缓存并重跑**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db
cargo test -p db test_support 2>&1 | tail -20
```

预期：`test result: ok. 3 passed`。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/migrations/20260916000000_add_local_issue_tracking.sql \
        crates/db/src/test_support.rs crates/db/src/lib.rs crates/db/Cargo.toml \
        crates/db/.sqlx Cargo.lock
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：新增本地需求存储迁移与测试数据库基座" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 2：本地项目模型与默认状态列

**Files:**
- Create: `crates/db/src/models/local_project.rs`
- Modify: `crates/db/src/models/mod.rs`
- Test: `crates/db/src/models/local_project.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：注册模块**

在 `crates/db/src/models/mod.rs` 按字母序插入：

```rust
pub mod local_project;
```

- [ ] **步骤 2：写失败的测试**

创建 `crates/db/src/models/local_project.rs`，先只写测试模块与空的类型骨架（下一步补实现）。完整测试代码：

```rust
#[cfg(test)]
mod tests {
    use api_types::project::{CreateProjectRequest, UpdateProjectRequest};

    use super::{DEFAULT_ORGANIZATION_ID, LocalProjects};
    use crate::test_support::TestDb;

    fn 建项目请求(name: &str) -> CreateProjectRequest {
        CreateProjectRequest {
            id: None,
            organization_id: DEFAULT_ORGANIZATION_ID,
            name: name.to_string(),
            color: "#6366f1".to_string(),
        }
    }

    #[tokio::test]
    async fn 新建项目自动创建五个默认状态列() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(test_db.pool(), &建项目请求("Vibe Kanban"))
            .await
            .expect("建项目失败");

        let statuses: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT name, stage_type, sort_order FROM project_statuses \
             WHERE project_id = ?1 ORDER BY sort_order",
        )
        .bind(project.id)
        .fetch_all(test_db.pool())
        .await
        .expect("读取状态列失败");

        assert_eq!(
            statuses,
            vec![
                ("待规划".to_string(), "backlog".to_string(), 0),
                ("待开发".to_string(), "todo".to_string(), 1),
                ("开发中".to_string(), "dev".to_string(), 2),
                ("待评审".to_string(), "review".to_string(), 3),
                ("已完成".to_string(), "done".to_string(), 4),
            ]
        );
    }

    #[tokio::test]
    async fn 建项目失败时不留下孤立状态列() {
        let test_db = TestDb::new().await;
        let mut req = 建项目请求("重复");
        let fixed_id = uuid::Uuid::from_u128(999);
        req.id = Some(fixed_id);

        LocalProjects::create(test_db.pool(), &req)
            .await
            .expect("首次建项目应成功");
        let second = LocalProjects::create(test_db.pool(), &req).await;
        assert!(second.is_err(), "主键重复应报错");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM project_statuses")
            .fetch_one(test_db.pool())
            .await
            .expect("统计失败");
        assert_eq!(count.0, 5, "第二次失败必须整体回滚，不得追加状态列");
    }

    #[tokio::test]
    async fn 列表只返回本组织项目且按_sort_order_排序() {
        let test_db = TestDb::new().await;
        let a = LocalProjects::create(test_db.pool(), &建项目请求("A"))
            .await
            .unwrap();
        let b = LocalProjects::create(test_db.pool(), &建项目请求("B"))
            .await
            .unwrap();

        LocalProjects::update(
            test_db.pool(),
            a.id,
            &UpdateProjectRequest {
                name: None,
                color: None,
                sort_order: Some(10),
            },
        )
        .await
        .unwrap();

        let list = LocalProjects::find_all(test_db.pool()).await.unwrap();
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![b.id, a.id],
            "sort_order 小的在前"
        );
        assert!(list.iter().all(|p| p.organization_id == DEFAULT_ORGANIZATION_ID));
    }

    #[tokio::test]
    async fn 删除项目级联删除状态列与需求() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(test_db.pool(), &建项目请求("X"))
            .await
            .unwrap();

        let affected = LocalProjects::delete(test_db.pool(), project.id)
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM project_statuses")
            .fetch_one(test_db.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0, "状态列应随项目级联删除");
    }

    #[tokio::test]
    async fn 删除不存在的项目返回零行() {
        let test_db = TestDb::new().await;
        let affected = LocalProjects::delete(test_db.pool(), uuid::Uuid::from_u128(12345))
            .await
            .unwrap();
        assert_eq!(affected, 0, "删除不存在的项目不得报错，也不得误删他人数据");
    }

    #[test]
    fn simple_id_前缀取名称首字母最多三位() {
        assert_eq!(LocalProjects::simple_id_prefix("Vibe Kanban Web"), "VKW");
        assert_eq!(LocalProjects::simple_id_prefix("Vibe Kanban Web Extra"), "VKW");
        assert_eq!(LocalProjects::simple_id_prefix("kanban"), "K");
        assert_eq!(LocalProjects::simple_id_prefix("需求管理"), "ISS");
        assert_eq!(LocalProjects::simple_id_prefix("   "), "ISS");
        assert_eq!(LocalProjects::simple_id_prefix(""), "ISS");
    }
}
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db local_project 2>&1 | tail -20
```

预期：编译失败，`cannot find value DEFAULT_ORGANIZATION_ID` / `cannot find type LocalProjects`。

- [ ] **步骤 4：写实现**

在 `crates/db/src/models/local_project.rs` 的测试模块之前插入：

```rust
use api_types::project::{CreateProjectRequest, Project, UpdateProjectRequest};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

/// 个人版固定组织：前端 identity.ts 必须使用同一字符串。
pub const DEFAULT_ORGANIZATION_ID: Uuid = Uuid::from_u128(1);
/// 个人版固定用户。
pub const DEFAULT_USER_ID: Uuid = Uuid::from_u128(2);

/// 新建项目时自动创建的状态列：(名称, 颜色, stage_type)
pub const DEFAULT_STATUSES: [(&str, &str, &str); 5] = [
    ("待规划", "#94a3b8", "backlog"),
    ("待开发", "#64748b", "todo"),
    ("开发中", "#3b82f6", "dev"),
    ("待评审", "#a855f7", "review"),
    ("已完成", "#22c55e", "done"),
];

/// 项目名称长度上限，防止超大字段撑爆快照响应。
pub const MAX_PROJECT_NAME_LEN: usize = 200;

pub struct LocalProjects;

impl LocalProjects {
    /// 取项目名首字母（最多 3 个 ASCII 字母，大写）作为 simple_id 前缀，无字母时用 ISS。
    pub fn simple_id_prefix(name: &str) -> String {
        let prefix: String = name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .filter(|c| c.is_ascii_alphabetic())
            .take(3)
            .map(|c| c.to_ascii_uppercase())
            .collect();

        if prefix.is_empty() {
            "ISS".to_string()
        } else {
            prefix
        }
    }

    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Project>, sqlx::Error> {
        sqlx::query_as!(
            Project,
            r#"SELECT id           AS "id!: Uuid",
                      organization_id AS "organization_id!: Uuid",
                      name         AS "name!",
                      color        AS "color!",
                      sort_order   AS "sort_order!: i32",
                      created_at   AS "created_at!: DateTime<Utc>",
                      updated_at   AS "updated_at!: DateTime<Utc>"
               FROM local_projects
               ORDER BY sort_order ASC, created_at ASC"#
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as!(
            Project,
            r#"SELECT id           AS "id!: Uuid",
                      organization_id AS "organization_id!: Uuid",
                      name         AS "name!",
                      color        AS "color!",
                      sort_order   AS "sort_order!: i32",
                      created_at   AS "created_at!: DateTime<Utc>",
                      updated_at   AS "updated_at!: DateTime<Utc>"
               FROM local_projects
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    /// 建项目 + 5 个默认状态列，同一事务内完成。
    pub async fn create(
        pool: &SqlitePool,
        data: &CreateProjectRequest,
    ) -> Result<Project, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name: String = data.name.chars().take(MAX_PROJECT_NAME_LEN).collect();
        let organization_id = DEFAULT_ORGANIZATION_ID;

        let mut tx = pool.begin().await?;

        let project = sqlx::query_as!(
            Project,
            r#"INSERT INTO local_projects (id, organization_id, name, color, sort_order)
               VALUES ($1, $2, $3, $4,
                       (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM local_projects))
               RETURNING id AS "id!: Uuid",
                         organization_id AS "organization_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            organization_id,
            name,
            data.color
        )
        .fetch_one(&mut *tx)
        .await?;

        for (index, (status_name, color, stage_type)) in DEFAULT_STATUSES.iter().enumerate() {
            let status_id = Uuid::new_v4();
            let sort_order = index as i64;
            sqlx::query!(
                r#"INSERT INTO project_statuses
                       (id, project_id, name, color, sort_order, hidden, stage_type)
                   VALUES ($1, $2, $3, $4, $5, 0, $6)"#,
                status_id,
                project.id,
                status_name,
                color,
                sort_order,
                stage_type
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(project)
    }

    /// 只更新请求中出现的字段，用 CASE 在单条语句里完成，避免读改写竞态。
    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateProjectRequest,
    ) -> Result<Project, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| n.chars().take(MAX_PROJECT_NAME_LEN).collect());
        let set_color = data.color.is_some();
        let set_sort_order = data.sort_order.is_some();

        sqlx::query_as!(
            Project,
            r#"UPDATE local_projects SET
                   name       = CASE WHEN $2 THEN $3 ELSE name END,
                   color      = CASE WHEN $4 THEN $5 ELSE color END,
                   sort_order = CASE WHEN $6 THEN $7 ELSE sort_order END,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         organization_id AS "organization_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            set_name,
            name,
            set_color,
            data.color,
            set_sort_order,
            data.sort_order
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM local_projects WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
```

- [ ] **步骤 5：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p db local_project 2>&1 | tail -20
```

预期：`test result: ok. 6 passed`。

- [ ] **步骤 6：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/src/models/local_project.rs crates/db/src/models/mod.rs crates/db/.sqlx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：本地项目模型与默认状态列" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 3：状态列模型（stage_type 与批量事务）

**Files:**
- Create: `crates/db/src/models/local_project_status.rs`
- Modify: `crates/db/src/models/mod.rs`
- Test: `crates/db/src/models/local_project_status.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：注册模块**

在 `crates/db/src/models/mod.rs` 插入：

```rust
pub mod local_project_status;
```

- [ ] **步骤 2：写失败的测试**

在 `crates/db/src/models/local_project_status.rs` 写入：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        project::CreateProjectRequest,
        project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest},
    };
    use uuid::Uuid;

    use super::{ProjectStatuses, StageType};
    use crate::{
        models::local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
        test_support::TestDb,
    };

    async fn 建项目(test_db: &TestDb, name: &str) -> Uuid {
        LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: name.to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .expect("建项目失败")
        .id
    }

    #[tokio::test]
    async fn 按项目列出状态列并带出_stage_type() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        assert_eq!(statuses.len(), 5);
        assert_eq!(statuses[0].stage_type, "backlog");
        assert_eq!(statuses[4].stage_type, "done");
        assert!(statuses.iter().all(|s| s.project_id == project_id));
    }

    #[tokio::test]
    async fn 列出状态列不会串到别的项目() {
        let test_db = TestDb::new().await;
        let a = 建项目(&test_db, "A").await;
        let b = 建项目(&test_db, "B").await;

        let statuses = ProjectStatuses::find_by_project(test_db.pool(), a).await.unwrap();
        assert!(
            statuses.iter().all(|s| s.project_id == a),
            "不得返回项目 {b} 的状态列"
        );
    }

    #[tokio::test]
    async fn 按阶段查找状态列() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let dev = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .expect("应存在 dev 阶段状态列");
        assert_eq!(dev.name, "开发中");

        // 用户改名后仍能按 stage_type 找到
        ProjectStatuses::update(
            test_db.pool(),
            dev.id,
            &UpdateProjectStatusRequest {
                name: Some("Coding".to_string()),
                color: None,
                sort_order: None,
                hidden: None,
            },
        )
        .await
        .unwrap();

        let dev_again = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .expect("改名后仍应按 stage_type 命中");
        assert_eq!(dev_again.name, "Coding");
        assert_eq!(dev_again.id, dev.id);
    }

    #[tokio::test]
    async fn 新建的状态列_stage_type_为_todo() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;

        let created = ProjectStatuses::create(
            test_db.pool(),
            &CreateProjectStatusRequest {
                id: None,
                project_id,
                name: "联调中".to_string(),
                color: "#f59e0b".to_string(),
                sort_order: 9,
                hidden: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(created.stage_type, "todo");
        assert_eq!(created.sort_order, 9);
        assert!(!created.hidden);
    }

    #[tokio::test]
    async fn 批量更新在单事务内全成或全败() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;
        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        let updates = vec![
            (
                statuses[0].id,
                UpdateProjectStatusRequest {
                    name: None,
                    color: None,
                    sort_order: Some(100),
                    hidden: None,
                },
            ),
            (
                Uuid::from_u128(424242),
                UpdateProjectStatusRequest {
                    name: None,
                    color: None,
                    sort_order: Some(200),
                    hidden: None,
                },
            ),
        ];

        let result = ProjectStatuses::bulk_update(test_db.pool(), &updates).await;
        assert!(result.is_err(), "含不存在 id 的批量更新必须失败");

        let after = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();
        assert_eq!(
            after[0].sort_order, 0,
            "批量更新失败必须回滚第一条，不得留下半截状态"
        );
    }

    #[tokio::test]
    async fn 批量更新成功时按顺序返回所有行() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db, "A").await;
        let statuses = ProjectStatuses::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap();

        let updates: Vec<_> = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                (
                    status.id,
                    UpdateProjectStatusRequest {
                        name: None,
                        color: None,
                        sort_order: Some(10 - index as i32),
                        hidden: None,
                    },
                )
            })
            .collect();

        let updated = ProjectStatuses::bulk_update(test_db.pool(), &updates)
            .await
            .unwrap();

        assert_eq!(updated.len(), 5);
        assert_eq!(updated[0].sort_order, 10);
        assert_eq!(updated[4].sort_order, 6);
    }

    #[test]
    fn 阶段枚举与字符串互转() {
        assert_eq!(StageType::Backlog.as_str(), "backlog");
        assert_eq!(StageType::Todo.as_str(), "todo");
        assert_eq!(StageType::Dev.as_str(), "dev");
        assert_eq!(StageType::Review.as_str(), "review");
        assert_eq!(StageType::Done.as_str(), "done");
    }
}
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db local_project_status 2>&1 | tail -20
```

预期：编译失败，`cannot find type ProjectStatuses` / `StageType`。

- [ ] **步骤 4：写实现**

在测试模块之前插入：

```rust
use api_types::project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// 本地状态列行。字段与 api_types::ProjectStatus 一一对应，
/// 额外多出 stage_type（个人版专有，用于状态自动流转；
/// 多出的 JSON 字段前端会原样忽略，不影响 shared/remote-types.ts）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalProjectStatus {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub color: String,
    pub sort_order: i32,
    pub hidden: bool,
    pub stage_type: String,
    pub created_at: DateTime<Utc>,
}

/// 流程阶段。用户可以给状态列改名，自动流转靠 stage_type 识别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageType {
    Backlog,
    Todo,
    Dev,
    Review,
    Done,
}

impl StageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StageType::Backlog => "backlog",
            StageType::Todo => "todo",
            StageType::Dev => "dev",
            StageType::Review => "review",
            StageType::Done => "done",
        }
    }
}

/// 状态列名称长度上限。
pub const MAX_STATUS_NAME_LEN: usize = 100;

pub struct ProjectStatuses;

impl ProjectStatuses {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE project_id = $1
               ORDER BY sort_order ASC, created_at ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_stage(
        pool: &SqlitePool,
        project_id: Uuid,
        stage: StageType,
    ) -> Result<Option<LocalProjectStatus>, sqlx::Error> {
        let stage_type = stage.as_str();
        sqlx::query_as!(
            LocalProjectStatus,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!",
                      sort_order AS "sort_order!: i32",
                      hidden     AS "hidden!: bool",
                      stage_type AS "stage_type!",
                      created_at AS "created_at!: DateTime<Utc>"
               FROM project_statuses
               WHERE project_id = $1 AND stage_type = $2
               ORDER BY sort_order ASC
               LIMIT 1"#,
            project_id,
            stage_type
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name: String = data.name.chars().take(MAX_STATUS_NAME_LEN).collect();

        sqlx::query_as!(
            LocalProjectStatus,
            r#"INSERT INTO project_statuses
                   (id, project_id, name, color, sort_order, hidden, stage_type)
               VALUES ($1, $2, $3, $4, $5, $6, 'todo')
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         hidden AS "hidden!: bool",
                         stage_type AS "stage_type!",
                         created_at AS "created_at!: DateTime<Utc>""#,
            id,
            data.project_id,
            name,
            data.color,
            data.sort_order,
            data.hidden
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        let mut tx = pool.begin().await?;
        let updated = Self::update_in_tx(&mut tx, id, data).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// 批量更新（拖拽排序）。单事务，任一条失败整体回滚。
    pub async fn bulk_update(
        pool: &SqlitePool,
        updates: &[(Uuid, UpdateProjectStatusRequest)],
    ) -> Result<Vec<LocalProjectStatus>, sqlx::Error> {
        let mut tx = pool.begin().await?;
        let mut rows = Vec::with_capacity(updates.len());
        for (id, data) in updates {
            rows.push(Self::update_in_tx(&mut tx, *id, data).await?);
        }
        tx.commit().await?;
        Ok(rows)
    }

    async fn update_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: Uuid,
        data: &UpdateProjectStatusRequest,
    ) -> Result<LocalProjectStatus, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| n.chars().take(MAX_STATUS_NAME_LEN).collect());
        let set_color = data.color.is_some();
        let set_sort_order = data.sort_order.is_some();
        let set_hidden = data.hidden.is_some();

        sqlx::query_as!(
            LocalProjectStatus,
            r#"UPDATE project_statuses SET
                   name       = CASE WHEN $2 THEN $3 ELSE name END,
                   color      = CASE WHEN $4 THEN $5 ELSE color END,
                   sort_order = CASE WHEN $6 THEN $7 ELSE sort_order END,
                   hidden     = CASE WHEN $8 THEN $9 ELSE hidden END
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!",
                         sort_order AS "sort_order!: i32",
                         hidden AS "hidden!: bool",
                         stage_type AS "stage_type!",
                         created_at AS "created_at!: DateTime<Utc>""#,
            id,
            set_name,
            name,
            set_color,
            data.color,
            set_sort_order,
            data.sort_order,
            set_hidden,
            data.hidden
        )
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM project_statuses WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
```

- [ ] **步骤 5：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p db local_project_status 2>&1 | tail -20
```

预期：`test result: ok. 7 passed`。

- [ ] **步骤 6：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/src/models/local_project_status.rs crates/db/src/models/mod.rs crates/db/.sqlx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：状态列模型与阶段类型" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 4：需求模型（编号并发、原子更新、批量事务、搜索分页）

**Files:**
- Create: `crates/db/src/models/issue.rs`
- Modify: `crates/db/src/models/mod.rs`
- Test: `crates/db/src/models/issue.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：注册模块**

在 `crates/db/src/models/mod.rs` 插入：

```rust
pub mod issue;
```

- [ ] **步骤 2：写失败的测试**

在 `crates/db/src/models/issue.rs` 写入：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        issue::{
            CreateIssueRequest, IssuePriority, IssueSortField, SearchIssuesRequest, SortDirection,
            UpdateIssueRequest,
        },
        project::CreateProjectRequest,
    };
    use uuid::Uuid;

    use super::{Issues, MAX_DESCRIPTION_LEN, MAX_TITLE_LEN};
    use crate::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

    struct 场景 {
        project_id: Uuid,
        todo_status_id: Uuid,
    }

    async fn 准备(test_db: &TestDb, name: &str) -> 场景 {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: name.to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .expect("建项目失败");

        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .expect("应有 todo 状态列");

        场景 {
            project_id: project.id,
            todo_status_id: todo.id,
        }
    }

    fn 建需求请求(场景: &场景, title: &str) -> CreateIssueRequest {
        CreateIssueRequest {
            id: None,
            project_id: 场景.project_id,
            status_id: 场景.todo_status_id,
            title: title.to_string(),
            description: None,
            priority: Some(IssuePriority::Medium),
            start_date: None,
            target_date: None,
            completed_at: None,
            sort_order: 0.0,
            parent_issue_id: None,
            parent_issue_sort_order: None,
            extension_metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn 需求编号按项目自增且_simple_id_带项目前缀() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let first = Issues::create(test_db.pool(), &建需求请求(&场景, "第一条"))
            .await
            .unwrap();
        let second = Issues::create(test_db.pool(), &建需求请求(&场景, "第二条"))
            .await
            .unwrap();

        assert_eq!(first.issue_number, 1);
        assert_eq!(second.issue_number, 2);
        assert_eq!(first.simple_id, "VK-1");
        assert_eq!(second.simple_id, "VK-2");
    }

    #[tokio::test]
    async fn 两个项目的编号互不干扰() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        let a1 = Issues::create(test_db.pool(), &建需求请求(&a, "A1")).await.unwrap();
        let b1 = Issues::create(test_db.pool(), &建需求请求(&b, "B1")).await.unwrap();

        assert_eq!(a1.issue_number, 1);
        assert_eq!(b1.issue_number, 1);
        assert_eq!(a1.simple_id, "A-1");
        assert_eq!(b1.simple_id, "B-1");
    }

    #[tokio::test]
    async fn 并发建需求不会产生重复编号() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let pool = test_db.pool().clone();

        let mut handles = Vec::new();
        for index in 0..8 {
            let pool = pool.clone();
            let request = 建需求请求(&场景, &format!("并发 {index}"));
            handles.push(tokio::spawn(async move {
                Issues::create(&pool, &request).await
            }));
        }

        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap().expect("并发建需求不应失败").issue_number);
        }
        numbers.sort_unstable();

        assert_eq!(numbers, (1..=8).collect::<Vec<i32>>(), "编号必须连续且唯一");
    }

    #[tokio::test]
    async fn 标题与描述超长时被截断而不是报错() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let mut request = 建需求请求(&场景, &"标".repeat(MAX_TITLE_LEN + 100));
        request.description = Some("描".repeat(MAX_DESCRIPTION_LEN + 100));

        let issue = Issues::create(test_db.pool(), &request).await.unwrap();

        assert_eq!(issue.title.chars().count(), MAX_TITLE_LEN);
        assert_eq!(
            issue.description.as_ref().unwrap().chars().count(),
            MAX_DESCRIPTION_LEN
        );
    }

    #[tokio::test]
    async fn 空标题被拒绝() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        let result = Issues::create(test_db.pool(), &建需求请求(&场景, "   ")).await;
        assert!(result.is_err(), "空白标题必须拒绝");
    }

    #[tokio::test]
    async fn 并发更新不同字段互不覆盖() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "原标题"))
            .await
            .unwrap();

        let pool_a = test_db.pool().clone();
        let pool_b = test_db.pool().clone();
        let id = issue.id;

        let a = tokio::spawn(async move {
            Issues::update(
                &pool_a,
                id,
                &UpdateIssueRequest {
                    title: Some("新标题".to_string()),
                    ..Default::default()
                },
            )
            .await
        });
        let b = tokio::spawn(async move {
            Issues::update(
                &pool_b,
                id,
                &UpdateIssueRequest {
                    priority: Some(Some(IssuePriority::Urgent)),
                    ..Default::default()
                },
            )
            .await
        });

        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let after = Issues::find_by_id(test_db.pool(), id).await.unwrap().unwrap();
        assert_eq!(after.title, "新标题", "并发更新不得丢失标题");
        assert!(
            matches!(after.priority, Some(IssuePriority::Urgent)),
            "并发更新不得丢失优先级"
        );
    }

    #[tokio::test]
    async fn 更新可以把描述显式置空() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let mut request = 建需求请求(&场景, "带描述");
        request.description = Some("正文".to_string());
        let issue = Issues::create(test_db.pool(), &request).await.unwrap();

        let updated = Issues::update(
            test_db.pool(),
            issue.id,
            &UpdateIssueRequest {
                description: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.description, None);
    }

    #[tokio::test]
    async fn 批量更新排序在单事务内全成或全败() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "拖拽"))
            .await
            .unwrap();

        let updates = vec![
            (
                issue.id,
                UpdateIssueRequest {
                    sort_order: Some(5.0),
                    ..Default::default()
                },
            ),
            (
                Uuid::from_u128(987654),
                UpdateIssueRequest {
                    sort_order: Some(6.0),
                    ..Default::default()
                },
            ),
        ];

        assert!(Issues::bulk_update(test_db.pool(), &updates).await.is_err());

        let after = Issues::find_by_id(test_db.pool(), issue.id).await.unwrap().unwrap();
        assert_eq!(after.sort_order, 0.0, "失败必须整体回滚");
    }

    #[tokio::test]
    async fn 搜索按标题模糊匹配且转义通配符() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        Issues::create(test_db.pool(), &建需求请求(&场景, "登录页面"))
            .await
            .unwrap();
        Issues::create(test_db.pool(), &建需求请求(&场景, "100%完成度"))
            .await
            .unwrap();

        let hit = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                search: Some("登录".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hit.issues.len(), 1);
        assert_eq!(hit.total_count, 1);

        // "%" 是 LIKE 通配符，必须被转义，否则会匹配到全部行
        let escaped = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                search: Some("%".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(escaped.issues.len(), 1, "% 必须按字面量匹配");
        assert_eq!(escaped.issues[0].title, "100%完成度");
    }

    #[tokio::test]
    async fn 搜索限制单页上限并返回总数() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;

        for index in 0..5 {
            Issues::create(test_db.pool(), &建需求请求(&场景, &format!("需求 {index}")))
                .await
                .unwrap();
        }

        let page = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                limit: Some(2),
                offset: Some(1),
                sort_field: Some(IssueSortField::CreatedAt),
                sort_direction: Some(SortDirection::Asc),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(page.issues.len(), 2);
        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, 2);
        assert_eq!(page.offset, 1);
        assert_eq!(page.issues[0].title, "需求 1");

        // 超出上限的 limit 被夹到 MAX_PAGE_SIZE
        let huge = Issues::search(
            test_db.pool(),
            &SearchIssuesRequest {
                project_id: 场景.project_id,
                limit: Some(100_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(huge.limit, super::MAX_PAGE_SIZE);
    }

    #[tokio::test]
    async fn 按项目列出时不返回其他项目的需求() {
        let test_db = TestDb::new().await;
        let a = 准备(&test_db, "Alpha").await;
        let b = 准备(&test_db, "Beta").await;

        Issues::create(test_db.pool(), &建需求请求(&a, "A1")).await.unwrap();
        Issues::create(test_db.pool(), &建需求请求(&b, "B1")).await.unwrap();

        let list = Issues::find_by_project(test_db.pool(), a.project_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "A1");
    }

    #[tokio::test]
    async fn 移动到已完成阶段会写入完成时间() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "待办"))
            .await
            .unwrap();
        assert!(issue.completed_at.is_none());

        let done = Issues::move_to_stage(test_db.pool(), issue.id, StageType::Done)
            .await
            .unwrap()
            .expect("应完成流转");
        assert!(done.completed_at.is_some());

        let done_status = ProjectStatuses::find_stage(test_db.pool(), 场景.project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status_id, done_status.id);

        // 再流转回开发中，完成时间必须清空
        let dev = Issues::move_to_stage(test_db.pool(), issue.id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        assert!(dev.completed_at.is_none());
    }

    #[tokio::test]
    async fn 流转不存在的需求返回_none_而不是报错() {
        let test_db = TestDb::new().await;
        let result = Issues::move_to_stage(test_db.pool(), Uuid::from_u128(5555), StageType::Dev)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn 删除需求把工作区的_issue_id_置空() {
        let test_db = TestDb::new().await;
        let 场景 = 准备(&test_db, "Vibe Kanban").await;
        let issue = Issues::create(test_db.pool(), &建需求请求(&场景, "要删的"))
            .await
            .unwrap();

        let workspace_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workspaces (id, branch, issue_id) VALUES (?1, 'vk/test', ?2)",
        )
        .bind(workspace_id)
        .bind(issue.id)
        .execute(test_db.pool())
        .await
        .expect("插入工作区失败");

        Issues::delete(test_db.pool(), issue.id).await.unwrap();

        let linked: (Option<Uuid>,) =
            sqlx::query_as("SELECT issue_id FROM workspaces WHERE id = ?1")
                .bind(workspace_id)
                .fetch_one(test_db.pool())
                .await
                .unwrap();
        assert!(linked.0.is_none(), "删除需求后工作区必须解绑而不是被删除");
    }
}
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db models::issue 2>&1 | tail -20
```

预期：编译失败，`cannot find type Issues`。

- [ ] **步骤 4：写实现**

在测试模块之前插入：

```rust
use api_types::issue::{
    CreateIssueRequest, Issue, IssuePriority, IssueSortField, ListIssuesResponse,
    SearchIssuesRequest, SortDirection, UpdateIssueRequest,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

use super::{
    local_project::LocalProjects,
    local_project_status::{ProjectStatuses, StageType},
};

/// 标题上限，超出部分截断（防止超大字段撑爆快照与 WS 推送）。
pub const MAX_TITLE_LEN: usize = 500;
/// 描述上限，超出部分截断。
pub const MAX_DESCRIPTION_LEN: usize = 100_000;
/// 单次搜索返回上限，防止前端一次拉取过多行。
pub const MAX_PAGE_SIZE: usize = 500;
/// 编号分配的重试次数（并发插入撞 UNIQUE 时重试）。
const NUMBER_RETRY: u32 = 8;

#[derive(Debug, Error)]
pub enum IssueError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

/// 用于 sqlx 解码的中间行：extension_metadata 在 SQLite 里是 TEXT。
struct IssueRow {
    id: Uuid,
    project_id: Uuid,
    issue_number: i32,
    simple_id: String,
    status_id: Uuid,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    start_date: Option<DateTime<Utc>>,
    target_date: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    sort_order: f64,
    parent_issue_id: Option<Uuid>,
    parent_issue_sort_order: Option<f64>,
    extension_metadata: String,
    creator_user_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<IssueRow> for Issue {
    fn from(row: IssueRow) -> Self {
        Issue {
            id: row.id,
            project_id: row.project_id,
            issue_number: row.issue_number,
            simple_id: row.simple_id,
            status_id: row.status_id,
            title: row.title,
            description: row.description,
            priority: row.priority.as_deref().and_then(parse_priority),
            start_date: row.start_date,
            target_date: row.target_date,
            completed_at: row.completed_at,
            sort_order: row.sort_order,
            parent_issue_id: row.parent_issue_id,
            parent_issue_sort_order: row.parent_issue_sort_order,
            extension_metadata: serde_json::from_str(&row.extension_metadata)
                .unwrap_or_else(|_| Value::Object(Default::default())),
            creator_user_id: row.creator_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn parse_priority(raw: &str) -> Option<IssuePriority> {
    match raw {
        "urgent" => Some(IssuePriority::Urgent),
        "high" => Some(IssuePriority::High),
        "medium" => Some(IssuePriority::Medium),
        "low" => Some(IssuePriority::Low),
        _ => None,
    }
}

fn priority_to_str(priority: IssuePriority) -> &'static str {
    match priority {
        IssuePriority::Urgent => "urgent",
        IssuePriority::High => "high",
        IssuePriority::Medium => "medium",
        IssuePriority::Low => "low",
    }
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 转义 LIKE 通配符，配合 SQL 中的 ESCAPE '\' 使用。
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

const ISSUE_COLUMNS: &str = r#"
    id                      AS "id!: Uuid",
    project_id              AS "project_id!: Uuid",
    issue_number            AS "issue_number!: i32",
    simple_id               AS "simple_id!",
    status_id               AS "status_id!: Uuid",
    title                   AS "title!",
    description,
    priority,
    start_date              AS "start_date: DateTime<Utc>",
    target_date             AS "target_date: DateTime<Utc>",
    completed_at            AS "completed_at: DateTime<Utc>",
    sort_order              AS "sort_order!: f64",
    parent_issue_id         AS "parent_issue_id: Uuid",
    parent_issue_sort_order AS "parent_issue_sort_order: f64",
    extension_metadata      AS "extension_metadata!",
    creator_user_id         AS "creator_user_id: Uuid",
    created_at              AS "created_at!: DateTime<Utc>",
    updated_at              AS "updated_at!: DateTime<Utc>"
"#;

pub struct Issues;

impl Issues {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<Issue>, sqlx::Error> {
        let rows = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE project_id = $1
               ORDER BY sort_order ASC, created_at ASC
               LIMIT $2"#,
            project_id,
            MAX_SNAPSHOT_ROWS
        )
        .fetch_all(pool)
        .await?;

        Ok(rows.into_iter().map(Issue::from).collect())
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Issue>, sqlx::Error> {
        let row = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Issue::from))
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<Issue>, sqlx::Error> {
        let row = sqlx::query_as!(
            IssueRow,
            r#"SELECT id                      AS "id!: Uuid",
                      project_id              AS "project_id!: Uuid",
                      issue_number            AS "issue_number!: i32",
                      simple_id               AS "simple_id!",
                      status_id               AS "status_id!: Uuid",
                      title                   AS "title!",
                      description,
                      priority,
                      start_date              AS "start_date: DateTime<Utc>",
                      target_date             AS "target_date: DateTime<Utc>",
                      completed_at            AS "completed_at: DateTime<Utc>",
                      sort_order              AS "sort_order!: f64",
                      parent_issue_id         AS "parent_issue_id: Uuid",
                      parent_issue_sort_order AS "parent_issue_sort_order: f64",
                      extension_metadata      AS "extension_metadata!",
                      creator_user_id         AS "creator_user_id: Uuid",
                      created_at              AS "created_at!: DateTime<Utc>",
                      updated_at              AS "updated_at!: DateTime<Utc>"
               FROM issues
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Issue::from))
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueRequest,
    ) -> Result<Issue, IssueError> {
        let title = truncate(data.title.trim(), MAX_TITLE_LEN);
        if title.is_empty() {
            return Err(IssueError::Validation("需求标题不能为空".to_string()));
        }
        let description = data
            .description
            .as_ref()
            .map(|d| truncate(d, MAX_DESCRIPTION_LEN));

        let project = LocalProjects::find_by_id(pool, data.project_id)
            .await?
            .ok_or_else(|| IssueError::Validation("项目不存在".to_string()))?;
        let prefix = LocalProjects::simple_id_prefix(&project.name);

        let status = ProjectStatuses::find_by_id(pool, data.status_id).await?;
        match status {
            Some(status) if status.project_id == data.project_id => {}
            Some(_) => {
                return Err(IssueError::Validation(
                    "状态列不属于该项目".to_string(),
                ));
            }
            None => return Err(IssueError::Validation("状态列不存在".to_string())),
        }

        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let priority = data.priority.map(priority_to_str);
        let metadata = data.extension_metadata.to_string();

        for attempt in 0..NUMBER_RETRY {
            let mut tx = pool.begin().await?;

            let next: (i64,) = sqlx::query_as(
                "SELECT COALESCE(MAX(issue_number), 0) + 1 FROM issues WHERE project_id = ?1",
            )
            .bind(data.project_id)
            .fetch_one(&mut *tx)
            .await?;
            let issue_number = next.0 as i32;
            let simple_id = format!("{prefix}-{issue_number}");

            let inserted = sqlx::query_as!(
                IssueRow,
                r#"INSERT INTO issues
                       (id, project_id, issue_number, simple_id, status_id, title, description,
                        priority, start_date, target_date, completed_at, sort_order,
                        parent_issue_id, parent_issue_sort_order, extension_metadata,
                        creator_user_id)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
                   RETURNING id                      AS "id!: Uuid",
                             project_id              AS "project_id!: Uuid",
                             issue_number            AS "issue_number!: i32",
                             simple_id               AS "simple_id!",
                             status_id               AS "status_id!: Uuid",
                             title                   AS "title!",
                             description,
                             priority,
                             start_date              AS "start_date: DateTime<Utc>",
                             target_date             AS "target_date: DateTime<Utc>",
                             completed_at            AS "completed_at: DateTime<Utc>",
                             sort_order              AS "sort_order!: f64",
                             parent_issue_id         AS "parent_issue_id: Uuid",
                             parent_issue_sort_order AS "parent_issue_sort_order: f64",
                             extension_metadata      AS "extension_metadata!",
                             creator_user_id         AS "creator_user_id: Uuid",
                             created_at              AS "created_at!: DateTime<Utc>",
                             updated_at              AS "updated_at!: DateTime<Utc>""#,
                id,
                data.project_id,
                issue_number,
                simple_id,
                data.status_id,
                title,
                description,
                priority,
                data.start_date,
                data.target_date,
                data.completed_at,
                data.sort_order,
                data.parent_issue_id,
                data.parent_issue_sort_order,
                metadata,
                super::local_project::DEFAULT_USER_ID
            )
            .fetch_one(&mut *tx)
            .await;

            match inserted {
                Ok(row) => {
                    tx.commit().await?;
                    return Ok(Issue::from(row));
                }
                Err(sqlx::Error::Database(err))
                    if err.is_unique_violation() && attempt + 1 < NUMBER_RETRY =>
                {
                    // 并发下另一个事务先拿走了这个编号，回滚后重试
                    drop(err);
                    tx.rollback().await?;
                    continue;
                }
                Err(err) => return Err(IssueError::Database(err)),
            }
        }

        Err(IssueError::Validation("分配需求编号失败，请重试".to_string()))
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> Result<Issue, IssueError> {
        let mut tx = pool.begin().await?;
        let updated = Self::update_in_tx(&mut tx, id, data).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// 拖拽排序等批量写入：单事务，任一条失败整体回滚。
    pub async fn bulk_update(
        pool: &SqlitePool,
        updates: &[(Uuid, UpdateIssueRequest)],
    ) -> Result<Vec<Issue>, IssueError> {
        let mut tx = pool.begin().await?;
        let mut rows = Vec::with_capacity(updates.len());
        for (id, data) in updates {
            rows.push(Self::update_in_tx(&mut tx, *id, data).await?);
        }
        tx.commit().await?;
        Ok(rows)
    }

    async fn update_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: Uuid,
        data: &UpdateIssueRequest,
    ) -> Result<Issue, IssueError> {
        // 每个字段配一个「是否设置」布尔量，用 CASE 在一条 UPDATE 里完成，
        // 避免先读后写的竞态：并发更新不同字段时互不覆盖。
        let set_status = data.status_id.is_some();
        let set_title = data.title.is_some();
        let title: Option<String> = match data.title.as_ref() {
            Some(raw) => {
                let trimmed = truncate(raw.trim(), MAX_TITLE_LEN);
                if trimmed.is_empty() {
                    return Err(IssueError::Validation("需求标题不能为空".to_string()));
                }
                Some(trimmed)
            }
            None => None,
        };
        let set_description = data.description.is_some();
        let description: Option<String> = data
            .description
            .clone()
            .flatten()
            .map(|d| truncate(&d, MAX_DESCRIPTION_LEN));
        let set_priority = data.priority.is_some();
        let priority = data.priority.flatten().map(priority_to_str);
        let set_start = data.start_date.is_some();
        let start_date = data.start_date.flatten();
        let set_target = data.target_date.is_some();
        let target_date = data.target_date.flatten();
        let set_completed = data.completed_at.is_some();
        let completed_at = data.completed_at.flatten();
        let set_sort = data.sort_order.is_some();
        let set_parent = data.parent_issue_id.is_some();
        let parent_issue_id = data.parent_issue_id.flatten();
        let set_parent_sort = data.parent_issue_sort_order.is_some();
        let parent_issue_sort_order = data.parent_issue_sort_order.flatten();
        let set_metadata = data.extension_metadata.is_some();
        let metadata = data.extension_metadata.as_ref().map(|v| v.to_string());

        let row = sqlx::query_as!(
            IssueRow,
            r#"UPDATE issues SET
                   status_id               = CASE WHEN $2  THEN $3  ELSE status_id END,
                   title                   = CASE WHEN $4  THEN $5  ELSE title END,
                   description             = CASE WHEN $6  THEN $7  ELSE description END,
                   priority                = CASE WHEN $8  THEN $9  ELSE priority END,
                   start_date              = CASE WHEN $10 THEN $11 ELSE start_date END,
                   target_date             = CASE WHEN $12 THEN $13 ELSE target_date END,
                   completed_at            = CASE WHEN $14 THEN $15 ELSE completed_at END,
                   sort_order              = CASE WHEN $16 THEN $17 ELSE sort_order END,
                   parent_issue_id         = CASE WHEN $18 THEN $19 ELSE parent_issue_id END,
                   parent_issue_sort_order = CASE WHEN $20 THEN $21 ELSE parent_issue_sort_order END,
                   extension_metadata      = CASE WHEN $22 THEN $23 ELSE extension_metadata END,
                   updated_at              = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id                      AS "id!: Uuid",
                         project_id              AS "project_id!: Uuid",
                         issue_number            AS "issue_number!: i32",
                         simple_id               AS "simple_id!",
                         status_id               AS "status_id!: Uuid",
                         title                   AS "title!",
                         description,
                         priority,
                         start_date              AS "start_date: DateTime<Utc>",
                         target_date             AS "target_date: DateTime<Utc>",
                         completed_at            AS "completed_at: DateTime<Utc>",
                         sort_order              AS "sort_order!: f64",
                         parent_issue_id         AS "parent_issue_id: Uuid",
                         parent_issue_sort_order AS "parent_issue_sort_order: f64",
                         extension_metadata      AS "extension_metadata!",
                         creator_user_id         AS "creator_user_id: Uuid",
                         created_at              AS "created_at!: DateTime<Utc>",
                         updated_at              AS "updated_at!: DateTime<Utc>""#,
            id,
            set_status,
            data.status_id,
            set_title,
            title,
            set_description,
            description,
            set_priority,
            priority,
            set_start,
            start_date,
            set_target,
            target_date,
            set_completed,
            completed_at,
            set_sort,
            data.sort_order,
            set_parent,
            parent_issue_id,
            set_parent_sort,
            parent_issue_sort_order,
            set_metadata,
            metadata
        )
        .fetch_one(&mut **tx)
        .await?;

        Ok(Issue::from(row))
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issues WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// 把需求移到项目里指定阶段的状态列。需求或状态列不存在时返回 None。
    pub async fn move_to_stage(
        pool: &SqlitePool,
        issue_id: Uuid,
        stage: StageType,
    ) -> Result<Option<Issue>, IssueError> {
        let Some(issue) = Self::find_by_id(pool, issue_id).await? else {
            return Ok(None);
        };
        let Some(status) = ProjectStatuses::find_stage(pool, issue.project_id, stage).await? else {
            return Ok(None);
        };

        let completed_at = if stage == StageType::Done {
            Some(Some(Utc::now()))
        } else {
            Some(None)
        };

        let updated = Self::update(
            pool,
            issue_id,
            &UpdateIssueRequest {
                status_id: Some(status.id),
                completed_at,
                ..Default::default()
            },
        )
        .await?;

        Ok(Some(updated))
    }

    pub async fn search(
        pool: &SqlitePool,
        request: &SearchIssuesRequest,
    ) -> Result<ListIssuesResponse, sqlx::Error> {
        let limit = request
            .limit
            .map(|l| (l.max(1) as usize).min(MAX_PAGE_SIZE))
            .unwrap_or(MAX_PAGE_SIZE);
        let offset = request.offset.map(|o| o.max(0) as usize).unwrap_or(0);

        let search_pattern = request
            .search
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!("%{}%", escape_like(s)));
        let has_search = search_pattern.is_some();
        let has_status = request.status_id.is_some();
        let has_priority = request.priority.is_some();
        let priority = request.priority.map(priority_to_str);
        let has_simple_id = request.simple_id.is_some();

        // 排序字段来自枚举，不来自用户输入的字符串，因此不存在 SQL 注入面。
        let ascending = !matches!(request.sort_direction, Some(SortDirection::Desc));
        let sort_field = request.sort_field.unwrap_or(IssueSortField::SortOrder);

        let total: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM issues
               WHERE project_id = ?1
                 AND (?2 = 0 OR title LIKE ?3 ESCAPE '\' OR IFNULL(description, '') LIKE ?3 ESCAPE '\')
                 AND (?4 = 0 OR status_id = ?5)
                 AND (?6 = 0 OR priority = ?7)
                 AND (?8 = 0 OR simple_id = ?9)"#,
        )
        .bind(request.project_id)
        .bind(has_search)
        .bind(search_pattern.clone())
        .bind(has_status)
        .bind(request.status_id)
        .bind(has_priority)
        .bind(priority)
        .bind(has_simple_id)
        .bind(request.simple_id.clone())
        .fetch_one(pool)
        .await?;

        let order_sql = match (sort_field, ascending) {
            (IssueSortField::SortOrder, true) => "sort_order ASC, created_at ASC",
            (IssueSortField::SortOrder, false) => "sort_order DESC, created_at DESC",
            (IssueSortField::Priority, true) => "priority ASC, created_at ASC",
            (IssueSortField::Priority, false) => "priority DESC, created_at DESC",
            (IssueSortField::CreatedAt, true) => "created_at ASC",
            (IssueSortField::CreatedAt, false) => "created_at DESC",
            (IssueSortField::UpdatedAt, true) => "updated_at ASC",
            (IssueSortField::UpdatedAt, false) => "updated_at DESC",
            (IssueSortField::Title, true) => "title ASC",
            (IssueSortField::Title, false) => "title DESC",
        };

        let sql = format!(
            r#"SELECT id, project_id, issue_number, simple_id, status_id, title, description,
                      priority, start_date, target_date, completed_at, sort_order,
                      parent_issue_id, parent_issue_sort_order, extension_metadata,
                      creator_user_id, created_at, updated_at
               FROM issues
               WHERE project_id = ?1
                 AND (?2 = 0 OR title LIKE ?3 ESCAPE '\' OR IFNULL(description, '') LIKE ?3 ESCAPE '\')
                 AND (?4 = 0 OR status_id = ?5)
                 AND (?6 = 0 OR priority = ?7)
                 AND (?8 = 0 OR simple_id = ?9)
               ORDER BY {order_sql}
               LIMIT ?10 OFFSET ?11"#
        );

        let rows = sqlx::query_as::<_, IssueSqlRow>(&sql)
            .bind(request.project_id)
            .bind(has_search)
            .bind(search_pattern)
            .bind(has_status)
            .bind(request.status_id)
            .bind(has_priority)
            .bind(priority)
            .bind(has_simple_id)
            .bind(request.simple_id.clone())
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(pool)
            .await?;

        Ok(ListIssuesResponse {
            issues: rows.into_iter().map(|row| Issue::from(row.0)).collect(),
            total_count: total.0 as usize,
            limit,
            offset,
        })
    }
}

/// 运行时（非宏）查询用的包装行。
struct IssueSqlRow(IssueRow);

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for IssueSqlRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(IssueSqlRow(IssueRow {
            id: row.try_get("id")?,
            project_id: row.try_get("project_id")?,
            issue_number: row.try_get("issue_number")?,
            simple_id: row.try_get("simple_id")?,
            status_id: row.try_get("status_id")?,
            title: row.try_get("title")?,
            description: row.try_get("description")?,
            priority: row.try_get("priority")?,
            start_date: row.try_get("start_date")?,
            target_date: row.try_get("target_date")?,
            completed_at: row.try_get("completed_at")?,
            sort_order: row.try_get("sort_order")?,
            parent_issue_id: row.try_get("parent_issue_id")?,
            parent_issue_sort_order: row.try_get("parent_issue_sort_order")?,
            extension_metadata: row.try_get("extension_metadata")?,
            creator_user_id: row.try_get("creator_user_id")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        }))
    }
}

/// 一次快照最多返回多少条需求。超过时前端应改用 /search 分页。
pub const MAX_SNAPSHOT_ROWS: i64 = 2000;
```

说明：`ISSUE_COLUMNS` 常量仅作为文档留存（`sqlx::query_as!` 宏不接受变量化的 SQL），实际查询把列表写全。若 clippy 报 `dead_code`，删除该常量。

- [ ] **步骤 5：给 UpdateIssueRequest 补 Default**

`api_types::issue::UpdateIssueRequest` 目前没有 `Default`，测试里的 `..Default::default()` 需要它。在 `crates/api-types/src/issue.rs` 的 `UpdateIssueRequest` 上补 derive（只加 `Default`，不改字段，因此不影响 `crates/remote` 与 `shared/remote-types.ts`）：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct UpdateIssueRequest {
```

同样给 `api_types::project_status::UpdateProjectStatusRequest` 加 `Default`：

```rust
#[derive(Debug, Clone, Default, Deserialize, TS)]
pub struct UpdateProjectStatusRequest {
```

- [ ] **步骤 6：给 SearchIssuesRequest 补 Default**

在 `crates/api-types/src/issue.rs` 的 `SearchIssuesRequest` 上加 `Default`（`project_id: Uuid` 的 `Default` 是全零 UUID，测试里总会显式赋值）：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct SearchIssuesRequest {
```

- [ ] **步骤 7：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p db models::issue 2>&1 | tail -25
```

预期：`test result: ok. 14 passed`。

- [ ] **步骤 8：确认生成类型未漂移**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run generate-types && git status --short shared/
```

预期：`shared/` 下无改动（只加了 `Default` derive，不影响 TS 输出）。

- [ ] **步骤 9：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/src/models/issue.rs crates/db/src/models/mod.rs \
        crates/api-types/src/issue.rs crates/api-types/src/project_status.rs crates/db/.sqlx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：需求模型，含编号并发、字段级原子更新与分页搜索" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 5：标签、需求标签与评论模型

**Files:**
- Create: `crates/db/src/models/issue_side.rs`
- Modify: `crates/db/src/models/mod.rs`
- Test: `crates/db/src/models/issue_side.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：注册模块**

在 `crates/db/src/models/mod.rs` 插入：

```rust
pub mod issue_side;
```

- [ ] **步骤 2：写失败的测试**

在 `crates/db/src/models/issue_side.rs` 写入：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        issue::CreateIssueRequest,
        issue_comment::{CreateIssueCommentRequest, UpdateIssueCommentRequest},
        issue_tag::CreateIssueTagRequest,
        project::CreateProjectRequest,
        tag::{CreateTagRequest, UpdateTagRequest},
    };
    use uuid::Uuid;

    use super::{IssueComments, IssueTags, MAX_COMMENT_LEN, ProjectTags};
    use crate::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();

        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();

        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例需求".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        (project.id, issue.id)
    }

    #[tokio::test]
    async fn 标签按项目隔离() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;

        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let same = ProjectTags::find_by_project(test_db.pool(), project_id).await.unwrap();
        assert_eq!(same.len(), 1);
        assert_eq!(same[0].id, tag.id);

        let other = ProjectTags::find_by_project(test_db.pool(), Uuid::from_u128(77))
            .await
            .unwrap();
        assert!(other.is_empty(), "不得返回其他项目的标签");
    }

    #[tokio::test]
    async fn 标签更新与删除() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let updated = ProjectTags::update(
            test_db.pool(),
            tag.id,
            &UpdateTagRequest {
                name: Some("后端".to_string()),
                color: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "后端");
        assert_eq!(updated.color, "#22c55e", "未传的字段保持原值");

        assert_eq!(ProjectTags::delete(test_db.pool(), tag.id).await.unwrap(), 1);
        assert_eq!(
            ProjectTags::delete(test_db.pool(), tag.id).await.unwrap(),
            0,
            "重复删除不得报错"
        );
    }

    #[tokio::test]
    async fn 需求标签关联按项目查询且去重() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let request = CreateIssueTagRequest {
            id: None,
            issue_id,
            tag_id: tag.id,
        };
        IssueTags::create(test_db.pool(), &request).await.unwrap();
        assert!(
            IssueTags::create(test_db.pool(), &request).await.is_err(),
            "同一需求同一标签不得重复关联"
        );

        let list = IssueTags::find_by_project(test_db.pool(), project_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].issue_id, issue_id);
    }

    #[tokio::test]
    async fn 删除需求级联删除其标签关联与评论() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag = ProjectTags::create(
            test_db.pool(),
            &CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        IssueTags::create(
            test_db.pool(),
            &CreateIssueTagRequest {
                id: None,
                issue_id,
                tag_id: tag.id,
            },
        )
        .await
        .unwrap();
        IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "评论".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        Issues::delete(test_db.pool(), issue_id).await.unwrap();

        assert!(IssueTags::find_by_project(test_db.pool(), project_id)
            .await
            .unwrap()
            .is_empty());
        assert!(IssueComments::find_by_issue(test_db.pool(), issue_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn 评论增改查删与超长截断() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let comment = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "评".repeat(MAX_COMMENT_LEN + 50),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(comment.message.chars().count(), MAX_COMMENT_LEN);
        assert_eq!(comment.author_id, Some(crate::models::local_project::DEFAULT_USER_ID));

        let updated = IssueComments::update(
            test_db.pool(),
            comment.id,
            &UpdateIssueCommentRequest {
                message: Some("改后".to_string()),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.message, "改后");
        assert!(updated.updated_at >= comment.updated_at);

        let list = IssueComments::find_by_issue(test_db.pool(), issue_id).await.unwrap();
        assert_eq!(list.len(), 1);

        assert_eq!(IssueComments::delete(test_db.pool(), comment.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn 空评论被拒绝() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;

        let result = IssueComments::create(
            test_db.pool(),
            &CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "  \n ".to_string(),
                parent_id: None,
            },
        )
        .await;
        assert!(result.is_err(), "空白评论必须拒绝");
    }
}
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db issue_side 2>&1 | tail -20
```

预期：编译失败，`cannot find type ProjectTags`。

- [ ] **步骤 4：写实现**

在测试模块之前插入：

```rust
use api_types::{
    issue_comment::{CreateIssueCommentRequest, IssueComment, UpdateIssueCommentRequest},
    issue_tag::{CreateIssueTagRequest, IssueTag},
    tag::{CreateTagRequest, Tag, UpdateTagRequest},
};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{issue::IssueError, local_project::DEFAULT_USER_ID};

/// 标签名长度上限。
pub const MAX_TAG_NAME_LEN: usize = 60;
/// 评论正文长度上限。
pub const MAX_COMMENT_LEN: usize = 20_000;
/// 单次快照返回的关联行上限。
pub const MAX_SIDE_ROWS: i64 = 5000;

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub struct ProjectTags;

impl ProjectTags {
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<Tag>, sqlx::Error> {
        sqlx::query_as!(
            Tag,
            r#"SELECT id         AS "id!: Uuid",
                      project_id AS "project_id!: Uuid",
                      name       AS "name!",
                      color      AS "color!"
               FROM project_tags
               WHERE project_id = $1
               ORDER BY name ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateTagRequest) -> Result<Tag, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        let name = truncate(data.name.trim(), MAX_TAG_NAME_LEN);

        sqlx::query_as!(
            Tag,
            r#"INSERT INTO project_tags (id, project_id, name, color)
               VALUES ($1, $2, $3, $4)
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!""#,
            id,
            data.project_id,
            name,
            data.color
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateTagRequest,
    ) -> Result<Tag, sqlx::Error> {
        let set_name = data.name.is_some();
        let name: Option<String> = data
            .name
            .as_ref()
            .map(|n| truncate(n.trim(), MAX_TAG_NAME_LEN));
        let set_color = data.color.is_some();

        sqlx::query_as!(
            Tag,
            r#"UPDATE project_tags SET
                   name  = CASE WHEN $2 THEN $3 ELSE name END,
                   color = CASE WHEN $4 THEN $5 ELSE color END
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         project_id AS "project_id!: Uuid",
                         name AS "name!",
                         color AS "color!""#,
            id,
            set_name,
            name,
            set_color,
            data.color
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM project_tags WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

pub struct IssueTags;

impl IssueTags {
    /// 按项目查：只返回该项目下需求的标签关联，跨项目不可见。
    pub async fn find_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<IssueTag>, sqlx::Error> {
        sqlx::query_as!(
            IssueTag,
            r#"SELECT it.id       AS "id!: Uuid",
                      it.issue_id AS "issue_id!: Uuid",
                      it.tag_id   AS "tag_id!: Uuid"
               FROM issue_tags it
               JOIN issues i ON i.id = it.issue_id
               WHERE i.project_id = $1
               ORDER BY it.rowid ASC
               LIMIT $2"#,
            project_id,
            MAX_SIDE_ROWS
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueTagRequest,
    ) -> Result<IssueTag, sqlx::Error> {
        let id = data.id.unwrap_or_else(Uuid::new_v4);
        sqlx::query_as!(
            IssueTag,
            r#"INSERT INTO issue_tags (id, issue_id, tag_id)
               VALUES ($1, $2, $3)
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         tag_id AS "tag_id!: Uuid""#,
            id,
            data.issue_id,
            data.tag_id
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issue_tags WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

pub struct IssueComments;

impl IssueComments {
    pub async fn find_by_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Vec<IssueComment>, sqlx::Error> {
        sqlx::query_as!(
            IssueComment,
            r#"SELECT id         AS "id!: Uuid",
                      issue_id   AS "issue_id!: Uuid",
                      author_id  AS "author_id: Uuid",
                      parent_id  AS "parent_id: Uuid",
                      message    AS "message!",
                      created_at AS "created_at!: DateTime<Utc>",
                      updated_at AS "updated_at!: DateTime<Utc>"
               FROM issue_comments
               WHERE issue_id = $1
               ORDER BY created_at ASC
               LIMIT $2"#,
            issue_id,
            MAX_SIDE_ROWS
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<IssueComment>, sqlx::Error> {
        sqlx::query_as!(
            IssueComment,
            r#"SELECT id         AS "id!: Uuid",
                      issue_id   AS "issue_id!: Uuid",
                      author_id  AS "author_id: Uuid",
                      parent_id  AS "parent_id: Uuid",
                      message    AS "message!",
                      created_at AS "created_at!: DateTime<Utc>",
                      updated_at AS "updated_at!: DateTime<Utc>"
               FROM issue_comments
               WHERE rowid = $1"#,
            rowid
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateIssueCommentRequest,
    ) -> Result<IssueComment, IssueError> {
        let message = truncate(data.message.trim(), MAX_COMMENT_LEN);
        if message.is_empty() {
            return Err(IssueError::Validation("评论内容不能为空".to_string()));
        }
        let id = data.id.unwrap_or_else(Uuid::new_v4);

        let comment = sqlx::query_as!(
            IssueComment,
            r#"INSERT INTO issue_comments (id, issue_id, author_id, parent_id, message)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         author_id AS "author_id: Uuid",
                         parent_id AS "parent_id: Uuid",
                         message AS "message!",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            data.issue_id,
            DEFAULT_USER_ID,
            data.parent_id,
            message
        )
        .fetch_one(pool)
        .await?;

        Ok(comment)
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateIssueCommentRequest,
    ) -> Result<IssueComment, IssueError> {
        let set_message = data.message.is_some();
        let message: Option<String> = match data.message.as_ref() {
            Some(raw) => {
                let trimmed = truncate(raw.trim(), MAX_COMMENT_LEN);
                if trimmed.is_empty() {
                    return Err(IssueError::Validation("评论内容不能为空".to_string()));
                }
                Some(trimmed)
            }
            None => None,
        };
        let set_parent = data.parent_id.is_some();
        let parent_id = data.parent_id.flatten();

        let comment = sqlx::query_as!(
            IssueComment,
            r#"UPDATE issue_comments SET
                   message    = CASE WHEN $2 THEN $3 ELSE message END,
                   parent_id  = CASE WHEN $4 THEN $5 ELSE parent_id END,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id AS "id!: Uuid",
                         issue_id AS "issue_id!: Uuid",
                         author_id AS "author_id: Uuid",
                         parent_id AS "parent_id: Uuid",
                         message AS "message!",
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            set_message,
            message,
            set_parent,
            parent_id
        )
        .fetch_one(pool)
        .await?;

        Ok(comment)
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM issue_comments WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
```

- [ ] **步骤 5：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p db issue_side 2>&1 | tail -20
```

预期：`test result: ok. 6 passed`。

- [ ] **步骤 6：跑一遍 db crate 全量测试**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db 2>&1 | tail -10
```

预期：全部通过。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/src/models/issue_side.rs crates/db/src/models/mod.rs crates/db/.sqlx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：标签、需求标签与评论模型" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 6：本地路由骨架与项目接口

**Files:**
- Create: `crates/server/src/routes/local_projects/mod.rs`
- Create: `crates/server/src/routes/local_projects/projects.rs`
- Modify: `crates/server/src/routes/mod.rs`
- Test: `crates/server/src/routes/local_projects/projects.rs`（模块内 `#[cfg(test)]`）

**约定：** 每个接口拆成两层——`handle_*(pool, ...)` 纯逻辑函数（可直接用 `TestDb` 测试），与三行的 axum 包装函数。测试只测 `handle_*`。

- [ ] **步骤 1：写骨架模块**

创建 `crates/server/src/routes/local_projects/mod.rs`：

```rust
//! 个人版本地需求接口。路径与云端 /v1 同构，挂在 /api/local 下。
//! 快照统一返回 { "<前端表名>": [ ...行 ] }，写操作统一返回 { "txid": 0 }
//! （个人版不使用 ElectricSQL 事务对账，txid 只是占位）。

pub mod projects;

use axum::{Json, Router};
use db::models::issue::IssueError;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Serialize)]
pub struct TxidResponse {
    pub txid: i64,
}

/// 写操作的统一响应。
pub fn txid() -> Json<TxidResponse> {
    Json(TxidResponse { txid: 0 })
}

/// 快照响应：key 必须与前端 ShapeDefinition.table 完全一致。
pub fn snapshot<T: Serialize>(table: &str, rows: Vec<T>) -> Json<Value> {
    Json(json!({ table: rows }))
}

/// 把模型层的校验错误映射成 400，其余映射成数据库错误。
pub fn map_issue_error(error: IssueError) -> ApiError {
    match error {
        IssueError::Validation(message) => ApiError::BadRequest(message),
        IssueError::Database(err) => ApiError::Database(err),
    }
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest("/local", projects::router())
}
```

- [ ] **步骤 2：写失败的测试**

创建 `crates/server/src/routes/local_projects/projects.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use api_types::project::{CreateProjectRequest, UpdateProjectRequest};
    use db::{models::local_project::DEFAULT_ORGANIZATION_ID, test_support::TestDb};
    use uuid::Uuid;

    use super::{handle_create, handle_delete, handle_list, handle_update};

    fn 建项目请求(name: &str) -> CreateProjectRequest {
        CreateProjectRequest {
            id: None,
            organization_id: DEFAULT_ORGANIZATION_ID,
            name: name.to_string(),
            color: "#6366f1".to_string(),
        }
    }

    #[tokio::test]
    async fn 列表响应使用_projects_作为_key() {
        let test_db = TestDb::new().await;
        handle_create(test_db.pool(), 建项目请求("A")).await.unwrap();

        let body = handle_list(test_db.pool()).await.unwrap().0;
        let rows = body
            .get("projects")
            .and_then(|v| v.as_array())
            .expect("响应必须有 projects 数组");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "A");
        assert_eq!(
            rows[0]["organization_id"],
            DEFAULT_ORGANIZATION_ID.to_string()
        );
    }

    #[tokio::test]
    async fn 空库返回空数组而不是_null() {
        let test_db = TestDb::new().await;
        let body = handle_list(test_db.pool()).await.unwrap().0;
        assert_eq!(body["projects"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn 写操作返回_txid_零() {
        let test_db = TestDb::new().await;
        let response = handle_create(test_db.pool(), 建项目请求("A")).await.unwrap();
        assert_eq!(response.0.txid, 0);
    }

    #[tokio::test]
    async fn 更新不存在的项目返回_404() {
        let test_db = TestDb::new().await;
        let result = handle_update(
            test_db.pool(),
            Uuid::from_u128(4242),
            UpdateProjectRequest {
                name: Some("X".to_string()),
                color: None,
                sort_order: None,
            },
        )
        .await;

        assert!(matches!(result, Err(crate::error::ApiError::NotFound)));
    }

    #[tokio::test]
    async fn 删除不存在的项目返回_404_而不是静默成功() {
        let test_db = TestDb::new().await;
        let result = handle_delete(test_db.pool(), Uuid::from_u128(4242)).await;
        assert!(matches!(result, Err(crate::error::ApiError::NotFound)));
    }

    #[tokio::test]
    async fn 删除只影响目标项目() {
        let test_db = TestDb::new().await;
        handle_create(test_db.pool(), 建项目请求("留下")).await.unwrap();
        let body = handle_list(test_db.pool()).await.unwrap().0;
        let keep_id: Uuid = body["projects"][0]["id"].as_str().unwrap().parse().unwrap();

        handle_create(test_db.pool(), 建项目请求("删掉")).await.unwrap();
        let body = handle_list(test_db.pool()).await.unwrap().0;
        let victim_id: Uuid = body["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "删掉")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        handle_delete(test_db.pool(), victim_id).await.unwrap();

        let after = handle_list(test_db.pool()).await.unwrap().0;
        let names: Vec<_> = after["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["留下".to_string()]);
        assert_ne!(keep_id, victim_id);
    }

    #[tokio::test]
    async fn 名称里的单引号不会破坏_sql() {
        let test_db = TestDb::new().await;
        handle_create(test_db.pool(), 建项目请求("O'Brien'); DROP TABLE issues;--"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool()).await.unwrap().0;
        assert_eq!(body["projects"][0]["name"], "O'Brien'); DROP TABLE issues;--");

        // issues 表必须还在
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
            .fetch_one(test_db.pool())
            .await
            .expect("issues 表必须还存在");
        assert_eq!(count.0, 0);
    }
}
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p server local_projects 2>&1 | tail -20
```

预期：编译失败，`cannot find function handle_list`。

- [ ] **步骤 4：写实现**

在 `crates/server/src/routes/local_projects/projects.rs` 的测试模块之前插入：

```rust
use api_types::project::{CreateProjectRequest, UpdateProjectRequest};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use db::models::local_project::LocalProjects;
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{TxidResponse, snapshot, txid};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(pool: &SqlitePool) -> Result<Json<Value>, ApiError> {
    let projects = LocalProjects::find_all(pool).await?;
    Ok(snapshot("projects", projects))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateProjectRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.name.trim().is_empty() {
        return Err(ApiError::BadRequest("项目名称不能为空".to_string()));
    }
    LocalProjects::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateProjectRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    LocalProjects::update(pool, id, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let affected = LocalProjects::delete(pool, id).await?;
    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

async fn list_projects(State(deployment): State<DeploymentImpl>) -> Result<Json<Value>, ApiError> {
    handle_list(&deployment.db().pool).await
}

async fn create_project(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update_project(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProjectRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn delete_project(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/projects",
        Router::new()
            .route("/", get(list_projects).post(create_project))
            .route("/{id}", axum::routing::patch(update_project).delete(delete_project)),
    )
}
```

说明：`Path(id): Path<Uuid>` 由 axum 负责解析，非法 UUID 直接 400，因此路径参数不可能携带 `../` 之类的穿越片段。

- [ ] **步骤 5：确认 ApiError 有 NotFound 变体**

```bash
cd /Users/admin/work/github/vibe-kanban && grep -n "NotFound" crates/server/src/error.rs | head
```

预期：能看到 `NotFound` 变体。若没有，在 `ApiError` 上补：

```rust
    #[error("Not found")]
    NotFound,
```

并在 `IntoResponse` 的 `match` 中按与 `ApiError::BadRequest` 同样的写法映射到 `StatusCode::NOT_FOUND`。

- [ ] **步骤 6：挂载路由**

在 `crates/server/src/routes/mod.rs` 的模块声明区加：

```rust
pub mod local_projects;
```

在 `relay_signed_routes` 的 `.merge(...)` 链中，`.merge(tags::router(&deployment))` 之后加一行：

```rust
        .merge(local_projects::router())
```

- [ ] **步骤 7：运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p server local_projects 2>&1 | tail -20
```

预期：`test result: ok. 7 passed`。

- [ ] **步骤 8：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/server/src/routes/local_projects crates/server/src/routes/mod.rs crates/server/src/error.rs
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：/api/local 路由骨架与项目接口" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 7：状态列与需求接口

**Files:**
- Create: `crates/server/src/routes/local_projects/statuses.rs`
- Create: `crates/server/src/routes/local_projects/issues.rs`
- Modify: `crates/server/src/routes/local_projects/mod.rs`
- Test: 两个文件各自的 `#[cfg(test)]` 模块

- [ ] **步骤 1：在 mod.rs 注册并挂载**

`crates/server/src/routes/local_projects/mod.rs`：

```rust
pub mod issues;
pub mod projects;
pub mod statuses;
```

并把 `router()` 改成：

```rust
pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/local",
        Router::new()
            .merge(projects::router())
            .merge(statuses::router())
            .merge(issues::router()),
    )
}
```

同时新增两个共享的查询结构体（放在 `mod.rs` 末尾）：

```rust
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ProjectScopedQuery {
    pub project_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IssueScopedQuery {
    pub issue_id: Uuid,
}

/// 批量更新请求体：{"updates": [{"id": "...", ...变更字段}]}
#[derive(Debug, Deserialize)]
pub struct BulkUpdateItem<T> {
    pub id: Uuid,
    #[serde(flatten)]
    pub changes: T,
}

#[derive(Debug, Deserialize)]
pub struct BulkUpdateRequest<T> {
    pub updates: Vec<BulkUpdateItem<T>>,
}

/// 一次批量更新的条数上限，防止前端误发超大请求打满事务。
pub const MAX_BULK_UPDATES: usize = 1000;
```

- [ ] **步骤 2：写状态列失败测试**

创建 `crates/server/src/routes/local_projects/statuses.rs`：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        project::CreateProjectRequest,
        project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest},
    };
    use db::{
        models::local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{handle_bulk_update, handle_create, handle_delete, handle_list};
    use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest};

    async fn 建项目(test_db: &TestDb) -> Uuid {
        LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn 列表用_project_statuses_作为_key_并只返回本项目() {
        let test_db = TestDb::new().await;
        let a = 建项目(&test_db).await;
        let b = 建项目(&test_db).await;

        let body = handle_list(test_db.pool(), a).await.unwrap().0;
        let rows = body["project_statuses"].as_array().unwrap();

        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r["project_id"] == a.to_string()));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn 快照包含_stage_type_字段() {
        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db).await;
        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(body["project_statuses"][2]["stage_type"], "dev");
    }

    #[tokio::test]
    async fn 新建状态列必须属于已存在的项目() {
        let test_db = TestDb::new().await;
        let result = handle_create(
            test_db.pool(),
            CreateProjectStatusRequest {
                id: None,
                project_id: Uuid::from_u128(9090),
                name: "野状态".to_string(),
                color: "#000000".to_string(),
                sort_order: 0,
                hidden: false,
            },
        )
        .await;
        assert!(result.is_err(), "项目不存在时必须拒绝");
    }

    #[tokio::test]
    async fn 批量更新超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let updates = (0..super::super::MAX_BULK_UPDATES + 1)
            .map(|_| BulkUpdateItem {
                id: Uuid::new_v4(),
                changes: UpdateProjectStatusRequest::default(),
            })
            .collect();

        let result = handle_bulk_update(test_db.pool(), BulkUpdateRequest { updates }).await;
        assert!(result.is_err(), "超过上限必须返回错误而不是进事务");
    }

    #[tokio::test]
    async fn 删除仍有需求引用的状态列被拒绝() {
        use api_types::issue::CreateIssueRequest;
        use db::models::{
            issue::Issues,
            local_project_status::{ProjectStatuses, StageType},
        };

        let test_db = TestDb::new().await;
        let project_id = 建项目(&test_db).await;
        let todo = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();

        Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id,
                status_id: todo.id,
                title: "占用".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        let result = handle_delete(test_db.pool(), todo.id).await;
        assert!(result.is_err(), "有需求引用的状态列不得删除");
    }
}
```

- [ ] **步骤 3：写状态列实现**

在测试模块之前插入：

```rust
use api_types::project_status::{CreateProjectStatusRequest, UpdateProjectStatusRequest};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use db::models::{local_project::LocalProjects, local_project_status::ProjectStatuses};
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, MAX_BULK_UPDATES, ProjectScopedQuery, TxidResponse, snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let statuses = ProjectStatuses::find_by_project(pool, project_id).await?;
    Ok(snapshot("project_statuses", statuses))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateProjectStatusRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, payload.project_id).await?.is_none() {
        return Err(ApiError::BadRequest("项目不存在".to_string()));
    }
    ProjectStatuses::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateProjectStatusRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if ProjectStatuses::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    ProjectStatuses::update(pool, id, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_bulk_update(
    pool: &SqlitePool,
    payload: BulkUpdateRequest<UpdateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.updates.len() > MAX_BULK_UPDATES {
        return Err(ApiError::BadRequest(format!(
            "单次最多更新 {MAX_BULK_UPDATES} 条状态列"
        )));
    }
    let updates: Vec<_> = payload
        .updates
        .into_iter()
        .map(|item| (item.id, item.changes))
        .collect();
    ProjectStatuses::bulk_update(pool, &updates).await?;
    Ok(txid())
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let in_use: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status_id = ?1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if in_use.0 > 0 {
        return Err(ApiError::Conflict(
            "该状态列下仍有需求，请先移走再删除".to_string(),
        ));
    }

    let affected = ProjectStatuses::delete(pool, id).await?;
    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

async fn list(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list(&deployment.db().pool, query.project_id).await
}

async fn create(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProjectStatusRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn bulk_update(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<BulkUpdateRequest<UpdateProjectStatusRequest>>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_bulk_update(&deployment.db().pool, payload).await
}

async fn delete(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/project_statuses",
        Router::new()
            .route("/", get(list).post(create))
            .route("/bulk", post(bulk_update))
            .route("/{id}", patch(update).delete(delete)),
    )
}
```

- [ ] **步骤 4：写需求接口失败测试**

创建 `crates/server/src/routes/local_projects/issues.rs`：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        issue::{CreateIssueRequest, SearchIssuesRequest, UpdateIssueRequest},
        project::CreateProjectRequest,
    };
    use db::{
        models::{
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{handle_bulk_update, handle_create, handle_delete, handle_list, handle_search};
    use crate::routes::local_projects::{BulkUpdateItem, BulkUpdateRequest};

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        (project.id, todo.id)
    }

    fn 建需求请求(project_id: Uuid, status_id: Uuid, title: &str) -> CreateIssueRequest {
        CreateIssueRequest {
            id: None,
            project_id,
            status_id,
            title: title.to_string(),
            description: None,
            priority: None,
            start_date: None,
            target_date: None,
            completed_at: None,
            sort_order: 0.0,
            parent_issue_id: None,
            parent_issue_sort_order: None,
            extension_metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn 列表用_issues_作为_key() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "第一条"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["simple_id"], "VK-1");
        assert_eq!(rows[0]["title"], "第一条");
    }

    #[tokio::test]
    async fn 快照不包含任何本地文件路径字段() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "第一条"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let text = body.to_string();
        for leak in ["container_ref", "worktree", "/Users/", "/home/", "asset_dir"] {
            assert!(!text.contains(leak), "需求快照不得泄露 {leak}");
        }
    }

    #[tokio::test]
    async fn 跨项目状态列被拒绝() {
        let test_db = TestDb::new().await;
        let (project_a, _) = 准备(&test_db).await;
        let (_, status_b) = 准备(&test_db).await;

        let result = handle_create(
            test_db.pool(),
            建需求请求(project_a, status_b, "越权"),
        )
        .await;
        assert!(result.is_err(), "状态列不属于该项目时必须拒绝");
    }

    #[tokio::test]
    async fn 批量更新排序成功后按新顺序返回() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "A"))
            .await
            .unwrap();
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "B"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let rows = body["issues"].as_array().unwrap().clone();
        let a_id: Uuid = rows
            .iter()
            .find(|r| r["title"] == "A")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        handle_bulk_update(
            test_db.pool(),
            BulkUpdateRequest {
                updates: vec![BulkUpdateItem {
                    id: a_id,
                    changes: UpdateIssueRequest {
                        sort_order: Some(99.0),
                        ..Default::default()
                    },
                }],
            },
        )
        .await
        .unwrap();

        let after = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(after["issues"][1]["title"], "A", "sort_order 大的排后面");
    }

    #[tokio::test]
    async fn 批量更新超过上限被拒绝() {
        let test_db = TestDb::new().await;
        let updates = (0..super::super::MAX_BULK_UPDATES + 1)
            .map(|_| BulkUpdateItem {
                id: Uuid::new_v4(),
                changes: UpdateIssueRequest::default(),
            })
            .collect();

        let result = handle_bulk_update(test_db.pool(), BulkUpdateRequest { updates }).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn 搜索接口返回列表与总数() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        for index in 0..3 {
            handle_create(
                test_db.pool(),
                建需求请求(project_id, status_id, &format!("需求 {index}")),
            )
            .await
            .unwrap();
        }

        let body = handle_search(
            test_db.pool(),
            SearchIssuesRequest {
                project_id,
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;

        assert_eq!(body.issues.len(), 2);
        assert_eq!(body.total_count, 3);
    }

    #[tokio::test]
    async fn 删除需求只影响目标行() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "留"))
            .await
            .unwrap();
        handle_create(test_db.pool(), 建需求请求(project_id, status_id, "删"))
            .await
            .unwrap();

        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let victim: Uuid = body["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["title"] == "删")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        handle_delete(test_db.pool(), victim).await.unwrap();

        let after = handle_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(after["issues"].as_array().unwrap().len(), 1);
        assert_eq!(after["issues"][0]["title"], "留");
    }

    #[tokio::test]
    async fn 一千条需求的快照能在合理时间内返回() {
        let test_db = TestDb::new().await;
        let (project_id, status_id) = 准备(&test_db).await;

        for index in 0..1000 {
            sqlx::query(
                "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title, \
                 sort_order, extension_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            )
            .bind(Uuid::new_v4())
            .bind(project_id)
            .bind(index + 1)
            .bind(format!("VK-{}", index + 1))
            .bind(status_id)
            .bind(format!("批量需求 {index}"))
            .bind(index as f64)
            .execute(test_db.pool())
            .await
            .unwrap();
        }

        let start = std::time::Instant::now();
        let body = handle_list(test_db.pool(), project_id).await.unwrap().0;
        let elapsed = start.elapsed();

        assert_eq!(body["issues"].as_array().unwrap().len(), 1000);
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "1000 条快照耗时 {elapsed:?}，超过 2 秒上限"
        );
    }
}
```

- [ ] **步骤 5：写需求接口实现**

在测试模块之前插入：

```rust
use api_types::issue::{
    CreateIssueRequest, ListIssuesResponse, SearchIssuesRequest, UpdateIssueRequest,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use db::models::issue::Issues;
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    BulkUpdateRequest, MAX_BULK_UPDATES, ProjectScopedQuery, TxidResponse, map_issue_error,
    snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    let issues = Issues::find_by_project(pool, project_id).await?;
    Ok(snapshot("issues", issues))
}

pub(crate) async fn handle_get(pool: &SqlitePool, id: Uuid) -> Result<Json<Value>, ApiError> {
    let issue = Issues::find_by_id(pool, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(serde_json::to_value(issue).unwrap_or(Value::Null)))
}

pub(crate) async fn handle_create(
    pool: &SqlitePool,
    payload: CreateIssueRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    Issues::create(pool, &payload).await.map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateIssueRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    Issues::update(pool, id, &payload).await.map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_bulk_update(
    pool: &SqlitePool,
    payload: BulkUpdateRequest<UpdateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    if payload.updates.len() > MAX_BULK_UPDATES {
        return Err(ApiError::BadRequest(format!(
            "单次最多更新 {MAX_BULK_UPDATES} 条需求"
        )));
    }
    let updates: Vec<_> = payload
        .updates
        .into_iter()
        .map(|item| (item.id, item.changes))
        .collect();
    Issues::bulk_update(pool, &updates).await.map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_search(
    pool: &SqlitePool,
    payload: SearchIssuesRequest,
) -> Result<Json<ListIssuesResponse>, ApiError> {
    Ok(Json(Issues::search(pool, &payload).await?))
}

pub(crate) async fn handle_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    let affected = Issues::delete(pool, id).await?;
    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

async fn list(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list(&deployment.db().pool, query.project_id).await
}

async fn get_one(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    handle_get(&deployment.db().pool, id).await
}

async fn create(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_create(&deployment.db().pool, payload).await
}

async fn update(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateIssueRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_update(&deployment.db().pool, id, payload).await
}

async fn bulk_update(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<BulkUpdateRequest<UpdateIssueRequest>>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_bulk_update(&deployment.db().pool, payload).await
}

async fn search(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<SearchIssuesRequest>,
) -> Result<Json<ListIssuesResponse>, ApiError> {
    handle_search(&deployment.db().pool, payload).await
}

async fn delete(
    State(deployment): State<DeploymentImpl>,
    Path(id): Path<Uuid>,
) -> Result<Json<TxidResponse>, ApiError> {
    handle_delete(&deployment.db().pool, id).await
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/issues",
        Router::new()
            .route("/", get(list).post(create))
            .route("/bulk", post(bulk_update))
            .route("/search", post(search))
            .route("/{id}", get(get_one).patch(update).delete(delete)),
    )
}
```

注意：`/bulk` 与 `/search` 必须注册在 `/{id}` 之前的同一 Router 里，axum 0.8 的路由匹配会优先静态段，不会把 `bulk` 当作 `{id}`。

- [ ] **步骤 6：运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p server local_projects 2>&1 | tail -25
```

预期：`test result: ok. 20 passed`（项目 7 + 状态列 5 + 需求 8）。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/server/src/routes/local_projects
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：状态列与需求接口，含批量事务与分页搜索" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 8：标签、评论、投影与空集合接口

**Files:**
- Create: `crates/server/src/routes/local_projects/side.rs`
- Create: `crates/server/src/routes/local_projects/projections.rs`
- Modify: `crates/server/src/routes/local_projects/mod.rs`
- Test: 两个文件各自的 `#[cfg(test)]` 模块

- [ ] **步骤 1：在 mod.rs 注册并挂载**

```rust
pub mod issues;
pub mod projections;
pub mod projects;
pub mod side;
pub mod statuses;
```

```rust
pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/local",
        Router::new()
            .merge(projects::router())
            .merge(statuses::router())
            .merge(issues::router())
            .merge(side::router())
            .merge(projections::router()),
    )
}
```

- [ ] **步骤 2：写 side.rs 失败测试**

创建 `crates/server/src/routes/local_projects/side.rs`：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        issue::CreateIssueRequest, issue_comment::CreateIssueCommentRequest,
        issue_tag::CreateIssueTagRequest, project::CreateProjectRequest, tag::CreateTagRequest,
    };
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{
        handle_comment_create, handle_comment_list, handle_issue_tag_create,
        handle_issue_tag_list, handle_tag_create, handle_tag_list,
    };

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        (project.id, issue.id)
    }

    #[tokio::test]
    async fn 标签列表的_key_是_tags_而不是数据库表名() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();

        let body = handle_tag_list(test_db.pool(), project_id).await.unwrap().0;
        assert!(body.get("tags").is_some(), "响应 key 必须是 tags");
        assert!(body.get("project_tags").is_none());
        assert_eq!(body["tags"][0]["name"], "前端");
    }

    #[tokio::test]
    async fn 需求标签列表的_key_是_issue_tags() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id) = 准备(&test_db).await;
        let tag_body = {
            handle_tag_create(
                test_db.pool(),
                CreateTagRequest {
                    id: None,
                    project_id,
                    name: "前端".to_string(),
                    color: "#22c55e".to_string(),
                },
            )
            .await
            .unwrap();
            handle_tag_list(test_db.pool(), project_id).await.unwrap().0
        };
        let tag_id: Uuid = tag_body["tags"][0]["id"].as_str().unwrap().parse().unwrap();

        handle_issue_tag_create(
            test_db.pool(),
            CreateIssueTagRequest {
                id: None,
                issue_id,
                tag_id,
            },
        )
        .await
        .unwrap();

        let body = handle_issue_tag_list(test_db.pool(), project_id).await.unwrap().0;
        assert_eq!(body["issue_tags"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn 给不存在的需求加标签被拒绝() {
        let test_db = TestDb::new().await;
        let (project_id, _) = 准备(&test_db).await;
        handle_tag_create(
            test_db.pool(),
            CreateTagRequest {
                id: None,
                project_id,
                name: "前端".to_string(),
                color: "#22c55e".to_string(),
            },
        )
        .await
        .unwrap();
        let body = handle_tag_list(test_db.pool(), project_id).await.unwrap().0;
        let tag_id: Uuid = body["tags"][0]["id"].as_str().unwrap().parse().unwrap();

        let result = handle_issue_tag_create(
            test_db.pool(),
            CreateIssueTagRequest {
                id: None,
                issue_id: Uuid::from_u128(31337),
                tag_id,
            },
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn 评论列表按需求过滤() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;
        handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "第一条评论".to_string(),
                parent_id: None,
            },
        )
        .await
        .unwrap();

        let body = handle_comment_list(test_db.pool(), issue_id).await.unwrap().0;
        assert_eq!(body["issue_comments"].as_array().unwrap().len(), 1);

        let other = handle_comment_list(test_db.pool(), Uuid::from_u128(4242))
            .await
            .unwrap()
            .0;
        assert_eq!(other["issue_comments"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn 空评论被接口拒绝() {
        let test_db = TestDb::new().await;
        let (_, issue_id) = 准备(&test_db).await;
        let result = handle_comment_create(
            test_db.pool(),
            CreateIssueCommentRequest {
                id: None,
                issue_id,
                message: "   ".to_string(),
                parent_id: None,
            },
        )
        .await;
        assert!(result.is_err());
    }
}
```

- [ ] **步骤 3：写 side.rs 实现**

在测试模块之前插入：

```rust
use api_types::{
    issue_comment::{CreateIssueCommentRequest, UpdateIssueCommentRequest},
    issue_tag::CreateIssueTagRequest,
    tag::{CreateTagRequest, UpdateTagRequest},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch},
};
use db::models::{
    issue::Issues,
    issue_side::{IssueComments, IssueTags, ProjectTags},
    local_project::LocalProjects,
};
use deployment::Deployment;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    IssueScopedQuery, ProjectScopedQuery, TxidResponse, map_issue_error, snapshot, txid,
};
use crate::{DeploymentImpl, error::ApiError};

pub(crate) async fn handle_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "tags",
        ProjectTags::find_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_tag_create(
    pool: &SqlitePool,
    payload: CreateTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if LocalProjects::find_by_id(pool, payload.project_id).await?.is_none() {
        return Err(ApiError::BadRequest("项目不存在".to_string()));
    }
    ProjectTags::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_tag_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    ProjectTags::update(pool, id, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if ProjectTags::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_list(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "issue_tags",
        IssueTags::find_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_issue_tag_create(
    pool: &SqlitePool,
    payload: CreateIssueTagRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, payload.issue_id).await?.is_none() {
        return Err(ApiError::BadRequest("需求不存在".to_string()));
    }
    IssueTags::create(pool, &payload).await?;
    Ok(txid())
}

pub(crate) async fn handle_issue_tag_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if IssueTags::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub(crate) async fn handle_comment_list(
    pool: &SqlitePool,
    issue_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "issue_comments",
        IssueComments::find_by_issue(pool, issue_id).await?,
    ))
}

pub(crate) async fn handle_comment_create(
    pool: &SqlitePool,
    payload: CreateIssueCommentRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    if Issues::find_by_id(pool, payload.issue_id).await?.is_none() {
        return Err(ApiError::BadRequest("需求不存在".to_string()));
    }
    IssueComments::create(pool, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_comment_update(
    pool: &SqlitePool,
    id: Uuid,
    payload: UpdateIssueCommentRequest,
) -> Result<Json<TxidResponse>, ApiError> {
    IssueComments::update(pool, id, &payload)
        .await
        .map_err(map_issue_error)?;
    Ok(txid())
}

pub(crate) async fn handle_comment_delete(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<Json<TxidResponse>, ApiError> {
    if IssueComments::delete(pool, id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(txid())
}

pub fn router() -> Router<DeploymentImpl> {
    let tags = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_tag_list(&d.db().pool, q.project_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>, Json(p): Json<CreateTagRequest>| async move {
                    handle_tag_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            patch(
                |State(d): State<DeploymentImpl>,
                 Path(id): Path<Uuid>,
                 Json(p): Json<UpdateTagRequest>| async move {
                    handle_tag_update(&d.db().pool, id, p).await
                },
            )
            .delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_tag_delete(&d.db().pool, id).await
                },
            ),
        );

    let issue_tags = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_issue_tag_list(&d.db().pool, q.project_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>, Json(p): Json<CreateIssueTagRequest>| async move {
                    handle_issue_tag_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            axum::routing::delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_issue_tag_delete(&d.db().pool, id).await
                },
            ),
        );

    let comments = Router::new()
        .route(
            "/",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<IssueScopedQuery>| async move {
                    handle_comment_list(&d.db().pool, q.issue_id).await
                },
            )
            .post(
                |State(d): State<DeploymentImpl>,
                 Json(p): Json<CreateIssueCommentRequest>| async move {
                    handle_comment_create(&d.db().pool, p).await
                },
            ),
        )
        .route(
            "/{id}",
            patch(
                |State(d): State<DeploymentImpl>,
                 Path(id): Path<Uuid>,
                 Json(p): Json<UpdateIssueCommentRequest>| async move {
                    handle_comment_update(&d.db().pool, id, p).await
                },
            )
            .delete(
                |State(d): State<DeploymentImpl>, Path(id): Path<Uuid>| async move {
                    handle_comment_delete(&d.db().pool, id).await
                },
            ),
        );

    Router::new()
        .nest("/tags", tags)
        .nest("/issue_tags", issue_tags)
        .nest("/issue_comments", comments)
}
```

- [ ] **步骤 4：写 projections.rs 失败测试**

创建 `crates/server/src/routes/local_projects/projections.rs`：

```rust
#[cfg(test)]
mod tests {
    use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{EMPTY_TABLES, handle_empty, handle_pull_requests, handle_workspaces};

    #[tokio::test]
    async fn 空集合按表名返回空数组() {
        let test_db = TestDb::new().await;
        for table in EMPTY_TABLES {
            let body = handle_empty(table).0;
            assert_eq!(
                body[*table],
                serde_json::json!([]),
                "{table} 必须返回空数组"
            );
        }
        // 确保没有误用数据库
        assert!(test_db.pool().is_closed() == false);
    }

    #[tokio::test]
    async fn 工作区投影只返回绑定本项目需求的工作区且字段与云端同构() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        let linked = Uuid::new_v4();
        let orphan = Uuid::new_v4();
        for (id, issue_id) in [(linked, Some(issue.id)), (orphan, None)] {
            sqlx::query(
                "INSERT INTO workspaces (id, branch, name, issue_id) VALUES (?1, 'vk/x', 'W', ?2)",
            )
            .bind(id)
            .bind(issue_id)
            .execute(test_db.pool())
            .await
            .unwrap();
        }

        let body = handle_workspaces(test_db.pool(), project.id).await.unwrap().0;
        let rows = body["workspaces"].as_array().unwrap();

        assert_eq!(rows.len(), 1, "未绑定需求的工作区不出现在项目投影里");
        assert_eq!(rows[0]["id"], linked.to_string());
        assert_eq!(rows[0]["project_id"], project.id.to_string());
        assert_eq!(rows[0]["issue_id"], issue.id.to_string());
        assert_eq!(rows[0]["owner_user_id"], DEFAULT_USER_ID.to_string());
        assert_eq!(rows[0]["local_workspace_id"], linked.to_string());
        assert_ne!(linked, orphan);
    }

    #[tokio::test]
    async fn 工作区投影不泄露本地路径() {
        let test_db = TestDb::new().await;
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO workspaces (id, branch, container_ref, issue_id) \
             VALUES (?1, 'vk/x', '/Users/secret/worktrees/abc', ?2)",
        )
        .bind(Uuid::new_v4())
        .bind(issue.id)
        .execute(test_db.pool())
        .await
        .unwrap();

        let body = handle_workspaces(test_db.pool(), project.id).await.unwrap().0;
        let text = body.to_string();
        assert!(!text.contains("/Users/secret"), "投影不得包含 container_ref");
        assert!(!text.contains("container_ref"));
    }

    #[tokio::test]
    async fn 拉取请求投影按需求反查() {
        let test_db = TestDb::new().await;
        let body = handle_pull_requests(test_db.pool(), Uuid::from_u128(1))
            .await
            .unwrap()
            .0;
        assert_eq!(body["pull_requests"], serde_json::json!([]));
    }
}
```

- [ ] **步骤 5：写 projections.rs 实现**

在测试模块之前插入：

```rust
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use db::models::local_project::DEFAULT_USER_ID;
use deployment::Deployment;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{ProjectScopedQuery, snapshot};
use crate::{DeploymentImpl, error::ApiError};

/// 个人版不实现的集合：返回空数组，保证前端订阅这些 shape 时不报错。
/// 数组内容必须与 shared/remote-types.ts 中各 ShapeDefinition 的 table 完全一致。
pub const EMPTY_TABLES: &[&str] = &[
    "issue_assignees",
    "issue_followers",
    "issue_relationships",
    "issue_comment_reactions",
    "pull_request_issues",
    "notifications",
    "users",
    "organization_member_metadata",
];

/// 云端 Workspace 行结构的本地投影。字段与 api_types::workspace::Workspace 一致。
#[derive(Debug, Serialize)]
struct ProjectedWorkspace {
    id: Uuid,
    project_id: Uuid,
    owner_user_id: Uuid,
    issue_id: Option<Uuid>,
    local_workspace_id: Option<Uuid>,
    name: Option<String>,
    archived: bool,
    files_changed: Option<i32>,
    lines_added: Option<i32>,
    lines_removed: Option<i32>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// 云端 PullRequest 行结构的本地投影。
#[derive(Debug, Serialize)]
struct ProjectedPullRequest {
    id: String,
    url: String,
    number: i32,
    status: String,
    merged_at: Option<DateTime<Utc>>,
    merge_commit_sha: Option<String>,
    target_branch_name: String,
    project_id: Uuid,
    issue_id: Option<Uuid>,
    workspace_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) fn handle_empty(table: &str) -> Json<Value> {
    Json(json!({ table: Vec::<Value>::new() }))
}

pub(crate) async fn handle_workspaces(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    struct Row {
        id: Uuid,
        issue_id: Option<Uuid>,
        name: Option<String>,
        archived: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    let rows = sqlx::query_as!(
        Row,
        r#"SELECT w.id         AS "id!: Uuid",
                  w.issue_id   AS "issue_id: Uuid",
                  w.name,
                  w.archived   AS "archived!: bool",
                  w.created_at AS "created_at!: DateTime<Utc>",
                  w.updated_at AS "updated_at!: DateTime<Utc>"
           FROM workspaces w
           JOIN issues i ON i.id = w.issue_id
           WHERE i.project_id = $1
           ORDER BY w.created_at DESC"#,
        project_id
    )
    .fetch_all(pool)
    .await?;

    // 只投影云端 Workspace 行结构中的字段，container_ref 等本地路径一律不输出。
    let projected: Vec<ProjectedWorkspace> = rows
        .into_iter()
        .map(|row| ProjectedWorkspace {
            id: row.id,
            project_id,
            owner_user_id: DEFAULT_USER_ID,
            issue_id: row.issue_id,
            local_workspace_id: Some(row.id),
            name: row.name,
            archived: row.archived,
            files_changed: None,
            lines_added: None,
            lines_removed: None,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(snapshot("workspaces", projected))
}

pub(crate) async fn handle_pull_requests(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    struct Row {
        id: String,
        pr_url: String,
        pr_number: i64,
        pr_status: String,
        merged_at: Option<DateTime<Utc>>,
        merge_commit_sha: Option<String>,
        target_branch_name: String,
        workspace_id: Option<Uuid>,
        issue_id: Option<Uuid>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    let rows = sqlx::query_as!(
        Row,
        r#"SELECT pr.id                 AS "id!",
                  pr.pr_url             AS "pr_url!",
                  pr.pr_number          AS "pr_number!: i64",
                  pr.pr_status          AS "pr_status!",
                  pr.merged_at          AS "merged_at: DateTime<Utc>",
                  pr.merge_commit_sha,
                  pr.target_branch_name AS "target_branch_name!",
                  pr.workspace_id       AS "workspace_id: Uuid",
                  w.issue_id            AS "issue_id: Uuid",
                  pr.created_at         AS "created_at!: DateTime<Utc>",
                  pr.updated_at         AS "updated_at!: DateTime<Utc>"
           FROM pull_requests pr
           JOIN workspaces w ON w.id = pr.workspace_id
           JOIN issues i ON i.id = w.issue_id
           WHERE i.project_id = $1
           ORDER BY pr.created_at DESC"#,
        project_id
    )
    .fetch_all(pool)
    .await?;

    let projected: Vec<ProjectedPullRequest> = rows
        .into_iter()
        .map(|row| ProjectedPullRequest {
            id: row.id,
            url: row.pr_url,
            number: row.pr_number as i32,
            status: row.pr_status,
            merged_at: row.merged_at,
            merge_commit_sha: row.merge_commit_sha,
            target_branch_name: row.target_branch_name,
            project_id,
            issue_id: row.issue_id,
            workspace_id: row.workspace_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(snapshot("pull_requests", projected))
}

pub fn router() -> Router<DeploymentImpl> {
    let mut router = Router::new()
        .route(
            "/workspaces",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_workspaces(&d.db().pool, q.project_id).await
                },
            ),
        )
        .route(
            "/pull_requests",
            get(
                |State(d): State<DeploymentImpl>, Query(q): Query<ProjectScopedQuery>| async move {
                    handle_pull_requests(&d.db().pool, q.project_id).await
                },
            ),
        );

    for table in EMPTY_TABLES {
        let table = *table;
        router = router.route(
            &format!("/{table}"),
            get(move || async move { handle_empty(table) }),
        );
    }

    router
}
```

- [ ] **步骤 6：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p server local_projects 2>&1 | tail -25
```

预期：`test result: ok. 29 passed`。

- [ ] **步骤 7：手动验证空集合接口清单与前端一致**

```bash
cd /Users/admin/work/github/vibe-kanban && grep -n "defineShape<" -A 2 shared/remote-types.ts | grep -o "'[a-z_]*'," | sort -u
```

预期：输出的表名集合被 `EMPTY_TABLES` ∪ {projects, project_statuses, issues, tags, issue_tags, issue_comments, workspaces, pull_requests} 完全覆盖。若有遗漏，补进 `EMPTY_TABLES`。

- [ ] **步骤 8：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/server/src/routes/local_projects crates/db/.sqlx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：标签评论接口与工作区、PR、空集合投影" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 9：变更钩子与需求 WebSocket 推送

**Files:**
- Modify: `crates/services/src/services/events/types.rs`
- Modify: `crates/services/src/services/events/patches.rs`
- Modify: `crates/services/src/services/events.rs`
- Modify: `crates/services/src/services/events/streams.rs`
- Create: `crates/server/src/routes/issues.rs`
- Modify: `crates/server/src/routes/mod.rs`
- Test: `crates/services/src/services/events/patches.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：写失败的测试**

在 `crates/services/src/services/events/patches.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use api_types::issue::Issue;
    use chrono::Utc;
    use uuid::Uuid;

    use super::{issue_comment_patch, issue_patch, project_status_patch};

    fn 示例需求(project_id: Uuid) -> Issue {
        Issue {
            id: Uuid::from_u128(7),
            project_id,
            issue_number: 1,
            simple_id: "VK-1".to_string(),
            status_id: Uuid::from_u128(8),
            title: "示例".to_string(),
            description: None,
            priority: None,
            start_date: None,
            target_date: None,
            completed_at: None,
            sort_order: 0.0,
            parent_issue_id: None,
            parent_issue_sort_order: None,
            extension_metadata: serde_json::json!({}),
            creator_user_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn 新增需求的_patch_路径为_issues_加_id() {
        let issue = 示例需求(Uuid::from_u128(1));
        let patch = issue_patch::add(&issue);
        let op = patch.0.first().expect("必须有一个操作");
        assert_eq!(
            op.path().to_string(),
            format!("/issues/{}", Uuid::from_u128(7))
        );
    }

    #[test]
    fn 需求_patch_的值包含_project_id_供前端按项目过滤() {
        let project_id = Uuid::from_u128(1);
        let patch = issue_patch::replace(&示例需求(project_id));
        let json_patch::PatchOperation::Replace(op) = patch.0.first().unwrap() else {
            panic!("应为 Replace 操作");
        };
        assert_eq!(op.value["project_id"], project_id.to_string());
    }

    #[test]
    fn 需求_patch_不包含任何本地路径信息() {
        let patch = issue_patch::replace(&示例需求(Uuid::from_u128(1)));
        let text = serde_json::to_string(&patch).unwrap();
        for leak in ["container_ref", "worktree", "/Users/", "/home/"] {
            assert!(!text.contains(leak), "需求 patch 不得泄露 {leak}");
        }
    }

    #[test]
    fn 删除需求生成_remove_操作() {
        let patch = issue_patch::remove(Uuid::from_u128(7));
        assert!(matches!(
            patch.0.first().unwrap(),
            json_patch::PatchOperation::Remove(_)
        ));
    }

    #[test]
    fn 状态列与评论的_patch_路径前缀正确() {
        let status = db::models::local_project_status::LocalProjectStatus {
            id: Uuid::from_u128(9),
            project_id: Uuid::from_u128(1),
            name: "开发中".to_string(),
            color: "#3b82f6".to_string(),
            sort_order: 2,
            hidden: false,
            stage_type: "dev".to_string(),
            created_at: Utc::now(),
        };
        assert_eq!(
            project_status_patch::add(&status).0[0].path().to_string(),
            format!("/project_statuses/{}", Uuid::from_u128(9))
        );

        let comment = api_types::issue_comment::IssueComment {
            id: Uuid::from_u128(10),
            issue_id: Uuid::from_u128(7),
            author_id: None,
            parent_id: None,
            message: "评论".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(
            issue_comment_patch::add(&comment).0[0].path().to_string(),
            format!("/issue_comments/{}", Uuid::from_u128(10))
        );
    }
}
```

- [ ] **步骤 2：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p services patches 2>&1 | tail -20
```

预期：编译失败，`cannot find module issue_patch`。

- [ ] **步骤 3：实现 patch 构造器**

在 `crates/services/src/services/events/patches.rs` 的 `scratch_patch` 模块之后插入：

```rust
/// 需求相关 patch。三张表都走 "/<表名>/<id>" 的路径，
/// 前端按 value.project_id 过滤，因此 value 必须包含 project_id。
macro_rules! id_keyed_patch_module {
    ($module:ident, $root:expr, $ty:ty) => {
        pub mod $module {
            use super::*;

            fn path_for(id: Uuid) -> String {
                format!("{}/{}", $root, escape_pointer_segment(&id.to_string()))
            }

            pub fn add(record: &$ty) -> Patch {
                Patch(vec![PatchOperation::Add(AddOperation {
                    path: path_for(record.id).try_into().expect("路径应合法"),
                    value: serde_json::to_value(record).expect("序列化不应失败"),
                })])
            }

            pub fn replace(record: &$ty) -> Patch {
                Patch(vec![PatchOperation::Replace(ReplaceOperation {
                    path: path_for(record.id).try_into().expect("路径应合法"),
                    value: serde_json::to_value(record).expect("序列化不应失败"),
                })])
            }

            pub fn remove(id: Uuid) -> Patch {
                Patch(vec![PatchOperation::Remove(RemoveOperation {
                    path: path_for(id).try_into().expect("路径应合法"),
                })])
            }
        }
    };
}

id_keyed_patch_module!(issue_patch, "/issues", api_types::issue::Issue);
id_keyed_patch_module!(
    project_status_patch,
    "/project_statuses",
    db::models::local_project_status::LocalProjectStatus
);
id_keyed_patch_module!(
    issue_comment_patch,
    "/issue_comments",
    api_types::issue_comment::IssueComment
);
```

- [ ] **步骤 4：扩展事件类型**

在 `crates/services/src/services/events/types.rs`：

`HookTables` 追加三个变体：

```rust
    #[strum(to_string = "issues")]
    Issues,
    #[strum(to_string = "project_statuses")]
    ProjectStatuses,
    #[strum(to_string = "issue_comments")]
    IssueComments,
```

`RecordTypes` 追加：

```rust
    Issue(api_types::issue::Issue),
    ProjectStatus(db::models::local_project_status::LocalProjectStatus),
    IssueComment(api_types::issue_comment::IssueComment),
    DeletedIssue { rowid: i64 },
    DeletedProjectStatus { rowid: i64 },
    DeletedIssueComment { rowid: i64 },
```

`LocalProjectStatus` 需要 `TS` derive 才能放进 `RecordTypes`（它派生了 `TS`）。在 `crates/db/src/models/local_project_status.rs` 把 derive 改成：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct LocalProjectStatus {
```

- [ ] **步骤 5：接入变更钩子**

在 `crates/services/src/services/events.rs` 的 `set_preupdate_hook` 的 `match preupdate.table` 中，`"scratch"` 分支之后插入：

```rust
                            "issues" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(issue_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate.push_patch(issue_patch::remove(issue_id));
                                }
                            }
                            "project_statuses" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(status_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(project_status_patch::remove(status_id));
                                }
                            }
                            "issue_comments" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(comment_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(issue_comment_patch::remove(comment_id));
                                }
                            }
```

在 `set_update_hook` 内的 `match (table, hook.operation.clone())` 中，`Delete` 早退分支追加三张表，并新增三个读取分支：

```rust
                                (HookTables::Issues, SqliteOperation::Delete)
                                | (HookTables::ProjectStatuses, SqliteOperation::Delete)
                                | (HookTables::IssueComments, SqliteOperation::Delete) => {
                                    return;
                                }
                                (HookTables::Issues, _) => {
                                    match db::models::issue::Issues::find_by_rowid(&db.pool, rowid)
                                        .await
                                    {
                                        Ok(Some(issue)) => RecordTypes::Issue(issue),
                                        Ok(None) => RecordTypes::DeletedIssue { rowid },
                                        Err(e) => {
                                            tracing::error!("读取 issue rowid={} 失败: {}", rowid, e);
                                            return;
                                        }
                                    }
                                }
                                (HookTables::ProjectStatuses, _) => {
                                    match db::models::local_project_status::ProjectStatuses::find_by_rowid(
                                        &db.pool, rowid,
                                    )
                                    .await
                                    {
                                        Ok(Some(status)) => RecordTypes::ProjectStatus(status),
                                        Ok(None) => RecordTypes::DeletedProjectStatus { rowid },
                                        Err(e) => {
                                            tracing::error!(
                                                "读取 project_status rowid={} 失败: {}",
                                                rowid,
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
                                (HookTables::IssueComments, _) => {
                                    match db::models::issue_side::IssueComments::find_by_rowid(
                                        &db.pool, rowid,
                                    )
                                    .await
                                    {
                                        Ok(Some(comment)) => RecordTypes::IssueComment(comment),
                                        Ok(None) => RecordTypes::DeletedIssueComment { rowid },
                                        Err(e) => {
                                            tracing::error!(
                                                "读取 issue_comment rowid={} 失败: {}",
                                                rowid,
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
```

并在后续把 `record_type` 转成 patch 推送的分支里追加（与既有 `RecordTypes::Workspace(..)` 分支同一 `match`，按其写法追加）：

```rust
                                RecordTypes::Issue(ref issue) => match hook.operation {
                                    SqliteOperation::Insert => {
                                        msg_store_for_hook.push_patch(issue_patch::add(issue))
                                    }
                                    _ => msg_store_for_hook.push_patch(issue_patch::replace(issue)),
                                },
                                RecordTypes::ProjectStatus(ref status) => match hook.operation {
                                    SqliteOperation::Insert => msg_store_for_hook
                                        .push_patch(project_status_patch::add(status)),
                                    _ => msg_store_for_hook
                                        .push_patch(project_status_patch::replace(status)),
                                },
                                RecordTypes::IssueComment(ref comment) => match hook.operation {
                                    SqliteOperation::Insert => msg_store_for_hook
                                        .push_patch(issue_comment_patch::add(comment)),
                                    _ => msg_store_for_hook
                                        .push_patch(issue_comment_patch::replace(comment)),
                                },
```

最后把文件顶部的 `pub use patches::{...}` 补上新模块：

```rust
pub use patches::{
    execution_process_patch, issue_comment_patch, issue_patch, project_status_patch,
    scratch_patch, workspace_patch,
};
```

- [ ] **步骤 6：实现需求流**

在 `crates/services/src/services/events/streams.rs` 的 `impl EventService` 内追加：

```rust
    /// 需求看板流：首帧给出 issues / project_statuses 全量，
    /// 之后只转发属于该项目的增量 patch。
    pub async fn stream_issues_raw(
        &self,
        project_id: uuid::Uuid,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<LogMsg, std::io::Error>>,
        super::types::EventError,
    > {
        use db::models::{issue::Issues, local_project_status::ProjectStatuses};

        let issues = Issues::find_by_project(&self.db.pool, project_id).await?;
        let issues_map: serde_json::Map<String, serde_json::Value> = issues
            .into_iter()
            .map(|issue| (issue.id.to_string(), serde_json::to_value(issue).unwrap()))
            .collect();

        let statuses = ProjectStatuses::find_by_project(&self.db.pool, project_id).await?;
        let statuses_map: serde_json::Map<String, serde_json::Value> = statuses
            .into_iter()
            .map(|status| (status.id.to_string(), serde_json::to_value(status).unwrap()))
            .collect();

        let initial_patch = json!([
            { "op": "replace", "path": "/issues", "value": issues_map },
            { "op": "replace", "path": "/project_statuses", "value": statuses_map }
        ]);
        let initial_msg = LogMsg::JsonPatch(serde_json::from_value(initial_patch).unwrap());

        let project_id_string = project_id.to_string();
        let filtered_stream = BroadcastStream::new(self.msg_store.get_receiver()).filter_map(
            move |msg_result| {
                let project_id_string = project_id_string.clone();
                async move {
                    let Ok(LogMsg::JsonPatch(patch)) = msg_result else {
                        return None;
                    };
                    let op = patch.0.first()?;
                    let path = op.path().to_string();
                    if !(path.starts_with("/issues")
                        || path.starts_with("/project_statuses")
                        || path.starts_with("/issue_comments"))
                    {
                        return None;
                    }

                    let value = match op {
                        json_patch::PatchOperation::Add(a) => Some(&a.value),
                        json_patch::PatchOperation::Replace(r) => Some(&r.value),
                        // 删除操作拿不到 project_id，直接透传，
                        // 客户端对不认识的 id 会忽略。
                        _ => return Some(Ok(LogMsg::JsonPatch(patch))),
                    };

                    // issue_comments 没有 project_id，按 issue 归属交给客户端过滤。
                    let belongs = value
                        .and_then(|v| v.get("project_id"))
                        .and_then(|v| v.as_str())
                        .map(|id| id == project_id_string)
                        .unwrap_or(true);

                    belongs.then(|| Ok(LogMsg::JsonPatch(patch)))
                }
            },
        );

        let initial_stream = futures::stream::iter(vec![Ok(initial_msg), Ok(LogMsg::Ready)]);
        Ok(initial_stream.chain(filtered_stream).boxed())
    }
```

- [ ] **步骤 7：新增 WS 路由**

创建 `crates/server/src/routes/issues.rs`：

```rust
use axum::{
    Router,
    extract::{Query, State, ws::Message},
    response::IntoResponse,
    routing::get,
};
use deployment::Deployment;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    middleware::signed_ws::{MaybeSignedWebSocket, SignedWsUpgrade},
};

#[derive(Debug, Deserialize)]
pub struct IssueStreamQuery {
    pub project_id: Uuid,
}

pub async fn stream_issues_ws(
    ws: SignedWsUpgrade,
    Query(query): Query<IssueStreamQuery>,
    State(deployment): State<DeploymentImpl>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_issues_ws(socket, deployment, query.project_id).await {
            tracing::warn!("issues WS closed: {}", e);
        }
    })
}

async fn handle_issues_ws(
    mut socket: MaybeSignedWebSocket,
    deployment: DeploymentImpl,
    project_id: Uuid,
) -> anyhow::Result<()> {
    use futures_util::{StreamExt, TryStreamExt};

    let mut stream = deployment
        .events()
        .stream_issues_raw(project_id)
        .await?
        .map_ok(|msg| msg.to_ws_message_unchecked());

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(Ok(msg)) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!("stream error: {}", e);
                        break;
                    }
                    None => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Ok(Some(Message::Close(_))) => break,
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().route("/issues/streams/ws", get(stream_issues_ws))
}
```

在 `crates/server/src/routes/mod.rs` 加模块声明 `pub mod issues;`，并在 `relay_signed_routes` 的链里加：

```rust
        .merge(issues::router())
```

- [ ] **步骤 8：给 services crate 补 api-types 依赖（若缺）**

```bash
cd /Users/admin/work/github/vibe-kanban && grep -n "api-types" crates/services/Cargo.toml
```

预期：已存在。若不存在，在 `[dependencies]` 加 `api-types = { path = "../api-types" }`。

- [ ] **步骤 9：运行测试与类型生成**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p services patches 2>&1 | tail -20
pnpm run generate-types && git status --short shared/
```

预期：`test result: ok. 5 passed`；`shared/types.ts` 因 `RecordTypes` 新增变体而变化，属预期改动。

- [ ] **步骤 10：编译整个工作区**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo check --workspace 2>&1 | tail -5
```

预期：`Finished`（本机约 9 分钟）。

- [ ] **步骤 11：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/services crates/server/src/routes/issues.rs crates/server/src/routes/mod.rs \
        crates/db/src/models/local_project_status.rs shared/types.ts
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：需求变更钩子与 /api/issues/streams/ws 推送" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 10：工作区绑定需求与「开发中」自动流转

**Files:**
- Modify: `crates/db/src/models/workspace.rs`
- Modify: `crates/server/src/routes/workspaces/create.rs`
- Test: `crates/db/src/models/workspace.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：写失败的测试**

在 `crates/db/src/models/workspace.rs` 已有的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[tokio::test]
    async fn 工作区可以绑定和解绑需求() {
        use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};

        use crate::{
            models::{
                issue::Issues,
                local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
                local_project_status::{ProjectStatuses, StageType},
                workspace::{CreateWorkspace, Workspace},
            },
            test_support::TestDb,
        };

        let test_db = TestDb::new().await;
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/demo".to_string(),
                name: Some("演示".to_string()),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        assert_eq!(workspace.issue_id, None, "新建工作区默认不绑定需求");

        Workspace::set_issue_id(test_db.pool(), workspace.id, Some(issue.id))
            .await
            .unwrap();
        let bound = Workspace::find_by_id(test_db.pool(), workspace.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bound.issue_id, Some(issue.id));

        Workspace::set_issue_id(test_db.pool(), workspace.id, None)
            .await
            .unwrap();
        let unbound = Workspace::find_by_id(test_db.pool(), workspace.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unbound.issue_id, None);
    }

    #[tokio::test]
    async fn 绑定到不存在的需求会被外键拒绝() {
        use crate::{
            models::workspace::{CreateWorkspace, Workspace},
            test_support::TestDb,
        };

        let test_db = TestDb::new().await;
        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/demo".to_string(),
                name: None,
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();

        let result =
            Workspace::set_issue_id(test_db.pool(), workspace.id, Some(Uuid::from_u128(999999)))
                .await;
        assert!(result.is_err(), "绑定不存在的需求必须被外键拒绝");
    }
```

- [ ] **步骤 2：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db workspace 2>&1 | tail -20
```

预期：编译失败，`no field issue_id` / `no function set_issue_id`。

- [ ] **步骤 3：给 Workspace 加字段**

在 `crates/db/src/models/workspace.rs` 的 `pub struct Workspace` 中，`task_id` 之后插入：

```rust
    /// 个人版：绑定的本地需求。需求删除时由外键置空。
    pub issue_id: Option<Uuid>,
```

然后把该文件内**所有** `FROM workspaces` 的 `SELECT` 列表补上这一列。用下面的命令定位全部位置：

```bash
cd /Users/admin/work/github/vibe-kanban && grep -n "task_id" crates/db/src/models/workspace.rs
```

每一处 `task_id AS "task_id: Uuid",`（或 `w.task_id as "task_id: Uuid",`）之后追加同样前缀的一行：

```sql
                          issue_id AS "issue_id: Uuid",
```

（带别名的查询写 `w.issue_id AS "issue_id: Uuid",`。）`INSERT ... RETURNING` 的列表同样要补。

- [ ] **步骤 4：新增 `set_issue_id`**

在 `impl Workspace` 内追加：

```rust
    /// 绑定/解绑需求。issue_id 为 None 时解绑。
    pub async fn set_issue_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
        issue_id: Option<Uuid>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE workspaces SET issue_id = $2, updated_at = datetime('now', 'subsec') \
             WHERE id = $1",
            workspace_id,
            issue_id
        )
        .execute(pool)
        .await?;
        Ok(())
    }
```

- [ ] **步骤 5：刷新 sqlx 缓存并运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db && cargo test -p db workspace 2>&1 | tail -20
```

预期：全部通过（含既有 3 个 `best_matching_container_ref` 测试）。

- [ ] **步骤 6：在建工作区流程里接上本地需求**

在 `crates/server/src/routes/workspaces/create.rs` 的 `create_and_start_workspace` 中，把现有的

```rust
    if let Some(linked_issue) = &linked_issue
        && let Ok(client) = deployment.remote_client()
    {
```

改成先判断需求是否在本地存在：

```rust
    // 个人版：linked_issue.issue_id 指向本地 issues 表时，绑定工作区并把需求推进到「开发中」，
    // 不再调用云端。团队版（本地查不到）保持原有的云端附件导入逻辑。
    let local_issue = match &linked_issue {
        Some(info) => db::models::issue::Issues::find_by_id(&deployment.db().pool, info.issue_id)
            .await
            .map_err(ApiError::Database)?,
        None => None,
    };

    if let Some(issue) = &local_issue {
        db::models::workspace::Workspace::set_issue_id(
            &deployment.db().pool,
            managed_workspace.workspace.id,
            Some(issue.id),
        )
        .await
        .map_err(ApiError::Database)?;

        if let Err(e) = db::models::issue::Issues::move_to_stage(
            &deployment.db().pool,
            issue.id,
            db::models::local_project_status::StageType::Dev,
        )
        .await
        {
            tracing::warn!("需求 {} 流转到开发中失败: {}", issue.id, e);
        }
    }

    if local_issue.is_none()
        && let Some(linked_issue) = &linked_issue
        && let Ok(client) = deployment.remote_client()
    {
```

（`if let Some(...) ... }` 后续块体保持不变。）

- [ ] **步骤 7：写建工作区的失败回归测试**

在 `crates/server/src/routes/workspaces/create.rs` 已有的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[tokio::test]
    async fn 本地需求存在时绑定工作区并流转到开发中() {
        use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
        use db::{
            models::{
                issue::Issues,
                local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
                local_project_status::{ProjectStatuses, StageType},
                workspace::{CreateWorkspace, Workspace},
            },
            test_support::TestDb,
        };
        use uuid::Uuid;

        let test_db = TestDb::new().await;
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/demo".to_string(),
                name: None,
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();

        // 模拟路由中的本地分支
        Workspace::set_issue_id(test_db.pool(), workspace.id, Some(issue.id))
            .await
            .unwrap();
        Issues::move_to_stage(test_db.pool(), issue.id, StageType::Dev)
            .await
            .unwrap();

        let dev = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        let after = Issues::find_by_id(test_db.pool(), issue.id).await.unwrap().unwrap();
        assert_eq!(after.status_id, dev.id);
        assert_eq!(
            Workspace::find_by_id(test_db.pool(), workspace.id)
                .await
                .unwrap()
                .unwrap()
                .issue_id,
            Some(issue.id)
        );
    }
```

- [ ] **步骤 8：运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p db workspace 2>&1 | tail -8 && cargo test -p server create 2>&1 | tail -8
```

预期：两个都是 `test result: ok`。

- [ ] **步骤 9：重新生成 TS 类型（Workspace 多了 issue_id）**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run generate-types && git diff --stat shared/types.ts
```

预期：`shared/types.ts` 中 `Workspace` 增加 `issue_id: string | null`。

- [ ] **步骤 10：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/db/src/models/workspace.rs crates/server/src/routes/workspaces/create.rs \
        crates/db/.sqlx shared/types.ts
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：工作区绑定本地需求并自动流转到开发中" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 11：PR 与合并触发「待评审 / 已完成」流转

**Files:**
- Create: `crates/services/src/services/issue_flow.rs`
- Modify: `crates/services/src/services/mod.rs`
- Modify: `crates/server/src/routes/workspaces/pr.rs`
- Modify: `crates/server/src/routes/workspaces/git.rs`
- Modify: `crates/db/src/models/pull_request.rs`
- Test: `crates/services/src/services/issue_flow.rs`（模块内 `#[cfg(test)]`）

- [ ] **步骤 1：写失败的测试**

创建 `crates/services/src/services/issue_flow.rs`：

```rust
#[cfg(test)]
mod tests {
    use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
            workspace::{CreateWorkspace, Workspace},
        },
        test_support::TestDb,
    };
    use uuid::Uuid;

    use super::{advance_issue_for_workspace, IssueFlowStage};

    async fn 准备(test_db: &TestDb) -> (Uuid, Uuid, Uuid) {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "Vibe Kanban".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let todo = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: todo.id,
                title: "示例".to_string(),
                description: None,
                priority: None,
                start_date: None,
                target_date: None,
                completed_at: None,
                sort_order: 0.0,
                parent_issue_id: None,
                parent_issue_sort_order: None,
                extension_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/demo".to_string(),
                name: None,
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        Workspace::set_issue_id(test_db.pool(), workspace.id, Some(issue.id))
            .await
            .unwrap();

        (project.id, issue.id, workspace.id)
    }

    #[tokio::test]
    async fn 建_pr_把绑定需求推进到待评审() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Review)
            .await
            .unwrap();

        let review = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Review)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();
        assert_eq!(issue.status_id, review.id);
        assert!(issue.completed_at.is_none(), "待评审不写完成时间");
    }

    #[tokio::test]
    async fn 合并把绑定需求推进到已完成并写完成时间() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();

        let done = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();
        assert_eq!(issue.status_id, done.id);
        assert!(issue.completed_at.is_some());
    }

    #[tokio::test]
    async fn 未绑定需求的工作区不做任何事() {
        let test_db = TestDb::new().await;
        let workspace = Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: "vk/none".to_string(),
                name: None,
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();

        let moved = advance_issue_for_workspace(
            test_db.pool(),
            workspace.id,
            IssueFlowStage::Done,
        )
        .await
        .unwrap();
        assert!(!moved, "未绑定需求时返回 false，且不得报错");
    }

    #[tokio::test]
    async fn 不存在的工作区不报错() {
        let test_db = TestDb::new().await;
        let moved = advance_issue_for_workspace(
            test_db.pool(),
            Uuid::from_u128(88888),
            IssueFlowStage::Review,
        )
        .await
        .unwrap();
        assert!(!moved);
    }

    #[tokio::test]
    async fn 重复流转是幂等的() {
        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();
        let first = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();

        advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Done)
            .await
            .unwrap();
        let second = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();

        let done = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.status_id, done.id);
        assert_eq!(second.status_id, done.id);
    }
}
```

- [ ] **步骤 2：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p services issue_flow 2>&1 | tail -20
```

预期：编译失败，`cannot find function advance_issue_for_workspace`。

- [ ] **步骤 3：写实现**

在测试模块之前插入：

```rust
//! 个人版需求状态自动流转。
//! 所有入口都以「工作区 → 绑定需求」为起点；工作区未绑定需求时静默跳过，
//! 因此团队版（需求在云端）不会被影响。

use db::models::{issue::Issues, local_project_status::StageType, workspace::Workspace};
use sqlx::SqlitePool;
use uuid::Uuid;

/// 可由开发流程触发的阶段。刻意不暴露 backlog/todo：那两个只由用户手动拖拽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueFlowStage {
    Dev,
    Review,
    Done,
}

impl From<IssueFlowStage> for StageType {
    fn from(stage: IssueFlowStage) -> Self {
        match stage {
            IssueFlowStage::Dev => StageType::Dev,
            IssueFlowStage::Review => StageType::Review,
            IssueFlowStage::Done => StageType::Done,
        }
    }
}

/// 把工作区绑定的本地需求推进到指定阶段。
/// 返回 true 表示确实改了需求；工作区不存在、未绑定需求、
/// 或项目里没有对应阶段的状态列时返回 false，不报错。
pub async fn advance_issue_for_workspace(
    pool: &SqlitePool,
    workspace_id: Uuid,
    stage: IssueFlowStage,
) -> Result<bool, sqlx::Error> {
    let Some(workspace) = Workspace::find_by_id(pool, workspace_id).await? else {
        return Ok(false);
    };
    let Some(issue_id) = workspace.issue_id else {
        return Ok(false);
    };

    match Issues::move_to_stage(pool, issue_id, stage.into()).await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(db::models::issue::IssueError::Database(err)) => Err(err),
        Err(err) => {
            tracing::warn!("需求 {} 自动流转失败: {}", issue_id, err);
            Ok(false)
        }
    }
}
```

在 `crates/services/src/services/mod.rs` 注册：

```rust
pub mod issue_flow;
```

- [ ] **步骤 4：在建 PR 处调用**

`crates/server/src/routes/workspaces/pr.rs` 中，三处 `PullRequest::create_for_workspace(` 调用之后各追加一段（用 `grep -n "create_for_workspace" crates/server/src/routes/workspaces/pr.rs` 定位三处，行号约 306 / 459 / 781）：

```rust
    if let Err(e) = services::services::issue_flow::advance_issue_for_workspace(
        &deployment.db().pool,
        workspace.id,
        services::services::issue_flow::IssueFlowStage::Review,
    )
    .await
    {
        tracing::warn!("PR 创建后需求流转失败: {}", e);
    }
```

若该作用域里工作区变量不叫 `workspace`，改用当时可见的工作区 id 表达式。

- [ ] **步骤 5：在本地合并处调用**

`crates/server/src/routes/workspaces/git.rs` 的 `merge_workspace` 在合并成功返回之前追加：

```rust
    if let Err(e) = services::services::issue_flow::advance_issue_for_workspace(
        &deployment.db().pool,
        workspace.id,
        services::services::issue_flow::IssueFlowStage::Done,
    )
    .await
    {
        tracing::warn!("本地合并后需求流转失败: {}", e);
    }
```

- [ ] **步骤 6：在 PR 状态同步为已合并时调用**

`crates/db/src/models/pull_request.rs` 的 `update_status` 是 db 层，不能依赖 services。改在调用方处理：用

```bash
cd /Users/admin/work/github/vibe-kanban && grep -rn "update_status(" crates/services/src crates/server/src | grep -i "pullrequest\|pull_request"
```

定位 PR 状态轮询同步的调用点，在把状态置为 `PullRequestStatus::Merged` 的分支后追加：

```rust
        if matches!(new_status, api_types::pull_request::PullRequestStatus::Merged)
            && let Some(workspace_id) = pr.workspace_id
            && let Err(e) = crate::services::issue_flow::advance_issue_for_workspace(
                &db.pool,
                workspace_id,
                crate::services::issue_flow::IssueFlowStage::Done,
            )
            .await
        {
            tracing::warn!("PR 合并后需求流转失败: {}", e);
        }
```

（若调用点在 `crates/server`，把 `crate::services::issue_flow` 换成 `services::services::issue_flow`，`&db.pool` 换成 `&deployment.db().pool`。）

- [ ] **步骤 7：运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test -p services issue_flow 2>&1 | tail -10 && cargo check --workspace 2>&1 | tail -3
```

预期：`test result: ok. 5 passed`；`cargo check` 以 `Finished` 结束。

- [ ] **步骤 8：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/services/src/services/issue_flow.rs crates/services/src/services/mod.rs \
        crates/server/src/routes/workspaces/pr.rs crates/server/src/routes/workspaces/git.rs
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：PR 与合并触发需求状态自动流转" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 12：为 web-core 引入 Vitest 并抽出数据行纯函数

**Files:**
- Create: `packages/web-core/vitest.config.ts`
- Create: `packages/web-core/src/shared/lib/electric/rows.ts`
- Create: `packages/web-core/src/shared/lib/electric/rows.test.ts`
- Modify: `packages/web-core/package.json`
- Modify: `packages/web-core/src/shared/lib/electric/collections.ts`
- Modify: `package.json`（根）

- [ ] **步骤 1：安装 Vitest**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm --filter @vibe/web-core add -D vitest@^3.2.4 @types/node@^22.10.2
```

预期：`packages/web-core/package.json` 的 `devDependencies` 出现 `vitest` 与 `@types/node`。

- [ ] **步骤 2：写 Vitest 配置**

创建 `packages/web-core/vitest.config.ts`：

```ts
import path from 'node:path';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@\//, replacement: `${path.resolve(__dirname, 'src')}/` },
      { find: 'shared', replacement: path.resolve(__dirname, '../../shared') },
    ],
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
```

- [ ] **步骤 3：加脚本**

`packages/web-core/package.json` 的 `scripts` 增加：

```json
    "test": "vitest run",
    "test:watch": "vitest"
```

根 `package.json` 的 `scripts` 增加：

```json
    "web-core:test": "pnpm --filter @vibe/web-core run test",
```

并把 `check` 改成（在末尾追加）：

```json
    "check": "pnpm run local-web:legacy-path-guard && pnpm run local-web:check && pnpm run remote-web:check && pnpm run web-core:check && pnpm run ui:check && pnpm run backend:check && pnpm run web-core:test",
```

- [ ] **步骤 4：写失败的测试**

创建 `packages/web-core/src/shared/lib/electric/rows.test.ts`：

```ts
import { describe, expect, it } from 'vitest';

import {
  extractFallbackRows,
  getRowKey,
  parseResponseError,
} from '@/shared/lib/electric/rows';

describe('getRowKey', () => {
  it('优先使用 id', () => {
    expect(getRowKey({ id: 'abc', issue_id: 'x', tag_id: 'y' })).toBe('abc');
  });

  it('没有 id 时按字母序拼接所有 *_id 字段', () => {
    expect(getRowKey({ tag_id: 'tag', issue_id: 'issue' })).toBe('issue-tag');
  });

  it('id 为空字符串时回退到复合键', () => {
    expect(getRowKey({ id: '', issue_id: 'issue', tag_id: 'tag' })).toBe(
      'issue-tag'
    );
  });
});

describe('extractFallbackRows', () => {
  it('按表名取出数组', () => {
    expect(extractFallbackRows({ issues: [{ id: '1' }] }, 'issues')).toEqual([
      { id: '1' },
    ]);
  });

  it('空数组是合法结果', () => {
    expect(extractFallbackRows({ issues: [] }, 'issues')).toEqual([]);
  });

  it('响应不是对象时抛错', () => {
    expect(() => extractFallbackRows(null, 'issues')).toThrow(
      /not an object/
    );
  });

  it('缺少目标数组时抛错并带上表名', () => {
    expect(() => extractFallbackRows({ other: [] }, 'issues')).toThrow(
      /issues/
    );
  });
});

describe('parseResponseError', () => {
  it('优先读 message 字段', async () => {
    const response = new Response(JSON.stringify({ message: '坏了' }), {
      status: 400,
    });
    await expect(parseResponseError(response, '默认')).resolves.toBe('坏了');
  });

  it('响应不是 JSON 时回退到默认文案', async () => {
    const response = new Response('<html>', { status: 500 });
    await expect(parseResponseError(response, '默认')).resolves.toBe('默认');
  });
});
```

- [ ] **步骤 5：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -20
```

预期：失败，`Failed to resolve import "@/shared/lib/electric/rows"`。

- [ ] **步骤 6：抽出纯函数**

创建 `packages/web-core/src/shared/lib/electric/rows.ts`：

```ts
export type ElectricRow = Record<string, unknown> & { [key: string]: unknown };

/**
 * 集合的主键：优先 id，否则按字母序拼接所有 *_id 字段。
 * 与 TanStack DB 的 getKey 约定保持一致。
 */
export function getRowKey(item: Record<string, unknown>): string {
  if ('id' in item && item.id) {
    return String(item.id);
  }

  return Object.entries(item)
    .filter(([key]) => key.endsWith('_id'))
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([, value]) => String(value))
    .join('-');
}

/**
 * 快照响应必须形如 { "<表名>": [ ...行 ] }。
 * 本地接口（/api/local/*）与云端后备接口（/v1/fallback/*）共用这一约定。
 */
export function extractFallbackRows(
  payload: unknown,
  table: string
): Array<ElectricRow> {
  if (!payload || typeof payload !== 'object') {
    throw new Error(`Fallback response for "${table}" is not an object`);
  }

  const rows = (payload as Record<string, unknown>)[table];
  if (!Array.isArray(rows)) {
    throw new Error(`Fallback response missing "${table}" array`);
  }

  return rows as Array<ElectricRow>;
}

export async function parseResponseError(
  response: Response,
  fallbackMessage: string
): Promise<string> {
  try {
    const body = (await response.json()) as {
      message?: string;
      error?: string;
    };
    return body.message || body.error || fallbackMessage;
  } catch {
    return fallbackMessage;
  }
}
```

- [ ] **步骤 7：让 collections.ts 复用它们**

在 `packages/web-core/src/shared/lib/electric/collections.ts`：

删除文件内的 `getRowKey`、`extractFallbackRows`、`parseResponseError` 三个函数定义与 `type ElectricRow = ...` 那一行，改为在文件顶部导入：

```ts
import {
  type ElectricRow,
  extractFallbackRows,
  getRowKey,
  parseResponseError,
} from '@/shared/lib/electric/rows';
```

其余调用点不变。

- [ ] **步骤 8：运行测试与类型检查**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -10 && pnpm run web-core:check
```

预期：`Test Files 1 passed`、`Tests 9 passed`；`tsc` 无输出。

- [ ] **步骤 9：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core/vitest.config.ts packages/web-core/package.json package.json \
        packages/web-core/src/shared/lib/electric/rows.ts \
        packages/web-core/src/shared/lib/electric/rows.test.ts \
        packages/web-core/src/shared/lib/electric/collections.ts pnpm-lock.yaml
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "前端：引入 Vitest 并抽出数据行纯函数" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 13：数据源开关、固定身份与本地端点映射

**Files:**
- Create: `packages/web-core/src/shared/lib/local/dataSource.ts`
- Create: `packages/web-core/src/shared/lib/local/identity.ts`
- Create: `packages/web-core/src/shared/lib/local/localEndpoints.ts`
- Create: `packages/web-core/src/shared/lib/local/localEndpoints.test.ts`

- [ ] **步骤 1：写失败的测试**

创建 `packages/web-core/src/shared/lib/local/localEndpoints.test.ts`：

```ts
import { beforeEach, describe, expect, it } from 'vitest';
import {
  ISSUE_COMMENTS_SHAPE,
  ISSUE_MUTATION,
  PROJECTS_SHAPE,
  PROJECT_ISSUES_SHAPE,
  PROJECT_ISSUE_ASSIGNEES_SHAPE,
  PROJECT_PROJECT_STATUSES_SHAPE,
  PROJECT_TAGS_SHAPE,
  PROJECT_WORKSPACES_SHAPE,
  PULL_REQUEST_ISSUE_MUTATION,
  USER_WORKSPACES_SHAPE,
} from 'shared/remote-types';

import {
  configureDataSource,
  getDataSourceMode,
  isLocalMode,
} from '@/shared/lib/local/dataSource';
import {
  resolveLocalMutationUrl,
  resolveLocalShapeEndpoint,
} from '@/shared/lib/local/localEndpoints';

describe('数据源开关', () => {
  beforeEach(() => {
    configureDataSource('remote');
  });

  it('默认是 remote，保证团队版行为不变', () => {
    expect(getDataSourceMode()).toBe('remote');
    expect(isLocalMode()).toBe(false);
  });

  it('切到 local 后 isLocalMode 为真', () => {
    configureDataSource('local');
    expect(getDataSourceMode()).toBe('local');
    expect(isLocalMode()).toBe(true);
  });
});

describe('resolveLocalShapeEndpoint', () => {
  it('项目集合不带查询参数', () => {
    expect(resolveLocalShapeEndpoint(PROJECTS_SHAPE, {})).toEqual({
      kind: 'rest',
      path: '/api/local/projects',
      wsPath: null,
    });
  });

  it('项目维度集合带上 project_id', () => {
    const endpoint = resolveLocalShapeEndpoint(PROJECT_ISSUES_SHAPE, {
      project_id: 'p1',
    });
    expect(endpoint).toEqual({
      kind: 'rest',
      path: '/api/local/issues?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('状态列也订阅同一个需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_PROJECT_STATUSES_SHAPE, {
        project_id: 'p1',
      })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/project_statuses?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('标签映射到 /api/local/tags', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_TAGS_SHAPE, { project_id: 'p1' }).path
    ).toBe('/api/local/tags?project_id=p1');
  });

  it('评论按 issue_id 过滤', () => {
    expect(
      resolveLocalShapeEndpoint(ISSUE_COMMENTS_SHAPE, { issue_id: 'i1' }).path
    ).toBe('/api/local/issue_comments?issue_id=i1');
  });

  it('项目维度工作区走投影接口', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_WORKSPACES_SHAPE, { project_id: 'p1' })
        .path
    ).toBe('/api/local/workspaces?project_id=p1');
  });

  it('个人版没有跨项目的用户工作区集合，返回空集合', () => {
    expect(
      resolveLocalShapeEndpoint(USER_WORKSPACES_SHAPE, { owner_user_id: 'u1' })
    ).toEqual({ kind: 'empty', table: 'workspaces' });
  });

  it('未实现的集合返回空集合而不是抛错', () => {
    expect(
      resolveLocalShapeEndpoint(PROJECT_ISSUE_ASSIGNEES_SHAPE, {
        project_id: 'p1',
      })
    ).toEqual({ kind: 'empty', table: 'issue_assignees' });
  });

  it('参数值被 URL 编码，防止拼接出越权路径', () => {
    const endpoint = resolveLocalShapeEndpoint(PROJECT_ISSUES_SHAPE, {
      project_id: '../../etc/passwd',
    });
    expect(endpoint).toEqual({
      kind: 'rest',
      path: '/api/local/issues?project_id=..%2F..%2Fetc%2Fpasswd',
      wsPath: '/api/issues/streams/ws?project_id=..%2F..%2Fetc%2Fpasswd',
    });
  });
});

describe('resolveLocalMutationUrl', () => {
  it('需求写接口指向 /api/local/issues', () => {
    expect(resolveLocalMutationUrl(ISSUE_MUTATION)).toBe('/api/local/issues');
  });

  it('个人版不支持的写操作返回 null', () => {
    expect(resolveLocalMutationUrl(PULL_REQUEST_ISSUE_MUTATION)).toBeNull();
  });
});
```

- [ ] **步骤 2：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -20
```

预期：失败，`Failed to resolve import "@/shared/lib/local/dataSource"`。

- [ ] **步骤 3：写数据源开关**

创建 `packages/web-core/src/shared/lib/local/dataSource.ts`：

```ts
export type DataSourceMode = 'local' | 'remote';

// 默认 remote：不显式调用 configureDataSource 时，团队版行为完全不变。
let dataSourceMode: DataSourceMode = 'remote';

export function configureDataSource(mode: DataSourceMode): void {
  dataSourceMode = mode;
}

export function getDataSourceMode(): DataSourceMode {
  return dataSourceMode;
}

export function isLocalMode(): boolean {
  return dataSourceMode === 'local';
}
```

- [ ] **步骤 4：写固定身份**

创建 `packages/web-core/src/shared/lib/local/identity.ts`：

```ts
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
```

- [ ] **步骤 5：写端点映射**

创建 `packages/web-core/src/shared/lib/local/localEndpoints.ts`：

```ts
import type { MutationDefinition, ShapeDefinition } from 'shared/remote-types';

const LOCAL_API_PREFIX = '/api/local';
const ISSUE_STREAM_PATH = '/api/issues/streams/ws';

export type LocalShapeEndpoint =
  | { kind: 'rest'; path: string; wsPath: string | null }
  | { kind: 'empty'; table: string };

/** 项目维度的需求流覆盖的表：它们共用 /api/issues/streams/ws。 */
const ISSUE_STREAM_TABLES = new Set([
  'issues',
  'project_statuses',
  'issue_comments',
]);

/** 前端表名 → 本地 REST 资源段。 */
const REST_RESOURCE: Record<string, string> = {
  projects: 'projects',
  project_statuses: 'project_statuses',
  issues: 'issues',
  tags: 'tags',
  issue_tags: 'issue_tags',
  issue_comments: 'issue_comments',
  workspaces: 'workspaces',
  pull_requests: 'pull_requests',
};

/** 每个表在个人版里允许的过滤参数。其他参数一律丢弃。 */
const ALLOWED_PARAM: Record<string, string | null> = {
  projects: null,
  project_statuses: 'project_id',
  issues: 'project_id',
  tags: 'project_id',
  issue_tags: 'project_id',
  issue_comments: 'issue_id',
  workspaces: 'project_id',
  pull_requests: 'project_id',
};

/** mutation.name → 本地写接口。null 表示个人版不支持该写操作。 */
const MUTATION_URL: Record<string, string | null> = {
  Project: `${LOCAL_API_PREFIX}/projects`,
  ProjectStatus: `${LOCAL_API_PREFIX}/project_statuses`,
  Issue: `${LOCAL_API_PREFIX}/issues`,
  Tag: `${LOCAL_API_PREFIX}/tags`,
  IssueTag: `${LOCAL_API_PREFIX}/issue_tags`,
  IssueComment: `${LOCAL_API_PREFIX}/issue_comments`,
  IssueAssignee: null,
  IssueFollower: null,
  IssueRelationship: null,
  IssueCommentReaction: null,
  PullRequestIssue: null,
  Notification: null,
};

export function resolveLocalShapeEndpoint(
  shape: ShapeDefinition<unknown>,
  params: Record<string, string>
): LocalShapeEndpoint {
  const resource = REST_RESOURCE[shape.table];
  if (!resource) {
    return { kind: 'empty', table: shape.table };
  }

  const paramName = ALLOWED_PARAM[shape.table] ?? null;
  if (paramName === null) {
    return { kind: 'rest', path: `${LOCAL_API_PREFIX}/${resource}`, wsPath: null };
  }

  const value = params[paramName];
  if (!value) {
    // 例如 USER_WORKSPACES_SHAPE 只带 owner_user_id：个人版没有这个维度。
    return { kind: 'empty', table: shape.table };
  }

  const encoded = encodeURIComponent(value);
  const query = `${paramName}=${encoded}`;
  const wsPath =
    ISSUE_STREAM_TABLES.has(shape.table) && paramName === 'project_id'
      ? `${ISSUE_STREAM_PATH}?${query}`
      : null;

  return {
    kind: 'rest',
    path: `${LOCAL_API_PREFIX}/${resource}?${query}`,
    wsPath,
  };
}

export function resolveLocalMutationUrl(
  mutation: MutationDefinition<unknown, unknown, unknown>
): string | null {
  return MUTATION_URL[mutation.name] ?? null;
}
```

注意：`issue_comments` 的 WS 路径为 `null`——评论按 `issue_id` 订阅，而需求流是按 `project_id` 的；评论的实时性由「写后立即刷新」保证。

- [ ] **步骤 6：运行测试**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -10
```

预期：`Tests 22 passed`（rows 9 + endpoints 13）。

- [ ] **步骤 7：类型检查**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check
```

预期：无输出。

- [ ] **步骤 8：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core/src/shared/lib/local
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "前端：数据源开关、固定身份与本地端点映射" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 14：本地集合实现（REST 快照 + WS 增量 + 写后刷新）

**Files:**
- Create: `packages/web-core/src/shared/lib/local/localCollections.ts`
- Create: `packages/web-core/src/shared/lib/local/localCollections.test.ts`
- Modify: `packages/web-core/src/shared/lib/electric/collections.ts`
- Modify: `packages/web-core/src/shared/lib/remoteApi.ts`

- [ ] **步骤 1：写失败的测试**

创建 `packages/web-core/src/shared/lib/local/localCollections.test.ts`：

```ts
import { describe, expect, it, vi } from 'vitest';

import {
  buildLocalMutationHandlers,
  patchToWrites,
} from '@/shared/lib/local/localCollections';

describe('patchToWrites', () => {
  it('首帧的 replace 整表变成 truncate + 批量 insert', () => {
    const writes = patchToWrites(
      [
        {
          op: 'replace',
          path: '/issues',
          value: {
            a: { id: 'a', title: 'A' },
            b: { id: 'b', title: 'B' },
          },
        },
      ],
      'issues'
    );

    expect(writes).toEqual([
      { type: 'truncate' },
      { type: 'insert', value: { id: 'a', title: 'A' } },
      { type: 'insert', value: { id: 'b', title: 'B' } },
    ]);
  });

  it('单行 add 变成 insert', () => {
    expect(
      patchToWrites(
        [{ op: 'add', path: '/issues/a', value: { id: 'a', title: 'A' } }],
        'issues'
      )
    ).toEqual([{ type: 'insert', value: { id: 'a', title: 'A' } }]);
  });

  it('单行 replace 变成 update', () => {
    expect(
      patchToWrites(
        [{ op: 'replace', path: '/issues/a', value: { id: 'a', title: 'A2' } }],
        'issues'
      )
    ).toEqual([{ type: 'update', value: { id: 'a', title: 'A2' } }]);
  });

  it('remove 变成 delete，并带上 id 以便按主键删除', () => {
    expect(patchToWrites([{ op: 'remove', path: '/issues/a' }], 'issues')).toEqual(
      [{ type: 'delete', value: { id: 'a' } }]
    );
  });

  it('忽略其他表的 patch', () => {
    expect(
      patchToWrites(
        [{ op: 'add', path: '/project_statuses/s1', value: { id: 's1' } }],
        'issues'
      )
    ).toEqual([]);
  });

  it('对 JSON Pointer 转义做还原', () => {
    expect(
      patchToWrites([{ op: 'remove', path: '/issues/a~1b~0c' }], 'issues')
    ).toEqual([{ type: 'delete', value: { id: 'a/b~c' } }]);
  });

  it('未知操作被忽略而不是抛错', () => {
    expect(
      patchToWrites(
        [{ op: 'test', path: '/issues/a', value: {} } as never],
        'issues'
      )
    ).toEqual([]);
  });
});

describe('buildLocalMutationHandlers', () => {
  const okResponse = () =>
    new Response(JSON.stringify({ txid: 0 }), { status: 200 });

  it('新增走 POST，并在成功后刷新集合', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onInsert({
      transaction: { mutations: [{ modified: { id: 'a', title: 'A' } }] },
    });

    expect(request).toHaveBeenCalledWith('/api/local/issues', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: 'a', title: 'A' }),
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('单条更新走 PATCH /{id}', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onUpdate({
      transaction: { mutations: [{ key: 'a', changes: { title: 'A2' } }] },
    });

    expect(request).toHaveBeenCalledWith('/api/local/issues/a', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: 'A2' }),
    });
  });

  it('多条更新合并成一次 POST /bulk，保证拖拽排序是一个事务', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onUpdate({
      transaction: {
        mutations: [
          { key: 'a', changes: { sort_order: 1 } },
          { key: 'b', changes: { sort_order: 2 } },
        ],
      },
    });

    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenCalledWith('/api/local/issues/bulk', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        updates: [
          { id: 'a', sort_order: 1 },
          { id: 'b', sort_order: 2 },
        ],
      }),
    });
  });

  it('删除走 DELETE /{id}', async () => {
    const request = vi.fn().mockResolvedValue(okResponse());
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await handlers.onDelete({ transaction: { mutations: [{ key: 'a' }] } });

    expect(request).toHaveBeenCalledWith('/api/local/issues/a', {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
    });
  });

  it('接口报错时抛出服务端文案，且不刷新集合', async () => {
    const request = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ message: '需求标题不能为空' }), {
          status: 400,
        })
      );
    const refresh = vi.fn().mockResolvedValue(undefined);
    const handlers = buildLocalMutationHandlers({
      name: 'Issue',
      url: '/api/local/issues',
      request,
      refresh,
    });

    await expect(
      handlers.onInsert({
        transaction: { mutations: [{ modified: { title: '' } }] },
      })
    ).rejects.toThrow('需求标题不能为空');
    expect(refresh).not.toHaveBeenCalled();
  });

  it('不支持的写操作直接抛出可读错误', async () => {
    const request = vi.fn();
    const refresh = vi.fn();
    const handlers = buildLocalMutationHandlers({
      name: 'PullRequestIssue',
      url: null,
      request,
      refresh,
    });

    await expect(
      handlers.onInsert({ transaction: { mutations: [{ modified: {} }] } })
    ).rejects.toThrow(/个人版不支持/);
    expect(request).not.toHaveBeenCalled();
  });
});
```

- [ ] **步骤 2：运行测试确认失败**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -20
```

预期：失败，`Failed to resolve import "@/shared/lib/local/localCollections"`。

- [ ] **步骤 3：写实现**

创建 `packages/web-core/src/shared/lib/local/localCollections.ts`：

```ts
import { createCollection } from '@tanstack/react-db';
import type { Operation } from 'rfc6902';
import type { MutationDefinition, ShapeDefinition } from 'shared/remote-types';

import {
  type ElectricRow,
  getRowKey,
  extractFallbackRows,
  parseResponseError,
} from '@/shared/lib/electric/rows';
import {
  makeLocalApiRequest,
  openLocalApiWebSocket,
} from '@/shared/lib/localApiTransport';
import {
  resolveLocalMutationUrl,
  resolveLocalShapeEndpoint,
} from '@/shared/lib/local/localEndpoints';
import type { CollectionConfig, SyncError } from '@/shared/lib/electric/types';

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const;
/** WS 断线时的兜底轮询间隔。 */
const POLL_INTERVAL_MS = 15 * 1000;
/** WS 重连退避上限。 */
const MAX_RECONNECT_DELAY_MS = 8000;

export type LocalWrite =
  | { type: 'truncate' }
  | { type: 'insert'; value: ElectricRow }
  | { type: 'update'; value: ElectricRow }
  | { type: 'delete'; value: ElectricRow };

function unescapePointerSegment(segment: string): string {
  return segment.replace(/~1/g, '/').replace(/~0/g, '~');
}

/**
 * 把后端推来的 JSON Patch 转成集合写操作。
 * 只处理属于本表的路径：/<table> 整表替换，/<table>/<id> 单行增删改。
 */
export function patchToWrites(
  operations: Operation[],
  table: string
): LocalWrite[] {
  const writes: LocalWrite[] = [];
  const rootPath = `/${table}`;

  for (const operation of operations) {
    const path = operation.path;
    if (path !== rootPath && !path.startsWith(`${rootPath}/`)) {
      continue;
    }

    const isRoot = path === rootPath;
    const id = isRoot
      ? null
      : unescapePointerSegment(path.slice(rootPath.length + 1));

    if (isRoot) {
      if (operation.op !== 'replace' && operation.op !== 'add') continue;
      writes.push({ type: 'truncate' });
      const value = (operation as { value: unknown }).value;
      const rows =
        value && typeof value === 'object'
          ? Object.values(value as Record<string, ElectricRow>)
          : [];
      for (const row of rows) {
        writes.push({ type: 'insert', value: row });
      }
      continue;
    }

    if (operation.op === 'add') {
      writes.push({
        type: 'insert',
        value: (operation as { value: ElectricRow }).value,
      });
    } else if (operation.op === 'replace') {
      writes.push({
        type: 'update',
        value: (operation as { value: ElectricRow }).value,
      });
    } else if (operation.op === 'remove') {
      writes.push({ type: 'delete', value: { id } as ElectricRow });
    }
  }

  return writes;
}

type MutationFnParams = {
  transaction: {
    mutations: Array<{
      modified?: unknown;
      original?: unknown;
      key?: string;
      changes?: unknown;
    }>;
  };
};

export function buildLocalMutationHandlers(args: {
  name: string;
  url: string | null;
  request: (path: string, init?: RequestInit) => Promise<Response>;
  refresh: () => Promise<void>;
}) {
  const { name, url, request, refresh } = args;

  const ensureUrl = (): string => {
    if (!url) {
      throw new Error(`个人版不支持修改 ${name}`);
    }
    return url;
  };

  const send = async (path: string, init: RequestInit): Promise<void> => {
    const response = await request(path, init);
    if (!response.ok) {
      throw new Error(
        await parseResponseError(response, `Failed to write ${name}`)
      );
    }
  };

  return {
    onInsert: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      for (const mutation of transaction.mutations) {
        await send(base, {
          method: 'POST',
          headers: JSON_HEADERS,
          body: JSON.stringify(mutation.modified),
        });
      }
      await refresh();
    },

    onUpdate: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      if (transaction.mutations.length > 1) {
        // 拖拽排序：合并成一次 /bulk，由后端放进一个事务。
        const updates = transaction.mutations.map((mutation) => {
          if (!mutation.key) {
            throw new Error(`Failed to update ${name}: missing key`);
          }
          return {
            id: String(mutation.key),
            ...(mutation.changes as Record<string, unknown>),
          };
        });
        await send(`${base}/bulk`, {
          method: 'POST',
          headers: JSON_HEADERS,
          body: JSON.stringify({ updates }),
        });
      } else {
        const mutation = transaction.mutations[0];
        if (!mutation?.key) {
          throw new Error(`Failed to update ${name}: missing key`);
        }
        await send(`${base}/${encodeURIComponent(String(mutation.key))}`, {
          method: 'PATCH',
          headers: JSON_HEADERS,
          body: JSON.stringify(mutation.changes),
        });
      }
      await refresh();
    },

    onDelete: async ({ transaction }: MutationFnParams): Promise<void> => {
      const base = ensureUrl();
      for (const mutation of transaction.mutations) {
        await send(`${base}/${encodeURIComponent(String(mutation.key))}`, {
          method: 'DELETE',
          headers: JSON_HEADERS,
        });
      }
      await refresh();
    },
  };
}

type SyncParams = {
  collection: { isReady: () => boolean };
  begin: () => void;
  write: (message: {
    type: 'insert' | 'update' | 'delete';
    value: ElectricRow;
    metadata?: Record<string, unknown>;
  }) => void;
  commit: () => void;
  markReady: () => void;
  truncate: () => void;
};

function applyWrites(syncParams: SyncParams, writes: LocalWrite[]): void {
  if (writes.length === 0) return;
  syncParams.begin();
  for (const write of writes) {
    if (write.type === 'truncate') {
      syncParams.truncate();
    } else {
      syncParams.write({ type: write.type, value: write.value, metadata: {} });
    }
  }
  syncParams.commit();
  syncParams.markReady();
}

/**
 * 个人版集合：REST 全量快照打底，WS JSON Patch 做增量，写成功后立即重拉。
 */
export function createLocalShapeCollection<TRow extends ElectricRow>(
  collectionId: string,
  shape: ShapeDefinition<TRow>,
  params: Record<string, string>,
  config?: CollectionConfig,
  mutation?: MutationDefinition<unknown, unknown, unknown>
) {
  const endpoint = resolveLocalShapeEndpoint(shape, params);
  const reportError = (error: SyncError) => config?.onError?.(error);

  let refreshNow: () => Promise<void> = async () => {};

  const mutationHandlers = mutation
    ? buildLocalMutationHandlers({
        name: mutation.name,
        url: resolveLocalMutationUrl(mutation),
        request: (path, init) => makeLocalApiRequest(path, init),
        refresh: () => refreshNow(),
      })
    : {};

  const sync = (syncParams: SyncParams) => {
    if (endpoint.kind === 'empty') {
      applyWrites(syncParams, [{ type: 'truncate' }]);
      syncParams.markReady();
      return { cleanup: () => {}, loadSubset: () => true };
    }

    let cleanedUp = false;
    let socket: WebSocket | null = null;
    let reconnectAttempt = 0;
    let reconnectTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
    let inFlight: Promise<void> | null = null;

    const fetchSnapshot = async (): Promise<void> => {
      if (inFlight) return inFlight;
      inFlight = (async () => {
        try {
          const response = await makeLocalApiRequest(endpoint.path, {
            method: 'GET',
            cache: 'no-store',
          });
          if (!response.ok) {
            throw new Error(
              await parseResponseError(
                response,
                `Failed to fetch local ${shape.table}`
              )
            );
          }
          const rows = extractFallbackRows(await response.json(), shape.table);
          if (cleanedUp) return;
          applyWrites(syncParams, [
            { type: 'truncate' },
            ...rows.map((row) => ({ type: 'insert' as const, value: row })),
          ]);
        } catch (error) {
          reportError({
            message:
              error instanceof Error ? error.message : 'Local fetch failed',
          });
          if (!cleanedUp && !syncParams.collection.isReady()) {
            syncParams.markReady();
          }
        } finally {
          inFlight = null;
        }
      })();
      return inFlight;
    };

    refreshNow = fetchSnapshot;
    void fetchSnapshot();

    const connect = async () => {
      if (cleanedUp || !endpoint.wsPath) return;
      try {
        socket = await openLocalApiWebSocket(endpoint.wsPath);
      } catch {
        scheduleReconnect();
        return;
      }

      socket.onmessage = (event) => {
        if (cleanedUp) return;
        try {
          const message = JSON.parse(String(event.data)) as {
            JsonPatch?: Operation[];
          };
          if (!message.JsonPatch) return;
          applyWrites(syncParams, patchToWrites(message.JsonPatch, shape.table));
        } catch {
          // 收到无法解析的帧时忽略，下一次快照会纠正状态
        }
      };
      socket.onopen = () => {
        reconnectAttempt = 0;
      };
      socket.onclose = () => {
        if (!cleanedUp) scheduleReconnect();
      };
      socket.onerror = () => {
        socket?.close();
      };
    };

    function scheduleReconnect() {
      if (cleanedUp || reconnectTimer) return;
      const delay = Math.min(
        MAX_RECONNECT_DELAY_MS,
        1000 * Math.pow(2, reconnectAttempt)
      );
      reconnectAttempt += 1;
      reconnectTimer = globalThis.setTimeout(() => {
        reconnectTimer = null;
        void fetchSnapshot();
        void connect();
      }, delay);
    }

    void connect();

    // WS 覆盖不到的表（例如评论）靠轮询兜底。
    const pollId = endpoint.wsPath
      ? null
      : globalThis.setInterval(() => void fetchSnapshot(), POLL_INTERVAL_MS);

    return {
      cleanup: () => {
        cleanedUp = true;
        if (pollId) globalThis.clearInterval(pollId);
        if (reconnectTimer) globalThis.clearTimeout(reconnectTimer);
        socket?.close();
      },
      loadSubset: () => true,
    };
  };

  return createCollection({
    id: collectionId,
    getKey: (item: ElectricRow) => getRowKey(item),
    sync: { sync },
    ...mutationHandlers,
  } as never) as unknown as ReturnType<typeof createCollection> & {
    __rowType?: TRow;
  };
}
```

- [ ] **步骤 4：接到 createShapeCollection**

在 `packages/web-core/src/shared/lib/electric/collections.ts` 顶部加导入：

```ts
import { isLocalMode } from '@/shared/lib/local/dataSource';
import { createLocalShapeCollection } from '@/shared/lib/local/localCollections';
```

在 `createShapeCollection` 内、`const cached = collectionCache.get(collectionId);` 之后的 `if (cached) { ... }` 之后插入：

```ts
  if (isLocalMode()) {
    const localCollection = createLocalShapeCollection(
      collectionId,
      shape,
      params,
      config,
      mutation
    );
    collectionCache.set(collectionId, localCollection);
    return localCollection as typeof localCollection & { __rowType?: TRow };
  }
```

- [ ] **步骤 5：让 bulk 写接口在个人版走本地**

在 `packages/web-core/src/shared/lib/remoteApi.ts` 顶部加导入：

```ts
import { isLocalMode } from '@/shared/lib/local/dataSource';
import { makeLocalApiRequest } from '@/shared/lib/localApiTransport';
```

新增一个分发函数，并让 `bulkUpdateProjects` / `bulkUpdateIssues` / `bulkUpdateProjectStatuses` 三个函数改用它：

```ts
/** 个人版走 /api/local，团队版走云端 /v1。 */
async function makeDataRequest(
  remotePath: string,
  localPath: string,
  options: RequestInit
): Promise<Response> {
  if (isLocalMode()) {
    return makeLocalApiRequest(localPath, {
      ...options,
      headers: { 'Content-Type': 'application/json', ...(options.headers ?? {}) },
    });
  }
  return makeRequest(remotePath, options);
}
```

`bulkUpdateIssues` 改为：

```ts
export async function bulkUpdateIssues(
  updates: BulkUpdateIssueItem[]
): Promise<void> {
  const response = await makeDataRequest(
    '/v1/issues/bulk',
    '/api/local/issues/bulk',
    {
      method: 'POST',
      body: JSON.stringify({
        updates: updates.map((u) => ({ id: u.id, ...u.changes })),
      }),
    }
  );
  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.message || 'Failed to bulk update issues');
  }
}
```

`bulkUpdateProjects` 同理，远端 `/v1/projects/bulk` ↔ 本地 `/api/local/projects/bulk`；`bulkUpdateProjectStatuses` 远端 `/v1/project_statuses/bulk` ↔ 本地 `/api/local/project_statuses/bulk`。

（注意：`/api/local/projects/bulk` 在任务 6 中未实现。若 `bulkUpdateProjects` 在个人版被调用到，先在 `crates/server/src/routes/local_projects/projects.rs` 按 `statuses.rs::handle_bulk_update` 的写法补一个 `handle_bulk_update` 与 `POST /bulk` 路由，并补一条与状态列同样的「超过上限被拒绝」测试。）

- [ ] **步骤 6：运行测试与检查**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:test 2>&1 | tail -10 && pnpm run web-core:check
```

预期：`Tests 36 passed`（rows 9 + endpoints 13 + localCollections 14）；`tsc` 无输出。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core/src/shared/lib/local packages/web-core/src/shared/lib/electric/collections.ts \
        packages/web-core/src/shared/lib/remoteApi.ts crates/server/src/routes/local_projects
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "前端：个人版本地集合，REST 快照加 WS 增量与写后刷新" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 15：注入数据源与默认身份

**Files:**
- Modify: `packages/web-core/src/shared/lib/local/identity.ts`
- Modify: `packages/web-core/src/shared/hooks/auth/useAuth.ts`
- Modify: `packages/web-core/src/shared/hooks/useUserOrganizations.ts`
- Modify: `packages/local-web/src/app/entry/Bootstrap.tsx`

- [ ] **步骤 1：查清组织响应的字段**

```bash
cd /Users/admin/work/github/vibe-kanban && grep -n "ListOrganizationsResponse\|OrganizationWithRole = \|export type Organization = " -A 6 shared/types.ts | head -40
```

预期：看到 `Organization`、`OrganizationWithRole`、`ListOrganizationsResponse` 三个类型的确切字段。

- [ ] **步骤 2：在 identity.ts 补一个类型完整的组织响应**

在 `packages/web-core/src/shared/lib/local/identity.ts` 追加（字段按上一步查到的类型逐一填写，`pnpm run web-core:check` 会校验完整性）：

```ts
import type { ListOrganizationsResponse } from 'shared/types';

/**
 * 个人版的固定组织列表。字段以 shared/types.ts 的 ListOrganizationsResponse 为准；
 * 若类型检查报缺字段，按类型补齐，不要用 as unknown 绕过。
 */
export const LOCAL_ORGANIZATIONS_RESPONSE: ListOrganizationsResponse = {
  organizations: [
    {
      id: LOCAL_ORGANIZATION_ID,
      name: '本机',
      slug: 'local',
      role: 'owner',
      created_at: '1970-01-01T00:00:00Z',
      updated_at: '1970-01-01T00:00:00Z',
    },
  ],
};
```

- [ ] **步骤 3：让 useAuth 在个人版视为已登录**

`packages/web-core/src/shared/hooks/auth/useAuth.ts`：

```ts
import { useContext } from 'react';
import { createHmrContext } from '@/shared/lib/hmrContext';
import { isLocalMode } from '@/shared/lib/local/dataSource';
import { LOCAL_USER_ID } from '@/shared/lib/local/identity';

export interface AuthContextValue {
  isSignedIn: boolean;
  isLoaded: boolean;
  userId: string | null;
}

export const AuthContext = createHmrContext<AuthContextValue | undefined>(
  'AuthContext',
  undefined
);

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);

  // 个人版没有登录概念：固定返回已登录的本机用户，
  // 这样所有依赖 isSignedIn 的组件与 shape 订阅都能正常工作。
  if (isLocalMode()) {
    return { isSignedIn: true, isLoaded: true, userId: LOCAL_USER_ID };
  }

  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
```

- [ ] **步骤 4：让 useUserOrganizations 在个人版返回固定组织**

`packages/web-core/src/shared/hooks/useUserOrganizations.ts` 的 `queryFn` 改成：

```ts
    queryFn: () =>
      isLocalMode()
        ? Promise.resolve(LOCAL_ORGANIZATIONS_RESPONSE)
        : organizationsApi.getUserOrganizations(),
```

并加导入：

```ts
import { isLocalMode } from '@/shared/lib/local/dataSource';
import { LOCAL_ORGANIZATIONS_RESPONSE } from '@/shared/lib/local/identity';
```

- [ ] **步骤 5：在 Bootstrap 注入数据源**

`packages/local-web/src/app/entry/Bootstrap.tsx`，在 `configureAuthRuntime({ ... });` 之前插入：

```tsx
// 数据源：显式设置 VITE_VK_DATA_SOURCE 时以它为准；
// 否则没有配置云端基址（VITE_VK_SHARED_API_BASE 为空）就走个人版。
const dataSourceEnv = import.meta.env.VITE_VK_DATA_SOURCE as
  | 'local'
  | 'remote'
  | undefined;
configureDataSource(
  dataSourceEnv ?? (import.meta.env.VITE_VK_SHARED_API_BASE ? 'remote' : 'local')
);
```

并在导入区加：

```tsx
import { configureDataSource } from '@/shared/lib/local/dataSource';
```

- [ ] **步骤 6：类型检查与 lint**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check && pnpm run local-web:check && pnpm run local-web:lint 2>&1 | tail -10
```

预期：三条命令都无错误输出。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core/src/shared/lib/local/identity.ts \
        packages/web-core/src/shared/hooks/auth/useAuth.ts \
        packages/web-core/src/shared/hooks/useUserOrganizations.ts \
        packages/local-web/src/app/entry/Bootstrap.tsx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "前端：注入数据源开关与个人版默认身份" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 16：恢复看板界面与导航

**Files:**
- Modify: `packages/web-core/src/pages/kanban/ProjectKanban.tsx`
- Modify: `packages/web-core/src/pages/kanban/LocalProjectKanban.tsx`
- Modify: `packages/web-core/src/pages/root/RootRedirectPage.tsx`

- [ ] **步骤 1：还原两个看板入口**

```bash
cd /Users/admin/work/github/vibe-kanban
git show 97123d52^:packages/web-core/src/pages/kanban/ProjectKanban.tsx \
  > packages/web-core/src/pages/kanban/ProjectKanban.tsx
git show 97123d52^:packages/web-core/src/pages/kanban/LocalProjectKanban.tsx \
  > packages/web-core/src/pages/kanban/LocalProjectKanban.tsx
```

- [ ] **步骤 2：确认还原结果**

```bash
cd /Users/admin/work/github/vibe-kanban && git diff --stat packages/web-core/src/pages/kanban/
```

预期：`ProjectKanban.tsx` 约 +226/-... 行，`LocalProjectKanban.tsx` 约 +25 行。

- [ ] **步骤 3：保留停服页作为团队版降级**

```bash
cd /Users/admin/work/github/vibe-kanban && ls packages/web-core/src/pages/kanban/ProjectSunsetPage.tsx
```

预期：文件仍存在。还原后的 `ProjectKanban.tsx` 不再引用它，这是预期的——它由团队版云端不可用时的其他入口继续使用。若 `pnpm run local-web:lint` 报「未使用的文件」类告警，忽略；若报未使用导入，删掉该导入即可。

- [ ] **步骤 4：类型检查**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check
```

预期：无输出。若报找不到 `ProjectRightSidebarContainer` 等模块，用

```bash
cd /Users/admin/work/github/vibe-kanban && ls packages/web-core/src/pages/kanban/
```

确认文件仍在原位；若路径变过，按现状修正 import 路径，不要重写组件。

- [ ] **步骤 5：调整根跳转**

`packages/web-core/src/pages/root/RootRedirectPage.tsx` 的 `useEffect` 中，把

```tsx
      if (loginStatus?.status !== 'loggedin') {
        appNavigation.goToWorkspacesCreate({ replace: true });
        return;
      }
```

改成

```tsx
      // 个人版没有登录态，直接进项目；无项目时下面的 destination 为空，
      // 会落到创建工作区页，用户从那里新建项目。
      if (!isLocalMode() && loginStatus?.status !== 'loggedin') {
        appNavigation.goToWorkspacesCreate({ replace: true });
        return;
      }
```

并加导入：

```tsx
import { isLocalMode } from '@/shared/lib/local/dataSource';
```

- [ ] **步骤 6：类型检查与 lint**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check && pnpm run local-web:check && pnpm run local-web:lint 2>&1 | tail -10
```

预期：都通过。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core/src/pages/kanban/ProjectKanban.tsx \
        packages/web-core/src/pages/kanban/LocalProjectKanban.tsx \
        packages/web-core/src/pages/root/RootRedirectPage.tsx
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "前端：恢复看板入口并调整个人版根跳转" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 17：端到端手动验收与文档

**Files:**
- Create: `docs/personal-mode.mdx`
- Modify: `docs/docs.json`
- Modify: `CLAUDE.md`

- [ ] **步骤 1：启动本地开发环境**

```bash
cd /Users/admin/work/github/vibe-kanban
export FRONTEND_PORT=$(node scripts/setup-dev-environment.js frontend)
export BACKEND_PORT=$(node scripts/setup-dev-environment.js backend)
export PREVIEW_PROXY_PORT=$(node scripts/setup-dev-environment.js preview_proxy)
export VK_ALLOWED_ORIGINS="http://localhost:$FRONTEND_PORT"
cargo run --bin server
```

另开一个终端：

```bash
cd /Users/admin/work/github/vibe-kanban
export FRONTEND_PORT=$(node scripts/setup-dev-environment.js frontend)
pnpm run local-web:dev
```

预期：后端打印监听端口；前端打印 `Local: http://localhost:<FRONTEND_PORT>/`。

- [ ] **步骤 2：验证接口层（不开浏览器）**

```bash
cd /Users/admin/work/github/vibe-kanban
BACKEND_PORT=$(node scripts/setup-dev-environment.js backend)
curl -s -X POST "http://localhost:$BACKEND_PORT/api/local/projects" \
  -H 'Content-Type: application/json' \
  -d '{"organization_id":"00000000-0000-0000-0000-000000000001","name":"Vibe Kanban","color":"#6366f1"}'
echo
curl -s "http://localhost:$BACKEND_PORT/api/local/projects"
echo
```

预期：第一条输出 `{"txid":0}`；第二条输出 `{"projects":[{"id":"...","organization_id":"00000000-0000-0000-0000-000000000001","name":"Vibe Kanban",...}]}`。

```bash
PROJECT_ID=$(curl -s "http://localhost:$BACKEND_PORT/api/local/projects" | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>console.log(JSON.parse(s).projects[0].id))")
curl -s "http://localhost:$BACKEND_PORT/api/local/project_statuses?project_id=$PROJECT_ID"
echo
curl -s "http://localhost:$BACKEND_PORT/api/local/issues?project_id=$PROJECT_ID"
echo
curl -s "http://localhost:$BACKEND_PORT/api/local/issue_assignees"
echo
```

预期：状态列 5 条且 `stage_type` 依次为 backlog/todo/dev/review/done；需求为 `{"issues":[]}`；空集合为 `{"issue_assignees":[]}`。

- [ ] **步骤 3：走一遍界面验收清单**

在浏览器打开 `http://localhost:$FRONTEND_PORT/`，逐项打勾：

- [ ] 未登录直接进入项目列表（或项目看板），不出现登录提示
- [ ] 新建项目后看板显示 5 个默认状态列
- [ ] 新建需求，卡片显示 `VK-1` 编号
- [ ] 把卡片从「待开发」拖到「开发中」，刷新页面后位置保持
- [ ] 打开需求详情，写一条评论并保存，刷新后仍在
- [ ] 在需求详情里「创建工作区」，选择仓库与编码智能体并启动
- [ ] 工作区启动后回到看板，该需求自动出现在「开发中」
- [ ] 工作区详情顶部显示需求编号与标题，点击可跳回需求
- [ ] 在工作区里执行本地合并，需求自动进入「已完成」且显示完成时间
- [ ] 打开第二个浏览器标签页同看同一个看板，在 A 页改需求标题，B 页 1 秒内更新
- [ ] 断开后端（Ctrl+C）后前端提示同步错误；重启后端后 8 秒内自动恢复
- [ ] 删除一个绑定了工作区的需求，工作区仍在列表中且不再显示需求信息

- [ ] **步骤 4：写文档**

创建 `docs/personal-mode.mdx`：

```mdx
---
title: "Personal mode"
description: "Run the full issue-to-merge flow on one machine, with no server, no login and no cloud."
---

Personal mode stores projects, issues and comments in the local SQLite database and serves them from `/api/local/*`. No Docker, Postgres or ElectricSQL is required.

## Prerequisites

- Node.js 20 or newer and pnpm 10
- A Rust toolchain matching `rust-toolchain.toml`

## Start the app

<Steps>
<Step title="Assign ports">
  ```bash
  export FRONTEND_PORT=$(node scripts/setup-dev-environment.js frontend)
  export BACKEND_PORT=$(node scripts/setup-dev-environment.js backend)
  export PREVIEW_PROXY_PORT=$(node scripts/setup-dev-environment.js preview_proxy)
  export VK_ALLOWED_ORIGINS="http://localhost:$FRONTEND_PORT"
  ```
</Step>

<Step title="Run the backend">
  ```bash
  cargo run --bin server
  ```
</Step>

<Step title="Run the web app">
  ```bash
  pnpm run local-web:dev
  ```

  <Check>
  Open `http://localhost:$FRONTEND_PORT/`. You land on the board without signing in.
  </Check>
</Step>
</Steps>

## Choose the data source

The web app picks its data source at boot:

| `VITE_VK_DATA_SOURCE` | `VITE_VK_SHARED_API_BASE` | Mode |
|---|---|---|
| `local` | any | Personal (local SQLite) |
| `remote` | any | Team (ElectricSQL) |
| unset | empty | Personal |
| unset | set | Team |

## Status automation

Each project starts with five status columns. The board maps them to pipeline stages through `stage_type`, so renaming a column keeps the automation working.

| Stage | Default column | Moves the issue when |
|---|---|---|
| `backlog` | 待规划 | manual only |
| `todo` | 待开发 | manual only |
| `dev` | 开发中 | you create a workspace from the issue |
| `review` | 待评审 | a pull request is opened |
| `done` | 已完成 | the branch is merged, locally or through the PR |

## Limits

- Board snapshots return at most 2000 issues per project. Beyond that, use the search endpoint with `limit` and `offset`.
- Assignees, followers, notifications and comment reactions are not stored in personal mode; those collections return empty arrays.
```

在 `docs/docs.json` 的导航里，把 `"personal-mode"` 加到与现有本地开发文档同一分组（用 `grep -n '"pages"' -A 8 docs/docs.json` 找到合适位置）。

- [ ] **步骤 5：更新仓库约定**

在 `CLAUDE.md` 的 `## Build, Test, and Development Commands` 中追加两行：

```markdown
- Web 数据层测试: `pnpm run web-core:test`（Vitest，位于 `packages/web-core`）
- 个人版（本地需求链路）：接口在 `/api/local/*`，实时流在 `/api/issues/streams/ws`，文档见 `docs/personal-mode.mdx`
```

- [ ] **步骤 6：校验文档**

```bash
cd /Users/admin/work/github/vibe-kanban && node -e "JSON.parse(require('fs').readFileSync('docs/docs.json','utf8'));console.log('docs.json 合法')"
```

预期：输出 `docs.json 合法`。

- [ ] **步骤 7：提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add docs/personal-mode.mdx docs/docs.json CLAUDE.md
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：补充使用文档与仓库约定" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 任务 18：全量门禁与合并

**Files:** 无新增

- [ ] **步骤 1：格式化**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run format
```

预期：无报错；`git status --short` 可能出现格式化改动。

- [ ] **步骤 2：sqlx 离线缓存校验**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run prepare-db:check
```

预期：以 `query data is up-to-date` 或同义信息结束。若失败，先跑 `pnpm run prepare-db` 再提交 `crates/db/.sqlx`。

- [ ] **步骤 3：生成类型校验**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run generate-types:check
```

预期：无差异报错。

- [ ] **步骤 4：Rust 测试与 lint**

```bash
cd /Users/admin/work/github/vibe-kanban && cargo test --workspace 2>&1 | tail -20
pnpm run lint 2>&1 | tail -20
```

预期：`cargo test` 全绿；`pnpm run lint` 无 error（含 `cargo clippy -- -D warnings` 与 i18n 检查）。

- [ ] **步骤 5：全量类型检查（含 Vitest）**

```bash
cd /Users/admin/work/github/vibe-kanban && pnpm run check 2>&1 | tail -20
```

预期：全部通过，最后一段是 Vitest 的 `Tests 36 passed`。

- [ ] **步骤 6：提交格式化产物**

```bash
cd /Users/admin/work/github/vibe-kanban && git status --short
git add -A
git -c user.name="jk-ai-automation" -c user.email="yixizidi@gmail.com" \
  commit -m "个人版：格式化与门禁修正" \
         -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

（若 `git status --short` 为空，跳过本步。）

- [ ] **步骤 7：开分支并提 PR**

```bash
cd /Users/admin/work/github/vibe-kanban
git checkout -b feat/personal-issue-flow
git push -u origin feat/personal-issue-flow
gh pr create --title "个人版需求链路（P2 第一期）" --body "$(cat <<'EOF'
## 做了什么

在不依赖 Docker、Postgres、ElectricSQL 与云端的前提下，打通本机的「建需求 → 看板推进 → 从需求创建工作区 → 合并 → 需求自动完成」。

- 本地 SQLite 新增 local_projects / project_statuses / issues / project_tags / issue_tags / issue_comments，以及 workspaces.issue_id
- 新增 /api/local/* REST 接口，路径与 JSON 与云端 /v1 同构
- 需求变更复用 SQLite 钩子推 JSON Patch，新增 /api/issues/streams/ws
- 前端在 createShapeCollection 加数据源开关：个人版走 REST 快照 + WS 增量 + 写后刷新
- 恢复看板入口，注入固定组织与用户
- 需求状态按 project_statuses.stage_type 自动流转

## 测试

- `cargo test --workspace`
- `pnpm run check`（含新引入的 `pnpm run web-core:test`）
- `pnpm run lint`
- 手动端到端验收清单见 `docs/superpowers/plans/2026-09-16-personal-issue-flow.md` 任务 17

## 兼容性

团队版（remote 模式）代码未删除，仅在个人版旁路；数据源默认按 `VITE_VK_SHARED_API_BASE` 是否配置推断。

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

预期：输出 PR 链接。

---

## 自查

### 1. 设计文档逐节覆盖

| 设计章节 | 对应任务 |
|---|---|
| §3 范围 1：本地需求存储（6 张表） | 任务 1、2、3、4、5 |
| §3 范围 2：本地 REST 接口，复用 api-types | 任务 6、7、8 |
| §3 范围 3：需求列表实时推送 | 任务 9 |
| §3 范围 4：恢复看板，数据源切到本地 | 任务 13、14、16 |
| §3 范围 5：需求 ↔ 工作区打通 | 任务 10、11 |
| §3 范围 6：模式开关 | 任务 13（`dataSource.ts`）、任务 15（Bootstrap 注入） |
| §5 数据模型（表与字段、id 第 0 列） | 任务 1（迁移 + 列顺序断言测试） |
| §5 `issue_number` 自增、`simple_id` 前缀规则 | 任务 2（`simple_id_prefix`）、任务 4（编号分配与并发测试） |
| §5 新建项目自动创建 5 个默认状态列 | 任务 2 |
| §5 指派/关注/通知/表情不建表，前端集合返回空数组 | 任务 8（`EMPTY_TABLES`）、任务 13（`kind: 'empty'`） |
| §6 项目接口 | 任务 6 |
| §6 状态列接口（含 `/bulk`） | 任务 7 |
| §6 需求接口（含 `/bulk`、`/search`） | 任务 7 |
| §6 标签、需求标签、评论接口 | 任务 8 |
| §6 空集合接口 | 任务 8 |
| §6 `pull_requests` 与 `workspaces` 投影 | 任务 8 |
| §6 快照 `{ "<表名>": [...] }`、写返回 `{ "txid": 0 }` | 任务 6（`snapshot()` / `txid()`）+ 任务 6、7、8 的断言 |
| §7.1 恢复看板、保留停服页 | 任务 16 |
| §7.2 数据源开关（`local` / `remote`，本地不带 Bearer、基址 `/api/local`） | 任务 13、14（走 `makeLocalApiRequest`，不经 `remoteApi.makeRequest`） |
| §7.3 写成功后立即刷新集合 | 任务 14（`buildLocalMutationHandlers` 的 `refresh` 与其测试） |
| §7.4 固定默认组织与用户 | 任务 15 |
| §7.5 `RootRedirectPage` 个人版进项目 | 任务 16 |
| §8.1 从需求创建工作区写 `workspaces.issue_id`，附件走本地 | 任务 10 |
| §8.2 工作区回显需求 | 任务 10（`Workspace.issue_id` 上到 `shared/types.ts`）+ 任务 17 验收清单第 8 项 |
| §8.3 状态自动流转与 `stage_type` | 任务 3（列与枚举）、任务 10（dev）、任务 11（review / done） |
| §9 事件白名单加 3 张表、新增 WS、前端增量更新 | 任务 9（后端）、任务 14（`patchToWrites`） |
| §10 风险：乐观更新语义 | 任务 14（写后刷新；失败抛错由 TanStack DB 回滚） |
| §10 风险：拖拽批量事务 | 任务 4（`bulk_update` 单事务 + 回滚测试）、任务 14（多条合并成一次 `/bulk`） |
| §10 风险：全量快照卡顿 | 任务 4（`MAX_SNAPSHOT_ROWS` = 2000、`MAX_PAGE_SIZE` = 500）、任务 7（1000 条性能断言）、任务 17 文档「Limits」 |
| §10 风险：与云端结构漂移 | 任务 6-8 全部复用 `api-types` DTO；任务 8 步骤 7 比对 shape 表名清单 |
| §10 风险：遗留表混淆 | 任务 1（迁移注释 + 「遗留表未被删除」测试） |
| §10 风险：`id` 第 0 列 | 任务 1（测试断言） |
| §11 验收 1：端到端流程 | 任务 17 |
| §11 验收 2：双标签页 1 秒内同步 | 任务 17 清单第 10 项 |
| §11 验收 3：测试与门禁 | 任务 18 |
| §11 验收 4：团队版不回退 | 任务 13（默认 `remote`）、任务 14（分支旁路）、任务 18（`pnpm run check` 含 remote-web） |

### 2. 安全与健壮性预置的落点

| 要求 | 落点 |
|---|---|
| SQL 注入（参数化） | 全部查询用 `sqlx::query_as!` 或 `bind`；任务 6「名称里的单引号不会破坏 SQL」、任务 4「`%` 按字面量匹配」 |
| 路径穿越 | 路径参数一律 `Path<Uuid>`；任务 13「参数值被 URL 编码」测试 |
| 跨项目误删/越权 | 任务 4「按项目列出时不返回其他项目的需求」、任务 7「跨项目状态列被拒绝」、任务 3「列出状态列不会串到别的项目」、任务 6「删除只影响目标项目」、任务 8「需求标签跨项目不可见」 |
| 大字段 | `MAX_TITLE_LEN` / `MAX_DESCRIPTION_LEN` / `MAX_COMMENT_LEN` / `MAX_TAG_NAME_LEN` / `MAX_PROJECT_NAME_LEN` / `MAX_STATUS_NAME_LEN`，任务 4、5 各有截断测试 |
| 并发写入 | 任务 4「并发更新不同字段互不覆盖」（单语句 CASE 更新，杜绝读改写竞态） |
| 拖拽批量事务性 | 任务 3、4 的「全成或全败」测试；任务 14「多条更新合并成一次 `/bulk`」 |
| 需求删除与工作区 SET NULL | 任务 1「workspaces.issue_id 外键为 SET NULL」、任务 4「删除需求把工作区的 issue_id 置空」 |
| `simple_id` / `issue_number` 并发安全 | 任务 4「并发建需求不会产生重复编号」（UNIQUE + 事务内重试） |
| WS/快照不泄露本地路径 | 任务 7「快照不包含任何本地文件路径字段」、任务 8「工作区投影不泄露本地路径」、任务 9「需求 patch 不包含任何本地路径信息」 |
| 分页与超大列表 | 任务 4「搜索限制单页上限并返回总数」、任务 7「一千条需求的快照能在合理时间内返回」、`MAX_BULK_UPDATES` = 1000 及两处「超过上限被拒绝」 |

### 3. 跨任务命名与签名一致性

- `DEFAULT_ORGANIZATION_ID` / `DEFAULT_USER_ID`（任务 2 定义）在任务 4、5、6、8、10、11 的测试与实现中一致引用；前端对应常量 `LOCAL_ORGANIZATION_ID` / `LOCAL_USER_ID`（任务 13）取值相同，任务 15 的组织响应复用它们。
- `StageType`（任务 3）被任务 4 的 `Issues::move_to_stage`、任务 10 的建工作区分支、任务 11 的 `IssueFlowStage::from` 一致使用；`IssueFlowStage` 只在 services 层出现，不与 `StageType` 混用。
- `LocalProjectStatus`（任务 3）在任务 8 的快照、任务 9 的 `RecordTypes::ProjectStatus` 与 `project_status_patch` 中签名一致；任务 9 步骤 4 补的 `TS` derive 是它唯一的后续改动。
- `Issues` / `ProjectStatuses` / `LocalProjects` / `ProjectTags` / `IssueTags` / `IssueComments` 六个门面类型名在任务 2-5 定义、任务 6-11 引用，无别名。
- `Issues::find_by_rowid`、`ProjectStatuses::find_by_rowid`、`IssueComments::find_by_rowid` 三个方法在任务 4、3、5 定义，任务 9 的更新钩子按同名调用。
- `Workspace::set_issue_id(pool, workspace_id, issue_id)` 在任务 10 定义，任务 11 的测试与任务 10 的路由分支签名一致。
- `snapshot(table, rows)` / `txid()` / `map_issue_error(err)` 在任务 6 的 `mod.rs` 定义，任务 7、8 一致引用。
- `ProjectScopedQuery` / `IssueScopedQuery` / `BulkUpdateRequest<T>` / `BulkUpdateItem<T>` / `MAX_BULK_UPDATES` 在任务 7 的 `mod.rs` 定义，任务 7、8 一致引用。
- 前端 `getRowKey` / `extractFallbackRows` / `parseResponseError` 在任务 12 迁到 `rows.ts`，任务 14 从该模块导入，`collections.ts` 不再各留一份。
- `resolveLocalShapeEndpoint` / `resolveLocalMutationUrl` / `LocalShapeEndpoint` 在任务 13 定义，任务 14 引用；`isLocalMode` 在任务 13 定义，任务 14、15、16 引用。
- `buildLocalMutationHandlers` / `patchToWrites` / `createLocalShapeCollection` 在任务 14 定义并同文件测试。
- 响应表名：后端 `snapshot("tags", ...)`（任务 8）与前端 `REST_RESOURCE.tags`（任务 13）、`ShapeDefinition.table === 'tags'` 三者一致；数据库表 `project_tags` 只在 SQL 层出现。

### 4. 未覆盖项与理由

| 未覆盖 | 理由 |
|---|---|
| 指派、关注、通知、评论表情 | 设计 §3「第一期不做」，接口按 §6 返回空数组（任务 8）。 |
| 需求关联关系（`issue_relationships`） | 同上，返回空数组。 |
| 附件云存储 | 设计 §3「第一期不做」；个人版用既有本地附件表（任务 10 保留 `attachment_ids` 原逻辑）。 |
| 团队版认证改造、多 Git 服务器、测试流水线与 AI 测试整合 | 设计 §3 明确排除，属路线图后续阶段。 |
| 需求搜索的全文索引与 1000 条以上的虚拟滚动优化 | 设计 §10 标注为「后续优化项」；本期给出 `MAX_SNAPSHOT_ROWS` / `MAX_PAGE_SIZE` 上限与性能断言，并在 `docs/personal-mode.mdx` 的 Limits 写明。 |
| 前端界面级自动化测试（组件 / E2E） | 仓库无 Playwright/Testing Library 基建，设计 §11 只要求「前端数据层有轻量测试」；本期引入 Vitest 覆盖数据层，界面用任务 17 的手动清单。 |
| `crates/remote` 的任何改动 | 该 crate 被 workspace `exclude`，本机无 Docker/Postgres 无法验证；本期所有改动都绕开它（不改 `api-types` 已有字段，只加 `Default` derive）。 |
| `project_statuses.stage_type` 暴露给 TypeScript | 会牵动 `shared/remote-types.ts` 与远端实现；个人版的状态流转全在后端完成，前端不需要该字段（详见下方「歧义与默认选择」）。 |

### 5. 设计中的歧义与本计划采用的默认选择

1. **「沿用现有 `tags` 表」与云端 `Tag` 结构冲突。** 本地既有 `tags` 表是 `(id, tag_name, content)` 的提示词片段表，与 `api_types::tag::Tag` 的 `(id, project_id, name, color)` 完全不同。默认选择：新建 `project_tags` 表，HTTP 响应仍以 `tags` 作为 key（前端 `ShapeDefinition.table` 决定 key，与数据库表名无关），既不动既有功能也不破坏前端契约。
2. **`project_statuses.stage_type` 放在哪一层。** 设计说「新增列」，但 `api_types::ProjectStatus` 被 `crates/remote` 共用，加字段会强制改远端。默认选择：列加在本地表，Rust 侧用本地专有的 `LocalProjectStatus` 携带它，TS 类型不变；多出的 JSON 字段前端原样忽略。
3. **`linked_issue` 如何区分本地与云端。** 设计说「增加本地分支」，但没说请求体怎么变。默认选择：不改 `LinkedIssueInfo`，后端用 `Issues::find_by_id` 探测 `issue_id` 是否是本地需求——命中即本地分支，未命中走原有云端逻辑。这样 `shared/types.ts` 不变，团队版零影响。
4. **`workspaces` 集合的 `owner_user_id` 维度。** 云端有 `USER_WORKSPACES_SHAPE`（按 `owner_user_id`），个人版没有跨项目的工作区-需求视图。默认选择：该 shape 在个人版返回空集合，项目维度的 `PROJECT_WORKSPACES_SHAPE` 走投影接口。
5. **评论的实时推送粒度。** 设计 §9 要求 `issue_comments` 进白名单，但 WS 是按 `project_id` 订阅、评论行没有 `project_id`。默认选择：评论变更照常进白名单并推 patch（跨标签页可见），但前端评论集合不绑 WS，靠「写后刷新 + 15 秒轮询」兜底；需求与状态列走 WS 实时通道。
6. **状态自动流转是否允许倒退。** 设计没说「已完成的需求再开工作区」该怎样。默认选择：流转始终生效（幂等、可预测），从 `done` 回到 `dev` 时清空 `completed_at`；这一行为在任务 4 与任务 11 各有测试固定。
7. **删除状态列时其下需求怎么办。** 设计未提。默认选择：拒绝删除并返回 409，提示先把需求移走——比静默级联删除需求安全得多。
8. **`txid` 的取值。** 设计说返回 `{ "txid": 0 }`。前端 `buildMutationHandlers` 在 Electric 模式下会用 txid 对账，但个人版走的是任务 14 的独立 handler，根本不读 txid，`0` 是纯占位。计划保留 `0` 以与设计一致，并在 `mod.rs` 注释写明原因。
9. **数据源默认值。** 设计只说「个人版（默认）」。默认选择：`VITE_VK_DATA_SOURCE` 显式优先；未设置时按 `VITE_VK_SHARED_API_BASE` 是否为空推断。这样既让本机开箱即个人版，又不会让已配置云端的部署被静默切走。
