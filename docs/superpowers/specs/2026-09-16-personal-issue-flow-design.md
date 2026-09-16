# 个人版需求链路设计（P2 第一期）

> 状态：设计中｜日期：2026-09-16
> 上级方案：`docs/superpowers/specs/2026-09-15-vibe-kanban-optimization-roadmap.md`

## 1. 目标

在**不依赖 Docker、Postgres、ElectricSQL 与云端**的前提下，让一个人在本机打通：

```
建需求 → 看板上推进 → 一键从需求创建工作区并启动编码智能体 → 查看代码变更与评审 → 合并 → 需求自动完成
```

这是路线图中 P2「个人版」的第一期，优先于团队版（团队版需要 Postgres 与 ElectricSQL，本机暂无 Docker）。

## 2. 现状要点（调查结论）

| 事实 | 影响 |
|---|---|
| 停服提交 `97123d52` 只改了 2 个前端文件，看板、需求面板等组件（约 2800 行）完好 | 恢复界面成本极低 |
| 前端数据层 `createShapeCollection` 内置「Electric 不可用时改用 HTTP 轮询」的后备路径 | 个人版可复用该路径，不必重写数据层 |
| 需求数据只在云端（Postgres + Electric），本地无存储 | 需要新建本地表与接口 |
| 本地遗留 `projects`/`tasks` 表已无人读写，`workspaces.task_id` 是废弃列 | 不复活旧表，新建与云端同构的表 |
| 共享类型 `api-types` 已定义 Issue/Project/ProjectStatus 等，前端类型由此生成 | 本地接口复用同一套类型，前端类型零改动 |
| 本地已有数据库变更钩子 + WebSocket JSON Patch 推送（工作区列表在用） | 需求列表复用同一机制做实时刷新 |
| 本地曾有 `routes/tasks.rs`、`projects.rs`，于 `139b1815` 删除 | 可从 git 历史取回作参考 |
| 云端代理 `/api/remote/*` 纯转发、无缓存，云端不可用即不可用 | 个人版完全绕开它 |

## 3. 范围

**第一期做**
1. 本地需求存储：`projects`、`project_statuses`、`issues`、`tags`、`issue_tags`、`issue_comments`。
2. 本地 REST 接口：与云端同构（路径、请求体、响应体），复用 `api-types`。
3. 需求列表实时推送：复用数据库钩子 + WebSocket JSON Patch。
4. 恢复看板界面，数据源切换到本地。
5. 需求 ↔ 工作区打通：从需求创建工作区、工作区回显需求、合并后需求自动完成。
6. 模式开关：个人版（默认，无登录）/ 团队版（连云端，保持现状）。

**第一期不做**：团队版认证改造、多 Git 服务器、指派与关注、通知、附件云存储、测试流水线与 AI 测试整合、需求搜索与分页优化。

## 4. 架构

```
浏览器
  └─ createShapeCollection（数据源开关）
       ├─ 个人版：REST 快照 + WebSocket JSON Patch  ──►  本地服务 /api/local/*
       └─ 团队版：ElectricSQL（现状）              ──►  云端 /v1/*
本地服务（crates/server）
  ├─ routes/local_projects/*     项目、状态列、需求、标签、评论
  ├─ routes/workspaces/create.rs 支持 linked_issue = 本地需求
  └─ EventService                issues 表变更 → JSON Patch → WS
本地 SQLite（crates/db）
  └─ projects / project_statuses / issues / tags / issue_tags / issue_comments / workspaces.issue_id
```

## 5. 数据模型

新建迁移（与云端同构，SQLite 化：UUID 存 `BLOB`、时间存 `TEXT`、JSON 存 `TEXT`；**`id` 必须是第 0 列**，变更钩子依赖该约定）。

| 表 | 关键字段 |
|---|---|
| `local_projects` | id、name、color、sort_order、created_at、updated_at |
| `project_statuses` | id、project_id、name、color、sort_order、hidden |
| `issues` | id、project_id、issue_number、simple_id、status_id、title、description、priority、start_date、target_date、completed_at、sort_order、parent_issue_id、parent_issue_sort_order、extension_metadata、created_at、updated_at |
| `issue_tags` | issue_id、tag_id |
| `issue_comments` | id、issue_id、body、created_at、updated_at |
| `workspaces` | 新增 `issue_id BLOB REFERENCES issues(id) ON DELETE SET NULL` |

说明：
- 沿用现有 `tags` 表（本地已有）。
- 遗留 `projects`/`tasks` 表保持不动（不读不写），本期不删，避免影响历史数据；新表名为 `local_projects` 以避免与遗留表冲突。
- `issue_number` 按项目自增，`simple_id` 形如 `VK-12`，前缀取项目名首字母（最多 3 个大写字母，缺省 `ISS`）。
- 新建项目时自动创建 5 个默认状态列：待规划、待开发、开发中、待评审、已完成（对应后续流程阶段，可改名与增删）。
- 指派、关注、通知、评论表情在个人版不建表；前端相关集合返回空数组。

## 6. 本地接口

挂在 `/api/local`，路径与云端 `/v1` 同构，DTO 复用 `api-types`：

| 方法与路径 | 说明 |
|---|---|
| `GET /api/local/projects`、`POST`、`GET/PATCH/DELETE /projects/{id}` | 项目 |
| `GET /api/local/project_statuses?project_id=`、`POST`、`PATCH/DELETE /{id}`、`POST /bulk` | 状态列 |
| `GET /api/local/issues?project_id=`、`POST`、`GET/PATCH/DELETE /{id}`、`POST /bulk`、`POST /search` | 需求 |
| `GET /api/local/tags?project_id=`、`POST`、`DELETE /{id}` | 标签 |
| `GET /api/local/issue_tags?project_id=`、`POST`、`DELETE /{id}` | 需求标签关联 |
| `GET /api/local/issue_comments?issue_id=`、`POST`、`PATCH/DELETE /{id}` | 评论 |
| `GET /api/local/issue_assignees`、`issue_followers`、`issue_relationships`、`pull_request_issues`、`notifications`、`users`、`organization_member_metadata` | 个人版返回空数组，保证前端集合可用 |
| `GET /api/local/pull_requests?project_id=` | 由本地 `pull_requests` 表按工作区 → 需求反查 |
| `GET /api/local/workspaces?project_id=` | 由本地 `workspaces` 表投影为云端 Workspace 行结构 |

快照响应统一为 `{ "<表名>": [ ...行 ] }`，与前端后备路径的解析约定一致。写操作返回 `{ "txid": 0 }`（个人版不使用 Electric 事务对账）。

## 7. 前端

1. **恢复看板**：把 `ProjectKanban.tsx`、`LocalProjectKanban.tsx` 还原到 `97123d52^`；`ProjectSunsetPage.tsx` 保留为团队版云端停服时的降级页。
2. **数据源开关**：新增运行时配置（类似现有的鉴权运行时），值为 `local` 或 `remote`。
   - `local`：集合走「REST 快照 + WS 增量」；请求不带 Bearer、基址为本地 `/api/local`。
   - `remote`：保持现状。
3. **写操作**：沿用集合的 mutation handler，指向本地写接口；**写成功后立即刷新该集合**（不再等 30 秒轮询），并依赖 WS 推送做跨标签页同步。
4. **单人身份**：注入固定的默认组织与默认用户（`id` 固定、名称「本机」），使依赖组织/成员/创建人的组件正常渲染；设置里的组织与云端项目分区在个人版隐藏。
5. **导航**：`RootRedirectPage` 在个人版进入项目列表（无项目时进入新建项目）。

## 8. 需求与工作区打通

1. **从需求创建工作区**：`POST /api/workspaces/start` 的 `linked_issue` 增加本地分支——写入 `workspaces.issue_id`，附件从本地附件表取（不再调用云端）。
2. **工作区回显需求**：工作区详情与列表显示 `simple_id` 与标题，可跳转需求。
3. **状态自动流转**（个人版本地实现）：
   - 从需求创建工作区 → 需求移到「开发中」；
   - 创建 PR 或本地合并完成 → 需求移到「待评审」；
   - PR 合并或本地合并 → 需求移到「已完成」并写 `completed_at`。
   - 状态映射通过 `project_statuses.stage_type`（新增列：`backlog`/`todo`/`dev`/`review`/`done`）识别，用户改名不影响自动流转。

## 9. 实时刷新

- `EventService` 的表白名单增加 `issues`（以及 `project_statuses`、`issue_comments`）。
- 新增 `/api/issues/streams/ws?project_id=`，输出 JSON Patch，前端复用现有 `useJsonPatchWsStream`。
- 前端集合在收到 patch 时做增量更新，避免整表重建。

## 10. 风险与对策

| 风险 | 对策 |
|---|---|
| 乐观更新语义变化（原由 Electric 事务对账） | 写成功后立即重拉该集合；写失败回滚并提示 |
| 拖拽排序需要批量事务更新 | `POST /issues/bulk` 在一个事务内更新 `sort_order` |
| 需求较多时全量快照卡顿 | 首期全量，WS 增量兜底；超过 1000 条时在计划中标记为后续优化项 |
| 新表与云端结构漂移 | DTO 复用 `api-types`；接口契约测试对齐两端行结构 |
| 遗留表与新表并存造成混淆 | 新表命名 `local_projects`，并在迁移注释中写明遗留表已废弃 |
| 变更钩子要求 `id` 为第 0 列 | 迁移中固定列顺序，并加测试断言 |

## 11. 验收标准

1. 本机执行 `pnpm run dev`，不联网、不登录，可完成：新建项目 → 新建需求 → 拖拽改状态 → 从需求创建工作区并启动编码智能体 → 查看变更 → 合并 → 需求自动变为「已完成」。
2. 两个浏览器标签页同时打开看板，一侧改动后另一侧 1 秒内更新。
3. `cargo test --workspace` 与 `pnpm run check`、`pnpm run lint` 通过；新增后端逻辑有单元测试，前端数据层有轻量测试。
4. 团队版（`remote` 模式）行为不回退：现有云端链路代码不删除，仅在个人版旁路。
