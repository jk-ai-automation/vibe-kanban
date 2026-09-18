# 交付流水线后端引擎与基础设施 实施计划（计划 A）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在本地后端落地「需求 → 规格 → 用例 → 开发 → 评审 → 测试 → 交付」七阶段流水线引擎（数据模型、状态机、关卡、回流、产出物、实时推送、接口），并补齐端到端测试所需的基础设施（`VK_ASSET_DIR`、`backend:dev:watch:qa`、qa-mode 模拟执行器按提示词产出文件）。

**Architecture:** 引擎放在 `crates/services/src/services/pipeline/`，对外只有一个 `PipelineService`，挂在 `Deployment` trait 上（与 `events()` 同级）。引擎不自己起进程：通过 `StageLauncher` 接缝调用现有容器服务开会话、跑智能体；本地容器在每个执行进程收尾后经 `mpsc` 通道把退出事件发给引擎，引擎在一把异步锁里串行推进状态机。状态转移、关卡判定、模板解析、提示词拼装都是纯函数，重点单测；数据落 SQLite 四张新表并接入现有变更钩子推送。

**Tech Stack:** Rust 2024（axum 0.8 / sqlx 0.8.6 SQLite / tokio / ts-rs 私有分支 / serde_yaml 0.9 新增）、pnpm 10。

---

## 0. 阅读顺序（执行前必读）

1. 本计划第 1～5 节（约定、已核实事实、对契约的修正、对设计文档的纠正、文件清单）。
2. 前后端契约：`docs/superpowers/plans/2026-09-18-pipeline-contract.md`（**类型名、字段、路径以它为准，不得改名**；本计划 §3 的修正已同步写回契约）。
3. 设计文档：`docs/superpowers/specs/2026-09-18-personal-pipeline-design.md`（308 行，§5～§7、§10、§11 与本计划直接相关）。
4. 仓库约定：`CLAUDE.md`。风格样板：`docs/superpowers/plans/2026-09-17-team-mode-and-ui.md`。

**不改 `crates/remote`，不改 `crates/api-types` 已有字段，不手改 `shared/types.ts`（只通过 `pnpm run generate-types` 生成）。**

**假设你对本仓库零了解。** 每一步都写了精确的文件路径、行号依据、命令与预期输出。标「**待核实**」的地方先按说明核实再动手。行号以本计划写作时的 `personal-pipeline-ui` 分支（`1951e6e7`）为准，前面的任务改过同一文件后行号会漂移，**以文中给出的代码片段定位为准**。

---

## 1. 跨任务共享约定（逐字一致）

### 1.1 Rust 名称与位置

| 名称 | 位置 | 任务 |
|---|---|---|
| 契约类型（`PipelineRun` 等 18 个）+ `PipelineRuns` / `PipelineStageRuns` / `PipelineGateDecisions` / `IssueArtifacts` 读写 + `truncate_for_storage` | `crates/db/src/models/pipeline.rs` | 3 |
| `ExecutionProcessRunReason::PipelineStep`（库内值 `pipelinestep`） | `crates/db/src/models/execution_process.rs` | 4 |
| `ScriptContext::PipelineCheck` | `crates/executors/src/actions/script.rs` | 4 |
| `pipeline_run_patch` / `pipeline_stage_run_patch` | `crates/services/src/services/events/patches.rs` | 5 |
| `PipelineTemplate` / `StageTemplate` / `StageGate` / `AutoGate` / `builtin_template` / `parse_template_yaml` / `load_template` | `crates/services/src/services/pipeline/template.rs` | 7 |
| `StageVerdict` / `evaluate_stage` / `dispatch_test_failure` | `crates/services/src/services/pipeline/gates.rs` | 8 |
| `Transition` / `next_transition` / `gate_decision_transition` / `FinishedProcess` / `PipelineExitEvent` / `is_stage_terminal` / `stage_column` | `crates/services/src/services/pipeline/transition.rs` | 9 |
| 提示词固定行常量 / `parse_pipeline_prompt` / `write_mock_artifacts` / `QaMarker` | `crates/executors/src/pipeline_prompt.rs`（**始终编译**，引擎与模拟器共用） | 10 |
| `build_stage_prompt` / `StagePromptInput` | `crates/services/src/services/pipeline/prompt.rs` | 11 |
| `artifacts_dir` / `list_existing` / `read_declared` / `ingest_stage_artifacts` | `crates/services/src/services/pipeline/artifacts.rs` | 13 |
| `ContainerService::start_new_session` | `crates/services/src/services/container.rs` | 14 |
| `StageLauncher` / `ContainerStageLauncher` / `NoopStageLauncher` / `LaunchedStep` / `checks_script` | `crates/services/src/services/pipeline/launcher.rs` | 14 |
| `PipelineService` / `PipelineError` / `StartPipelineInput` | `crates/services/src/services/pipeline/engine.rs` | 15 |
| `Deployment::pipeline()` | `crates/deployment/src/lib.rs` | 17 |
| `LocalContainerService::set_pipeline_exit_notifier` | `crates/local-deployment/src/container.rs` | 17 |
| `create_workspace_with_repos` / `link_local_issue` | `crates/server/src/routes/workspaces/create.rs` | 6、18 |
| 流水线路由 | `crates/server/src/routes/local_projects/pipeline.rs` | 18 |

### 1.2 数据库（迁移 `20260918000000_add_pipeline.sql`）

四张表 `id` 都是第 0 列。对外类型之外的**内部列**：`pipeline_runs.template_json`（解析后的模板快照，保证运行中途改 YAML 不影响在跑的运行）、`pipeline_runs.template_warning`、`pipeline_runs.executor_config`（JSON，后续阶段开会话用）、`pipeline_stage_runs.feedback`（本次尝试要带进提示词的打回意见/失败原因）。`issue_artifacts.truncated` 是契约 `IssueArtifactSummary.truncated` 的落库列。

「未结束的运行」= `status NOT IN ('completed','cancelled')`（`failed` 可继续，算未结束）。同一需求最多一条未结束运行，由部分唯一索引兜底。

### 1.3 引擎行为总表（实现与测试都按这张表）

| 事件 | 条件 | 结果 |
|---|---|---|
| 启动 | 需求有未结束运行 | 409 |
| 启动 | 正常 | 建运行（running）+ 首阶段尝试 1（running）→ 开会话（先跑 setup 脚本）→ 需求移到首阶段对应列 |
| 进程退出 | 不是阶段链终点（见 `is_stage_terminal`） | 忽略 |
| 进程退出 | 阶段有 `checks` 且智能体成功、检查未跑 | 同会话跑检查脚本（run_reason = `PipelineStep`），阶段仍 running |
| 进程退出 | 其余 | 读产出物入库 → `evaluate_stage` → `next_transition` |
| 判定 WaitHuman | — | 阶段 waiting_gate；运行 waiting_gate（运行已暂停则保持 paused） |
| 判定 Pass | 有下一阶段 | 阶段 passed → 进入下一阶段 |
| 判定 Pass | 无下一阶段 | 阶段 passed → 运行 completed，需求到「交付（done）」列 |
| 判定 Fail / FailBackTo | 本阶段累计失败次数 < max_rounds | 阶段 failed → 进入同阶段（Fail）或目标阶段（FailBackTo：test→develop / test→test_design） |
| 判定 Fail / FailBackTo | 累计失败次数 ≥ max_rounds | 阶段 failed → 运行 failed（进工作台「需要你处理」） |
| 人工通过 | 阶段 waiting_gate | 记 approve → 进入下一阶段或完成 |
| 人工打回 | 阶段 waiting_gate 且意见非空 | 记 reject → 阶段 rejected → 同阶段新尝试，意见进提示词 |
| 进入阶段 | 运行 paused | 新尝试建成 pending，不启动 |
| 暂停 | 运行 running / waiting_gate | 运行 paused；正在跑的进程不杀 |
| 继续 | 运行 paused / failed | 按最后一条尝试：pending → 启动；running → 运行 running；waiting_gate → 运行 waiting_gate；failed（运行 failed）→ 同阶段新尝试 |
| 取消 | 运行未结束 | 最后一条尝试若 pending/running/waiting_gate → skipped；运行 cancelled；若在跑则停工作区进程 |
| 服务启动 | 有 running 的尝试 | 尝试 failed（「服务重启，阶段执行被中断」），运行 failed |

`max_rounds` 默认 3（人工关卡阶段也适用：缺产出物/进程失败会重试）；人工打回不计入失败次数。

（C11）阶段尝试进入 `waiting_gate` / `failed` 时写 `finished_at`（= 执行结束时刻），后续转 `passed` / `rejected` 不覆盖（`COALESCE(finished_at, 现在)`）。工作台「等了多久」用 `stage_run.finished_at`（`list_pending` 按 `COALESCE(s.finished_at, r.updated_at)` 排序）。

模型层护栏（任务 3）：`mark_started` 只接受 pending / running 的尝试；`set_status` 不允许终态（passed / rejected / failed / skipped）改回非终态；两者违规时不改数据、返回 `sqlx::Error::RowNotFound`。运行的 `finished_at` 只在 completed / cancelled 时写且不覆盖，回到非终态清空。所有写方法都包 `retry_on_busy`。

### 1.4 阶段 → 看板列（设计 §6.4）

`requirement`→backlog、`spec`/`test_design`→todo、`develop`→dev、`review`→review、`test`→test、`deliver`→done。进入阶段时移动需求；`deliver` 进入 done 会写 `completed_at`（`Issues::move_to_stage` 的既有行为，`crates/db/src/models/issue.rs:701-731`）。

### 1.5 提交信息

每个任务末尾提交一次，中文，结尾：

```
Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
```

---

## 2. 已核实的现状（文件:行号）

### 2.1 基础设施

- `crates/utils/src/assets.rs:6-22`：`asset_dir()` debug 构建写死 `CARGO_MANIFEST_DIR/../../dev_assets`，release 用 `ProjectDirs`；没有环境变量覆盖。文件里没有 `#[cfg(test)]`。
- `package.json:35`：`"backend:dev:watch": "DISABLE_WORKTREE_CLEANUP=1 RUST_LOG=debug cargo watch -w crates -x 'run --bin server'"`；`package.json:36` 的 `dev:qa` 调用了不存在的 `backend:dev:watch:qa`。
- qa-mode 特性：`crates/server/Cargo.toml:76-78` `qa-mode = ["services/qa-mode", "executors/qa-mode"]`；`crates/services/Cargo.toml:9`；`crates/executors/Cargo.toml:64`。根 `Cargo.toml:2` `resolver = "3"`，server 包名 `server`（`crates/server/Cargo.toml:2`），因此脚本用 `run -p server --bin server --features qa-mode`。`package.json:31` 的 `backend:lint` 已经是 `cargo clippy --workspace --all-targets --features qa-mode`。
- 本机已装 `cargo-watch 8.5.3`、`sqlx-cli 0.8.6`（执行 `cargo sqlx --version` 核实过）。

### 2.2 数据库

- `ExecutionProcessRunReason`：`crates/db/src/models/execution_process.rs:50-59`，`#[sqlx(rename_all = "lowercase")]` + `#[serde(rename_all = "lowercase")]`，库里存 `setupscript/cleanupscript/archivescript/codingagent/devserver`。新值因此是 **`pipelinestep`**。
- CHECK 约束最近一次改写：`crates/db/migrations/20260203000000_add_archive_script_to_repos.sql:5-45`（加新列 → 拷数据 → 删 3 个索引 → 删旧列 → 改名 → 重建 3 个索引）。之后的迁移没有再动 `execution_processes`（`grep -l execution_processes crates/db/migrations/2026*` 只命中这一个）。
- `.sqlx` 离线缓存：`crates/db/.sqlx/` 有 182 个文件，`pnpm run prepare-db` = `scripts/prepare-db.js`（在临时库上跑迁移再 `cargo sqlx prepare`）；CI `.github/workflows/test.yml:237,240` 跑 `generate-types:check` 与 `prepare-db:check`。**本计划的新查询一律用运行时 `sqlx::query_as::<_, T>`**（同文件风格已有先例：`crates/db/src/models/local_auth.rs` 32 处运行时查询），不新增 `query!` 宏，因此不产生新的 `.sqlx` 文件；迁移只改 `execution_processes` 的列顺序，不改列类型与可空性，预期 `prepare-db:check` 无差异（任务 20 验证）。
- `crates/db` 没有 `build.rs`，`sqlx::migrate!("./migrations")`（`crates/db/src/lib.rs:19`）**不会自动感知新增的迁移文件**，新增后要 `touch crates/db/src/lib.rs` 触发重新展开（任务 2 步骤里有）。
- `TestDb`：`crates/db/src/test_support.rs:14-83`，`TestDb::new()` 与 `pool()`，`pub db: DBService`。迁移测试样板：同文件 `迁移后新表存在且_id_是第零列`（:103-121）。
- `repos` 表**没有** `test_script` / `lint_script`：`crates/db/src/models/repo.rs:37-54` 只有 setup/cleanup/archive/dev_server 脚本。

### 2.3 执行链路

- `start_workspace`：`crates/services/src/services/container.rs:1047-1131`：`self.create(workspace)` → 重新读工作区 → `Session::create` → setup 链（并行或顺序）→ 编码智能体（`CodingAgentInitialRequest`，`next_action` = cleanup 链）。
- `start_execution`：同文件 `:1133-1356`，按 action 类型建 `coding_agent_turn`、起日志归一化（qa-mode 下用 `QaMockExecutor`，`:1316-1320`）。
- `try_start_next_action`：同文件 `:1358-1389`，**next 的 run_reason 由 action 类型推出**（脚本→脚本 = SetupScript、智能体→脚本 = CleanupScript、→智能体 = CodingAgent），与当前进程的 run_reason 无关。
- `ensure_container_exists`：`crates/local-deployment/src/container.rs:1243-1287`，幂等：没有 `container_ref` 就算出目录并写回，worktree 缺了就补。
- 退出钩子：`crates/local-deployment/src/container.rs:470-812`（`spawn_exit_monitor`）。**陷阱**：`:590-621`，编码智能体成功退出但没有产生提交时 `should_start_next = false`，**不跑 next_action（cleanup）**，直接 `finalize_task`。流水线的需求/规格/用例阶段只写 `.vk/` 下的文件（不在任何 git 仓库里），必然走这个分支——所以引擎不能靠「cleanup 跑完」判断阶段结束，退出钩子要把「链是否继续」显式告诉引擎（任务 17 的 `chain_continues`）。
- `should_finalize`：`crates/services/src/services/container.rs:204-236`，并行模式的 setup 脚本（无 next_action）不 finalize。
- 按 `'codingagent'` 过滤的查询：`crates/db/src/models/coding_agent_turn.rs:48`（续聊找会话）、`crates/db/src/models/execution_process.rs:284,641`、`crates/db/src/models/workspace.rs:538,547,634,643`（工作区「运行中」状态）；前端 `packages/web-core/src/features/workspace-chat/model/hooks/useConversationHistory.ts:87-93`（对话时间线只收 setup/cleanup/archive/codingagent）。**所以流水线阶段的智能体进程必须保持 `codingagent`**，`PipelineStep` 只给引擎自己跑的检查脚本（见 §4 纠正 1）。
- 匹配 `ExecutionProcessRunReason` 的 Rust 代码全部是 `matches!` 或带 `_` 分支（`crates/local-deployment/src/container.rs:345-392` 有 `_ =>`；其余 20 处是构造或 `matches!`），加新变体**不会**有非穷尽匹配编译错误。前端 `packages/ui/src/components/ProcessListItem.tsx:19-50` 的 switch 有 `default`。
- `ScriptContext`：`crates/executors/src/actions/script.rs:21-28`，Rust 与前端都没有穷尽匹配（前端只有 `deriveConversationSemanticTimeline.ts:133` 的 `=== 'SetupScript'`）。
- `Session::create`：`crates/db/src/models/session.rs:145-170`；单仓库工作区的 `agent_working_dir` = 仓库子目录（`:172-190`），所以智能体在仓库子目录里跑，产出物目录必须用**绝对路径**写进提示词。
- `qa_mock`：`crates/executors/src/executors/mod.rs:42-43` 只在 `feature = "qa-mode"` 下编译；`spawn`（`qa_mock.rs:36-78`）先 `perform_file_operations`（`:128-205`，在工作目录下 `max_depth(3)` **随机删改 md/json/txt 文件**——会误删产出物），再逐行 `sleep 1` 输出 10 行日志（一个阶段至少 10 秒）。模拟器拿不到需求对象，只拿得到提示词：需求标题必须由引擎写进提示词的「需求：」行（设计 §6.2 本来就有这一行）。
- `crates/executors/Cargo.toml` 没有 `[dev-dependencies]`；`serde_json`、`uuid` 是正式依赖（`:12,:16`）。

### 2.4 需求流转、推送、路由

- 需求自动流转两处：`crates/server/src/routes/workspaces/create.rs:257-283`（建工作区绑本地需求 → Dev）与 `crates/services/src/services/issue_flow.rs:30-51`（`advance_issue_for_workspace`，调用方：`routes/workspaces/git.rs:265`、`routes/workspaces/pr.rs:319,479,813`、`services/pr_monitor.rs:166`）。在 `advance_issue_for_workspace` 里短路一次即可覆盖全部调用方。
- `HookTables`：`crates/services/src/services/events/types.rs:21-35`；`RecordTypes`：`:37-68`（不在 `generate_types.rs` 里，不影响 TS）。
- 变更钩子：`crates/services/src/services/events.rs:78-146`（preupdate，DELETE 取第 0 列 id）、`:148-400`（update，按 rowid 反查再推 patch）。钩子测试夹具 `setup()` / `wait_for_patch()`：同文件 `:453-514`（`TestDb::new_with_hook` 不适用：`create_hook` 需要一个指向同一文件的查询库，夹具就是为此写的）。
- patch 宏：`crates/services/src/services/events/patches.rs:139-185` `id_keyed_patch_module!`。
- 需求流：`crates/services/src/services/events/streams.rs:318-381`，首帧两条 replace，路径前缀白名单 `:358-360`，按 `value.project_id` 过滤。
- 本地路由：`crates/server/src/routes/local_projects/mod.rs`，`LocalRoutes`（:43-83，挂路由同时登记 `ROUTE_REGISTRY`）、`snapshot()`（:97-99，返回 `{ "<表名>": [...] }`）、`router()`（:141-151）；路由契约测试 `前端用到的本地端点都挂上了路由`（同文件 tests）。本地快照接口**不包 `ApiResponse`**，前端 `packages/web-core/src/shared/lib/electric/rows.ts:23-37` `extractFallbackRows` 读 `payload[table]`。
- handler 单测样板：`crates/server/src/routes/local_projects/issues.rs:21-131`（`handle_xxx(pool, ...)`）与 `:194-` 的 tests。
- `ApiError` 有 `NotFound` / `BadRequest(String)` / `Conflict(String)` / `Database` / `Io`：`crates/server/src/error.rs:35-94`。`ApiResponse`：`crates/utils/src/response.rs:5-40`。`CurrentUser`：`crates/server/src/middleware/local_session.rs:66-70`。
- `Deployment` trait：`crates/deployment/src/lib.rs:79-145`（`fn events(&self) -> &EventService;` 在 :105）；唯一实现 `crates/local-deployment/src/lib.rs:91`；容器构造 `:282-296`；`workspace_manager()` 是 `LocalDeployment` 的固有方法（:462）。
- ts-rs 用的是私有分支（根 `Cargo.toml:50`），`i64` 生成 `bigint`（`shared/types.ts:169` `exit_code: bigint | null`）。
- `crates/services/Cargo.toml` 没有任何 YAML 库；本机 `~/.cargo/registry` 里也没有缓存（任务 7 需要联网拉 `serde_yaml`）。
- 仓库里**没有**构造 `LocalContainerService` / `LocalDeployment` 的测试夹具（`grep -rn "LocalDeployment::new" crates` 无测试命中）。

---

## 3. 对契约的修正（已同步写回 `2026-09-18-pipeline-contract.md`）

| # | 契约原文 | 问题（证据） | 修正 |
|---|---|---|---|
| C1 | §3 快照接口 `GET /api/local/pipeline_runs?project_id=` → `PipelineRun[]` | 本地集合机制读 `payload[table]`（`rows.ts:23-37`），所有既有快照接口都用 `snapshot()` 返回 `{表名: 行}`（`local_projects/mod.rs:97-99`），裸数组会被前端判为「missing array」 | 返回 `{ "pipeline_runs": PipelineRun[] }` 与 `{ "pipeline_stage_runs": PipelineStageRun[] }`，**不包 `ApiResponse`**；§2 的 8 个接口仍包 `ApiResponse` |
| C2 | §1 `i64` 字段（`template_version`、`attempt`、`version`、`max_rounds`） | ts-rs 默认把 `i64` 生成 `bigint`（`shared/types.ts:169`），前端做算术会出错 | 这些字段加 `#[ts(type = "number")]`，TS 里是 `number`；Rust 类型与字段名不变 |
| C3 | §4「develop 阶段关卡看 checks（仓库的 lint/test 脚本退出码）」 | `repos` 表没有 lint/test 脚本（`repo.rs:37-54`） | `checks` = 模板阶段里的 shell 命令列表（`.vibe/pipeline.yaml` 的 `checks: ["pnpm run lint", "cargo test"]`）；内置模板为空。`checks_passed` = 智能体退出码 0，且（若有 checks）检查脚本退出码 0。检查脚本作为 run_reason = `pipelinestep` 的执行进程在同一会话里跑 |
| C4 | §2 路径参数写作 `{issue_id}` | axum/matchit 要求同一位置的参数同名，`/api/local/issues/{id}` 已存在（`local_projects/issues.rs` 的 `router()`） | 路径不变，路由表里登记为 `/api/local/issues/{id}/pipeline`；其余接口参数名一律 `{id}`。前端拼 URL 不受影响 |
| C5 | §2 409「已有未结束的运行」 | 未定义「结束」 | 结束 = `completed` / `cancelled`。`failed` 可「继续」，算未结束；要重开须先取消。`resume` 允许 `paused` 与 `failed`；`pause` 允许 `running` 与 `waiting_gate`；`cancel` 允许所有未结束状态 |
| C6 | §1 `IssuePipelineView.stages`「按 started_at 升序」 | `pending` 尝试的 `started_at` 为空 | 按创建顺序（等价于已启动者按 `started_at` 升序，pending 排最后） |
| C7 | §1 `PipelineStageRun.execution_process_id` | 未定义指哪个进程 | 启动时是会话里第一个进程（有 setup 脚本时是 setup 进程），智能体结束后改为该智能体进程；检查脚本运行期间指向检查进程 |
| C8 | §1 `IssueArtifact` 同时写 `#[serde(flatten)] #[ts(flatten)]` | 已核实：锁定的 ts-rs 分支（`Cargo.lock` 中 `xazukx/ts-rs#b5c8277`）默认开 `serde-compat`，`macros/src/attr/field.rs:35-37,184` 把 `#[serde(flatten)]` 解析成 flatten，与 `#[ts(flatten)]` 按「或」合并（:66），写一个就够；另外 sqlx 的 `FromRow` 需要 `#[sqlx(flatten)]` 才能从同一行读出嵌套结构 | 写 `#[serde(flatten)]` + `#[sqlx(flatten)]`，不写 `#[ts(flatten)]`（写了也无害） |
| C9 | §5 模拟器行为 | 模拟器原本会随机删改工作目录下的文件（`qa_mock.rs:128-205`） | 提示词里有「产出物目录」行时只写产出物、不做随机删改，每行日志间隔 0.1 秒（原 1 秒）；否则行为不变 |

---

## 4. 对设计文档的纠正

1. **§5 末行「`ExecutionProcessRunReason` 新增 `PipelineStep`」的用法**：若把阶段智能体进程标成 `PipelineStep`，续聊（`coding_agent_turn.rs:48`）、工作区运行中状态（`workspace.rs:538-643`）、对话时间线（`useConversationHistory.ts:87-93`）全部看不到它；且经 setup 链启动的智能体会被 `try_start_next_action` 重新标成 `CodingAgent`（`container.rs:1368-1384`），做不到一致。**决定**：智能体进程保持 `codingagent`，引擎靠 `pipeline_stage_runs.session_id` 认领；`PipelineStep` 只用于引擎跑的检查脚本进程。迁移照样放开 CHECK。
2. **§6.2 第 2 步「首个技能步骤时创建工作区」**：首阶段必然是技能步骤，所以在启动接口里就建工作区（复用从 `create_and_start_workspace` 抽出的 `create_workspace_with_repos`），`workspace_id` 实际不会为空；类型仍是 `Option`（契约不动）。
3. **§6.2「复用 `create_and_start_workspace` 的内部路径」**：它的执行部分在 `ContainerService::start_workspace`（services 层），而工作区记录部分在 server 层。分别抽成 `ContainerService::start_new_session(workspace, executor_config, prompt, run_setup)`（`start_workspace` 改为 `create` + 它）与 server 层的 `create_workspace_with_repos`，不复制代码。
4. **§6.2 第 4 步退出钩子**：见 §2.3 的「无提交不跑 cleanup」陷阱。钩子在进程收尾的**最后**（提交、next_action、HEAD 记录之后）发 `PipelineExitEvent { execution_process_id, chain_continues }`，引擎用 `is_stage_terminal` 判断这是不是阶段链的终点。
5. **§6.1 YAML 里 `checks: [lint_script, test_script]`**：这两个名字不存在（见契约修正 C3），`checks` 改为 shell 命令列表。
6. **§7 接口表 `GET /api/local/pipeline/runs/{id}/artifacts/{kind}`**：以契约 `GET /api/local/pipeline/artifacts/{artifact_id}` 为准。
7. **§5 表结构**：补内部列 `template_json`、`template_warning`、`executor_config`、`feedback`，以及 `issue_artifacts.truncated`（契约类型需要）。时间列不设 DEFAULT，由 Rust 显式写入（与 `20260917100000_add_test_stage_status.sql` 的注释约定一致）。
8. **§6.1「解析失败回落内置模板并在运行记录里写警告」**：写在内部列 `pipeline_runs.template_warning` 并打 `warn` 日志；契约类型不暴露（不加字段）。
9. **§6.4**：`issue_flow` 的短路放在 `advance_issue_for_workspace` 里，一处覆盖 PR 创建、合并、PR 监控三条路径。
10. **§9.2 交付阶段自动提 PR**：U2 不做，交付阶段只要求产出 `delivery-report.md`（`artifacts_present`）。
11. **已知副作用**：每个阶段的智能体结束都会走 `finalize_task` 发一次「Workspace Complete」系统通知（`container.rs:238-270`）。U2 不改，记入任务 20 的遗留清单。
12. **变更钩子读到未提交前的旧数据（任务 5 之后追加修复）**：sqlite 的 `update_hook` 在语句执行时、事务提交**之前**触发，`events.rs` 里 spawn 出的反查用另一个连接按 rowid 读，WAL 下若早于提交就读到旧快照——UPDATE 推出旧值（如 `status` 仍为 `running`）、INSERT 读不到行而丢掉 add，且之后不会补推，前端停在旧状态。所有钩子表（workspaces、execution_processes、scratch、issues、project_statuses、issue_comments、pipeline_runs、pipeline_stage_runs）都受影响，流水线状态推送尤其明显（任务 5 的测试在并行负载下约 1/3 概率失败）。**修复**：`create_hook` 里建一个专用的屏障池（同库同参数、`max_connections = 2`、懒连接、不装钩子，与业务主池和钩子反查池分开）；每个非删除事件先在屏障池上 `BEGIN IMMEDIATE`（`acquire_commit_barrier`），它要拿写锁，因此会等到触发钩子的写事务提交或回滚（最多 busy_timeout），失败时打 error 并丢弃这次推送；**持有屏障期间**在反查池上完成全部读（含 `find_by_id_with_status`、`push_workspace_update_for_session` 这类二次查询）并推送，再回滚屏障。持有期间别的写者提交不了，所以读到的是同一份已提交快照，推送顺序也与提交顺序一致。回归测试：`写事务延迟提交时_replace_补丁带的是提交后的值`、`写事务延迟提交时_insert_仍产出_add_补丁`（主池事务写后 sleep 200ms 再提交）。

---

## 5. 文件清单

**新建**

| 文件 | 职责 |
|---|---|
| `crates/db/migrations/20260918000000_add_pipeline.sql` | 四张表 + run_reason CHECK |
| `crates/db/src/models/pipeline.rs` | 契约类型 + 读写 + 单测 |
| `crates/executors/src/pipeline_prompt.rs` | 提示词固定行常量、解析、模拟产出物（引擎与模拟器共用） |
| `crates/services/src/services/pipeline/mod.rs` | 模块入口与再导出 |
| `crates/services/src/services/pipeline/template.rs` | 模板 |
| `crates/services/src/services/pipeline/gates.rs` | 关卡判定 |
| `crates/services/src/services/pipeline/transition.rs` | 状态转移纯函数 |
| `crates/services/src/services/pipeline/prompt.rs` | 提示词拼装 |
| `crates/services/src/services/pipeline/artifacts.rs` | 产出物读盘入库 |
| `crates/services/src/services/pipeline/launcher.rs` | 执行层接缝 |
| `crates/services/src/services/pipeline/engine.rs` | `PipelineService` |
| `crates/services/src/services/pipeline/test_support.rs` | 测试夹具（`#[cfg(test)]`） |
| `crates/services/src/services/pipeline/tests.rs` | 引擎集成测试（黄金路径 + 异常路径） |
| `crates/server/src/routes/local_projects/pipeline.rs` | 路由 + handler 单测 |

**修改**：`crates/utils/src/assets.rs`、`package.json`、`crates/db/src/models/mod.rs`、`crates/db/src/test_support.rs`、`crates/db/src/models/execution_process.rs`、`crates/executors/src/actions/script.rs`、`crates/executors/src/lib.rs`、`crates/executors/Cargo.toml`、`crates/executors/src/executors/qa_mock.rs`、`crates/services/Cargo.toml`、`crates/services/src/services/mod.rs`、`crates/services/src/services/events.rs`、`crates/services/src/services/events/types.rs`、`crates/services/src/services/events/patches.rs`、`crates/services/src/services/events/streams.rs`、`crates/services/src/services/issue_flow.rs`、`crates/services/src/services/container.rs`、`crates/deployment/src/lib.rs`、`crates/local-deployment/src/lib.rs`、`crates/local-deployment/src/container.rs`、`crates/server/src/routes/workspaces/create.rs`、`crates/server/src/routes/local_projects/mod.rs`、`crates/server/src/bin/generate_types.rs`、`shared/types.ts`（生成）、`docs/superpowers/plans/2026-09-18-pipeline-contract.md`（已改）。

---

## 6. 任务总览

| # | 任务 | 依赖 |
|---|---|---|
| 1 | 基础设施：`VK_ASSET_DIR` 与 `backend:dev:watch:qa` | — |
| 2 | 迁移：四张表 + run_reason CHECK | — |
| 3 | 模型 `pipeline.rs` | 2 |
| 4 | `ExecutionProcessRunReason::PipelineStep` 与 `ScriptContext::PipelineCheck` | 2 |
| 5 | 实时推送 | 3 |
| 6 | 需求状态双写短路 | 3 |
| 7 | 模板 `template.rs` | 3 |
| 8 | 关卡 `gates.rs` | 7 |
| 9 | 状态转移 `transition.rs` | 4、8 |
| 10 | 提示词格式与模拟产出物 `executors::pipeline_prompt` | — |
| 11 | 引擎提示词 `prompt.rs` | 10 |
| 12 | qa-mode 模拟执行器接线 | 10 |
| 13 | 产出物 `artifacts.rs` | 3 |
| 14 | `start_new_session` 抽取与 `launcher.rs` | 4 |
| 15 | 引擎 `engine.rs` | 9、11、13、14 |
| 16 | 引擎集成测试（黄金路径 + 异常路径） | 15 |
| 17 | 退出钩子与 deployment 接线 | 15 |
| 18 | 路由与 `create.rs` 抽取 | 17 |
| 19 | 类型生成 | 18 |
| 20 | 收尾校验 | 全部 |

---

### Task 1: 基础设施——`VK_ASSET_DIR` 覆盖与 `backend:dev:watch:qa`

**Files:**
- Modify: `crates/utils/src/assets.rs:6-22`（`asset_dir()`），文件末尾追加测试
- Modify: `package.json:35`（其后插入一行）

- [ ] **Step 1: 写失败测试**

在 `crates/utils/src/assets.rs` 文件末尾（`ScriptAssets` 之后）追加：

```rust
#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{default_asset_dir, resolve_asset_dir};

    #[test]
    fn 未设置覆盖时沿用默认目录() {
        assert_eq!(resolve_asset_dir(None), default_asset_dir());
    }

    #[test]
    fn 覆盖为空串时沿用默认目录() {
        assert_eq!(resolve_asset_dir(Some(OsString::new())), default_asset_dir());
    }

    #[test]
    fn 设置覆盖时使用指定目录() {
        let dir = PathBuf::from("/tmp/vk-e2e-case-1");
        assert_eq!(resolve_asset_dir(Some(dir.clone().into_os_string())), dir);
    }

    #[test]
    fn debug_构建默认目录仍是_dev_assets() {
        if cfg!(debug_assertions) {
            assert!(default_asset_dir().ends_with("dev_assets"));
        }
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p utils assets::tests`
Expected: 编译失败，`cannot find function \`resolve_asset_dir\` in module \`super\``。

- [ ] **Step 3: 最小实现**

把 `crates/utils/src/assets.rs:6-22` 的 `asset_dir()` 整体替换为：

```rust
/// 覆盖数据目录的环境变量。端到端测试给每个用例一个独立的临时目录。
/// 未设置或为空时行为与原来完全一致。
pub const ASSET_DIR_ENV: &str = "VK_ASSET_DIR";

pub fn asset_dir() -> std::path::PathBuf {
    let path = resolve_asset_dir(std::env::var_os(ASSET_DIR_ENV));

    // Ensure the directory exists
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("Failed to create asset directory");
    }

    path
    // ✔ macOS → ~/Library/Application Support/MyApp
    // ✔ Linux → ~/.local/share/myapp   (respects XDG_DATA_HOME)
    // ✔ Windows → %APPDATA%\Example\MyApp
}

/// 纯函数：由环境变量的值算出数据目录。拆出来是为了测试时不改进程环境变量
/// （edition 2024 下 `std::env::set_var` 是 unsafe，且会污染并行跑的其它测试）。
fn resolve_asset_dir(override_dir: Option<std::ffi::OsString>) -> std::path::PathBuf {
    match override_dir {
        Some(dir) if !dir.is_empty() => std::path::PathBuf::from(dir),
        _ => default_asset_dir(),
    }
}

fn default_asset_dir() -> std::path::PathBuf {
    if cfg!(debug_assertions) {
        std::path::PathBuf::from(PROJECT_ROOT).join("../../dev_assets")
    } else {
        prod_asset_dir_path()
    }
}
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p utils assets::tests`
Expected: `test result: ok. 4 passed; 0 failed`

- [ ] **Step 5: 补 `backend:dev:watch:qa` 脚本**

在 `package.json` 的 `"backend:dev:watch": ...`（第 35 行）之后插入一行：

```json
    "backend:dev:watch:qa": "DISABLE_WORKTREE_CLEANUP=1 RUST_LOG=debug cargo watch -w crates -x 'run -p server --bin server --features qa-mode'",
```

- [ ] **Step 6: 验证脚本存在且 qa-mode 能编译**

Run: `node -e "console.log(require('./package.json').scripts['backend:dev:watch:qa'])"`
Expected: 打印上面那条命令。

Run: `cargo build -p server --bin server --features qa-mode`
Expected: `Finished` 且无 error（首次编译较久）。

- [ ] **Step 7: 提交**

```bash
git add crates/utils/src/assets.rs package.json
git commit -m "$(cat <<'EOF'
基础设施：数据目录支持 VK_ASSET_DIR 覆盖，补 backend:dev:watch:qa 脚本

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 迁移——四张表与 run_reason CHECK

**Files:**
- Create: `crates/db/migrations/20260918000000_add_pipeline.sql`
- Modify: `crates/db/src/test_support.rs`（tests 模块末尾追加 3 个测试）
- Touch: `crates/db/src/lib.rs`（让 `sqlx::migrate!` 重新展开）

- [ ] **Step 1: 写失败测试**

在 `crates/db/src/test_support.rs` 的 `mod tests` 末尾（最后一个测试 `创建者账号被删除后工作区不被连带删除` 之后、模块右花括号之前）追加：

```rust
    #[tokio::test]
    async fn 流水线四张表存在且_id_是第零列() {
        let test_db = TestDb::new().await;
        for table in [
            "pipeline_runs",
            "pipeline_stage_runs",
            "pipeline_gate_decisions",
            "issue_artifacts",
        ] {
            let columns = column_names(test_db.pool(), table).await;
            assert!(!columns.is_empty(), "表 {table} 不存在");
            assert_eq!(columns[0], "id", "表 {table} 的 id 必须是第 0 列");
        }
    }

    /// 插一条最小的工作区 + 会话，返回会话 id。给 run_reason 约束测试用。
    async fn 建会话(pool: &sqlx::SqlitePool) -> uuid::Uuid {
        let workspace_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces (id, branch, name) VALUES (?1, 'vk/p', 'p')")
            .bind(workspace_id)
            .execute(pool)
            .await
            .expect("插入工作区失败");
        let session_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO sessions (id, workspace_id) VALUES (?1, ?2)")
            .bind(session_id)
            .bind(workspace_id)
            .execute(pool)
            .await
            .expect("插入会话失败");
        session_id
    }

    #[tokio::test]
    async fn 执行进程_run_reason_接受_pipelinestep() {
        let test_db = TestDb::new().await;
        let session_id = 建会话(test_db.pool()).await;
        sqlx::query(
            "INSERT INTO execution_processes (id, session_id, run_reason) VALUES (?1, ?2, 'pipelinestep')",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(session_id)
        .execute(test_db.pool())
        .await
        .expect("run_reason = 'pipelinestep' 应被 CHECK 接受");
    }

    #[tokio::test]
    async fn 执行进程_run_reason_仍拒绝未知取值且索引重建() {
        let test_db = TestDb::new().await;
        let session_id = 建会话(test_db.pool()).await;
        let result = sqlx::query(
            "INSERT INTO execution_processes (id, session_id, run_reason) VALUES (?1, ?2, 'bogus')",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(session_id)
        .execute(test_db.pool())
        .await;
        assert!(result.is_err(), "未知 run_reason 必须被 CHECK 拒绝");

        let indexes: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'execution_processes' \
             AND name LIKE 'idx_execution_processes_%run_reason%'",
        )
        .fetch_all(test_db.pool())
        .await
        .unwrap();
        assert_eq!(indexes.len(), 3, "run_reason 相关的 3 个索引必须重建，实际：{indexes:?}");
    }

    /// `20260918000000_add_pipeline.sql` 重建了 execution_processes.run_reason 列：
    /// 在只跑到它之前的旧库上造三条不同 run_reason 的进程，升级后数据与 run_reason 必须原样保留。
    #[tokio::test]
    async fn 流水线迁移保留已有执行进程的数据与_run_reason() {
        use sqlx::Row;

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("legacy_pipeline.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("连接失败");

        // 只跑到流水线迁移之前，模拟升级前的库。
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
            .await
            .expect("读取迁移目录失败");
        let legacy: Vec<_> = migrator
            .iter()
            .filter(|m| m.version < 20260918000000)
            .cloned()
            .collect();
        assert!(
            migrator.iter().any(|m| m.version == 20260918000000),
            "迁移目录里应有流水线迁移"
        );
        let mut legacy_migrator =
            sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await
                .expect("读取迁移目录失败");
        legacy_migrator.migrations = legacy.into();
        legacy_migrator.run(&pool).await.expect("旧迁移应成功");

        let session_id = 建会话(&pool).await;
        let rows = [
            (
                uuid::Uuid::new_v4(),
                "codingagent",
                "completed",
                Some(0_i64),
                r#"{"k":"agent"}"#,
            ),
            (
                uuid::Uuid::new_v4(),
                "archivescript",
                "failed",
                Some(2),
                r#"{"k":"archive"}"#,
            ),
            (
                uuid::Uuid::new_v4(),
                "devserver",
                "running",
                None,
                r#"{"k":"dev"}"#,
            ),
        ];
        for (id, run_reason, status, exit_code, action) in &rows {
            sqlx::query(
                "INSERT INTO execution_processes \
                 (id, session_id, run_reason, executor_action, status, exit_code, \
                  started_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '2026-09-01T00:00:00Z', \
                         '2026-09-01T00:00:00Z', '2026-09-01T00:00:01Z')",
            )
            .bind(id)
            .bind(session_id)
            .bind(run_reason)
            .bind(action)
            .bind(status)
            .bind(exit_code)
            .execute(&pool)
            .await
            .expect("旧库插入执行进程失败");
        }
        // 旧库的 CHECK 还不认识 pipelinestep。
        assert!(
            sqlx::query(
                "INSERT INTO execution_processes (id, session_id, run_reason) VALUES (?1, ?2, 'pipelinestep')",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(session_id)
            .execute(&pool)
            .await
            .is_err(),
            "升级前 pipelinestep 应被拒绝"
        );

        migrator
            .run(&pool)
            .await
            .expect("流水线迁移应能应用在旧库上");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM execution_processes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 3, "升级不增不减执行进程");
        for (id, run_reason, status, exit_code, action) in &rows {
            let row = sqlx::query(
                "SELECT session_id, run_reason, executor_action, status, exit_code, \
                        started_at, created_at, updated_at \
                 FROM execution_processes WHERE id = ?1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("升级后执行进程应仍在");
            assert_eq!(row.get::<uuid::Uuid, _>("session_id"), session_id);
            assert_eq!(row.get::<String, _>("run_reason"), *run_reason);
            assert_eq!(row.get::<String, _>("executor_action"), *action);
            assert_eq!(row.get::<String, _>("status"), *status);
            assert_eq!(row.get::<Option<i64>, _>("exit_code"), *exit_code);
            assert_eq!(row.get::<String, _>("started_at"), "2026-09-01T00:00:00Z");
            assert_eq!(row.get::<String, _>("created_at"), "2026-09-01T00:00:00Z");
            assert_eq!(row.get::<String, _>("updated_at"), "2026-09-01T00:00:01Z");
        }

        // 升级后新值可写入，且可重复执行迁移。
        sqlx::query(
            "INSERT INTO execution_processes (id, session_id, run_reason) VALUES (?1, ?2, 'pipelinestep')",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(session_id)
        .execute(&pool)
        .await
        .expect("升级后 pipelinestep 应被接受");
        migrator.run(&pool).await.expect("重复执行迁移应成功");
    }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p db test_support::tests`
Expected: `流水线四张表存在且_id_是第零列` 失败（`表 pipeline_runs 不存在`），`执行进程_run_reason_接受_pipelinestep` 失败（`CHECK constraint failed`），`流水线迁移保留已有执行进程的数据与_run_reason` 失败（`迁移目录里应有流水线迁移`）。

- [ ] **Step 3: 写迁移**

新建 `crates/db/migrations/20260918000000_add_pipeline.sql`：

```sql
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
```

- [ ] **Step 4: 让 `sqlx::migrate!` 感知新文件并运行测试**

Run: `touch crates/db/src/lib.rs && cargo test -p db test_support::tests`
Expected: 全部通过（含原有 8 个 + 新增 4 个），`test result: ok. 12 passed`。
（若新增的两个仍报「表不存在」，说明宏没重新展开：执行 `cargo clean -p db` 后重跑。）

- [ ] **Step 5: 确认老库能升级**

Run: `cargo test -p db 新迁移可以应用在已有旧迁移的库上并且可重复执行`
Expected: PASS（该测试跑到最新迁移，覆盖本迁移在有数据的旧库上的应用）。

- [ ] **Step 6: 提交**

```bash
git add crates/db/migrations/20260918000000_add_pipeline.sql crates/db/src/test_support.rs
git commit -m "$(cat <<'EOF'
迁移：新增流水线四张表，run_reason 放开 pipelinestep

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 模型 `crates/db/src/models/pipeline.rs`

**Files:**
- Create: `crates/db/src/models/pipeline.rs`
- Modify: `crates/db/src/models/mod.rs`（在 `pub mod merge;` 之后加 `pub mod pipeline;`）

本任务先写全部类型与读写（Step 1），再写测试（Step 2），因为测试要编译就需要类型存在；「失败」一步用一个故意断言错的测试验证测试确实在跑（Step 3），然后改正。

- [ ] **Step 1: 写类型与读写**

新建 `crates/db/src/models/pipeline.rs`：

```rust
//! 交付流水线（U2）的数据模型与读写。
//!
//! 类型名与字段名是前后端契约（`docs/superpowers/plans/2026-09-18-pipeline-contract.md` §1），
//! 改名先改契约。查询一律用运行时 `sqlx::query_as::<_, T>(..)`，不进 `.sqlx` 离线缓存。

use chrono::{DateTime, Utc};
use executors::profile::ExecutorConfig;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, SqlitePool, Type, sqlite::SqliteRow};
use ts_rs::TS;
use uuid::Uuid;

use super::db_retry::retry_on_busy;

/// 产出物入库上限：超过部分只留在磁盘（设计 §5）。
pub const MAX_ARTIFACT_BYTES: usize = 256 * 1024;

/// 截断后追加在库内内容末尾的标记。
pub const TRUNCATION_MARKER: &str =
    "\n\n……（内容超过 256 KB，库里只保留前 256 KB；完整内容见工作区产出物目录）\n";

// ---------------------------------------------------------------------------
// 枚举
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_stage_key", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageKey {
    Requirement,
    Spec,
    TestDesign,
    Develop,
    Review,
    Test,
    Deliver,
}

impl PipelineStageKey {
    /// 标准顺序。模板里的阶段必须按这个顺序出现（可以省略某些阶段）。
    pub const ALL: [PipelineStageKey; 7] = [
        PipelineStageKey::Requirement,
        PipelineStageKey::Spec,
        PipelineStageKey::TestDesign,
        PipelineStageKey::Develop,
        PipelineStageKey::Review,
        PipelineStageKey::Test,
        PipelineStageKey::Deliver,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PipelineStageKey::Requirement => "requirement",
            PipelineStageKey::Spec => "spec",
            PipelineStageKey::TestDesign => "test_design",
            PipelineStageKey::Develop => "develop",
            PipelineStageKey::Review => "review",
            PipelineStageKey::Test => "test",
            PipelineStageKey::Deliver => "deliver",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == value)
    }

    /// 在标准顺序里的位置，从 0 开始。
    pub fn order(self) -> usize {
        Self::ALL
            .iter()
            .position(|key| *key == self)
            .expect("ALL 覆盖全部取值")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_run_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    Running,
    WaitingGate,
    Paused,
    Failed,
    Completed,
    Cancelled,
}

impl PipelineRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineRunStatus::Running => "running",
            PipelineRunStatus::WaitingGate => "waiting_gate",
            PipelineRunStatus::Paused => "paused",
            PipelineRunStatus::Failed => "failed",
            PipelineRunStatus::Completed => "completed",
            PipelineRunStatus::Cancelled => "cancelled",
        }
    }

    /// 已结束：不能再暂停、继续或决策。`failed` 可以继续，所以不算结束。
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            PipelineRunStatus::Completed | PipelineRunStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "pipeline_stage_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageStatus {
    Pending,
    Running,
    WaitingGate,
    Passed,
    Rejected,
    Failed,
    Skipped,
}

impl PipelineStageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineStageStatus::Pending => "pending",
            PipelineStageStatus::Running => "running",
            PipelineStageStatus::WaitingGate => "waiting_gate",
            PipelineStageStatus::Passed => "passed",
            PipelineStageStatus::Rejected => "rejected",
            PipelineStageStatus::Failed => "failed",
            PipelineStageStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "gate_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Human,
    Auto,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "gate_decision_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GateDecisionKind {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "artifact_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Requirement,
    Spec,
    Plan,
    TestCases,
    TraceMatrix,
    Review,
    TestReport,
    DeliveryReport,
}

impl ArtifactKind {
    /// 契约 §4：产出物文件名 → kind。不在表里的文件名不入库。
    pub fn from_file_name(name: &str) -> Option<Self> {
        match name {
            "requirement.md" => Some(ArtifactKind::Requirement),
            "spec.md" => Some(ArtifactKind::Spec),
            "plan.md" => Some(ArtifactKind::Plan),
            "test-cases.csv" => Some(ArtifactKind::TestCases),
            "trace-matrix.md" => Some(ArtifactKind::TraceMatrix),
            "review.json" => Some(ArtifactKind::Review),
            "test-report.json" => Some(ArtifactKind::TestReport),
            "delivery-report.md" => Some(ArtifactKind::DeliveryReport),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 行类型（契约 §1）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineRun {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub template_key: String,
    #[ts(type = "number")]
    pub template_version: i64,
    pub status: PipelineRunStatus,
    pub current_stage_key: PipelineStageKey,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineStageRun {
    pub id: Uuid,
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub stage_key: PipelineStageKey,
    #[ts(type = "number")]
    pub attempt: i64,
    pub status: PipelineStageStatus,
    pub gate_kind: GateKind,
    pub session_id: Option<Uuid>,
    pub execution_process_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct PipelineGateDecision {
    pub id: Uuid,
    pub stage_run_id: Uuid,
    pub decision: GateDecisionKind,
    pub comment: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct IssueArtifactSummary {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub stage_run_id: Uuid,
    pub kind: ArtifactKind,
    pub rel_path: String,
    #[ts(type = "number")]
    pub version: i64,
    pub truncated: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize, TS)]
pub struct IssueArtifact {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub summary: IssueArtifactSummary,
    pub content: String,
}

// ---------------------------------------------------------------------------
// 视图与请求类型（契约 §1）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineTemplateStageView {
    pub key: PipelineStageKey,
    pub skill: String,
    pub gate_kind: GateKind,
    /// 人工关卡中文名，如「需求确认」。
    pub gate_label: Option<String>,
    /// 自动关卡的判定名（`checks_passed` / `no_blocking_findings` / `all_cases_passed` /
    /// `artifacts_present`）；人工关卡与无关卡为 None。契约修订 C10。
    pub gate_condition: Option<String>,
    #[ts(type = "number")]
    pub max_rounds: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineTemplateView {
    pub key: String,
    #[ts(type = "number")]
    pub version: i64,
    pub stages: Vec<PipelineTemplateStageView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct IssuePipelineView {
    pub run: PipelineRun,
    /// 全部尝试，按创建顺序（已启动者即 started_at 升序，pending 排最后）。
    pub stages: Vec<PipelineStageRun>,
    /// 全部关卡决策，按 decided_at 升序。
    pub decisions: Vec<PipelineGateDecision>,
    /// 该需求每个 kind 的全部版本。
    pub artifacts: Vec<IssueArtifactSummary>,
    pub template: PipelineTemplateView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct StartPipelineRepo {
    pub repo_id: Uuid,
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StartPipelineRequest {
    pub repos: Vec<StartPipelineRepo>,
    /// 与 `create_and_start_workspace` 相同的类型。
    pub executor_config: ExecutorConfig,
    /// 缺省 "standard"。
    pub template_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GateDecisionRequest {
    pub decision: GateDecisionKind,
    /// reject 时必填，后端校验非空。
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PendingPipelineItem {
    pub run: PipelineRun,
    /// 当前等待或失败的那一条。
    pub stage_run: PipelineStageRun,
    pub issue_simple_id: String,
    pub issue_title: String,
}

// ---------------------------------------------------------------------------
// 写入参数与内部列
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CreatePipelineRun {
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub template_key: String,
    pub template_version: i64,
    pub template_json: String,
    pub template_warning: Option<String>,
    pub executor_config_json: String,
    pub first_stage: PipelineStageKey,
}

/// `pipeline_runs` 里不对外暴露的列。
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineRunInternals {
    pub template_json: String,
    pub template_warning: Option<String>,
    pub executor_config_json: String,
}

#[derive(Debug, Clone)]
pub struct CreateStageRun {
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub stage_key: PipelineStageKey,
    pub gate_kind: GateKind,
    /// 只允许 `Running`（立即启动）或 `Pending`（运行已暂停）。
    pub status: PipelineStageStatus,
    /// 本次尝试要带进提示词的打回意见或失败原因。
    pub feedback: Option<String>,
}

const RUN_COLUMNS: &str = "id, issue_id, project_id, workspace_id, template_key, template_version, \
     status, current_stage_key, created_at, updated_at, finished_at";

const STAGE_COLUMNS: &str = "id, run_id, project_id, stage_key, attempt, status, gate_kind, \
     session_id, execution_process_id, started_at, finished_at, summary, error";

const DECISION_COLUMNS: &str = "id, stage_run_id, decision, comment, decided_by, decided_at";

const ARTIFACT_SUMMARY_COLUMNS: &str =
    "id, issue_id, stage_run_id, kind, rel_path, version, truncated, created_at";

// ---------------------------------------------------------------------------
// pipeline_runs
// ---------------------------------------------------------------------------

pub struct PipelineRuns;

impl PipelineRuns {
    pub async fn create(
        pool: &SqlitePool,
        data: &CreatePipelineRun,
    ) -> Result<PipelineRun, sqlx::Error> {
        let sql = format!(
            "INSERT INTO pipeline_runs (id, issue_id, project_id, workspace_id, template_key, \
             template_version, template_json, template_warning, executor_config, status, \
             current_stage_key, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'running', ?10, ?11, ?11) \
             RETURNING {RUN_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, PipelineRun>(&sql)
                .bind(Uuid::new_v4())
                .bind(data.issue_id)
                .bind(data.project_id)
                .bind(data.workspace_id)
                .bind(&data.template_key)
                .bind(data.template_version)
                .bind(&data.template_json)
                .bind(&data.template_warning)
                .bind(&data.executor_config_json)
                .bind(data.first_stage)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE id = ?1");
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE rowid = ?1");
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(rowid)
            .fetch_optional(pool)
            .await
    }

    /// 该需求最近一条运行（不论状态）。
    pub async fn find_latest_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE issue_id = ?1 \
             ORDER BY created_at DESC, rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(issue_id)
            .fetch_optional(pool)
            .await
    }

    /// 该需求未结束（非 completed / cancelled）的运行。
    pub async fn find_active_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Option<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE issue_id = ?1 \
             AND status NOT IN ('completed', 'cancelled') ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(issue_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn has_active_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        Ok(Self::find_active_for_issue(pool, issue_id).await?.is_some())
    }

    pub async fn list_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<PipelineRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM pipeline_runs WHERE project_id = ?1 \
             ORDER BY created_at, rowid"
        );
        sqlx::query_as::<_, PipelineRun>(&sql)
            .bind(project_id)
            .fetch_all(pool)
            .await
    }

    pub async fn internals(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineRunInternals>, sqlx::Error> {
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT template_json, template_warning, executor_config FROM pipeline_runs WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        Ok(
            row.map(|(template_json, template_warning, executor_config_json)| {
                PipelineRunInternals {
                    template_json,
                    template_warning,
                    executor_config_json,
                }
            }),
        )
    }

    /// 改运行状态；`current_stage` 为 None 时保持不变。
    /// finished_at：进入 completed / cancelled 时写入且已有值不覆盖；其余状态置 NULL。
    pub async fn update_status(
        pool: &SqlitePool,
        id: Uuid,
        status: PipelineRunStatus,
        current_stage: Option<PipelineStageKey>,
    ) -> Result<PipelineRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_runs SET status = ?2, \
             current_stage_key = COALESCE(?3, current_stage_key), updated_at = ?4, \
             finished_at = CASE WHEN ?2 IN ('completed', 'cancelled') \
                                THEN COALESCE(finished_at, ?4) ELSE NULL END \
             WHERE id = ?1 RETURNING {RUN_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, PipelineRun>(&sql)
                .bind(id)
                .bind(status)
                .bind(current_stage)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }

    /// 工作台「需要你确认 / 需要你处理」：waiting_gate 与 failed 的运行，等得最久的在前。
    ///
    /// 一条 SQL：每个运行配它最新的一条阶段尝试与需求。「等了多久」以阶段的 finished_at
    /// （执行结束时刻，契约修订 C11）为准，没有时退回运行的 updated_at。
    /// 没有任何阶段尝试或需求已删除的运行不出现（内连接）。
    pub async fn list_pending(
        pool: &SqlitePool,
        project_id: Option<Uuid>,
    ) -> Result<Vec<PendingPipelineItem>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT r.id, r.issue_id, r.project_id, r.workspace_id, r.template_key, \
                    r.template_version, r.status, r.current_stage_key, r.created_at, \
                    r.updated_at, r.finished_at, \
                    s.id AS s_id, s.run_id AS s_run_id, s.project_id AS s_project_id, \
                    s.stage_key AS s_stage_key, s.attempt AS s_attempt, s.status AS s_status, \
                    s.gate_kind AS s_gate_kind, s.session_id AS s_session_id, \
                    s.execution_process_id AS s_execution_process_id, \
                    s.started_at AS s_started_at, s.finished_at AS s_finished_at, \
                    s.summary AS s_summary, s.error AS s_error, \
                    i.simple_id AS issue_simple_id, i.title AS issue_title \
             FROM pipeline_runs r \
             JOIN pipeline_stage_runs s \
               ON s.rowid = (SELECT MAX(rowid) FROM pipeline_stage_runs WHERE run_id = r.id) \
             JOIN issues i ON i.id = r.issue_id \
             WHERE r.status IN ('waiting_gate', 'failed') AND (?1 IS NULL OR r.project_id = ?1) \
             ORDER BY COALESCE(s.finished_at, r.updated_at) ASC, r.rowid ASC",
        )
        .bind(project_id)
        .fetch_all(pool)
        .await?;
        rows.iter().map(pending_item_from_row).collect()
    }
}

fn pending_item_from_row(row: &SqliteRow) -> Result<PendingPipelineItem, sqlx::Error> {
    Ok(PendingPipelineItem {
        run: PipelineRun::from_row(row)?,
        stage_run: PipelineStageRun {
            id: row.try_get("s_id")?,
            run_id: row.try_get("s_run_id")?,
            project_id: row.try_get("s_project_id")?,
            stage_key: row.try_get("s_stage_key")?,
            attempt: row.try_get("s_attempt")?,
            status: row.try_get("s_status")?,
            gate_kind: row.try_get("s_gate_kind")?,
            session_id: row.try_get("s_session_id")?,
            execution_process_id: row.try_get("s_execution_process_id")?,
            started_at: row.try_get("s_started_at")?,
            finished_at: row.try_get("s_finished_at")?,
            summary: row.try_get("s_summary")?,
            error: row.try_get("s_error")?,
        },
        issue_simple_id: row.try_get("issue_simple_id")?,
        issue_title: row.try_get("issue_title")?,
    })
}

// ---------------------------------------------------------------------------
// pipeline_stage_runs
// ---------------------------------------------------------------------------

pub struct PipelineStageRuns;

impl PipelineStageRuns {
    /// 新建一次尝试；attempt = 同运行同阶段已有尝试数 + 1。status 为 Running 时写 started_at。
    pub async fn create(
        pool: &SqlitePool,
        data: &CreateStageRun,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let sql = format!(
            "INSERT INTO pipeline_stage_runs (id, run_id, project_id, stage_key, attempt, status, \
             gate_kind, feedback, started_at) \
             SELECT ?1, ?2, ?3, ?4, COALESCE(MAX(attempt), 0) + 1, ?5, ?6, ?7, ?8 \
             FROM pipeline_stage_runs WHERE run_id = ?2 AND stage_key = ?4 \
             RETURNING {STAGE_COLUMNS}"
        );
        retry_on_busy(|| async {
            let started_at = (data.status == PipelineStageStatus::Running).then(Utc::now);
            sqlx::query_as::<_, PipelineStageRun>(&sql)
                .bind(Uuid::new_v4())
                .bind(data.run_id)
                .bind(data.project_id)
                .bind(data.stage_key)
                .bind(data.status)
                .bind(data.gate_kind)
                .bind(&data.feedback)
                .bind(started_at)
                .fetch_one(pool)
                .await
        })
        .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!("SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE id = ?1");
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn find_by_rowid(
        pool: &SqlitePool,
        rowid: i64,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!("SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE rowid = ?1");
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(rowid)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE run_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(run_id)
            .fetch_all(pool)
            .await
    }

    pub async fn list_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE project_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(project_id)
            .fetch_all(pool)
            .await
    }

    pub async fn find_latest_for_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE run_id = ?1 \
             ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(run_id)
            .fetch_optional(pool)
            .await
    }

    /// 会话里正在跑的那条尝试。执行进程退出回调靠它认领。
    pub async fn find_running_by_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Option<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs \
             WHERE session_id = ?1 AND status = 'running' ORDER BY rowid DESC LIMIT 1"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .bind(session_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_running(pool: &SqlitePool) -> Result<Vec<PipelineStageRun>, sqlx::Error> {
        let sql = format!(
            "SELECT {STAGE_COLUMNS} FROM pipeline_stage_runs WHERE status = 'running' ORDER BY rowid"
        );
        sqlx::query_as::<_, PipelineStageRun>(&sql)
            .fetch_all(pool)
            .await
    }

    /// 同运行同阶段 status = failed 的尝试数（人工打回是 rejected，不计入）。
    pub async fn count_failed(
        pool: &SqlitePool,
        run_id: Uuid,
        stage_key: PipelineStageKey,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM pipeline_stage_runs \
             WHERE run_id = ?1 AND stage_key = ?2 AND status = 'failed'",
        )
        .bind(run_id)
        .bind(stage_key)
        .fetch_one(pool)
        .await
    }

    /// 该运行是否已经启动过任何会话（决定首个会话是否跑 setup 脚本）。
    pub async fn any_launched(pool: &SqlitePool, run_id: Uuid) -> Result<bool, sqlx::Error> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pipeline_stage_runs WHERE run_id = ?1 AND session_id IS NOT NULL",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn feedback(pool: &SqlitePool, id: Uuid) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT feedback FROM pipeline_stage_runs WHERE id = ?1")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        Ok(row.and_then(|(feedback,)| feedback))
    }

    /// 会话已开：记会话与首个进程，状态置 running，started_at 只写一次。
    ///
    /// 只允许从 pending（首次启动）或 running（已建成 running 的尝试补记会话）启动；
    /// 其它状态不改任何数据，返回 `sqlx::Error::RowNotFound`（尝试不存在时也是它）。
    pub async fn mark_started(
        pool: &SqlitePool,
        id: Uuid,
        session_id: Uuid,
        execution_process_id: Uuid,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_stage_runs SET status = 'running', session_id = ?2, \
             execution_process_id = ?3, started_at = COALESCE(started_at, ?4) \
             WHERE id = ?1 AND status IN ('pending', 'running') RETURNING {STAGE_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, PipelineStageRun>(&sql)
                .bind(id)
                .bind(session_id)
                .bind(execution_process_id)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }

    pub async fn set_execution_process(
        pool: &SqlitePool,
        id: Uuid,
        execution_process_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        retry_on_busy(|| async {
            sqlx::query("UPDATE pipeline_stage_runs SET execution_process_id = ?2 WHERE id = ?1")
                .bind(id)
                .bind(execution_process_id)
                .execute(pool)
                .await
        })
        .await?;
        Ok(())
    }

    /// 改尝试状态。summary / error 为 None 时保持原值。
    /// finished_at 表示「执行结束时刻」（契约修订 C11）：进入 waiting_gate 或终态时写入，
    /// 已有值不覆盖（waiting_gate → passed / rejected 保留进入等待的时刻）。
    ///
    /// 终态（passed / rejected / failed / skipped）不能改回非终态（pending / running /
    /// waiting_gate）：这种调用不改任何数据，返回 `sqlx::Error::RowNotFound`
    /// （尝试不存在时也是它）。终态之间的改写不受限。
    pub async fn set_status(
        pool: &SqlitePool,
        id: Uuid,
        status: PipelineStageStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<PipelineStageRun, sqlx::Error> {
        let sql = format!(
            "UPDATE pipeline_stage_runs SET status = ?2, summary = COALESCE(?3, summary), \
             error = COALESCE(?4, error), \
             finished_at = CASE WHEN ?2 IN ('waiting_gate', 'passed', 'rejected', 'failed', 'skipped') \
                                THEN COALESCE(finished_at, ?5) ELSE finished_at END \
             WHERE id = ?1 \
               AND NOT (status IN ('passed', 'rejected', 'failed', 'skipped') \
                        AND ?2 IN ('pending', 'running', 'waiting_gate')) \
             RETURNING {STAGE_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, PipelineStageRun>(&sql)
                .bind(id)
                .bind(status)
                .bind(summary)
                .bind(error)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// pipeline_gate_decisions
// ---------------------------------------------------------------------------

pub struct PipelineGateDecisions;

impl PipelineGateDecisions {
    pub async fn create(
        pool: &SqlitePool,
        stage_run_id: Uuid,
        decision: GateDecisionKind,
        comment: Option<&str>,
        decided_by: Option<Uuid>,
    ) -> Result<PipelineGateDecision, sqlx::Error> {
        let sql = format!(
            "INSERT INTO pipeline_gate_decisions (id, stage_run_id, decision, comment, decided_by, decided_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING {DECISION_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, PipelineGateDecision>(&sql)
                .bind(Uuid::new_v4())
                .bind(stage_run_id)
                .bind(decision)
                .bind(comment)
                .bind(decided_by)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }

    pub async fn list_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<PipelineGateDecision>, sqlx::Error> {
        sqlx::query_as::<_, PipelineGateDecision>(
            "SELECT d.id, d.stage_run_id, d.decision, d.comment, d.decided_by, d.decided_at \
             FROM pipeline_gate_decisions d \
             JOIN pipeline_stage_runs s ON s.id = d.stage_run_id \
             WHERE s.run_id = ?1 ORDER BY d.decided_at, d.rowid",
        )
        .bind(run_id)
        .fetch_all(pool)
        .await
    }
}

// ---------------------------------------------------------------------------
// issue_artifacts
// ---------------------------------------------------------------------------

/// 入库前截断：不超过 256 KB 原样返回；超过则在字符边界截断并追加标记。
pub fn truncate_for_storage(content: &str) -> (String, bool) {
    if content.len() <= MAX_ARTIFACT_BYTES {
        return (content.to_string(), false);
    }
    let mut end = MAX_ARTIFACT_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut stored = content[..end].to_string();
    stored.push_str(TRUNCATION_MARKER);
    (stored, true)
}

pub struct IssueArtifacts;

impl IssueArtifacts {
    /// 写一个新版本；version = 同需求同 kind 的最大版本 + 1。content 超限会被截断。
    pub async fn insert_version(
        pool: &SqlitePool,
        issue_id: Uuid,
        stage_run_id: Uuid,
        kind: ArtifactKind,
        rel_path: &str,
        content: &str,
    ) -> Result<IssueArtifactSummary, sqlx::Error> {
        let (stored, truncated) = truncate_for_storage(content);
        let sql = format!(
            "INSERT INTO issue_artifacts (id, issue_id, stage_run_id, kind, rel_path, content, \
             truncated, version, created_at) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(MAX(version), 0) + 1, ?8 \
             FROM issue_artifacts WHERE issue_id = ?2 AND kind = ?4 \
             RETURNING {ARTIFACT_SUMMARY_COLUMNS}"
        );
        retry_on_busy(|| async {
            sqlx::query_as::<_, IssueArtifactSummary>(&sql)
                .bind(Uuid::new_v4())
                .bind(issue_id)
                .bind(stage_run_id)
                .bind(kind)
                .bind(rel_path)
                .bind(&stored)
                .bind(truncated)
                .bind(Utc::now())
                .fetch_one(pool)
                .await
        })
        .await
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<IssueArtifact>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS}, content FROM issue_artifacts WHERE id = ?1"
        );
        sqlx::query_as::<_, IssueArtifact>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn latest_for_kind(
        pool: &SqlitePool,
        issue_id: Uuid,
        kind: ArtifactKind,
    ) -> Result<Option<IssueArtifact>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS}, content FROM issue_artifacts \
             WHERE issue_id = ?1 AND kind = ?2 ORDER BY version DESC LIMIT 1"
        );
        sqlx::query_as::<_, IssueArtifact>(&sql)
            .bind(issue_id)
            .bind(kind)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_summaries_for_issue(
        pool: &SqlitePool,
        issue_id: Uuid,
    ) -> Result<Vec<IssueArtifactSummary>, sqlx::Error> {
        let sql = format!(
            "SELECT {ARTIFACT_SUMMARY_COLUMNS} FROM issue_artifacts WHERE issue_id = ?1 ORDER BY rowid"
        );
        sqlx::query_as::<_, IssueArtifactSummary>(&sql)
            .bind(issue_id)
            .fetch_all(pool)
            .await
    }
}
```

在 `crates/db/src/models/mod.rs` 的 `pub mod merge;` 之后加一行：

```rust
pub mod pipeline;
```

Run: `cargo check -p db`
Expected: `Finished`，无 error。

- [ ] **Step 2: 写测试**

在 `crates/db/src/models/pipeline.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use api_types::{
        issue::{CreateIssueRequest, Issue},
        project::CreateProjectRequest,
    };
    use uuid::Uuid;

    use super::*;
    use crate::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
        },
        test_support::TestDb,
    };

    async fn 准备需求(test_db: &TestDb, title: &str) -> Issue {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "流水线项目".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let backlog = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Backlog)
            .await
            .unwrap()
            .unwrap();
        Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: backlog.id,
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
            },
            DEFAULT_USER_ID,
        )
        .await
        .unwrap()
    }

    fn 建运行参数(issue: &Issue) -> CreatePipelineRun {
        CreatePipelineRun {
            issue_id: issue.id,
            project_id: issue.project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: PipelineStageKey::Requirement,
        }
    }

    fn 建阶段参数(
        run: &PipelineRun,
        stage_key: PipelineStageKey,
        status: PipelineStageStatus,
    ) -> CreateStageRun {
        CreateStageRun {
            run_id: run.id,
            project_id: run.project_id,
            stage_key,
            gate_kind: GateKind::Human,
            status,
            feedback: None,
        }
    }

    #[tokio::test]
    async fn 创建运行后可按需求查到最近一条且内部列不外露() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "导出报表").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();

        assert_eq!(run.status, PipelineRunStatus::Running);
        assert_eq!(run.current_stage_key, PipelineStageKey::Requirement);
        assert!(run.finished_at.is_none());

        let latest = PipelineRuns::find_latest_for_issue(test_db.pool(), issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, run);

        let internals = PipelineRuns::internals(test_db.pool(), run.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(internals.executor_config_json, "{}");

        let json = serde_json::to_value(&run).unwrap();
        assert!(
            json.get("template_json").is_none(),
            "内部列不能出现在对外类型里"
        );
        assert_eq!(json["status"], "running");
        assert_eq!(json["current_stage_key"], "requirement");
    }

    #[tokio::test]
    async fn 同一需求不能同时有两条未结束的运行() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "唯一").await;
        PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let second = PipelineRuns::create(test_db.pool(), &建运行参数(&issue)).await;
        let error = second.expect_err("第二条未结束的运行必须被唯一索引拒绝");
        assert!(crate::models::db_retry::is_unique_violation(&error));
        assert!(
            PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn 运行取消后可以再开一条且写结束时间() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "重开").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let cancelled =
            PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Cancelled, None)
                .await
                .unwrap();
        assert!(cancelled.finished_at.is_some());
        assert_eq!(cancelled.current_stage_key, PipelineStageKey::Requirement);
        assert!(
            !PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
        PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .expect("旧运行结束后应能新开");
    }

    #[tokio::test]
    async fn 失败的运行算未结束() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "失败").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let failed =
            PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Failed, None)
                .await
                .unwrap();
        assert!(failed.finished_at.is_none(), "failed 可继续，不写结束时间");
        assert!(
            PipelineRuns::has_active_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn 阶段尝试号按阶段递增() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "尝试号").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let pool = test_db.pool();
        let a1 = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        let a2 = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Pending,
            ),
        )
        .await
        .unwrap();
        let s1 = PipelineStageRuns::create(
            pool,
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        assert_eq!((a1.attempt, a2.attempt, s1.attempt), (1, 2, 1));
        assert!(a1.started_at.is_some(), "running 创建即写开始时间");
        assert!(a2.started_at.is_none(), "pending 不写开始时间");

        let all = PipelineStageRuns::list_by_run(pool, run.id).await.unwrap();
        assert_eq!(
            all.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![a1.id, a2.id, s1.id],
            "按创建顺序返回"
        );
        assert_eq!(
            PipelineStageRuns::find_latest_for_run(pool, run.id)
                .await
                .unwrap()
                .unwrap()
                .id,
            s1.id
        );
    }

    #[tokio::test]
    async fn 进入等待确认或失败即写结束时间且后续转终态不覆盖() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "终态").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        assert!(stage.finished_at.is_none(), "running 不写结束时间");

        // C11：finished_at 表示「执行结束时刻」，进入 waiting_gate 就写。
        let waiting = PipelineStageRuns::set_status(
            test_db.pool(),
            stage.id,
            PipelineStageStatus::WaitingGate,
            Some("等待人工确认"),
            None,
        )
        .await
        .unwrap();
        assert!(
            waiting.finished_at.is_some(),
            "进入 waiting_gate 写结束时间"
        );
        assert_eq!(waiting.summary.as_deref(), Some("等待人工确认"));

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let passed = PipelineStageRuns::set_status(
            test_db.pool(),
            stage.id,
            PipelineStageStatus::Passed,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            passed.finished_at, waiting.finished_at,
            "人工通过不覆盖执行结束时刻"
        );
        assert_eq!(
            passed.summary.as_deref(),
            Some("等待人工确认"),
            "None 不覆盖原值"
        );

        // failed 同样写结束时间，error 按传入值落库。
        let second = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let failed = PipelineStageRuns::set_status(
            test_db.pool(),
            second.id,
            PipelineStageStatus::Failed,
            None,
            Some("进程退出码 1"),
        )
        .await
        .unwrap();
        assert!(failed.finished_at.is_some(), "进入 failed 写结束时间");
        assert_eq!(failed.error.as_deref(), Some("进程退出码 1"));
    }

    #[tokio::test]
    async fn 失败次数只数_failed_且按会话认领正在跑的尝试() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "计数").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let pool = test_db.pool();
        for status in [PipelineStageStatus::Failed, PipelineStageStatus::Rejected] {
            let stage = PipelineStageRuns::create(
                pool,
                &建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Running),
            )
            .await
            .unwrap();
            PipelineStageRuns::set_status(pool, stage.id, status, None, None)
                .await
                .unwrap();
        }
        assert_eq!(
            PipelineStageRuns::count_failed(pool, run.id, PipelineStageKey::Review)
                .await
                .unwrap(),
            1
        );
        assert!(!PipelineStageRuns::any_launched(pool, run.id).await.unwrap());

        let running = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                feedback: Some("上次失败原因".to_string()),
                ..建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Pending)
            },
        )
        .await
        .unwrap();
        let session_id = Uuid::new_v4();
        let process_id = Uuid::new_v4();
        let started = PipelineStageRuns::mark_started(pool, running.id, session_id, process_id)
            .await
            .unwrap();
        assert_eq!(started.status, PipelineStageStatus::Running);
        assert!(started.started_at.is_some());
        assert_eq!(started.execution_process_id, Some(process_id));
        assert!(PipelineStageRuns::any_launched(pool, run.id).await.unwrap());
        assert_eq!(
            PipelineStageRuns::feedback(pool, running.id)
                .await
                .unwrap()
                .as_deref(),
            Some("上次失败原因")
        );
        assert_eq!(
            PipelineStageRuns::find_running_by_session(pool, session_id)
                .await
                .unwrap()
                .unwrap()
                .id,
            running.id
        );
        assert!(
            PipelineStageRuns::find_running_by_session(pool, Uuid::new_v4())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 产出物版本号按需求和种类递增() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "产出物").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let pool = test_db.pool();
        let v1 = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Spec,
            ".vk/runs/X/spec.md",
            "第一版",
        )
        .await
        .unwrap();
        let v2 = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Spec,
            ".vk/runs/X/spec.md",
            "第二版",
        )
        .await
        .unwrap();
        let plan = IssueArtifacts::insert_version(
            pool,
            issue.id,
            stage.id,
            ArtifactKind::Plan,
            ".vk/runs/X/plan.md",
            "计划",
        )
        .await
        .unwrap();
        assert_eq!((v1.version, v2.version, plan.version), (1, 2, 1));
        assert!(!v1.truncated);

        let latest = IssueArtifacts::latest_for_kind(pool, issue.id, ArtifactKind::Spec)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.content, "第二版");
        assert_eq!(latest.summary.id, v2.id);

        let fetched = IssueArtifacts::find_by_id(pool, plan.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.content, "计划");
        let json = serde_json::to_value(&fetched).unwrap();
        assert_eq!(json["kind"], "plan", "summary 字段平铺在顶层");
        assert_eq!(json["content"], "计划");

        let all = IssueArtifacts::list_summaries_for_issue(pool, issue.id)
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn 超过_256kb_的产出物截断入库并打标() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "大文件").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(&run, PipelineStageKey::Review, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let big = "测".repeat(MAX_ARTIFACT_BYTES / 3 + 10);
        let summary = IssueArtifacts::insert_version(
            test_db.pool(),
            issue.id,
            stage.id,
            ArtifactKind::Review,
            ".vk/runs/X/review.json",
            &big,
        )
        .await
        .unwrap();
        assert!(summary.truncated);
        let stored = IssueArtifacts::find_by_id(test_db.pool(), summary.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.content.ends_with(TRUNCATION_MARKER));
        assert!(stored.content.len() <= MAX_ARTIFACT_BYTES + TRUNCATION_MARKER.len());
    }

    #[test]
    fn 截断落在字符边界上() {
        // 「测」是 3 字节，256 KB 不是 3 的倍数，截断点必须回退到字符边界。
        let text = "测".repeat(MAX_ARTIFACT_BYTES / 3 + 1);
        let (stored, truncated) = truncate_for_storage(&text);
        assert!(truncated);
        let body = stored.strip_suffix(TRUNCATION_MARKER).unwrap();
        assert!(body.chars().all(|c| c == '测'));
        assert!(body.len() <= MAX_ARTIFACT_BYTES);

        let (short, short_truncated) = truncate_for_storage("短文本");
        assert_eq!((short.as_str(), short_truncated), ("短文本", false));
    }

    #[tokio::test]
    async fn 待处理列表只含等待确认与失败且等得久的在前() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let a = 准备需求(&test_db, "甲").await;
        let b = 准备需求(&test_db, "乙").await;
        let c = 准备需求(&test_db, "丙").await;

        let mut runs = Vec::new();
        for issue in [&a, &b, &c] {
            let run = PipelineRuns::create(pool, &建运行参数(issue))
                .await
                .unwrap();
            PipelineStageRuns::create(
                pool,
                &建阶段参数(
                    &run,
                    PipelineStageKey::Requirement,
                    PipelineStageStatus::Running,
                ),
            )
            .await
            .unwrap();
            runs.push(run);
        }
        // 乙先进入等待，甲后失败，丙仍在跑。
        PipelineRuns::update_status(pool, runs[1].id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        PipelineRuns::update_status(pool, runs[0].id, PipelineRunStatus::Failed, None)
            .await
            .unwrap();

        let pending = PipelineRuns::list_pending(pool, None).await.unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|p| p.issue_title.as_str())
                .collect::<Vec<_>>(),
            vec!["乙", "甲"]
        );
        assert_eq!(
            pending[0].stage_run.stage_key,
            PipelineStageKey::Requirement
        );
        assert_eq!(pending[0].issue_simple_id, b.simple_id);

        let only_a = PipelineRuns::list_pending(pool, Some(a.project_id))
            .await
            .unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].run.issue_id, a.id);
    }

    #[tokio::test]
    async fn 关卡决策按运行列出() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "决策").await;
        let run = PipelineRuns::create(test_db.pool(), &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        PipelineGateDecisions::create(
            test_db.pool(),
            stage.id,
            GateDecisionKind::Reject,
            Some("补充边界"),
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
        PipelineGateDecisions::create(
            test_db.pool(),
            stage.id,
            GateDecisionKind::Approve,
            None,
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
        let decisions = PipelineGateDecisions::list_by_run(test_db.pool(), run.id)
            .await
            .unwrap();
        assert_eq!(
            decisions.iter().map(|d| d.decision).collect::<Vec<_>>(),
            vec![GateDecisionKind::Reject, GateDecisionKind::Approve]
        );
        assert_eq!(decisions[0].comment.as_deref(), Some("补充边界"));
    }

    #[test]
    fn 枚举序列化与库内取值一致() {
        assert_eq!(
            serde_json::to_value(PipelineStageKey::TestDesign).unwrap(),
            "test_design"
        );
        assert_eq!(
            serde_json::to_value(PipelineRunStatus::WaitingGate).unwrap(),
            "waiting_gate"
        );
        assert_eq!(
            serde_json::to_value(ArtifactKind::DeliveryReport).unwrap(),
            "delivery_report"
        );
        for key in PipelineStageKey::ALL {
            assert_eq!(PipelineStageKey::parse(key.as_str()), Some(key));
        }
        assert_eq!(
            ArtifactKind::from_file_name("test-cases.csv"),
            Some(ArtifactKind::TestCases)
        );
        assert_eq!(ArtifactKind::from_file_name("notes.md"), None);
    }

    #[test]
    fn 模板阶段视图序列化带_gate_condition() {
        let auto = PipelineTemplateStageView {
            key: PipelineStageKey::Develop,
            skill: "develop".to_string(),
            gate_kind: GateKind::Auto,
            gate_label: None,
            gate_condition: Some("checks_passed".to_string()),
            max_rounds: 3,
        };
        let json = serde_json::to_value(&auto).unwrap();
        assert_eq!(json["gate_condition"], "checks_passed");
        assert_eq!(json["gate_kind"], "auto");
        assert_eq!(json["max_rounds"], 3);

        let human = PipelineTemplateStageView {
            key: PipelineStageKey::Requirement,
            skill: "requirement".to_string(),
            gate_kind: GateKind::Human,
            gate_label: Some("需求确认".to_string()),
            gate_condition: None,
            max_rounds: 3,
        };
        let json = serde_json::to_value(&human).unwrap();
        assert!(
            json["gate_condition"].is_null(),
            "人工关卡的 gate_condition 为 null"
        );
        let back: PipelineTemplateStageView = serde_json::from_value(json).unwrap();
        assert_eq!(back, human);
    }

    #[tokio::test]
    async fn 待处理列表按阶段执行结束时刻排序而非运行更新时间() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let a = 准备需求(&test_db, "甲").await;
        let b = 准备需求(&test_db, "乙").await;
        let pause = || tokio::time::sleep(std::time::Duration::from_millis(5));

        let run_a = PipelineRuns::create(pool, &建运行参数(&a)).await.unwrap();
        let run_b = PipelineRuns::create(pool, &建运行参数(&b)).await.unwrap();
        let stage_a = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run_a,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        let stage_b = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run_b,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();

        // 甲的阶段先结束进入等待，乙的后结束；但甲的运行行最后才更新。
        PipelineStageRuns::set_status(
            pool,
            stage_a.id,
            PipelineStageStatus::WaitingGate,
            None,
            None,
        )
        .await
        .unwrap();
        pause().await;
        PipelineStageRuns::set_status(
            pool,
            stage_b.id,
            PipelineStageStatus::WaitingGate,
            None,
            None,
        )
        .await
        .unwrap();
        pause().await;
        PipelineRuns::update_status(pool, run_b.id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();
        pause().await;
        PipelineRuns::update_status(pool, run_a.id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();

        let pending = PipelineRuns::list_pending(pool, None).await.unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|p| p.issue_title.as_str())
                .collect::<Vec<_>>(),
            vec!["甲", "乙"],
            "按阶段 finished_at 排序：甲等得更久"
        );
        assert_eq!(pending[0].stage_run.id, stage_a.id);
        assert_eq!(
            pending[0].stage_run.status,
            PipelineStageStatus::WaitingGate
        );
        assert!(pending[0].stage_run.finished_at.is_some());
        assert_eq!(pending[0].run.id, run_a.id);
        assert_eq!(pending[0].issue_simple_id, a.simple_id);
    }

    #[tokio::test]
    async fn 只允许从待启动或运行中标记启动() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let issue = 准备需求(&test_db, "启动").await;
        let run = PipelineRuns::create(pool, &建运行参数(&issue))
            .await
            .unwrap();

        let running = PipelineStageRuns::create(
            pool,
            &建阶段参数(
                &run,
                PipelineStageKey::Requirement,
                PipelineStageStatus::Running,
            ),
        )
        .await
        .unwrap();
        PipelineStageRuns::mark_started(pool, running.id, Uuid::new_v4(), Uuid::new_v4())
            .await
            .expect("running 的尝试可以补记会话");

        let waiting = PipelineStageRuns::set_status(
            pool,
            running.id,
            PipelineStageStatus::WaitingGate,
            None,
            None,
        )
        .await
        .unwrap();
        let error =
            PipelineStageRuns::mark_started(pool, waiting.id, Uuid::new_v4(), Uuid::new_v4())
                .await
                .expect_err("waiting_gate 不能被重新启动");
        assert!(
            matches!(error, sqlx::Error::RowNotFound),
            "实际错误：{error:?}"
        );
        let unchanged = PipelineStageRuns::find_by_id(pool, waiting.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged, waiting, "拒绝时不改任何数据");

        let missing =
            PipelineStageRuns::mark_started(pool, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .expect_err("不存在的尝试");
        assert!(matches!(missing, sqlx::Error::RowNotFound));
    }

    #[tokio::test]
    async fn 终态不能改回非终态但终态之间可以改() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let issue = 准备需求(&test_db, "终态保护").await;
        let run = PipelineRuns::create(pool, &建运行参数(&issue))
            .await
            .unwrap();
        let stage = PipelineStageRuns::create(
            pool,
            &建阶段参数(&run, PipelineStageKey::Spec, PipelineStageStatus::Running),
        )
        .await
        .unwrap();
        let passed =
            PipelineStageRuns::set_status(pool, stage.id, PipelineStageStatus::Passed, None, None)
                .await
                .unwrap();

        for status in [
            PipelineStageStatus::Pending,
            PipelineStageStatus::Running,
            PipelineStageStatus::WaitingGate,
        ] {
            let error =
                PipelineStageRuns::set_status(pool, stage.id, status, Some("不该写入"), None)
                    .await
                    .expect_err("终态不能改回非终态");
            assert!(
                matches!(error, sqlx::Error::RowNotFound),
                "{status:?}：{error:?}"
            );
        }
        let unchanged = PipelineStageRuns::find_by_id(pool, stage.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged, passed, "拒绝时不改任何数据");

        let skipped =
            PipelineStageRuns::set_status(pool, stage.id, PipelineStageStatus::Skipped, None, None)
                .await
                .expect("终态之间可以改");
        assert_eq!(skipped.status, PipelineStageStatus::Skipped);
        assert_eq!(skipped.finished_at, passed.finished_at);
    }

    #[tokio::test]
    async fn 运行结束时间只写一次且回到非终态时清空() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let issue = 准备需求(&test_db, "运行结束时间").await;
        let run = PipelineRuns::create(pool, &建运行参数(&issue))
            .await
            .unwrap();

        let completed =
            PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Completed, None)
                .await
                .unwrap();
        assert!(completed.finished_at.is_some());
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let cancelled =
            PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Cancelled, None)
                .await
                .unwrap();
        assert_eq!(
            cancelled.finished_at, completed.finished_at,
            "已有结束时间不覆盖"
        );
        assert!(cancelled.updated_at > completed.updated_at);

        let reopened = PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Paused, None)
            .await
            .unwrap();
        assert!(reopened.finished_at.is_none(), "非终态清空结束时间");
    }
}
```

- [ ] **Step 3: 故意制造一次失败，确认测试在跑**

把 `阶段尝试号按阶段递增` 里的 `(1, 2, 1)` 临时改成 `(1, 1, 1)`。

Run: `cargo test -p db models::pipeline::tests::阶段尝试号按阶段递增`
Expected: FAIL，`assertion \`left == right\` failed`，left 为 `(1, 2, 1)`。改回 `(1, 2, 1)`。

- [ ] **Step 4: 运行全部模型测试**

Run: `cargo test -p db models::pipeline`
Expected: `test result: ok. 18 passed; 0 failed`
（**已核实**：sqlx 0.8.6 对字符串枚举的 SQLite 派生就是 `type_info = <str>::type_info()`、`compatible = <&str>::compatible()`（`~/.cargo/registry/src/*/sqlx-macros-core-0.8.6/src/derives/type.rs:218-223`），运行时 `query_as` 解码 TEXT 列没有问题。）

- [ ] **Step 5: 提交**

```bash
git add crates/db/src/models/pipeline.rs crates/db/src/models/mod.rs
git commit -m "$(cat <<'EOF'
模型：流水线运行、阶段尝试、关卡决策、产出物的类型与读写

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `ExecutionProcessRunReason::PipelineStep` 与 `ScriptContext::PipelineCheck`

**Files:**
- Modify: `crates/db/src/models/execution_process.rs:50-59`（枚举），文件末尾追加测试模块
- Modify: `crates/executors/src/actions/script.rs:21-28`（枚举）

**核实结论（不需要改其它 match）：** 全仓匹配 `ExecutionProcessRunReason` 的地方都是 `matches!`、构造或带 `_ =>` 的 match（`crates/local-deployment/src/container.rs:345-392`），加变体不产生非穷尽匹配错误。`try_start_next_action`（`crates/services/src/services/container.rs:1358-1389`）按 action 类型推 run_reason，不看当前进程的 run_reason；引擎发出的检查脚本 action 不带 `next_action`，走 `Ok(())` 早返回。`should_finalize`（同文件 :204-236）对 `PipelineStep` 的行为与普通脚本一致（成功且无 next → finalize）。`ScriptContext` 只被构造，不被匹配。

- [ ] **Step 1: 写失败测试**

在 `crates/db/src/models/execution_process.rs` 文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use executors::actions::{
        ExecutorAction, ExecutorActionType,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    };
    use uuid::Uuid;

    use super::{CreateExecutionProcess, ExecutionProcess, ExecutionProcessRunReason};
    use crate::{
        models::{
            local_project::DEFAULT_USER_ID,
            session::{CreateSession, Session},
            workspace::{CreateWorkspace, Workspace},
        },
        test_support::TestDb,
    };

    #[test]
    fn pipeline_step_序列化为全小写() {
        assert_eq!(
            serde_json::to_value(ExecutionProcessRunReason::PipelineStep).unwrap(),
            "pipelinestep"
        );
    }

    #[tokio::test]
    async fn pipeline_step_进程可以落库并读回() {
        let test_db = TestDb::new().await;
        let pool = test_db.pool();
        let workspace = Workspace::create(
            pool,
            &CreateWorkspace {
                branch: "vk/pipeline".to_string(),
                name: None,
            },
            Uuid::new_v4(),
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        let session = Session::create(
            pool,
            &CreateSession {
                executor: None,
                name: None,
            },
            Uuid::new_v4(),
            workspace.id,
        )
        .await
        .unwrap();
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script: "set -e\ntrue\n".to_string(),
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::PipelineCheck,
                working_dir: None,
            }),
            None,
        );
        let process = ExecutionProcess::create(
            pool,
            &CreateExecutionProcess {
                session_id: session.id,
                executor_action: action,
                run_reason: ExecutionProcessRunReason::PipelineStep,
            },
            Uuid::new_v4(),
            &[],
        )
        .await
        .expect("PipelineStep 进程应能落库");
        assert_eq!(process.run_reason, ExecutionProcessRunReason::PipelineStep);
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p db models::execution_process::tests`
Expected: 编译失败，`no variant ... named \`PipelineStep\``、`no variant ... named \`PipelineCheck\``。

- [ ] **Step 3: 最小实现**

`crates/db/src/models/execution_process.rs` 的枚举改为：

```rust
#[derive(Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS)]
#[sqlx(type_name = "execution_process_run_reason", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ExecutionProcessRunReason {
    SetupScript,
    CleanupScript,
    ArchiveScript,
    CodingAgent,
    DevServer,
    /// 流水线引擎自己跑的检查脚本（开发阶段的 checks）。库内值 `pipelinestep`，
    /// 迁移 20260918000000_add_pipeline.sql 放开了 CHECK。
    /// 流水线阶段的编码智能体进程仍然是 `CodingAgent`，见计划 A §4 纠正 1。
    PipelineStep,
}
```

`crates/executors/src/actions/script.rs` 的枚举改为：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub enum ScriptContext {
    SetupScript,
    CleanupScript,
    ArchiveScript,
    DevServer,
    ToolInstallScript,
    /// 流水线开发阶段的检查脚本（模板 `checks`）。
    PipelineCheck,
}
```

- [ ] **Step 4: 运行，确认通过并全量编译**

Run: `cargo test -p db models::execution_process::tests`
Expected: `test result: ok. 2 passed`

Run: `cargo check --workspace --features qa-mode`
Expected: `Finished`，无 error、无 non-exhaustive 报错。

- [ ] **Step 5: 提交**

```bash
git add crates/db/src/models/execution_process.rs crates/executors/src/actions/script.rs
git commit -m "$(cat <<'EOF'
执行进程：新增 PipelineStep 运行原因与 PipelineCheck 脚本上下文

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 实时推送（HookTables、patch、需求流首帧与过滤）

**Files:**
- Modify: `crates/services/src/services/events/types.rs:21-68`
- Modify: `crates/services/src/services/events/patches.rs:173-185`（宏调用之后追加）
- Modify: `crates/services/src/services/events.rs:23-26`（再导出）、`:81-144`（preupdate）、`:155-235`（update 取数）、`:258-376`（推 patch），tests 模块追加 2 个测试
- Modify: `crates/services/src/services/events/streams.rs:318-381`

- [ ] **Step 1: 写失败测试**

在 `crates/services/src/services/events.rs` 的 `mod tests` 里，`patch_path` 函数之后追加辅助函数：

```rust
    async fn insert_issue(pool: &sqlx::SqlitePool, project_id: Uuid, status_id: Uuid) -> Uuid {
        let issue_id = Uuid::new_v4();
        let number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(issue_number), 0) + 1 FROM issues WHERE project_id = ?1",
        )
        .bind(project_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO issues (id, project_id, issue_number, simple_id, status_id, title) \
             VALUES (?1, ?2, ?3, ?4, ?5, '流水线推送测试')",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(number)
        .bind(format!("PL-{number}"))
        .bind(status_id)
        .execute(pool)
        .await
        .expect("插入 issue 失败");
        issue_id
    }

    fn run_params(
        project_id: Uuid,
        issue_id: Uuid,
    ) -> db::models::pipeline::CreatePipelineRun {
        db::models::pipeline::CreatePipelineRun {
            issue_id,
            project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: db::models::pipeline::PipelineStageKey::Requirement,
        }
    }
```

在 `mod tests` 末尾追加两个测试：

```rust
    #[tokio::test]
    async fn 流水线运行与阶段的增改删都产出_json_patch() {
        use db::models::pipeline::{
            CreateStageRun, GateKind, PipelineRunStatus, PipelineRuns, PipelineStageKey,
            PipelineStageRuns, PipelineStageStatus,
        };

        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_id, status_id) = insert_project_and_status(&pool).await;
        let issue_id = insert_issue(&pool, project_id, status_id).await;

        let run = PipelineRuns::create(&pool, &run_params(project_id, issue_id))
            .await
            .unwrap();
        let run_path = format!("/pipeline_runs/{}", run.id);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        PipelineRuns::update_status(&pool, run.id, PipelineRunStatus::Paused, None)
            .await
            .unwrap();
        let replace = wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path
                && matches!(op, PatchOperation::Replace(r) if r.value["status"] == "paused")
        })
        .await;
        let PatchOperation::Replace(replace) = replace else {
            unreachable!()
        };
        assert_eq!(replace.value["project_id"], project_id.to_string());

        let stage = PipelineStageRuns::create(
            &pool,
            &CreateStageRun {
                run_id: run.id,
                project_id,
                stage_key: PipelineStageKey::Requirement,
                gate_kind: GateKind::Human,
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await
        .unwrap();
        let stage_path = format!("/pipeline_stage_runs/{}", stage.id);
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == stage_path && matches!(op, PatchOperation::Add(_))
        })
        .await;

        sqlx::query("DELETE FROM pipeline_stage_runs WHERE id = ?1")
            .bind(stage.id)
            .execute(&pool)
            .await
            .unwrap();
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == stage_path && matches!(op, PatchOperation::Remove(_))
        })
        .await;

        sqlx::query("DELETE FROM pipeline_runs WHERE id = ?1")
            .bind(run.id)
            .execute(&pool)
            .await
            .unwrap();
        wait_for_patch(&fixture.msg_store, |op| {
            patch_path(op) == run_path && matches!(op, PatchOperation::Remove(_))
        })
        .await;
    }

    #[tokio::test]
    async fn 需求流首帧带流水线快照且只转发本项目的流水线增量() {
        use db::models::pipeline::PipelineRuns;
        use futures::StreamExt;

        use super::pipeline_run_patch;

        let fixture = setup(SqliteJournalMode::Wal).await;
        let pool = fixture.db.pool.clone();
        let (project_a, status_a) = insert_project_and_status(&pool).await;
        let (project_b, status_b) = insert_project_and_status(&pool).await;
        let issue_a = insert_issue(&pool, project_a, status_a).await;
        let issue_b = insert_issue(&pool, project_b, status_b).await;
        let run_a = PipelineRuns::create(&pool, &run_params(project_a, issue_a))
            .await
            .unwrap();
        let run_b = PipelineRuns::create(&pool, &run_params(project_b, issue_b))
            .await
            .unwrap();

        let events = EventService::new(
            fixture.db.clone(),
            fixture.msg_store.clone(),
            Arc::new(RwLock::new(0)),
        );
        let mut stream = events.stream_issues_raw(project_a).await.unwrap();

        let Some(Ok(LogMsg::JsonPatch(first))) = stream.next().await else {
            panic!("首帧应是 JSON Patch");
        };
        let snapshot = serde_json::to_value(&first).unwrap();
        let ops = snapshot.as_array().unwrap();
        let runs_op = ops
            .iter()
            .find(|op| op["path"] == "/pipeline_runs")
            .expect("首帧必须带 /pipeline_runs");
        assert!(runs_op["value"].get(run_a.id.to_string()).is_some());
        assert!(
            runs_op["value"].get(run_b.id.to_string()).is_none(),
            "首帧不得带别的项目的运行"
        );
        assert!(
            ops.iter().any(|op| op["path"] == "/pipeline_stage_runs"),
            "首帧必须带 /pipeline_stage_runs"
        );

        fixture
            .msg_store
            .push_patch(pipeline_run_patch::replace(&run_b));
        fixture
            .msg_store
            .push_patch(pipeline_run_patch::replace(&run_a));

        let path_a = format!("/pipeline_runs/{}", run_a.id);
        let path_b = format!("/pipeline_runs/{}", run_b.id);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let Some(msg) = stream.next().await else {
                    panic!("需求流意外结束");
                };
                let Ok(LogMsg::JsonPatch(patch)) = msg else {
                    continue;
                };
                let path = patch.0[0].path().to_string();
                assert_ne!(path, path_b, "不得转发别的项目的流水线");
                if path == path_a {
                    return;
                }
            }
        })
        .await
        .expect("应收到本项目流水线的增量");
    }
```

在 `crates/services/src/services/events/streams.rs` 文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::is_issue_stream_path;

    #[test]
    fn 需求流转发流水线两张表的路径() {
        for path in [
            "/issues/1",
            "/project_statuses/1",
            "/issue_comments/1",
            "/pipeline_runs/1",
            "/pipeline_stage_runs/1",
        ] {
            assert!(is_issue_stream_path(path), "{path} 应转发");
        }
        for path in ["/workspaces/1", "/execution_processes/1", "/scratch"] {
            assert!(!is_issue_stream_path(path), "{path} 不应转发");
        }
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p services events`
Expected: 编译失败，`cannot find ... \`pipeline_run_patch\``、`cannot find function \`is_issue_stream_path\``。

- [ ] **Step 3: 实现——types.rs**

`crates/services/src/services/events/types.rs` 的 `HookTables` 末尾（`IssueComments` 之后）加：

```rust
    #[strum(to_string = "pipeline_runs")]
    PipelineRuns,
    #[strum(to_string = "pipeline_stage_runs")]
    PipelineStageRuns,
```

`RecordTypes` 末尾（`DeletedIssueComment { rowid: i64 },` 之后）加：

```rust
    PipelineRun(db::models::pipeline::PipelineRun),
    PipelineStageRun(db::models::pipeline::PipelineStageRun),
    DeletedPipelineRun {
        rowid: i64,
    },
    DeletedPipelineStageRun {
        rowid: i64,
    },
```

- [ ] **Step 4: 实现——patches.rs**

在 `crates/services/src/services/events/patches.rs` 的 `id_keyed_patch_module!(issue_comment_patch, ...)` 调用之后追加：

```rust
// 流水线两张表（契约 §3）：路径 /pipeline_runs/{id}、/pipeline_stage_runs/{id}，
// 值带 project_id，需求流按它过滤。
id_keyed_patch_module!(
    pipeline_run_patch,
    "/pipeline_runs",
    db::models::pipeline::PipelineRun
);
id_keyed_patch_module!(
    pipeline_stage_run_patch,
    "/pipeline_stage_runs",
    db::models::pipeline::PipelineStageRun
);
```

- [ ] **Step 5: 实现——events.rs**

1）`:23-26` 的再导出改为：

```rust
pub use patches::{
    execution_process_patch, issue_comment_patch, issue_patch, pipeline_run_patch,
    pipeline_stage_run_patch, project_status_patch, scratch_patch, workspace_patch,
};
```

2）preupdate 的 `match preupdate.table {`（:85）里，在 `"issue_comments" => { ... }` 分支之后、`_ => {}` 之前加：

```rust
                            "pipeline_runs" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(run_id) = <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(pipeline_run_patch::remove(run_id));
                                }
                            }
                            "pipeline_stage_runs" => {
                                if let Ok(value) = preupdate.get_old_column_value(0)
                                    && let Ok(stage_run_id) =
                                        <Uuid as Decode<Sqlite>>::decode(value)
                                {
                                    msg_store_for_preupdate
                                        .push_patch(pipeline_stage_run_patch::remove(stage_run_id));
                                }
                            }
```

3）update hook 里「DELETE 直接 return」的 match 臂（:156-163）改为：

```rust
                                (HookTables::Workspaces, SqliteOperation::Delete)
                                | (HookTables::ExecutionProcesses, SqliteOperation::Delete)
                                | (HookTables::Scratch, SqliteOperation::Delete)
                                | (HookTables::Issues, SqliteOperation::Delete)
                                | (HookTables::ProjectStatuses, SqliteOperation::Delete)
                                | (HookTables::IssueComments, SqliteOperation::Delete)
                                | (HookTables::PipelineRuns, SqliteOperation::Delete)
                                | (HookTables::PipelineStageRuns, SqliteOperation::Delete) => {
                                    return;
                                }
```

在 `(HookTables::IssueComments, _) => { ... }` 臂之后加：

```rust
                                (HookTables::PipelineRuns, _) => {
                                    match db::models::pipeline::PipelineRuns::find_by_rowid(
                                        &db.pool, rowid,
                                    )
                                    .await
                                    {
                                        Ok(Some(run)) => RecordTypes::PipelineRun(run),
                                        Ok(None) => RecordTypes::DeletedPipelineRun { rowid },
                                        Err(e) => {
                                            tracing::error!(
                                                "读取 pipeline_run rowid={} 失败: {}",
                                                rowid,
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
                                (HookTables::PipelineStageRuns, _) => {
                                    match db::models::pipeline::PipelineStageRuns::find_by_rowid(
                                        &db.pool, rowid,
                                    )
                                    .await
                                    {
                                        Ok(Some(stage_run)) => {
                                            RecordTypes::PipelineStageRun(stage_run)
                                        }
                                        Ok(None) => {
                                            RecordTypes::DeletedPipelineStageRun { rowid }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "读取 pipeline_stage_run rowid={} 失败: {}",
                                                rowid,
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
```

4）推 patch 的 `match &record_type {` 里，在 `RecordTypes::IssueComment(comment) => { ... return; }` 之后、`_ => {}` 之前加：

```rust
                                RecordTypes::PipelineRun(run) => {
                                    let patch = match hook.operation {
                                        SqliteOperation::Insert => pipeline_run_patch::add(run),
                                        _ => pipeline_run_patch::replace(run),
                                    };
                                    msg_store_for_hook.push_patch(patch);
                                    return;
                                }
                                RecordTypes::PipelineStageRun(stage_run) => {
                                    let patch = match hook.operation {
                                        SqliteOperation::Insert => {
                                            pipeline_stage_run_patch::add(stage_run)
                                        }
                                        _ => pipeline_stage_run_patch::replace(stage_run),
                                    };
                                    msg_store_for_hook.push_patch(patch);
                                    return;
                                }
                                // 删除已由 preupdate 推过 remove，这里不再走旧的 entries 兜底格式。
                                RecordTypes::DeletedPipelineRun { .. }
                                | RecordTypes::DeletedPipelineStageRun { .. } => {
                                    return;
                                }
```

- [ ] **Step 6: 实现——streams.rs**

把 `stream_issues_raw`（:318-381）整体替换为（并在函数之前加常量与辅助函数）：

```rust
/// 需求流转发的 patch 路径前缀（契约 §3）。
const ISSUE_STREAM_PREFIXES: [&str; 5] = [
    "/issues",
    "/project_statuses",
    "/issue_comments",
    "/pipeline_runs",
    "/pipeline_stage_runs",
];

fn is_issue_stream_path(path: &str) -> bool {
    ISSUE_STREAM_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}
```

（上面两项放在 `impl EventService {` **之前**的模块顶层。）

```rust
    /// 需求看板流：首帧给出 issues / project_statuses / pipeline_runs / pipeline_stage_runs 全量，
    /// 之后只转发属于该项目的增量 patch。
    pub async fn stream_issues_raw(
        &self,
        project_id: uuid::Uuid,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<LogMsg, std::io::Error>>,
        super::types::EventError,
    > {
        use db::models::{
            issue::Issues,
            local_project_status::ProjectStatuses,
            pipeline::{PipelineRuns, PipelineStageRuns},
        };

        let issues = Issues::find_by_project(&self.db.pool, project_id).await?;
        let issues_map: serde_json::Map<String, serde_json::Value> = issues
            .issues
            .into_iter()
            .map(|issue| (issue.id.to_string(), serde_json::to_value(issue).unwrap()))
            .collect();

        let statuses = ProjectStatuses::find_by_project(&self.db.pool, project_id).await?;
        let statuses_map: serde_json::Map<String, serde_json::Value> = statuses
            .into_iter()
            .map(|status| (status.id.to_string(), serde_json::to_value(status).unwrap()))
            .collect();

        let runs = PipelineRuns::list_by_project(&self.db.pool, project_id).await?;
        let runs_map: serde_json::Map<String, serde_json::Value> = runs
            .into_iter()
            .map(|run| (run.id.to_string(), serde_json::to_value(run).unwrap()))
            .collect();

        let stage_runs = PipelineStageRuns::list_by_project(&self.db.pool, project_id).await?;
        let stage_runs_map: serde_json::Map<String, serde_json::Value> = stage_runs
            .into_iter()
            .map(|stage| (stage.id.to_string(), serde_json::to_value(stage).unwrap()))
            .collect();

        let initial_patch = json!([
            { "op": "replace", "path": "/issues", "value": issues_map },
            { "op": "replace", "path": "/project_statuses", "value": statuses_map },
            { "op": "replace", "path": "/pipeline_runs", "value": runs_map },
            { "op": "replace", "path": "/pipeline_stage_runs", "value": stage_runs_map }
        ]);
        let initial_msg = LogMsg::JsonPatch(serde_json::from_value(initial_patch).unwrap());

        let project_id_string = project_id.to_string();
        let filtered_stream =
            BroadcastStream::new(self.msg_store.get_receiver()).filter_map(move |msg_result| {
                let project_id_string = project_id_string.clone();
                async move {
                    let Ok(LogMsg::JsonPatch(patch)) = msg_result else {
                        return None;
                    };
                    let op = patch.0.first()?;
                    let path = op.path().to_string();
                    if !is_issue_stream_path(&path) {
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
            });

        let initial_stream = futures::stream::iter(vec![Ok(initial_msg), Ok(LogMsg::Ready)]);
        Ok(initial_stream.chain(filtered_stream).boxed())
    }
```

- [ ] **Step 7: 运行，确认通过**

Run: `cargo test -p services events`
Expected: 原有测试 + 新增 3 个全部通过，`test result: ok.`

- [ ] **Step 8: 提交**

```bash
git add crates/services/src/services/events.rs crates/services/src/services/events/
git commit -m "$(cat <<'EOF'
推送：流水线运行与阶段接入变更钩子和需求流首帧

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 需求状态双写短路

**Files:**
- Modify: `crates/services/src/services/issue_flow.rs:30-51`，tests 模块追加 1 个测试
- Modify: `crates/server/src/routes/workspaces/create.rs:257-283`（抽出 `link_local_issue`），tests 模块追加 2 个测试

- [ ] **Step 1: 写失败测试（issue_flow）**

在 `crates/services/src/services/issue_flow.rs` 的 `mod tests` 末尾追加：

```rust
    #[tokio::test]
    async fn 需求有未结束的流水线时不自动流转() {
        use db::models::pipeline::{CreatePipelineRun, PipelineRuns, PipelineStageKey};

        let test_db = TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备(&test_db).await;
        PipelineRuns::create(
            test_db.pool(),
            &CreatePipelineRun {
                issue_id,
                project_id,
                workspace_id: Some(workspace_id),
                template_key: "standard".to_string(),
                template_version: 1,
                template_json: "{}".to_string(),
                template_warning: None,
                executor_config_json: "{}".to_string(),
                first_stage: PipelineStageKey::Requirement,
            },
        )
        .await
        .unwrap();

        let moved = advance_issue_for_workspace(test_db.pool(), workspace_id, IssueFlowStage::Review)
            .await
            .unwrap();
        assert!(!moved, "流水线在跑时 issue_flow 必须让路");

        let todo = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue.status_id, todo.id, "需求列不应被改动");
    }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p services issue_flow`
Expected: `需求有未结束的流水线时不自动流转` FAIL，`流水线在跑时 issue_flow 必须让路`。

- [ ] **Step 3: 实现（issue_flow）**

`crates/services/src/services/issue_flow.rs` 顶部 `use` 改为：

```rust
use db::models::{
    issue::Issues, local_project_status::StageType, pipeline::PipelineRuns, workspace::Workspace,
};
```

在 `advance_issue_for_workspace` 里 `let Some(issue_id) = workspace.issue_id else { return Ok(false); };` 之后插入：

```rust
    // 需求有未结束的流水线时，阶段只由流水线引擎写（设计 §6.4），这里让路，避免双写。
    // 覆盖全部调用方：建 PR（routes/workspaces/pr.rs）、合并（git.rs）、PR 监控（pr_monitor.rs）。
    if PipelineRuns::has_active_for_issue(pool, issue_id).await? {
        tracing::debug!("需求 {} 有未结束的流水线，跳过自动流转", issue_id);
        return Ok(false);
    }
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p services issue_flow`
Expected: `test result: ok.`（原有测试 + 新增 1 个）

- [ ] **Step 5: 写失败测试（create.rs）**

在 `crates/server/src/routes/workspaces/create.rs` 的 `mod tests` 末尾追加：

```rust
    async fn 准备本地需求与工作区(
        test_db: &db::test_support::TestDb,
    ) -> (Uuid, Uuid, Uuid) {
        use api_types::{issue::CreateIssueRequest, project::CreateProjectRequest};
        use db::models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
            workspace::{CreateWorkspace, Workspace},
        };

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
            DEFAULT_USER_ID,
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
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        (project.id, issue.id, workspace.id)
    }

    #[tokio::test]
    async fn link_local_issue_无流水线时绑定并推进到开发中() {
        use db::models::{
            issue::Issues,
            local_project_status::{ProjectStatuses, StageType},
            workspace::Workspace,
        };

        let test_db = db::test_support::TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备本地需求与工作区(&test_db).await;

        super::link_local_issue(test_db.pool(), workspace_id, issue_id)
            .await
            .unwrap();

        let dev = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Dev)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();
        assert_eq!(issue.status_id, dev.id);
        let workspace = Workspace::find_by_id(test_db.pool(), workspace_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(workspace.issue_id, Some(issue_id));
    }

    #[tokio::test]
    async fn link_local_issue_有未结束流水线时只绑定不流转() {
        use db::models::{
            issue::Issues,
            local_project_status::{ProjectStatuses, StageType},
            pipeline::{CreatePipelineRun, PipelineRuns, PipelineStageKey},
            workspace::Workspace,
        };

        let test_db = db::test_support::TestDb::new().await;
        let (project_id, issue_id, workspace_id) = 准备本地需求与工作区(&test_db).await;
        PipelineRuns::create(
            test_db.pool(),
            &CreatePipelineRun {
                issue_id,
                project_id,
                workspace_id: None,
                template_key: "standard".to_string(),
                template_version: 1,
                template_json: "{}".to_string(),
                template_warning: None,
                executor_config_json: "{}".to_string(),
                first_stage: PipelineStageKey::Requirement,
            },
        )
        .await
        .unwrap();

        super::link_local_issue(test_db.pool(), workspace_id, issue_id)
            .await
            .unwrap();

        let todo = ProjectStatuses::find_stage(test_db.pool(), project_id, StageType::Todo)
            .await
            .unwrap()
            .unwrap();
        let issue = Issues::find_by_id(test_db.pool(), issue_id).await.unwrap().unwrap();
        assert_eq!(issue.status_id, todo.id, "流水线在跑时不得改需求列");
        let workspace = Workspace::find_by_id(test_db.pool(), workspace_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(workspace.issue_id, Some(issue_id), "绑定照常");
    }
```

- [ ] **Step 6: 运行，确认失败**

Run: `cargo test -p server routes::workspaces::create::tests::link_local_issue`
Expected: 编译失败，`cannot find function \`link_local_issue\` in module \`super\``。

- [ ] **Step 7: 实现（create.rs）**

在 `crates/server/src/routes/workspaces/create.rs` 的 `create_and_start_workspace` 之前加：

```rust
/// 把本地需求绑定到工作区；需求没有未结束的流水线时顺带推进到「开发中」。
/// 流水线在跑时只绑定不流转：阶段由流水线引擎独占写（设计 §6.4）。
pub(crate) async fn link_local_issue(
    pool: &sqlx::SqlitePool,
    workspace_id: Uuid,
    issue_id: Uuid,
) -> Result<(), ApiError> {
    Workspace::set_issue_id(pool, workspace_id, Some(issue_id))
        .await
        .map_err(ApiError::Database)?;

    match db::models::pipeline::PipelineRuns::has_active_for_issue(pool, issue_id).await {
        Ok(true) => {
            tracing::debug!("需求 {} 有未结束的流水线，建工作区时不改需求列", issue_id);
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!("查询需求 {} 的流水线失败，跳过自动流转: {}", issue_id, e);
            return Ok(());
        }
    }

    if let Err(e) = db::models::issue::Issues::move_to_stage(
        pool,
        issue_id,
        db::models::local_project_status::StageType::Dev,
    )
    .await
    {
        tracing::warn!("需求 {} 流转到开发中失败: {}", issue_id, e);
    }
    Ok(())
}
```

把 `create_and_start_workspace` 里 `if let Some(issue) = &local_issue { ... }`（:267-283，含 `set_issue_id` 与 `move_to_stage`）整段替换为：

```rust
    if let Some(issue) = &local_issue {
        link_local_issue(&deployment.db().pool, managed_workspace.workspace.id, issue.id).await?;
    }
```

- [ ] **Step 8: 运行，确认通过**

Run: `cargo test -p server routes::workspaces::create`
Expected: `test result: ok.`（原有 7 个 + 新增 2 个）

- [ ] **Step 9: 提交**

```bash
git add crates/services/src/services/issue_flow.rs crates/server/src/routes/workspaces/create.rs
git commit -m "$(cat <<'EOF'
需求流转：需求有未结束的流水线时跳过 issue_flow 与建工作区的自动流转

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: 模板 `template.rs`

**Files:**
- Modify: `crates/services/Cargo.toml`（加 `serde_yaml`）
- Create: `crates/services/src/services/pipeline/mod.rs`
- Create: `crates/services/src/services/pipeline/template.rs`
- Modify: `crates/services/src/services/mod.rs`（`pub mod oauth_handoff;` 之后加 `pub mod pipeline;`）

- [ ] **Step 1: 加依赖**

Run: `cargo add serde_yaml@0.9 -p services`
Expected: `Adding serde_yaml v0.9.x to dependencies`。

**已核实**：写计划时 `cargo search serde_yaml` 可联网返回 `serde_yaml = "0.9.34+deprecated"`（作者停止维护，API 稳定，本仓库只用 `from_str` 解析一个小文件，风险可接受）；另有 YAML 组织维护的分叉 `yaml_serde = "0.10.7"`，API 与 serde_yaml 相同。选择 `serde_yaml` 是为了与生态里的大多数示例一致；若评审要求换成维护中的分叉，只需把 `cargo add` 与 `serde_yaml::from_str` 改成 `yaml_serde`，其余代码不变。若执行时断网，`cargo add` 会失败：改用 `.vibe/pipeline.json` + `serde_json::from_str`（`REPO_TEMPLATE_PATH` 与测试里的 YAML 文本同步换成等价 JSON），并在任务 20 的遗留里记一笔。

- [ ] **Step 2: 建模块骨架**

新建 `crates/services/src/services/pipeline/mod.rs`：

```rust
//! 交付流水线引擎（设计 docs/superpowers/specs/2026-09-18-personal-pipeline-design.md §6）。
//! 对外只暴露 [`PipelineService`]（任务 15 起）；其余子模块是它的纯函数零件。

pub mod template;
```

在 `crates/services/src/services/mod.rs` 的 `pub mod oauth_handoff;` 之后加：

```rust
pub mod pipeline;
```

- [ ] **Step 3: 写失败测试**

新建 `crates/services/src/services/pipeline/template.rs`，先只放测试：

```rust
#[cfg(test)]
mod tests {
    use db::models::pipeline::{GateKind, PipelineStageKey};

    use super::*;

    /// 设计文档 §6.1 的 YAML 原文（checks 按契约修正 C3 改成 shell 命令）。
    const DESIGN_YAML: &str = r#"
version: 1
stages:
  - key: requirement
    skill: vk-requirement
    artifacts: [requirement.md]
    gate: { human: 需求确认 }
  - key: spec
    skill: vk-spec
    artifacts: [spec.md, plan.md]
    gate: { human: 设计规格确认 }
  - key: test_design
    skill: prd2testcase
    artifacts: [test-cases.csv, trace-matrix.md]
    gate: { human: 用例设计确认 }
  - key: develop
    skill: vk-develop
    checks: ["pnpm run lint", "cargo test"]
    gate: { auto: checks_passed }
    retry: { max_rounds: 3 }
  - key: review
    skill: vk-review
    artifacts: [review.json]
    gate: { auto: no_blocking_findings }
    retry: { max_rounds: 3 }
  - key: test
    skill: atp-run
    artifacts: [test-report.json]
    gate: { auto: all_cases_passed }
    retry: { max_rounds: 3 }
  - key: deliver
    skill: vk-deliver
    artifacts: [delivery-report.md]
    gate: { auto: artifacts_present }
"#;

    #[test]
    fn 内置模板是标准七阶段且三道人工关卡() {
        let template = builtin_template();
        assert_eq!(template.key, BUILTIN_TEMPLATE_KEY);
        assert_eq!(template.version, 1);
        assert_eq!(
            template.stages.iter().map(|s| s.key).collect::<Vec<_>>(),
            PipelineStageKey::ALL.to_vec()
        );
        let humans: Vec<_> = template
            .stages
            .iter()
            .filter_map(|s| s.gate.human_label())
            .collect();
        assert_eq!(humans, vec!["需求确认", "设计规格确认", "用例设计确认"]);
        assert!(template.stages.iter().all(|s| s.max_rounds == DEFAULT_MAX_ROUNDS));
        assert!(
            template.stage(PipelineStageKey::Develop).unwrap().checks.is_empty(),
            "内置模板不带检查命令（repos 表没有 lint/test 脚本）"
        );
    }

    #[test]
    fn 设计文档里的_yaml_能解析且与内置模板只差_key_和_checks() {
        let parsed = parse_template_yaml(DESIGN_YAML).expect("设计文档 YAML 应能解析");
        assert_eq!(parsed.key, REPO_TEMPLATE_KEY);
        let mut builtin = builtin_template();
        builtin.key = REPO_TEMPLATE_KEY.to_string();
        builtin
            .stages
            .iter_mut()
            .find(|s| s.key == PipelineStageKey::Develop)
            .unwrap()
            .checks = vec!["pnpm run lint".to_string(), "cargo test".to_string()];
        assert_eq!(parsed, builtin);
    }

    #[test]
    fn 下一阶段与首阶段() {
        let template = builtin_template();
        assert_eq!(template.first_stage().key, PipelineStageKey::Requirement);
        assert_eq!(
            template.next_stage(PipelineStageKey::Test).map(|s| s.key),
            Some(PipelineStageKey::Deliver)
        );
        assert!(template.next_stage(PipelineStageKey::Deliver).is_none());
    }

    #[test]
    fn 视图只暴露契约字段() {
        let view = builtin_template().to_view();
        assert_eq!(view.stages.len(), 7);
        assert_eq!(view.stages[0].gate_kind, GateKind::Human);
        assert_eq!(view.stages[0].gate_label.as_deref(), Some("需求确认"));
        assert_eq!(view.stages[0].gate_condition, None, "人工关卡没有判定名");
        assert_eq!(view.stages[3].gate_kind, GateKind::Auto);
        assert_eq!(view.stages[3].gate_label, None);
        assert_eq!(view.stages[3].gate_condition.as_deref(), Some("checks_passed"));
        for stage in &view.stages {
            assert_eq!(
                stage.gate_condition.is_some(),
                stage.gate_kind == GateKind::Auto,
                "只有自动关卡有 gate_condition：{stage:?}"
            );
        }
    }

    #[test]
    fn 省略阶段也能解析且默认轮次为三() {
        let parsed = parse_template_yaml(
            "version: 1\nstages:\n  - key: develop\n    skill: vk-develop\n    gate: { auto: checks_passed }\n  - key: deliver\n    skill: vk-deliver\n    artifacts: [delivery-report.md]\n",
        )
        .unwrap();
        assert_eq!(parsed.stages.len(), 2);
        assert_eq!(parsed.stages[0].max_rounds, DEFAULT_MAX_ROUNDS);
        assert_eq!(parsed.stages[1].gate, StageGate::None);
    }

    fn 断言错误(yaml: &str, expected: impl Fn(&TemplateError) -> bool) {
        let error = parse_template_yaml(yaml).expect_err(yaml);
        assert!(expected(&error), "YAML:\n{yaml}\n实际错误：{error:?}");
    }

    #[test]
    fn 非法模板给出明确错误() {
        断言错误("version: 2\nstages: []\n", |e| {
            matches!(e, TemplateError::UnsupportedVersion(2))
        });
        断言错误("version: 1\nstages: []\n", |e| matches!(e, TemplateError::Empty));
        断言错误("version: 1\nstages:\n  - key: design\n    skill: x\n", |e| {
            matches!(e, TemplateError::UnknownStage(k) if k == "design")
        });
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n  - key: requirement\n    skill: y\n",
            |e| matches!(e, TemplateError::BadOrder(k) if k == "requirement"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n  - key: spec\n    skill: y\n",
            |e| matches!(e, TemplateError::BadOrder(k) if k == "spec"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    artifacts: [notes.md]\n",
            |e| matches!(e, TemplateError::UnknownArtifact { file, .. } if file == "notes.md"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { human: 确认, auto: checks_passed }\n",
            |e| matches!(e, TemplateError::AmbiguousGate(_)),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { auto: tests_green }\n",
            |e| matches!(e, TemplateError::UnknownAutoGate { rule, .. } if rule == "tests_green"),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    gate: { human: \"  \" }\n",
            |e| matches!(e, TemplateError::EmptyHumanLabel(_)),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    retry: { max_rounds: 0 }\n",
            |e| matches!(e, TemplateError::BadMaxRounds { value: 0, .. }),
        );
        断言错误(
            "version: 1\nstages:\n  - key: spec\n    skill: x\n    timeout: 3\n",
            |e| matches!(e, TemplateError::Yaml(_)),
        );
    }

    #[test]
    fn 仓库没有模板文件时用内置模板且无警告() {
        let dir = tempfile::tempdir().unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template, builtin_template());
        assert!(warning.is_none());
    }

    #[test]
    fn 仓库模板解析失败时回落内置模板并给警告() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
        std::fs::write(dir.path().join(REPO_TEMPLATE_PATH), "version: 9\nstages: []\n").unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template, builtin_template());
        let warning = warning.expect("应给出警告");
        assert!(warning.contains(REPO_TEMPLATE_PATH), "{warning}");
    }

    #[test]
    fn 仓库模板合法时以仓库为准() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
        std::fs::write(dir.path().join(REPO_TEMPLATE_PATH), DESIGN_YAML).unwrap();
        let (template, warning) = load_template(dir.path());
        assert_eq!(template.key, REPO_TEMPLATE_KEY);
        assert!(warning.is_none());
    }

    #[test]
    fn 模板可以序列化成_json_再读回() {
        let template = builtin_template();
        let json = serde_json::to_string(&template).unwrap();
        let back: PipelineTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, template);
    }
}
```

在 `crates/services/src/services/pipeline/mod.rs` 保持 `pub mod template;`。

- [ ] **Step 4: 运行，确认失败**

Run: `cargo test -p services pipeline::template`
Expected: 编译失败，`cannot find function \`builtin_template\``等。

- [ ] **Step 5: 实现**

在 `crates/services/src/services/pipeline/template.rs` 的测试模块**之前**写入：

```rust
//! 流水线模板：内置「标准七阶段」+ 仓库 `.vibe/pipeline.yaml`（设计 §6.1）。
//!
//! 运行启动时把解析好的模板序列化进 `pipeline_runs.template_json`，运行中途改 YAML
//! 不影响在跑的运行。

use std::path::Path;

use db::models::pipeline::{
    ArtifactKind, GateKind, PipelineStageKey, PipelineTemplateStageView, PipelineTemplateView,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BUILTIN_TEMPLATE_KEY: &str = "standard";
pub const REPO_TEMPLATE_KEY: &str = "repo";
/// 相对仓库根目录。
pub const REPO_TEMPLATE_PATH: &str = ".vibe/pipeline.yaml";
pub const DEFAULT_MAX_ROUNDS: i64 = 3;
pub const MAX_ROUNDS_LIMIT: i64 = 10;

/// 自动判定只有四种，不做表达式引擎（设计 §6.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoGate {
    ChecksPassed,
    NoBlockingFindings,
    AllCasesPassed,
    ArtifactsPresent,
}

impl AutoGate {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "checks_passed" => Some(AutoGate::ChecksPassed),
            "no_blocking_findings" => Some(AutoGate::NoBlockingFindings),
            "all_cases_passed" => Some(AutoGate::AllCasesPassed),
            "artifacts_present" => Some(AutoGate::ArtifactsPresent),
            _ => None,
        }
    }

    /// 判定名，与 YAML 里的写法一致；也是契约 `PipelineTemplateStageView.gate_condition` 的取值（C10）。
    pub fn as_str(self) -> &'static str {
        match self {
            AutoGate::ChecksPassed => "checks_passed",
            AutoGate::NoBlockingFindings => "no_blocking_findings",
            AutoGate::AllCasesPassed => "all_cases_passed",
            AutoGate::ArtifactsPresent => "artifacts_present",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageGate {
    Human { label: String },
    Auto { rule: AutoGate },
    None,
}

impl StageGate {
    pub fn kind(&self) -> GateKind {
        match self {
            StageGate::Human { .. } => GateKind::Human,
            StageGate::Auto { .. } => GateKind::Auto,
            StageGate::None => GateKind::None,
        }
    }

    pub fn human_label(&self) -> Option<&str> {
        match self {
            StageGate::Human { label } => Some(label.as_str()),
            _ => None,
        }
    }

    pub fn auto_rule(&self) -> Option<AutoGate> {
        match self {
            StageGate::Auto { rule } => Some(*rule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTemplate {
    pub key: PipelineStageKey,
    /// U3 前技能不存在，引擎只把名字写进提示词。
    pub skill: String,
    /// 本阶段必须产出的文件名（契约 §4）。
    pub artifacts: Vec<String>,
    /// 检查命令（契约修正 C3）；非空时智能体成功后在同会话里依次执行，任一失败即不通过。
    pub checks: Vec<String>,
    pub gate: StageGate,
    pub max_rounds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineTemplate {
    pub key: String,
    pub version: i64,
    /// 解析时保证非空、按标准顺序、不重复。
    pub stages: Vec<StageTemplate>,
}

impl PipelineTemplate {
    pub fn stage(&self, key: PipelineStageKey) -> Option<&StageTemplate> {
        self.stages.iter().find(|stage| stage.key == key)
    }

    pub fn first_stage(&self) -> &StageTemplate {
        &self.stages[0]
    }

    pub fn next_stage(&self, key: PipelineStageKey) -> Option<&StageTemplate> {
        let index = self.stages.iter().position(|stage| stage.key == key)?;
        self.stages.get(index + 1)
    }

    pub fn to_view(&self) -> PipelineTemplateView {
        PipelineTemplateView {
            key: self.key.clone(),
            version: self.version,
            stages: self
                .stages
                .iter()
                .map(|stage| PipelineTemplateStageView {
                    key: stage.key,
                    skill: stage.skill.clone(),
                    gate_kind: stage.gate.kind(),
                    gate_label: stage.gate.human_label().map(str::to_string),
                    // C10：自动关卡填判定名，人工与无关卡为 None。
                    gate_condition: stage.gate.auto_rule().map(|rule| rule.as_str().to_string()),
                    max_rounds: stage.max_rounds,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("YAML 解析失败：{0}")]
    Yaml(String),
    #[error("不支持的模板版本 {0}，目前只支持 1")]
    UnsupportedVersion(i64),
    #[error("模板至少要有一个阶段")]
    Empty,
    #[error("未知阶段：{0}")]
    UnknownStage(String),
    #[error(
        "阶段 {0} 重复或顺序不对（必须按 requirement → spec → test_design → develop → review → test → deliver 排列）"
    )]
    BadOrder(String),
    #[error("阶段 {stage} 的产出物 {file} 不在约定清单里")]
    UnknownArtifact { stage: String, file: String },
    #[error("阶段 {0} 的关卡只能写 human 或 auto 其中一个")]
    AmbiguousGate(String),
    #[error("阶段 {stage} 的自动关卡 {rule} 不存在")]
    UnknownAutoGate { stage: String, rule: String },
    #[error("阶段 {0} 的人工关卡名称不能为空")]
    EmptyHumanLabel(String),
    #[error("阶段 {stage} 的 max_rounds 必须在 1 到 10 之间，实际 {value}")]
    BadMaxRounds { stage: String, value: i64 },
}

pub fn builtin_template() -> PipelineTemplate {
    fn stage(
        key: PipelineStageKey,
        skill: &str,
        artifacts: &[&str],
        gate: StageGate,
    ) -> StageTemplate {
        StageTemplate {
            key,
            skill: skill.to_string(),
            artifacts: artifacts.iter().map(|name| name.to_string()).collect(),
            checks: Vec::new(),
            gate,
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }
    fn human(label: &str) -> StageGate {
        StageGate::Human {
            label: label.to_string(),
        }
    }
    fn auto(rule: AutoGate) -> StageGate {
        StageGate::Auto { rule }
    }

    PipelineTemplate {
        key: BUILTIN_TEMPLATE_KEY.to_string(),
        version: 1,
        stages: vec![
            stage(
                PipelineStageKey::Requirement,
                "vk-requirement",
                &["requirement.md"],
                human("需求确认"),
            ),
            stage(
                PipelineStageKey::Spec,
                "vk-spec",
                &["spec.md", "plan.md"],
                human("设计规格确认"),
            ),
            stage(
                PipelineStageKey::TestDesign,
                "prd2testcase",
                &["test-cases.csv", "trace-matrix.md"],
                human("用例设计确认"),
            ),
            stage(
                PipelineStageKey::Develop,
                "vk-develop",
                &[],
                auto(AutoGate::ChecksPassed),
            ),
            stage(
                PipelineStageKey::Review,
                "vk-review",
                &["review.json"],
                auto(AutoGate::NoBlockingFindings),
            ),
            stage(
                PipelineStageKey::Test,
                "atp-run",
                &["test-report.json"],
                auto(AutoGate::AllCasesPassed),
            ),
            stage(
                PipelineStageKey::Deliver,
                "vk-deliver",
                &["delivery-report.md"],
                auto(AutoGate::ArtifactsPresent),
            ),
        ],
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    version: i64,
    stages: Vec<RawStage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStage {
    key: String,
    skill: String,
    #[serde(default)]
    artifacts: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    gate: Option<RawGate>,
    #[serde(default)]
    retry: Option<RawRetry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate {
    #[serde(default)]
    human: Option<String>,
    #[serde(default)]
    auto: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRetry {
    max_rounds: i64,
}

pub fn parse_template_yaml(text: &str) -> Result<PipelineTemplate, TemplateError> {
    let raw: RawTemplate =
        serde_yaml::from_str(text).map_err(|e| TemplateError::Yaml(e.to_string()))?;
    if raw.version != 1 {
        return Err(TemplateError::UnsupportedVersion(raw.version));
    }
    if raw.stages.is_empty() {
        return Err(TemplateError::Empty);
    }

    let mut stages = Vec::with_capacity(raw.stages.len());
    let mut last_order: Option<usize> = None;
    for raw_stage in raw.stages {
        let name = raw_stage.key.clone();
        let key = PipelineStageKey::parse(&name)
            .ok_or_else(|| TemplateError::UnknownStage(name.clone()))?;
        if last_order.is_some_and(|last| key.order() <= last) {
            return Err(TemplateError::BadOrder(name));
        }
        last_order = Some(key.order());

        for file in &raw_stage.artifacts {
            if ArtifactKind::from_file_name(file).is_none() {
                return Err(TemplateError::UnknownArtifact {
                    stage: name.clone(),
                    file: file.clone(),
                });
            }
        }

        let gate = match raw_stage.gate {
            None
            | Some(RawGate {
                human: None,
                auto: None,
            }) => StageGate::None,
            Some(RawGate {
                human: Some(_),
                auto: Some(_),
            }) => return Err(TemplateError::AmbiguousGate(name)),
            Some(RawGate {
                human: Some(label),
                auto: None,
            }) => {
                let label = label.trim().to_string();
                if label.is_empty() {
                    return Err(TemplateError::EmptyHumanLabel(name));
                }
                StageGate::Human { label }
            }
            Some(RawGate {
                human: None,
                auto: Some(rule),
            }) => StageGate::Auto {
                rule: AutoGate::parse(&rule).ok_or_else(|| TemplateError::UnknownAutoGate {
                    stage: name.clone(),
                    rule: rule.clone(),
                })?,
            },
        };

        let max_rounds = raw_stage
            .retry
            .map(|retry| retry.max_rounds)
            .unwrap_or(DEFAULT_MAX_ROUNDS);
        if !(1..=MAX_ROUNDS_LIMIT).contains(&max_rounds) {
            return Err(TemplateError::BadMaxRounds {
                stage: name,
                value: max_rounds,
            });
        }

        stages.push(StageTemplate {
            key,
            skill: raw_stage.skill,
            artifacts: raw_stage.artifacts,
            checks: raw_stage.checks,
            gate,
            max_rounds,
        });
    }

    Ok(PipelineTemplate {
        key: REPO_TEMPLATE_KEY.to_string(),
        version: raw.version,
        stages,
    })
}

/// 读仓库模板：文件不存在用内置模板；读取或解析失败回落内置模板并返回警告（设计 §6.1）。
pub fn load_template(repo_root: &Path) -> (PipelineTemplate, Option<String>) {
    let path = repo_root.join(REPO_TEMPLATE_PATH);
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse_template_yaml(&text) {
            Ok(template) => (template, None),
            Err(e) => (
                builtin_template(),
                Some(format!("{REPO_TEMPLATE_PATH} 解析失败，已改用内置模板：{e}")),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (builtin_template(), None),
        Err(e) => (
            builtin_template(),
            Some(format!("读取 {REPO_TEMPLATE_PATH} 失败，已改用内置模板：{e}")),
        ),
    }
}
```

- [ ] **Step 6: 运行，确认通过**

Run: `cargo test -p services pipeline::template`
Expected: `test result: ok. 10 passed`

- [ ] **Step 7: 提交**

```bash
git add crates/services/Cargo.toml Cargo.lock crates/services/src/services/mod.rs crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：内置标准七阶段模板与仓库 .vibe/pipeline.yaml 解析

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: 关卡判定 `gates.rs`

**Files:**
- Create: `crates/services/src/services/pipeline/gates.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`（加 `pub mod gates;`）

- [ ] **Step 1: 写失败测试**

新建 `crates/services/src/services/pipeline/gates.rs`，先只放测试；`mod.rs` 加 `pub mod gates;`：

```rust
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use db::models::pipeline::PipelineStageKey;

    use super::*;
    use crate::services::pipeline::template::{PipelineTemplate, builtin_template};

    fn 模板() -> PipelineTemplate {
        builtin_template()
    }

    fn 产出(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn 进程失败直接不通过且带原因() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Requirement).unwrap();
        let verdict = evaluate_stage(stage, false, Some("编码智能体 进程未成功结束"), &产出(&[]));
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "编码智能体 进程未成功结束".to_string()
            }
        );
    }

    #[test]
    fn 缺产出物按失败处理并列出缺哪些() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Spec).unwrap();
        let verdict = evaluate_stage(stage, true, None, &产出(&[("spec.md", "# 规格")]));
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "技能没有产出 plan.md".to_string()
            }
        );
        let blank = evaluate_stage(
            stage,
            true,
            None,
            &产出(&[("spec.md", "# 规格"), ("plan.md", "  \n")]),
        );
        assert!(matches!(blank, StageVerdict::Fail { .. }), "空白文件等于没产出");
    }

    #[test]
    fn 人工关卡产出齐全后等人确认() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Requirement).unwrap();
        assert_eq!(
            evaluate_stage(stage, true, None, &产出(&[("requirement.md", "# 需求")])),
            StageVerdict::WaitHuman
        );
    }

    #[test]
    fn 开发阶段进程成功即通过() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Develop).unwrap();
        assert_eq!(evaluate_stage(stage, true, None, &产出(&[])), StageVerdict::Pass);
    }

    #[test]
    fn 交付阶段产出齐全即通过() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Deliver).unwrap();
        assert_eq!(
            evaluate_stage(stage, true, None, &产出(&[("delivery-report.md", "# 交付")])),
            StageVerdict::Pass
        );
    }

    #[test]
    fn 评审没有阻断项即通过() {
        assert_eq!(
            check_no_blocking_findings(
                r#"{"findings":[{"severity":"major","file":"a.rs","line":3,"message":"命名"}]}"#
            ),
            StageVerdict::Pass
        );
        assert_eq!(check_no_blocking_findings(r#"{"findings":[]}"#), StageVerdict::Pass);
    }

    #[test]
    fn 评审有阻断项不通过并列出位置() {
        let verdict = check_no_blocking_findings(
            r#"{"findings":[{"severity":"blocker","file":"src/lib.rs","line":7,"message":"空指针"},{"severity":"minor","message":"格式"}]}"#,
        );
        assert_eq!(
            verdict,
            StageVerdict::Fail {
                reason: "评审发现 1 个阻断项：src/lib.rs:7 空指针".to_string()
            }
        );
    }

    #[test]
    fn 评审结果不是合法_json_时不通过() {
        assert!(matches!(
            check_no_blocking_findings("not json"),
            StageVerdict::Fail { reason } if reason.contains("review.json")
        ));
    }

    #[test]
    fn 测试全部通过且至少一条才算通过() {
        assert_eq!(
            check_all_cases_passed(r#"{"total":3,"passed":3,"failed":0,"cases":[]}"#),
            StageVerdict::Pass
        );
        assert!(matches!(
            check_all_cases_passed(r#"{"total":0,"passed":0,"failed":0,"cases":[]}"#),
            StageVerdict::Fail { reason } if reason.contains("total = 0")
        ));
    }

    #[test]
    fn 测试失败归因代码缺陷回到开发() {
        let verdict = check_all_cases_passed(
            r#"{"total":3,"passed":2,"failed":1,"cases":[{"id":"TC-003","status":"failed","attribution":"code","message":"金额算错"}]}"#,
        );
        assert_eq!(
            verdict,
            StageVerdict::FailBackTo {
                stage: PipelineStageKey::Develop,
                reason: "测试失败 1 条（共 3 条）：TC-003：金额算错".to_string()
            }
        );
    }

    #[test]
    fn 测试失败任一归因用例问题回到用例设计() {
        let verdict = check_all_cases_passed(
            r#"{"total":3,"passed":1,"failed":2,"cases":[{"id":"TC-1","status":"failed","attribution":"code"},{"id":"TC-2","status":"failed","attribution":"case"}]}"#,
        );
        assert!(matches!(
            verdict,
            StageVerdict::FailBackTo { stage: PipelineStageKey::TestDesign, .. }
        ));
    }

    #[test]
    fn 测试失败没有归因时默认回到开发() {
        let verdict = check_all_cases_passed(
            r#"{"total":1,"passed":0,"failed":1,"cases":[{"id":"TC-1","status":"failed","attribution":null}]}"#,
        );
        assert!(matches!(
            verdict,
            StageVerdict::FailBackTo { stage: PipelineStageKey::Develop, .. }
        ));
    }

    #[test]
    fn 评审阶段走到阻断判定() {
        let t = 模板();
        let stage = t.stage(PipelineStageKey::Review).unwrap();
        let verdict = evaluate_stage(
            stage,
            true,
            None,
            &产出(&[("review.json", r#"{"findings":[{"severity":"blocker","message":"x"}]}"#)]),
        );
        assert!(verdict.is_failure());
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p services pipeline::gates`
Expected: 编译失败，`cannot find function \`evaluate_stage\``等。

- [ ] **Step 3: 实现**

在 `gates.rs` 测试模块之前写入：

```rust
//! 关卡判定（设计 §6.1、§6.3；契约 §4）。全部是纯函数。

use std::collections::BTreeMap;

use db::models::pipeline::PipelineStageKey;
use serde::Deserialize;

use super::template::{AutoGate, StageGate, StageTemplate};

pub const REVIEW_FILE: &str = "review.json";
pub const TEST_REPORT_FILE: &str = "test-report.json";

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewReport {
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFinding {
    pub severity: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestReport {
    pub total: i64,
    pub passed: i64,
    pub failed: i64,
    #[serde(default)]
    pub cases: Vec<TestCaseResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestCaseResult {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub attribution: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageVerdict {
    /// 产出齐全，等人工确认。
    WaitHuman,
    /// 自动通过。
    Pass,
    /// 本阶段不通过：按轮次重跑本阶段。
    Fail { reason: String },
    /// 测试不通过且归因明确：回到指定阶段。
    FailBackTo {
        stage: PipelineStageKey,
        reason: String,
    },
}

impl StageVerdict {
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            StageVerdict::Fail { .. } | StageVerdict::FailBackTo { .. }
        )
    }
}

/// 声明了但没读到（不存在或内容为空白）的产出物。
pub fn missing_artifacts(required: &[String], present: &BTreeMap<String, String>) -> Vec<String> {
    required
        .iter()
        .filter(|name| {
            present
                .get(name.as_str())
                .is_none_or(|content| content.trim().is_empty())
        })
        .cloned()
        .collect()
}

/// 阶段判定：进程失败 → 缺产出物 → 按关卡类型。
///
/// `process_ok` 已经包含智能体与检查脚本（若有）的退出码，所以 `checks_passed`
/// 走到这里就是通过。
pub fn evaluate_stage(
    stage: &StageTemplate,
    process_ok: bool,
    process_error: Option<&str>,
    artifacts: &BTreeMap<String, String>,
) -> StageVerdict {
    if !process_ok {
        return StageVerdict::Fail {
            reason: process_error.unwrap_or("执行进程失败").to_string(),
        };
    }
    let missing = missing_artifacts(&stage.artifacts, artifacts);
    if !missing.is_empty() {
        return StageVerdict::Fail {
            reason: format!("技能没有产出 {}", missing.join("、")),
        };
    }
    match &stage.gate {
        StageGate::Human { .. } => StageVerdict::WaitHuman,
        StageGate::None => StageVerdict::Pass,
        StageGate::Auto { rule } => evaluate_auto(*rule, artifacts),
    }
}

pub fn evaluate_auto(rule: AutoGate, artifacts: &BTreeMap<String, String>) -> StageVerdict {
    match rule {
        AutoGate::ChecksPassed | AutoGate::ArtifactsPresent => StageVerdict::Pass,
        AutoGate::NoBlockingFindings => match artifacts.get(REVIEW_FILE) {
            Some(text) => check_no_blocking_findings(text),
            None => StageVerdict::Fail {
                reason: format!("缺少 {REVIEW_FILE}，无法判定评审结果"),
            },
        },
        AutoGate::AllCasesPassed => match artifacts.get(TEST_REPORT_FILE) {
            Some(text) => check_all_cases_passed(text),
            None => StageVerdict::Fail {
                reason: format!("缺少 {TEST_REPORT_FILE}，无法判定测试结果"),
            },
        },
    }
}

/// `no_blocking_findings` = 没有 `severity == "blocker"`（契约 §4）。
pub fn check_no_blocking_findings(text: &str) -> StageVerdict {
    let report: ReviewReport = match serde_json::from_str(text) {
        Ok(report) => report,
        Err(e) => {
            return StageVerdict::Fail {
                reason: format!("{REVIEW_FILE} 不是合法的评审结果：{e}"),
            };
        }
    };
    let blockers: Vec<&ReviewFinding> = report
        .findings
        .iter()
        .filter(|finding| finding.severity == "blocker")
        .collect();
    if blockers.is_empty() {
        return StageVerdict::Pass;
    }
    let details = blockers
        .iter()
        .map(|finding| describe_finding(finding))
        .collect::<Vec<_>>()
        .join("；");
    StageVerdict::Fail {
        reason: format!("评审发现 {} 个阻断项：{details}", blockers.len()),
    }
}

fn describe_finding(finding: &ReviewFinding) -> String {
    let location = match (&finding.file, finding.line) {
        (Some(file), Some(line)) => format!("{file}:{line} "),
        (Some(file), None) => format!("{file} "),
        _ => String::new(),
    };
    format!(
        "{location}{}",
        finding.message.as_deref().unwrap_or("（无说明）")
    )
}

/// `all_cases_passed` = `failed == 0 && total > 0`（契约 §4）。失败时按归因分派回流目标。
pub fn check_all_cases_passed(text: &str) -> StageVerdict {
    let report: TestReport = match serde_json::from_str(text) {
        Ok(report) => report,
        Err(e) => {
            return StageVerdict::Fail {
                reason: format!("{TEST_REPORT_FILE} 不是合法的测试报告：{e}"),
            };
        }
    };
    if report.total <= 0 {
        return StageVerdict::Fail {
            reason: "测试报告里没有任何用例（total = 0）".to_string(),
        };
    }
    if report.failed == 0 {
        return StageVerdict::Pass;
    }
    let details = report
        .cases
        .iter()
        .filter(|case| case.status != "passed")
        .map(|case| format!("{}：{}", case.id, case.message.as_deref().unwrap_or("失败")))
        .collect::<Vec<_>>()
        .join("；");
    StageVerdict::FailBackTo {
        stage: dispatch_test_failure(&report),
        reason: format!(
            "测试失败 {} 条（共 {} 条）：{details}",
            report.failed, report.total
        ),
    }
}

/// 测试归因分派（设计 §6.3、契约 §4）：任一 `attribution == "case"` → 回到用例设计
/// （重新走人工关卡）；否则 → 回到开发。
pub fn dispatch_test_failure(report: &TestReport) -> PipelineStageKey {
    let any_case = report
        .cases
        .iter()
        .any(|case| case.attribution.as_deref() == Some("case"));
    if any_case {
        PipelineStageKey::TestDesign
    } else {
        PipelineStageKey::Develop
    }
}
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p services pipeline::gates`
Expected: `test result: ok. 13 passed`

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：四种自动关卡判定与测试归因分派

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: 状态转移 `transition.rs`

**Files:**
- Create: `crates/services/src/services/pipeline/transition.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`（加 `pub mod transition;` 与再导出）

- [ ] **Step 1: 写失败测试**

新建 `transition.rs`，先只放测试；`mod.rs` 加 `pub mod transition;`：

```rust
#[cfg(test)]
mod tests {
    use db::models::{
        execution_process::{ExecutionProcessRunReason, ExecutionProcessStatus},
        local_project_status::StageType,
        pipeline::{GateDecisionKind, PipelineStageKey},
    };
    use uuid::Uuid;

    use super::*;
    use crate::services::pipeline::{gates::StageVerdict, template::builtin_template};

    fn 进程(
        run_reason: ExecutionProcessRunReason,
        has_next_action: bool,
        chain_continues: bool,
    ) -> FinishedProcess {
        FinishedProcess {
            execution_process_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_reason,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action,
            chain_continues,
        }
    }

    #[test]
    fn 通过后进入下一阶段_最后一阶段则完成() {
        let t = builtin_template();
        assert_eq!(
            next_transition(&t, PipelineStageKey::Develop, &StageVerdict::Pass, 0),
            Transition::Advance {
                to: PipelineStageKey::Review
            }
        );
        assert_eq!(
            next_transition(&t, PipelineStageKey::Deliver, &StageVerdict::Pass, 0),
            Transition::Complete
        );
    }

    #[test]
    fn 等人工() {
        let t = builtin_template();
        assert_eq!(
            next_transition(&t, PipelineStageKey::Spec, &StageVerdict::WaitHuman, 0),
            Transition::WaitHuman
        );
    }

    #[test]
    fn 失败未到上限时同阶段重跑并回喂原因() {
        let t = builtin_template();
        let verdict = StageVerdict::Fail {
            reason: "评审发现 1 个阻断项".to_string(),
        };
        assert_eq!(
            next_transition(&t, PipelineStageKey::Review, &verdict, 2),
            Transition::Rerun {
                stage: PipelineStageKey::Review,
                feedback: "评审发现 1 个阻断项".to_string()
            }
        );
    }

    #[test]
    fn 失败达到上限时运行失败() {
        let t = builtin_template();
        let verdict = StageVerdict::Fail {
            reason: "评审发现 1 个阻断项".to_string(),
        };
        let Transition::FailRun { reason } = next_transition(&t, PipelineStageKey::Review, &verdict, 3)
        else {
            panic!("第 3 次失败应转人工");
        };
        assert!(reason.contains("review") && reason.contains("3"), "{reason}");
    }

    #[test]
    fn 测试失败按归因回退() {
        let t = builtin_template();
        for target in [PipelineStageKey::Develop, PipelineStageKey::TestDesign] {
            let verdict = StageVerdict::FailBackTo {
                stage: target,
                reason: "测试失败".to_string(),
            };
            assert_eq!(
                next_transition(&t, PipelineStageKey::Test, &verdict, 1),
                Transition::Rerun {
                    stage: target,
                    feedback: "测试失败".to_string()
                }
            );
        }
    }

    #[test]
    fn 回退目标不在模板里时重跑本阶段() {
        let mut t = builtin_template();
        t.stages.retain(|s| s.key != PipelineStageKey::TestDesign);
        let verdict = StageVerdict::FailBackTo {
            stage: PipelineStageKey::TestDesign,
            reason: "用例问题".to_string(),
        };
        assert_eq!(
            next_transition(&t, PipelineStageKey::Test, &verdict, 1),
            Transition::Rerun {
                stage: PipelineStageKey::Test,
                feedback: "用例问题".to_string()
            }
        );
    }

    #[test]
    fn 人工通过与打回() {
        let t = builtin_template();
        assert_eq!(
            gate_decision_transition(&t, PipelineStageKey::TestDesign, GateDecisionKind::Approve, None),
            Transition::Advance {
                to: PipelineStageKey::Develop
            }
        );
        assert_eq!(
            gate_decision_transition(
                &t,
                PipelineStageKey::Requirement,
                GateDecisionKind::Reject,
                Some(" 补充验收标准 ")
            ),
            Transition::Rerun {
                stage: PipelineStageKey::Requirement,
                feedback: "补充验收标准".to_string()
            }
        );
    }

    #[test]
    fn 阶段链终点判定() {
        use ExecutionProcessRunReason::*;
        // 智能体没提交、cleanup 没跑：退出钩子报 chain_continues = false → 是终点（§2.3 的陷阱）。
        assert!(is_stage_terminal(&进程(CodingAgent, true, false)));
        // 智能体有提交、cleanup 已启动 → 不是终点，等 cleanup。
        assert!(!is_stage_terminal(&进程(CodingAgent, true, true)));
        assert!(is_stage_terminal(&进程(CleanupScript, false, false)));
        assert!(is_stage_terminal(&进程(PipelineStep, false, false)));
        // 顺序 setup 成功后接着起智能体 → 不是终点；失败没接上 → 是终点。
        assert!(!is_stage_terminal(&进程(SetupScript, true, true)));
        assert!(is_stage_terminal(&进程(SetupScript, true, false)));
        // 并行 setup（没有 next_action）与阶段结果无关。
        assert!(!is_stage_terminal(&进程(SetupScript, false, false)));
        assert!(!is_stage_terminal(&进程(DevServer, false, false)));
        assert!(!is_stage_terminal(&进程(ArchiveScript, false, false)));
    }

    #[test]
    fn 进程成功要求完成且退出码为零() {
        let mut p = 进程(ExecutionProcessRunReason::CodingAgent, false, false);
        assert!(process_succeeded(&p));
        p.exit_code = Some(1);
        assert!(!process_succeeded(&p));
        p.exit_code = Some(0);
        p.status = ExecutionProcessStatus::Killed;
        assert!(!process_succeeded(&p));
    }

    #[test]
    fn 阶段到看板列的映射() {
        assert_eq!(stage_column(PipelineStageKey::Requirement), StageType::Backlog);
        assert_eq!(stage_column(PipelineStageKey::Spec), StageType::Todo);
        assert_eq!(stage_column(PipelineStageKey::TestDesign), StageType::Todo);
        assert_eq!(stage_column(PipelineStageKey::Develop), StageType::Dev);
        assert_eq!(stage_column(PipelineStageKey::Review), StageType::Review);
        assert_eq!(stage_column(PipelineStageKey::Test), StageType::Test);
        assert_eq!(stage_column(PipelineStageKey::Deliver), StageType::Done);
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p services pipeline::transition`
Expected: 编译失败，`cannot find type \`FinishedProcess\``等。

- [ ] **Step 3: 实现**

在 `transition.rs` 测试模块之前写入：

```rust
//! 流水线状态转移（纯函数）。读写数据库的部分在 engine.rs。

use db::models::{
    execution_process::{ExecutionProcessRunReason, ExecutionProcessStatus},
    local_project_status::StageType,
    pipeline::{GateDecisionKind, PipelineStageKey},
};
use uuid::Uuid;

use super::{
    gates::StageVerdict,
    template::{DEFAULT_MAX_ROUNDS, PipelineTemplate},
};

/// 容器退出钩子发给引擎的消息（任务 17 接线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineExitEvent {
    pub execution_process_id: Uuid,
    /// 这个进程收尾时是否接着启动了同会话的下一个进程（next_action 或排队的续聊）。
    pub chain_continues: bool,
}

/// 引擎判定用的执行进程快照。
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedProcess {
    pub execution_process_id: Uuid,
    pub session_id: Uuid,
    pub run_reason: ExecutionProcessRunReason,
    pub status: ExecutionProcessStatus,
    pub exit_code: Option<i64>,
    /// 这个进程的 action 是否带 next_action。
    pub has_next_action: bool,
    pub chain_continues: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// 阶段等人工确认。
    WaitHuman,
    /// 阶段通过，进入下一阶段。
    Advance { to: PipelineStageKey },
    /// 最后一个阶段通过，运行完成。
    Complete,
    /// 本阶段不通过（或被打回），进入 `stage` 的新一次尝试，`feedback` 带进提示词。
    Rerun {
        stage: PipelineStageKey,
        feedback: String,
    },
    /// 轮次用尽，运行失败转人工。
    FailRun { reason: String },
}

/// 自动判定后的转移。`failed_attempts` = 本阶段累计失败次数（**含本次**）。
pub fn next_transition(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    verdict: &StageVerdict,
    failed_attempts: i64,
) -> Transition {
    match verdict {
        StageVerdict::WaitHuman => Transition::WaitHuman,
        StageVerdict::Pass => advance_or_complete(template, current),
        StageVerdict::Fail { reason } => {
            rerun_or_fail(template, current, current, reason, failed_attempts)
        }
        StageVerdict::FailBackTo { stage, reason } => {
            let target = if template.stage(*stage).is_some() {
                *stage
            } else {
                current
            };
            rerun_or_fail(template, current, target, reason, failed_attempts)
        }
    }
}

/// 人工关卡决策后的转移。打回不计入失败次数，不受 max_rounds 限制。
pub fn gate_decision_transition(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    decision: GateDecisionKind,
    comment: Option<&str>,
) -> Transition {
    match decision {
        GateDecisionKind::Approve => advance_or_complete(template, current),
        GateDecisionKind::Reject => Transition::Rerun {
            stage: current,
            feedback: comment.unwrap_or_default().trim().to_string(),
        },
    }
}

fn advance_or_complete(template: &PipelineTemplate, current: PipelineStageKey) -> Transition {
    match template.next_stage(current) {
        Some(next) => Transition::Advance { to: next.key },
        None => Transition::Complete,
    }
}

fn rerun_or_fail(
    template: &PipelineTemplate,
    current: PipelineStageKey,
    target: PipelineStageKey,
    reason: &str,
    failed_attempts: i64,
) -> Transition {
    let max_rounds = template
        .stage(current)
        .map(|stage| stage.max_rounds)
        .unwrap_or(DEFAULT_MAX_ROUNDS);
    if failed_attempts >= max_rounds {
        Transition::FailRun {
            reason: format!(
                "阶段 {} 已累计失败 {} 次（上限 {}），转人工处理。最后一次原因：{}",
                current.as_str(),
                failed_attempts,
                max_rounds,
                reason
            ),
        }
    } else {
        Transition::Rerun {
            stage: target,
            feedback: reason.to_string(),
        }
    }
}

/// 这个进程结束是否意味着「阶段的执行链走完了」。
///
/// - 进程收尾时接着起了下一个进程（`chain_continues`）→ 不是终点。
/// - setup 脚本：带 next_action（顺序模式）却没接上 → 链断了，是终点；
///   不带 next_action（并行模式）→ 智能体另行启动，与阶段结果无关。
/// - 开发服务器、归档脚本与阶段无关。
/// - 编码智能体、cleanup 脚本、检查脚本 → 是终点。编码智能体成功但没提交时
///   容器不跑 cleanup（`local-deployment/src/container.rs:590-621`），所以这里不能等 cleanup。
pub fn is_stage_terminal(process: &FinishedProcess) -> bool {
    if process.chain_continues {
        return false;
    }
    match process.run_reason {
        ExecutionProcessRunReason::SetupScript => process.has_next_action,
        ExecutionProcessRunReason::DevServer | ExecutionProcessRunReason::ArchiveScript => false,
        ExecutionProcessRunReason::CodingAgent
        | ExecutionProcessRunReason::CleanupScript
        | ExecutionProcessRunReason::PipelineStep => true,
    }
}

pub fn process_succeeded(process: &FinishedProcess) -> bool {
    process.status == ExecutionProcessStatus::Completed && process.exit_code == Some(0)
}

/// 阶段 → 看板列（设计 §6.4）。
pub fn stage_column(key: PipelineStageKey) -> StageType {
    match key {
        PipelineStageKey::Requirement => StageType::Backlog,
        PipelineStageKey::Spec | PipelineStageKey::TestDesign => StageType::Todo,
        PipelineStageKey::Develop => StageType::Dev,
        PipelineStageKey::Review => StageType::Review,
        PipelineStageKey::Test => StageType::Test,
        PipelineStageKey::Deliver => StageType::Done,
    }
}
```

`mod.rs` 在模块声明之后追加：

```rust
pub use transition::{FinishedProcess, PipelineExitEvent};
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p services pipeline::transition`
Expected: `test result: ok. 10 passed`

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：状态转移纯函数与阶段链终点判定

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: 提示词固定格式与模拟产出物 `executors::pipeline_prompt`

**为什么放在 executors 且始终编译：** 引擎（services）拼提示词、qa-mode 模拟器（executors，只在 `qa-mode` 下编译）解析提示词、引擎集成测试（services，不开 qa-mode）模拟产出物，三方要用同一份常量与同一份模拟逻辑。services 依赖 executors，反之不行，所以放在 executors；不放进 `qa_mock.rs` 是因为那个模块在默认构建里不存在（`crates/executors/src/executors/mod.rs:42-43`），`cargo test --workspace` 会漏测。

**Files:**
- Create: `crates/executors/src/pipeline_prompt.rs`
- Modify: `crates/executors/src/lib.rs`（加 `pub mod pipeline_prompt;`）
- Modify: `crates/executors/Cargo.toml`（文件末尾加 `[dev-dependencies] tempfile = "3"`）

- [ ] **Step 1: 加测试依赖与模块声明**

在 `crates/executors/Cargo.toml` 末尾追加：

```toml
[dev-dependencies]
tempfile = "3"
```

在 `crates/executors/src/lib.rs` 的 `pub mod model_selector;` 之后加：

```rust
pub mod pipeline_prompt;
```

- [ ] **Step 2: 写失败测试**

新建 `crates/executors/src/pipeline_prompt.rs`，先只放测试：

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn 提示词(dir: &std::path::Path, required: &str, title: &str) -> String {
        format!(
            "使用技能 vk-review。\n需求：VK-7 {title}\n产出物目录（绝对路径）：{}\n必须产出：{required}\n上一阶段产出：spec.md\n这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。",
            dir.display()
        )
    }

    #[test]
    fn 解析两行固定格式() {
        let parsed = parse_pipeline_prompt(&提示词(
            std::path::Path::new("/abs/ws/.vk/runs/VK-7"),
            "requirement.md, spec.md",
            "导出报表",
        ))
        .unwrap();
        assert_eq!(parsed.artifacts_dir, PathBuf::from("/abs/ws/.vk/runs/VK-7"));
        assert_eq!(parsed.required, vec!["requirement.md", "spec.md"]);
        assert_eq!(parsed.requirement, "VK-7 导出报表");
    }

    #[test]
    fn 必须产出为无时是空清单() {
        let parsed = parse_pipeline_prompt(&提示词(
            std::path::Path::new("/abs/x"),
            NONE_VALUE,
            "开发",
        ))
        .unwrap();
        assert!(parsed.required.is_empty());
    }

    #[test]
    fn 普通提示词不是流水线提示词() {
        assert!(parse_pipeline_prompt("帮我修个 bug").is_none());
        assert!(
            parse_pipeline_prompt("产出物目录（绝对路径）：relative/dir\n必须产出：a.md").is_none(),
            "相对路径不接受"
        );
        assert!(
            parse_pipeline_prompt("产出物目录（绝对路径）：/abs\n").is_none(),
            "缺「必须产出」行不接受"
        );
    }

    #[test]
    fn 只认第一次出现的固定行() {
        let prompt = format!(
            "{}\n上一次被打回的意见：\n必须产出：evil.md\n",
            提示词(std::path::Path::new("/abs/x"), "review.json", "t")
        );
        assert_eq!(parse_pipeline_prompt(&prompt).unwrap().required, vec!["review.json"]);
    }

    #[test]
    fn 标记从需求行解析() {
        assert_eq!(
            qa_markers("VK-1 评审 [qa:review-blocker-once] 与 [qa:test-fail-case-once]"),
            vec![QaMarker::ReviewBlockerOnce, QaMarker::TestFailCaseOnce]
        );
        assert!(qa_markers("VK-1 普通需求").is_empty());
    }

    #[test]
    fn 写出占位产出物且第二次把旧文件改名为_prev() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts_dir = dir.path().join(".vk/runs/VK-7");
        let parsed = PipelinePrompt {
            requirement: "VK-7 导出 [qa:review-blocker-once]".to_string(),
            artifacts_dir: artifacts_dir.clone(),
            required: vec![
                "requirement.md".to_string(),
                "test-cases.csv".to_string(),
                "review.json".to_string(),
            ],
        };

        let written = write_mock_artifacts(&parsed).unwrap();
        assert_eq!(written.len(), 3);
        let md = std::fs::read_to_string(artifacts_dir.join("requirement.md")).unwrap();
        assert!(md.starts_with("# 需求说明"), "{md}");
        let csv = std::fs::read_to_string(artifacts_dir.join("test-cases.csv")).unwrap();
        assert!(csv.starts_with(MOCK_TEST_CASES_HEADER));
        assert_eq!(csv.lines().count(), 2, "表头加一行");
        let header_fields = csv.lines().next().unwrap().split(',').count();
        let row_fields = csv.lines().nth(1).unwrap().split(',').count();
        assert_eq!(header_fields, row_fields, "占位行字段数与表头一致");
        let first_review = std::fs::read_to_string(artifacts_dir.join("review.json")).unwrap();
        assert!(first_review.contains("\"blocker\""), "第一次评审带阻断项");

        write_mock_artifacts(&parsed).unwrap();
        assert!(artifacts_dir.join("review.json.prev").exists());
        let second_review = std::fs::read_to_string(artifacts_dir.join("review.json")).unwrap();
        assert_eq!(second_review, r#"{"findings":[]}"#, "第二次起为空");
    }

    #[test]
    fn 测试报告按标记与轮次变化() {
        let pass: serde_json::Value = serde_json::from_str(&mock_artifact_content(
            "test-report.json",
            true,
            &[],
            "VK-1",
        ))
        .unwrap();
        assert_eq!((pass["total"].as_i64(), pass["failed"].as_i64()), (Some(3), Some(0)));

        for (marker, attribution) in [
            (QaMarker::TestFailCodeOnce, "code"),
            (QaMarker::TestFailCaseOnce, "case"),
        ] {
            let first: serde_json::Value = serde_json::from_str(&mock_artifact_content(
                "test-report.json",
                true,
                &[marker],
                "VK-1",
            ))
            .unwrap();
            assert_eq!(first["failed"].as_i64(), Some(1));
            assert!(
                first["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["attribution"] == attribution)
            );
            let later: serde_json::Value = serde_json::from_str(&mock_artifact_content(
                "test-report.json",
                false,
                &[marker],
                "VK-1",
            ))
            .unwrap();
            assert_eq!(later["failed"].as_i64(), Some(0), "第二次起全过");
        }
    }

    #[test]
    fn 评审始终阻断标记每次都有阻断项() {
        for first in [true, false] {
            let text = mock_artifact_content(
                "review.json",
                first,
                &[QaMarker::AlwaysFailReview],
                "VK-1",
            );
            assert!(text.contains("\"blocker\""));
        }
    }

    #[test]
    fn 带路径的文件名被跳过() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = PipelinePrompt {
            requirement: "VK-1".to_string(),
            artifacts_dir: dir.path().join("out"),
            required: vec!["../escape.md".to_string(), "spec.md".to_string()],
        };
        let written = write_mock_artifacts(&parsed).unwrap();
        assert_eq!(written, vec![dir.path().join("out/spec.md")]);
        assert!(!dir.path().join("escape.md").exists());
    }
}
```

- [ ] **Step 3: 运行，确认失败**

Run: `cargo test -p executors pipeline_prompt`
Expected: 编译失败，`cannot find function \`parse_pipeline_prompt\``等。

- [ ] **Step 4: 实现**

在 `pipeline_prompt.rs` 测试模块之前写入：

```rust
//! 流水线提示词的固定格式（契约 §5）与 qa-mode 模拟产出物。
//!
//! 引擎（`services::services::pipeline::prompt`）按这里的常量拼提示词；qa-mode 模拟执行器
//! （`executors::executors::qa_mock`）与引擎集成测试按这里的函数解析提示词、写占位产出物。
//! 改格式只改这一处。

use std::path::{Path, PathBuf};

pub const SKILL_PREFIX: &str = "使用技能 ";
pub const REQUIREMENT_LABEL: &str = "需求：";
pub const ARTIFACTS_DIR_LABEL: &str = "产出物目录（绝对路径）：";
pub const REQUIRED_ARTIFACTS_LABEL: &str = "必须产出：";
pub const PREVIOUS_ARTIFACTS_LABEL: &str = "上一阶段产出：";
pub const FEEDBACK_LABEL: &str = "上一次被打回的意见：";
pub const NONE_VALUE: &str = "无";
pub const ARTIFACT_SEPARATOR: &str = ", ";

/// atp 旧 CSV 表头，原样取自 atp 仓库 `legacy/test_cases/api_custom/api_custom.csv` 第一行
/// （去掉 BOM）。列定义保持旧版不变；模拟器只用它写占位用例。
pub const MOCK_TEST_CASES_HEADER: &str = "CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinePrompt {
    /// 「需求：」行冒号后的内容：`{simple_id} {title}`。
    pub requirement: String,
    pub artifacts_dir: PathBuf,
    pub required: Vec<String>,
}

/// 解析流水线提示词。缺「产出物目录」或「必须产出」行、或目录不是绝对路径时返回 None
/// （即不是流水线提示词）。每种固定行只认第一次出现，防止打回意见里的同名行干扰。
pub fn parse_pipeline_prompt(prompt: &str) -> Option<PipelinePrompt> {
    let mut requirement: Option<String> = None;
    let mut artifacts_dir: Option<PathBuf> = None;
    let mut required: Option<Vec<String>> = None;
    for line in prompt.lines() {
        let line = line.trim();
        if requirement.is_none()
            && let Some(rest) = line.strip_prefix(REQUIREMENT_LABEL)
        {
            requirement = Some(rest.trim().to_string());
        } else if artifacts_dir.is_none()
            && let Some(rest) = line.strip_prefix(ARTIFACTS_DIR_LABEL)
        {
            artifacts_dir = Some(PathBuf::from(rest.trim()));
        } else if required.is_none()
            && let Some(rest) = line.strip_prefix(REQUIRED_ARTIFACTS_LABEL)
        {
            required = Some(parse_artifact_list(rest));
        }
    }
    let artifacts_dir = artifacts_dir?;
    if !artifacts_dir.is_absolute() {
        return None;
    }
    Some(PipelinePrompt {
        requirement: requirement.unwrap_or_default(),
        artifacts_dir,
        required: required?,
    })
}

pub fn parse_artifact_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() || value == NONE_VALUE {
        return Vec::new();
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn format_artifact_list(names: &[String]) -> String {
    if names.is_empty() {
        NONE_VALUE.to_string()
    } else {
        names.join(ARTIFACT_SEPARATOR)
    }
}

/// 端到端测试用的需求标题标记（契约 §5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaMarker {
    ReviewBlockerOnce,
    TestFailCodeOnce,
    TestFailCaseOnce,
    AlwaysFailReview,
}

impl QaMarker {
    pub const ALL: [QaMarker; 4] = [
        QaMarker::ReviewBlockerOnce,
        QaMarker::TestFailCodeOnce,
        QaMarker::TestFailCaseOnce,
        QaMarker::AlwaysFailReview,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            QaMarker::ReviewBlockerOnce => "[qa:review-blocker-once]",
            QaMarker::TestFailCodeOnce => "[qa:test-fail-code-once]",
            QaMarker::TestFailCaseOnce => "[qa:test-fail-case-once]",
            QaMarker::AlwaysFailReview => "[qa:always-fail-review]",
        }
    }
}

pub fn qa_markers(requirement: &str) -> Vec<QaMarker> {
    QaMarker::ALL
        .into_iter()
        .filter(|marker| requirement.contains(marker.tag()))
        .collect()
}

/// 在产出物目录里写出「必须产出」的每个文件。已存在的旧文件先改名为 `*.prev`
/// ——模拟器靠「有没有旧文件」判断是不是第一次（契约 §5）。
/// 只接受纯文件名，带路径分隔符或 `..` 的跳过。
pub fn write_mock_artifacts(prompt: &PipelinePrompt) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(&prompt.artifacts_dir)?;
    let markers = qa_markers(&prompt.requirement);
    let mut written = Vec::new();
    for name in &prompt.required {
        if !is_plain_file_name(name) {
            continue;
        }
        let path = prompt.artifacts_dir.join(name);
        let first = !path.exists();
        if !first {
            std::fs::rename(&path, prompt.artifacts_dir.join(format!("{name}.prev")))?;
        }
        std::fs::write(
            &path,
            mock_artifact_content(name, first, &markers, &prompt.requirement),
        )?;
        written.push(path);
    }
    Ok(written)
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).file_name().is_some()
}

/// 单个占位产出物的内容。`first` = 目录里原本没有这个文件。
pub fn mock_artifact_content(
    name: &str,
    first: bool,
    markers: &[QaMarker],
    requirement: &str,
) -> String {
    match name {
        "review.json" => {
            let blocker = markers.contains(&QaMarker::AlwaysFailReview)
                || (first && markers.contains(&QaMarker::ReviewBlockerOnce));
            if blocker {
                serde_json::json!({
                    "findings": [{
                        "severity": "blocker",
                        "file": "src/lib.rs",
                        "line": 1,
                        "message": "模拟评审阻断项"
                    }]
                })
                .to_string()
            } else {
                r#"{"findings":[]}"#.to_string()
            }
        }
        "test-report.json" => {
            let attribution = if first && markers.contains(&QaMarker::TestFailCodeOnce) {
                Some("code")
            } else if first && markers.contains(&QaMarker::TestFailCaseOnce) {
                Some("case")
            } else {
                None
            };
            let passed = |id: &str| {
                serde_json::json!({"id": id, "status": "passed", "attribution": null, "message": ""})
            };
            match attribution {
                Some(attribution) => serde_json::json!({
                    "total": 3,
                    "passed": 2,
                    "failed": 1,
                    "cases": [
                        passed("TC-001"),
                        passed("TC-002"),
                        {"id": "TC-003", "status": "failed", "attribution": attribution, "message": "模拟失败"}
                    ]
                })
                .to_string(),
                None => serde_json::json!({
                    "total": 3,
                    "passed": 3,
                    "failed": 0,
                    "cases": [passed("TC-001"), passed("TC-002"), passed("TC-003")]
                })
                .to_string(),
            }
        }
        name if name.ends_with(".csv") => mock_test_cases_csv(),
        name if name.ends_with(".json") => "{}".to_string(),
        name => format!(
            "# {}\n\n由 qa-mode 模拟执行器生成的占位内容。\n\n{REQUIREMENT_LABEL}{requirement}\n",
            markdown_title(name)
        ),
    }
}

/// 表头 + 一行占位用例，字段数与表头一致。
fn mock_test_cases_csv() -> String {
    let columns: Vec<&str> = MOCK_TEST_CASES_HEADER.split(',').collect();
    let row: Vec<&str> = columns
        .iter()
        .map(|column| match *column {
            "CaseID" => "TC-001",
            "ScenarioName" => "占位场景",
            "Priority" => "P1",
            "stepkw" => "GET",
            "StepSummary" => "由 qa-mode 模拟执行器生成",
            "ExpectedResult" => "返回成功",
            _ => "",
        })
        .collect();
    format!("{MOCK_TEST_CASES_HEADER}\n{}\n", row.join(","))
}

fn markdown_title(name: &str) -> &str {
    match name {
        "requirement.md" => "需求说明",
        "spec.md" => "设计规格",
        "plan.md" => "实施计划",
        "trace-matrix.md" => "追踪矩阵",
        "delivery-report.md" => "交付报告",
        other => other,
    }
}
```

**已核实**：`MOCK_TEST_CASES_HEADER` 与 `/Users/admin/work/github/atp/legacy/test_cases/api_custom/api_custom.csv` 第一行（去 BOM）逐字一致（写计划时 `head -1` 取得，46 列）。U4 的 prd2testcase 若改用 web 用例表头（`legacy/web_test_cases/web_test_cases.csv`），同步改这里。引擎不解析 CSV，不影响任何关卡。

- [ ] **Step 5: 运行，确认通过**

Run: `cargo test -p executors pipeline_prompt`
Expected: `test result: ok. 9 passed`

- [ ] **Step 6: 提交**

```bash
git add crates/executors/Cargo.toml Cargo.lock crates/executors/src/lib.rs crates/executors/src/pipeline_prompt.rs
git commit -m "$(cat <<'EOF'
执行器：流水线提示词固定格式解析与模拟产出物

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: 引擎提示词 `prompt.rs`

**Files:**
- Create: `crates/services/src/services/pipeline/prompt.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`（加 `pub mod prompt;`）

- [ ] **Step 1: 写失败测试**

新建 `prompt.rs`，先只放测试；`mod.rs` 加 `pub mod prompt;`：

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    use executors::pipeline_prompt::parse_pipeline_prompt;

    use super::*;

    fn 输入<'a>(
        required: &'a [String],
        existing: &'a [String],
        feedback: Option<&'a str>,
    ) -> StagePromptInput<'a> {
        StagePromptInput {
            skill: "vk-spec",
            simple_id: "VK-12",
            title: "导出报表\n[qa:review-blocker-once]",
            artifacts_dir: Path::new("/abs/ws/.vk/runs/VK-12"),
            required,
            existing,
            feedback,
        }
    }

    #[test]
    fn 按设计固定结构拼装() {
        let required = vec!["spec.md".to_string(), "plan.md".to_string()];
        let existing = vec!["requirement.md".to_string()];
        let prompt = build_stage_prompt(&输入(&required, &existing, None));
        assert_eq!(
            prompt,
            "使用技能 vk-spec。\n\
             需求：VK-12 导出报表 [qa:review-blocker-once]\n\
             产出物目录（绝对路径）：/abs/ws/.vk/runs/VK-12\n\
             必须产出：spec.md, plan.md\n\
             上一阶段产出：requirement.md\n\
             这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。"
        );
    }

    #[test]
    fn 打回意见单独一行且空白意见不输出() {
        let prompt = build_stage_prompt(&输入(&[], &[], Some("补充验收标准")));
        assert!(prompt.contains("\n上一次被打回的意见：补充验收标准\n"));
        assert!(prompt.contains("必须产出：无"));
        assert!(prompt.contains("上一阶段产出：无"));
        let blank = build_stage_prompt(&输入(&[], &[], Some("   ")));
        assert!(!blank.contains("上一次被打回的意见"));
    }

    #[test]
    fn 模拟器能解析引擎拼出的提示词() {
        let required = vec!["review.json".to_string()];
        let prompt = build_stage_prompt(&输入(&required, &[], Some("多行\n意见\n必须产出：evil.md")));
        let parsed = parse_pipeline_prompt(&prompt).unwrap();
        assert_eq!(parsed.required, vec!["review.json"]);
        assert_eq!(parsed.requirement, "VK-12 导出报表 [qa:review-blocker-once]");
        assert_eq!(parsed.artifacts_dir, Path::new("/abs/ws/.vk/runs/VK-12"));
    }
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p services pipeline::prompt`
Expected: 编译失败，`cannot find struct \`StagePromptInput\``。

- [ ] **Step 3: 实现**

在测试模块之前写入：

```rust
//! 阶段提示词拼装（设计 §6.2，格式常量见 `executors::pipeline_prompt`）。

use std::path::Path;

use executors::pipeline_prompt::{
    ARTIFACTS_DIR_LABEL, FEEDBACK_LABEL, PREVIOUS_ARTIFACTS_LABEL, REQUIRED_ARTIFACTS_LABEL,
    REQUIREMENT_LABEL, SKILL_PREFIX, format_artifact_list,
};

pub const AUTOMATION_NOTICE: &str =
    "这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。";

pub struct StagePromptInput<'a> {
    pub skill: &'a str,
    pub simple_id: &'a str,
    pub title: &'a str,
    pub artifacts_dir: &'a Path,
    pub required: &'a [String],
    /// 产出物目录里已有的文件（不含 `*.prev`）。
    pub existing: &'a [String],
    /// 打回意见或上次失败原因。
    pub feedback: Option<&'a str>,
}

pub fn build_stage_prompt(input: &StagePromptInput<'_>) -> String {
    let mut lines = Vec::with_capacity(7);
    lines.push(format!("{SKILL_PREFIX}{}。", input.skill));
    lines.push(format!(
        "{REQUIREMENT_LABEL}{} {}",
        input.simple_id,
        single_line(input.title)
    ));
    lines.push(format!("{ARTIFACTS_DIR_LABEL}{}", input.artifacts_dir.display()));
    lines.push(format!(
        "{REQUIRED_ARTIFACTS_LABEL}{}",
        format_artifact_list(input.required)
    ));
    lines.push(format!(
        "{PREVIOUS_ARTIFACTS_LABEL}{}",
        format_artifact_list(input.existing)
    ));
    if let Some(feedback) = input.feedback.map(str::trim).filter(|f| !f.is_empty()) {
        lines.push(format!("{FEEDBACK_LABEL}{feedback}"));
    }
    lines.push(AUTOMATION_NOTICE.to_string());
    lines.join("\n")
}

/// 标题压成一行：「需求：」行必须是单行，模拟器按行解析。
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p services pipeline::prompt`
Expected: `test result: ok. 3 passed`

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：按设计固定结构拼装阶段提示词

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: qa-mode 模拟执行器接线

**Files:**
- Modify: `crates/executors/src/executors/qa_mock.rs:36-78`（`spawn`），新增 `apply_side_effects`，tests 模块追加 2 个测试

- [ ] **Step 1: 写失败测试**

在 `qa_mock.rs` 的 `mod tests` 末尾追加：

```rust
    #[tokio::test]
    async fn 流水线提示词只写产出物不做随机改动() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("repo");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("a.md"), "a").unwrap();
        std::fs::write(work.join("b.md"), "b").unwrap();
        let artifacts = dir.path().join(".vk/runs/VK-1");
        let prompt = format!(
            "使用技能 vk-requirement。\n需求：VK-1 示例\n产出物目录（绝对路径）：{}\n必须产出：requirement.md\n",
            artifacts.display()
        );

        assert!(apply_side_effects(&work, &prompt).await);
        assert!(artifacts.join("requirement.md").exists());
        assert_eq!(std::fs::read_to_string(work.join("a.md")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(work.join("b.md")).unwrap(), "b");
        assert_eq!(
            std::fs::read_dir(&work).unwrap().count(),
            2,
            "流水线模式不应新建 qa_created_*.txt"
        );
    }

    #[tokio::test]
    async fn 普通提示词仍走随机文件操作() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!apply_side_effects(dir.path(), "随便改点东西").await);
        let created = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("qa_created_"));
        assert!(created, "普通模式行为不变：会新建 qa_created_*.txt");
    }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p executors --features qa-mode qa_mock`
Expected: 编译失败，`cannot find function \`apply_side_effects\``。

- [ ] **Step 3: 实现**

在 `qa_mock.rs` 的 `perform_file_operations` 函数之前加：

```rust
/// 流水线提示词（带「产出物目录」行，契约 §5）：只写约定产出物，不做随机删改
/// ——随机删改会波及工作区里的 `.vk/` 产出物。普通提示词：沿用原来的随机文件操作。
/// 返回是否是流水线模式。
async fn apply_side_effects(current_dir: &Path, prompt: &str) -> bool {
    match crate::pipeline_prompt::parse_pipeline_prompt(prompt) {
        Some(parsed) => {
            match crate::pipeline_prompt::write_mock_artifacts(&parsed) {
                Ok(paths) => info!("QA Mock: 写出流水线产出物 {:?}", paths),
                Err(e) => warn!("QA Mock: 写流水线产出物失败: {}", e),
            }
            true
        }
        None => {
            perform_file_operations(current_dir).await;
            false
        }
    }
}
```

把 `spawn` 里：

```rust
        // 1. Perform file operations before spawning the log output process
        perform_file_operations(current_dir).await;
```

改为：

```rust
        // 1. Perform file operations (or write pipeline artifacts) before spawning the log output process
        let pipeline_mode = apply_side_effects(current_dir, prompt).await;
```

把生成脚本的那段：

```rust
        let script = format!(
            r#"while IFS= read -r line; do echo "$line"; sleep 1; done < "{}"; rm -f "{}""#,
            log_file.display(),
            log_file.display()
        );
```

改为：

```rust
        // 流水线模式每行 0.1 秒：七个阶段按原来的 1 秒/行要 70 秒以上，端到端测试太慢。
        let line_delay = if pipeline_mode { "0.1" } else { "1" };
        let script = format!(
            r#"while IFS= read -r line; do echo "$line"; sleep {}; done < "{}"; rm -f "{}""#,
            line_delay,
            log_file.display(),
            log_file.display()
        );
```

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p executors --features qa-mode qa_mock`
Expected: `test result: ok. 6 passed`（原有 4 个 + 新增 2 个）

- [ ] **Step 5: 提交**

```bash
git add crates/executors/src/executors/qa_mock.rs
git commit -m "$(cat <<'EOF'
qa-mode：模拟执行器按流水线提示词写产出物，支持四个测试标记

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: 产出物读盘入库 `artifacts.rs`

**Files:**
- Create: `crates/services/src/services/pipeline/artifacts.rs`
- Create: `crates/services/src/services/pipeline/test_support.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`（加 `pub mod artifacts;` 与 `#[cfg(test)] mod test_support;`）

- [ ] **Step 1: 建测试夹具**

新建 `crates/services/src/services/pipeline/test_support.rs`：

```rust
//! 流水线测试夹具（只在测试里编译）。

use api_types::{
    issue::{CreateIssueRequest, Issue},
    project::CreateProjectRequest,
};
use db::{
    models::{
        issue::Issues,
        local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
        local_project_status::{ProjectStatuses, StageType},
        pipeline::{
            CreatePipelineRun, CreateStageRun, GateKind, PipelineRun, PipelineRuns,
            PipelineStageKey, PipelineStageRun, PipelineStageRuns, PipelineStageStatus,
        },
        workspace::{CreateWorkspace, Workspace},
    },
    test_support::TestDb,
};
use uuid::Uuid;

pub(crate) async fn 准备需求(test_db: &TestDb, title: &str) -> Issue {
    let project = LocalProjects::create(
        test_db.pool(),
        &CreateProjectRequest {
            id: None,
            organization_id: DEFAULT_ORGANIZATION_ID,
            name: "流水线项目".to_string(),
            color: "#6366f1".to_string(),
        },
    )
    .await
    .unwrap();
    let backlog = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Backlog)
        .await
        .unwrap()
        .unwrap();
    Issues::create(
        test_db.pool(),
        &CreateIssueRequest {
            id: None,
            project_id: project.id,
            status_id: backlog.id,
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
        },
        DEFAULT_USER_ID,
    )
    .await
    .unwrap()
}

pub(crate) async fn 准备工作区(test_db: &TestDb) -> Uuid {
    Workspace::create(
        test_db.pool(),
        &CreateWorkspace {
            branch: "vk/pipeline".to_string(),
            name: None,
        },
        Uuid::new_v4(),
        DEFAULT_USER_ID,
    )
    .await
    .unwrap()
    .id
}

/// 直接落一条运行与一条尝试（不经引擎）。
pub(crate) async fn 准备运行与阶段(
    test_db: &TestDb,
    issue: &Issue,
    stage_key: PipelineStageKey,
) -> (PipelineRun, PipelineStageRun) {
    let run = PipelineRuns::create(
        test_db.pool(),
        &CreatePipelineRun {
            issue_id: issue.id,
            project_id: issue.project_id,
            workspace_id: None,
            template_key: "standard".to_string(),
            template_version: 1,
            template_json: "{}".to_string(),
            template_warning: None,
            executor_config_json: "{}".to_string(),
            first_stage: stage_key,
        },
    )
    .await
    .unwrap();
    let stage = PipelineStageRuns::create(
        test_db.pool(),
        &CreateStageRun {
            run_id: run.id,
            project_id: run.project_id,
            stage_key,
            gate_kind: GateKind::Human,
            status: PipelineStageStatus::Running,
            feedback: None,
        },
    )
    .await
    .unwrap();
    (run, stage)
}
```

`mod.rs` 加：

```rust
pub mod artifacts;

#[cfg(test)]
mod test_support;
```

- [ ] **Step 2: 写失败测试**

新建 `artifacts.rs`，先只放测试：

```rust
#[cfg(test)]
mod tests {
    use db::{
        models::pipeline::{ArtifactKind, IssueArtifacts, PipelineStageKey},
        test_support::TestDb,
    };

    use super::*;
    use crate::services::pipeline::test_support::{准备运行与阶段, 准备需求};

    #[test]
    fn 产出物目录在工作区根下的_vk_runs() {
        assert_eq!(
            artifacts_dir(Path::new("/ws"), "VK-12"),
            PathBuf::from("/ws/.vk/runs/VK-12")
        );
        assert_eq!(sanitize_segment("../x y"), "___x_y");
        assert_eq!(sanitize_segment(""), "_");
        assert_eq!(relative_path("VK-12", "spec.md"), ".vk/runs/VK-12/spec.md");
    }

    #[test]
    fn 已有产出物清单排序且不含旧版本() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["spec.md", "requirement.md", "review.json.prev"] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(list_existing(dir.path()), vec!["requirement.md", "spec.md"]);
        assert!(list_existing(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn 只读声明过且非空的产出物() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# 规格").unwrap();
        std::fs::write(dir.path().join("plan.md"), "  \n").unwrap();
        std::fs::write(dir.path().join("other.md"), "不该读").unwrap();
        let declared = vec!["spec.md".to_string(), "plan.md".to_string(), "x.md".to_string()];
        let read = read_declared(dir.path(), &declared);
        assert_eq!(read.keys().collect::<Vec<_>>(), vec!["spec.md"]);
    }

    #[tokio::test]
    async fn 入库时内容未变不重复写版本() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "入库").await;
        let (_run, stage) = 准备运行与阶段(&test_db, &issue, PipelineStageKey::Spec).await;
        let dir = tempfile::tempdir().unwrap();
        let declared = vec!["spec.md".to_string(), "plan.md".to_string()];

        std::fs::write(dir.path().join("spec.md"), "第一版").unwrap();
        let read = ingest_stage_artifacts(
            test_db.pool(),
            issue.id,
            &issue.simple_id,
            stage.id,
            dir.path(),
            &declared,
        )
        .await
        .unwrap();
        assert_eq!(read.len(), 1, "plan.md 不存在，不读");

        ingest_stage_artifacts(test_db.pool(), issue.id, &issue.simple_id, stage.id, dir.path(), &declared)
            .await
            .unwrap();
        std::fs::write(dir.path().join("spec.md"), "第二版").unwrap();
        ingest_stage_artifacts(test_db.pool(), issue.id, &issue.simple_id, stage.id, dir.path(), &declared)
            .await
            .unwrap();

        let all = IssueArtifacts::list_summaries_for_issue(test_db.pool(), issue.id)
            .await
            .unwrap();
        assert_eq!(all.len(), 2, "相同内容只入库一次");
        assert_eq!(all[1].version, 2);
        assert_eq!(all[1].kind, ArtifactKind::Spec);
        assert_eq!(
            all[1].rel_path,
            format!(".vk/runs/{}/spec.md", sanitize_segment(&issue.simple_id))
        );
    }
}
```

- [ ] **Step 3: 运行，确认失败**

Run: `cargo test -p services pipeline::artifacts`
Expected: 编译失败，`cannot find function \`artifacts_dir\``等。

- [ ] **Step 4: 实现**

在测试模块之前写入：

```rust
//! 产出物读盘与入库（设计 §5、§6.2）。
//!
//! 产出物目录 = `<工作区目录>/.vk/runs/<simple_id>/`。工作区目录下每个仓库是子目录
//! （`crates/workspace-manager/src/workspace_manager.rs:312`），这个位置不在任何 git 仓库里。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use db::models::pipeline::{ArtifactKind, IssueArtifacts, truncate_for_storage};
use sqlx::SqlitePool;
use uuid::Uuid;

pub fn artifacts_dir(workspace_root: &Path, simple_id: &str) -> PathBuf {
    workspace_root
        .join(".vk")
        .join("runs")
        .join(sanitize_segment(simple_id))
}

/// 相对工作区根目录的路径，存进 `issue_artifacts.rel_path`。
pub fn relative_path(simple_id: &str, file_name: &str) -> String {
    format!(".vk/runs/{}/{}", sanitize_segment(simple_id), file_name)
}

/// 目录名只保留 ASCII 字母数字、`-`、`_`，其余替换成 `_`，防止路径穿越。
pub fn sanitize_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// 目录里已有的产出物文件名（不含 `*.prev` 旧版本与子目录），排序后返回。
pub fn list_existing(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.ends_with(".prev"))
        .collect();
    names.sort();
    names
}

/// 读阶段声明的产出物：存在且非空白的才收进来。非 UTF-8 按有损转换。
pub fn read_declared(dir: &Path, declared: &[String]) -> BTreeMap<String, String> {
    declared
        .iter()
        .filter_map(|name| {
            let bytes = std::fs::read(dir.join(name)).ok()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            (!text.trim().is_empty()).then(|| (name.clone(), text))
        })
        .collect()
}

/// 读盘并入库：与该 kind 最近一版（截断后）内容相同的不重复入库。
/// 返回读到的全文，给关卡判定用（判定看磁盘全文，不看库里截断后的内容）。
pub async fn ingest_stage_artifacts(
    pool: &SqlitePool,
    issue_id: Uuid,
    simple_id: &str,
    stage_run_id: Uuid,
    dir: &Path,
    declared: &[String],
) -> Result<BTreeMap<String, String>, sqlx::Error> {
    let present = read_declared(dir, declared);
    for (name, content) in &present {
        let Some(kind) = ArtifactKind::from_file_name(name) else {
            continue;
        };
        let (stored, _) = truncate_for_storage(content);
        if let Some(latest) = IssueArtifacts::latest_for_kind(pool, issue_id, kind).await?
            && latest.content == stored
        {
            continue;
        }
        IssueArtifacts::insert_version(
            pool,
            issue_id,
            stage_run_id,
            kind,
            &relative_path(simple_id, name),
            content,
        )
        .await?;
    }
    Ok(present)
}
```

- [ ] **Step 5: 运行，确认通过**

Run: `cargo test -p services pipeline::artifacts`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：产出物目录约定、读盘与去重入库

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 14: 抽出 `start_new_session` 与执行层接缝 `launcher.rs`

**Files:**
- Modify: `crates/services/src/services/container.rs:1047-1131`（`start_workspace` 拆成两段）
- Create: `crates/services/src/services/pipeline/launcher.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`（加 `pub mod launcher;`）

**为什么没有容器层单测：** 仓库里没有构造 `LocalContainerService` 的测试夹具（它要真 git 仓库、真子进程、`ExecutorConfigs` 全局缓存），`start_workspace` 现在也没有单测。本任务对容器是**行为不变的重构**，用编译 + 既有测试兜底；真实执行链路由计划 B 的 e2e（qa-mode 跑真后端）覆盖。引擎逻辑在任务 16 用假启动器测透。

- [ ] **Step 1: 重构 `start_workspace`**

把 `crates/services/src/services/container.rs:1047-1131` 的 `async fn start_workspace(...) { ... }` 整个替换为下面两个方法（`start_new_session` 的函数体就是原 `start_workspace` 从「重新读工作区」到结尾的原样代码，只改了 `repos_with_setup` 那一行）：

```rust
    async fn start_workspace(
        &self,
        workspace: &Workspace,
        executor_config: ExecutorConfig,
        prompt: String,
    ) -> Result<ExecutionProcess, ContainerError> {
        // Create container
        self.create(workspace).await?;
        self.start_new_session(workspace, executor_config, prompt, true)
            .await
    }

    /// 在已建好目录的工作区里开一个新会话并启动编码智能体。
    ///
    /// `run_setup = true` 时先跑仓库 setup 脚本（与原 `start_workspace` 完全一致）；
    /// 流水线后续阶段在同一工作区开新会话时传 `false`，跳过 setup。
    /// 智能体之后照常串上仓库 cleanup 脚本。
    async fn start_new_session(
        &self,
        workspace: &Workspace,
        executor_config: ExecutorConfig,
        prompt: String,
        run_setup: bool,
    ) -> Result<ExecutionProcess, ContainerError> {
        let repos = WorkspaceRepo::find_repos_for_workspace(&self.db().pool, workspace.id).await?;

        let workspace = Workspace::find_by_id(&self.db().pool, workspace.id)
            .await?
            .ok_or(SqlxError::RowNotFound)?;

        // Create a session for this workspace
        let session = Session::create(
            &self.db().pool,
            &CreateSession {
                executor: Some(executor_config.executor.to_string()),
                name: None,
            },
            Uuid::new_v4(),
            workspace.id,
        )
        .await?;

        let repos_with_setup: Vec<_> = if run_setup {
            repos.iter().filter(|r| r.setup_script.is_some()).collect()
        } else {
            Vec::new()
        };

        let all_parallel = repos_with_setup.iter().all(|r| r.parallel_setup_script);

        let cleanup_action = self.cleanup_actions_for_repos(&repos);

        let working_dir = session
            .agent_working_dir
            .as_ref()
            .filter(|dir| !dir.is_empty())
            .cloned();

        let coding_action = ExecutorAction::new(
            ExecutorActionType::CodingAgentInitialRequest(CodingAgentInitialRequest {
                prompt,
                executor_config: executor_config.clone(),
                working_dir,
            }),
            cleanup_action.map(Box::new),
        );

        let execution_process = if all_parallel {
            // All parallel: start each setup independently, then start coding agent
            for repo in &repos_with_setup {
                if let Some(action) = Self::setup_action_for_repo(repo)
                    && let Err(e) = self
                        .start_execution(
                            &workspace,
                            &session,
                            &action,
                            &ExecutionProcessRunReason::SetupScript,
                        )
                        .await
                {
                    tracing::warn!(?e, "Failed to start setup script in parallel mode");
                }
            }
            self.start_execution(
                &workspace,
                &session,
                &coding_action,
                &ExecutionProcessRunReason::CodingAgent,
            )
            .await?
        } else {
            // Any sequential: chain ALL setups → coding agent via next_action
            let main_action = Self::build_sequential_setup_chain(&repos_with_setup, coding_action);
            self.start_execution(
                &workspace,
                &session,
                &main_action,
                &ExecutionProcessRunReason::SetupScript,
            )
            .await?
        };

        Ok(execution_process)
    }
```

（`repos_with_setup` 为空时 `all_parallel` 为 `true`，直接起智能体，这与原实现对「没有 setup 脚本的仓库」的行为相同。）

Run: `cargo check --workspace --features qa-mode`
Expected: `Finished`，无 error。

Run: `cargo test -p services -p local-deployment`
Expected: 全部通过（与重构前一致）。

- [ ] **Step 2: 写失败测试（launcher）**

新建 `crates/services/src/services/pipeline/launcher.rs`，先只放测试；`mod.rs` 加 `pub mod launcher;`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 检查脚本逐行执行且任一失败即退出() {
        let script = checks_script(&["pnpm run lint".to_string(), "  cargo test  ".to_string()]);
        assert_eq!(script, "set -e\npnpm run lint\ncargo test\n");
    }

    #[tokio::test]
    async fn 空启动器所有启动都报错() {
        let launcher = NoopStageLauncher;
        let id = Uuid::new_v4();
        assert!(launcher.prepare_workspace(id).await.is_err());
        assert!(
            launcher
                .start_agent_session(
                    id,
                    &ExecutorConfig::new(executors::executors::BaseCodingAgent::ClaudeCode),
                    "p".to_string(),
                    true
                )
                .await
                .is_err()
        );
        assert!(launcher.start_checks(id, id, "true".to_string()).await.is_err());
        launcher.stop_workspace(id).await;
    }
}
```

- [ ] **Step 3: 运行，确认失败**

Run: `cargo test -p services pipeline::launcher`
Expected: 编译失败，`cannot find function \`checks_script\``等。

- [ ] **Step 4: 实现**

`launcher.rs` 测试模块之前写入：

```rust
//! 引擎与执行层之间的接缝。引擎只依赖 [`StageLauncher`]：生产环境用
//! [`ContainerStageLauncher`] 接到容器服务；测试用假实现或 [`NoopStageLauncher`]。

use std::path::PathBuf;

use async_trait::async_trait;
use db::models::{
    execution_process::ExecutionProcessRunReason, session::Session, workspace::Workspace,
};
use executors::{
    actions::{
        ExecutorAction, ExecutorActionType,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    },
    profile::ExecutorConfig,
};
use uuid::Uuid;

use super::engine::PipelineError;
use crate::services::container::ContainerService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedStep {
    pub session_id: Uuid,
    /// 会话里第一个进程（有 setup 脚本时是 setup 进程）。
    pub execution_process_id: Uuid,
}

#[async_trait]
pub trait StageLauncher: Send + Sync {
    /// 确保工作区目录与各仓库 worktree 存在，返回工作区根目录（container_ref）。
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError>;

    /// 在工作区里开新会话跑编码智能体；`run_setup` 为 true 时先跑 setup 脚本。
    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 在已有会话里跑检查脚本（run_reason = PipelineStep）。
    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError>;

    /// 停掉工作区里正在跑的进程（取消运行时用）。
    async fn stop_workspace(&self, workspace_id: Uuid);
}

/// 把模板的 checks 拼成一个脚本：`set -e`，任一命令失败即以非零退出。
pub fn checks_script(checks: &[String]) -> String {
    let mut script = String::from("set -e\n");
    for check in checks {
        script.push_str(check.trim());
        script.push('\n');
    }
    script
}

pub struct ContainerStageLauncher<C> {
    container: C,
}

impl<C> ContainerStageLauncher<C> {
    pub fn new(container: C) -> Self {
        Self { container }
    }
}

impl<C> ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync,
{
    async fn load_workspace(&self, workspace_id: Uuid) -> Result<Workspace, PipelineError> {
        Workspace::find_by_id(&self.container.db().pool, workspace_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("工作区 {workspace_id}")))
    }
}

fn launch_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Launch(error.to_string())
}

#[async_trait]
impl<C> StageLauncher for ContainerStageLauncher<C>
where
    C: ContainerService + Send + Sync + 'static,
{
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let root = self
            .container
            .ensure_container_exists(&workspace)
            .await
            .map_err(launch_error)?;
        Ok(PathBuf::from(root))
    }

    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let process = self
            .container
            .start_new_session(&workspace, executor_config.clone(), prompt, run_setup)
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id: process.session_id,
            execution_process_id: process.id,
        })
    }

    async fn start_checks(
        &self,
        workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        let workspace = self.load_workspace(workspace_id).await?;
        let session = Session::find_by_id(&self.container.db().pool, session_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("会话 {session_id}")))?;
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script,
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::PipelineCheck,
                working_dir: session.agent_working_dir.clone(),
            }),
            None,
        );
        let process = self
            .container
            .start_execution(
                &workspace,
                &session,
                &action,
                &ExecutionProcessRunReason::PipelineStep,
            )
            .await
            .map_err(launch_error)?;
        Ok(LaunchedStep {
            session_id,
            execution_process_id: process.id,
        })
    }

    async fn stop_workspace(&self, workspace_id: Uuid) {
        if let Ok(workspace) = self.load_workspace(workspace_id).await {
            self.container.try_stop(&workspace, false).await;
        }
    }
}

/// 不接执行层的启动器：所有启动都报错。给只测状态与错误映射的路由单测用。
pub struct NoopStageLauncher;

#[async_trait]
impl StageLauncher for NoopStageLauncher {
    async fn prepare_workspace(&self, _workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_agent_session(
        &self,
        _workspace_id: Uuid,
        _executor_config: &ExecutorConfig,
        _prompt: String,
        _run_setup: bool,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn start_checks(
        &self,
        _workspace_id: Uuid,
        _session_id: Uuid,
        _script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn stop_workspace(&self, _workspace_id: Uuid) {}
}
```

本步骤引用了 `super::engine::PipelineError`，它在任务 15 才建。**为了让本任务能单独编译**，先在 `mod.rs` 加 `pub mod engine;`，并新建只含错误类型的 `crates/services/src/services/pipeline/engine.rs`：

```rust
//! 流水线引擎（任务 15 补全 `PipelineService`）。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("找不到{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("启动执行失败：{0}")]
    Launch(String),
}
```

- [ ] **Step 5: 运行，确认通过**

Run: `cargo test -p services pipeline::launcher`
Expected: `test result: ok. 2 passed`

- [ ] **Step 6: 提交**

```bash
git add crates/services/src/services/container.rs crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
容器：抽出 start_new_session；流水线：执行层接缝 StageLauncher

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 15: 引擎 `engine.rs`（`PipelineService`）

**Files:**
- Modify: `crates/services/src/services/pipeline/engine.rs`（在任务 14 的错误类型之后补全）
- Modify: `crates/services/src/services/pipeline/mod.rs`（再导出）

本任务只写实现并确保编译；行为测试集中在任务 16（黄金路径 + 12 条异常路径），因为引擎的每个公开方法都要一个带假启动器的完整场景才有意义。

- [ ] **Step 1: 写实现**

把 `crates/services/src/services/pipeline/engine.rs` 整个替换为：

```rust
//! 流水线引擎：对外唯一入口 [`PipelineService`]（设计 §6，行为总表见计划 A §1.3）。
//!
//! 所有状态变更都在一把异步锁里串行执行：HTTP 请求（启动、关卡、暂停……）与
//! 执行进程退出回调可能并发到达，串行后状态机不会交错。锁内会调用启动器
//! （开会话、起进程），启动器不会回调本服务，所以不会死锁。

use std::{path::PathBuf, sync::Arc};

use api_types::issue::Issue;
use db::{
    DBService,
    models::{
        db_retry::is_unique_violation,
        execution_process::{ExecutionProcess, ExecutionProcessRunReason},
        issue::Issues,
        local_project_status::StageType,
        pipeline::{
            CreatePipelineRun, CreateStageRun, GateDecisionKind, GateDecisionRequest,
            IssueArtifacts, IssuePipelineView, PipelineGateDecisions, PipelineRun,
            PipelineRunStatus, PipelineRuns, PipelineStageKey, PipelineStageRun,
            PipelineStageRuns, PipelineStageStatus,
        },
        workspace::Workspace,
    },
};
use executors::profile::ExecutorConfig;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    artifacts,
    gates::{self, StageVerdict},
    launcher::{self, StageLauncher},
    prompt::{self, StagePromptInput},
    template::{self, PipelineTemplate},
    transition::{self, FinishedProcess, PipelineExitEvent, Transition},
};

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("找不到{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("启动执行失败：{0}")]
    Launch(String),
}

pub struct StartPipelineInput {
    pub issue: Issue,
    pub workspace_id: Uuid,
    /// 读 `.vibe/pipeline.yaml` 的仓库根目录（第一个仓库的主检出目录）。
    pub repo_root: PathBuf,
    pub executor_config: ExecutorConfig,
    pub template_key: Option<String>,
}

const ACTIVE_RUN_CONFLICT: &str = "该需求已有未结束的流水线";

#[derive(Clone)]
pub struct PipelineService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DBService,
    launcher: Arc<dyn StageLauncher>,
    lock: Mutex<()>,
}

impl PipelineService {
    pub fn new(db: DBService, launcher: Arc<dyn StageLauncher>) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                launcher,
                lock: Mutex::new(()),
            }),
        }
    }

    fn pool(&self) -> &SqlitePool {
        &self.inner.db.pool
    }

    // ------------------------------------------------------------------
    // 查询（不加锁）
    // ------------------------------------------------------------------

    /// 需求最近一条运行的完整视图；没有运行时 None。
    pub async fn view_for_issue(
        &self,
        issue_id: Uuid,
    ) -> Result<Option<IssuePipelineView>, PipelineError> {
        match PipelineRuns::find_latest_for_issue(self.pool(), issue_id).await? {
            Some(run) => Ok(Some(self.view_for_run(run).await?)),
            None => Ok(None),
        }
    }

    pub async fn view_for_run_id(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let run = self.load_run(run_id).await?;
        self.view_for_run(run).await
    }

    async fn view_for_run(&self, run: PipelineRun) -> Result<IssuePipelineView, PipelineError> {
        let template = self.template_for(&run).await?;
        let pool = self.pool();
        Ok(IssuePipelineView {
            stages: PipelineStageRuns::list_by_run(pool, run.id).await?,
            decisions: PipelineGateDecisions::list_by_run(pool, run.id).await?,
            artifacts: IssueArtifacts::list_summaries_for_issue(pool, run.issue_id).await?,
            template: template.to_view(),
            run,
        })
    }

    // ------------------------------------------------------------------
    // 启动
    // ------------------------------------------------------------------

    /// 启动流水线。启动首阶段失败不返回错误：运行与阶段记为 failed，返回视图。
    pub async fn start(&self, input: StartPipelineInput) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();

        if PipelineRuns::has_active_for_issue(pool, input.issue.id).await? {
            return Err(PipelineError::Conflict(ACTIVE_RUN_CONFLICT.to_string()));
        }
        let (template, warning) = match input.template_key.as_deref() {
            None | Some(template::BUILTIN_TEMPLATE_KEY) => template::load_template(&input.repo_root),
            Some(other) => {
                return Err(PipelineError::BadRequest(format!("未知模板：{other}")));
            }
        };
        if let Some(warning) = &warning {
            tracing::warn!(issue_id = %input.issue.id, "{warning}");
        }

        let first = template.first_stage().clone();
        let run = PipelineRuns::create(
            pool,
            &CreatePipelineRun {
                issue_id: input.issue.id,
                project_id: input.issue.project_id,
                workspace_id: Some(input.workspace_id),
                template_key: template.key.clone(),
                template_version: template.version,
                template_json: serde_json::to_string(&template).expect("模板可以序列化"),
                template_warning: warning,
                executor_config_json: serde_json::to_string(&input.executor_config)
                    .expect("执行器配置可以序列化"),
                first_stage: first.key,
            },
        )
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                PipelineError::Conflict(ACTIVE_RUN_CONFLICT.to_string())
            } else {
                PipelineError::Database(e)
            }
        })?;

        let stage_run = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: first.key,
                gate_kind: first.gate.kind(),
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await?;
        self.move_issue(run.issue_id, transition::stage_column(first.key))
            .await;
        self.launch(&run, &stage_run, &template).await;

        self.view_for_run_id(run.id).await
    }

    // ------------------------------------------------------------------
    // 执行进程退出
    // ------------------------------------------------------------------

    /// 容器退出钩子发来的事件：读执行进程快照后交给 [`Self::on_process_finished`]。
    /// 错误只记日志——回调没有调用方可以上报。
    pub async fn handle_exit_event(&self, event: PipelineExitEvent) {
        let process = match ExecutionProcess::find_by_id(self.pool(), event.execution_process_id)
            .await
        {
            Ok(Some(process)) => process,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(
                    "流水线读取执行进程 {} 失败: {}",
                    event.execution_process_id,
                    e
                );
                return;
            }
        };
        let has_next_action = process
            .executor_action()
            .ok()
            .and_then(|action| action.next_action())
            .is_some();
        let finished = FinishedProcess {
            execution_process_id: process.id,
            session_id: process.session_id,
            run_reason: process.run_reason.clone(),
            status: process.status.clone(),
            exit_code: process.exit_code,
            has_next_action,
            chain_continues: event.chain_continues,
        };
        if let Err(e) = self.on_process_finished(finished).await {
            tracing::error!(
                "流水线处理执行进程 {} 退出失败: {}",
                event.execution_process_id,
                e
            );
        }
    }

    pub async fn on_process_finished(&self, process: FinishedProcess) -> Result<(), PipelineError> {
        if !transition::is_stage_terminal(&process) {
            return Ok(());
        }
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();

        let Some(stage_run) =
            PipelineStageRuns::find_running_by_session(pool, process.session_id).await?
        else {
            // 不是流水线的会话，或阶段已被取消/中断。
            return Ok(());
        };
        let run = self.load_run(stage_run.run_id).await?;
        if run.status.is_finished() {
            return Ok(());
        }
        let template = self.template_for(&run).await?;
        let Some(stage) = template.stage(stage_run.stage_key).cloned() else {
            let reason = format!("模板里没有阶段 {}", stage_run.stage_key.as_str());
            self.fail_run(&run, &stage_run, &reason).await?;
            return Ok(());
        };
        let ok = transition::process_succeeded(&process);

        // 开发阶段：智能体成功后先在同会话跑检查脚本，检查脚本退出后再判关卡。
        if ok
            && !stage.checks.is_empty()
            && process.run_reason != ExecutionProcessRunReason::PipelineStep
        {
            let Some(workspace_id) = run.workspace_id else {
                self.fail_run(&run, &stage_run, "流水线没有关联工作区").await?;
                return Ok(());
            };
            match self
                .inner
                .launcher
                .start_checks(
                    workspace_id,
                    process.session_id,
                    launcher::checks_script(&stage.checks),
                )
                .await
            {
                Ok(step) => {
                    PipelineStageRuns::set_execution_process(
                        pool,
                        stage_run.id,
                        step.execution_process_id,
                    )
                    .await?;
                }
                Err(e) => {
                    let reason = format!("启动检查脚本失败：{e}");
                    self.fail_run(&run, &stage_run, &reason).await?;
                }
            }
            return Ok(());
        }

        if process.run_reason == ExecutionProcessRunReason::CodingAgent {
            PipelineStageRuns::set_execution_process(
                pool,
                stage_run.id,
                process.execution_process_id,
            )
            .await?;
        }

        let error = (!ok).then(|| describe_process_failure(&process));
        let issue = self.load_issue(run.issue_id).await?;
        let collected = match self.workspace_root(&run).await? {
            Some(root) => {
                artifacts::ingest_stage_artifacts(
                    pool,
                    issue.id,
                    &issue.simple_id,
                    stage_run.id,
                    &artifacts::artifacts_dir(&root, &issue.simple_id),
                    &stage.artifacts,
                )
                .await?
            }
            None => Default::default(),
        };

        let verdict = gates::evaluate_stage(&stage, ok, error.as_deref(), &collected);
        let failed_before =
            PipelineStageRuns::count_failed(pool, run.id, stage_run.stage_key).await?;
        let failed_attempts = failed_before + i64::from(verdict.is_failure());
        let next =
            transition::next_transition(&template, stage_run.stage_key, &verdict, failed_attempts);
        let summary = summarize(&verdict);
        self.apply(
            &run,
            &stage_run,
            &template,
            next,
            PipelineStageStatus::Failed,
            Some(summary),
        )
        .await
    }

    // ------------------------------------------------------------------
    // 人工关卡
    // ------------------------------------------------------------------

    pub async fn decide_gate(
        &self,
        stage_run_id: Uuid,
        request: GateDecisionRequest,
        decided_by: Option<Uuid>,
    ) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();

        let stage_run = PipelineStageRuns::find_by_id(pool, stage_run_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("阶段记录 {stage_run_id}")))?;
        if stage_run.status != PipelineStageStatus::WaitingGate {
            return Err(PipelineError::Conflict(format!(
                "该阶段当前状态是 {}，不在等待确认",
                stage_run.status.as_str()
            )));
        }
        let comment = request
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|comment| !comment.is_empty());
        if request.decision == GateDecisionKind::Reject && comment.is_none() {
            return Err(PipelineError::BadRequest("打回必须填写意见".to_string()));
        }
        let run = self.load_run(stage_run.run_id).await?;
        if run.status.is_finished() {
            return Err(PipelineError::Conflict("流水线已结束".to_string()));
        }

        PipelineGateDecisions::create(pool, stage_run.id, request.decision, comment, decided_by)
            .await?;
        let template = self.template_for(&run).await?;
        let next = transition::gate_decision_transition(
            &template,
            stage_run.stage_key,
            request.decision,
            comment,
        );
        self.apply(
            &run,
            &stage_run,
            &template,
            next,
            PipelineStageStatus::Rejected,
            None,
        )
        .await?;
        self.view_for_run_id(run.id).await
    }

    // ------------------------------------------------------------------
    // 暂停 / 继续 / 取消
    // ------------------------------------------------------------------

    pub async fn pause(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let run = self.load_run(run_id).await?;
        match run.status {
            PipelineRunStatus::Running | PipelineRunStatus::WaitingGate => {
                PipelineRuns::update_status(self.pool(), run.id, PipelineRunStatus::Paused, None)
                    .await?;
            }
            other => {
                return Err(PipelineError::Conflict(format!(
                    "当前状态 {} 不能暂停",
                    other.as_str()
                )));
            }
        }
        self.view_for_run_id(run_id).await
    }

    pub async fn resume(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();
        let run = self.load_run(run_id).await?;
        if !matches!(
            run.status,
            PipelineRunStatus::Paused | PipelineRunStatus::Failed
        ) {
            return Err(PipelineError::Conflict(format!(
                "当前状态 {} 不能继续",
                run.status.as_str()
            )));
        }
        let template = self.template_for(&run).await?;
        let latest = PipelineStageRuns::find_latest_for_run(pool, run.id)
            .await?
            .ok_or_else(|| PipelineError::Conflict("流水线没有任何阶段记录".to_string()))?;

        match latest.status {
            PipelineStageStatus::Pending => {
                let run = PipelineRuns::update_status(
                    pool,
                    run.id,
                    PipelineRunStatus::Running,
                    Some(latest.stage_key),
                )
                .await?;
                self.launch(&run, &latest, &template).await;
            }
            PipelineStageStatus::Running => {
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Running, None).await?;
            }
            PipelineStageStatus::WaitingGate => {
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::WaitingGate, None)
                    .await?;
            }
            PipelineStageStatus::Failed if run.status == PipelineRunStatus::Failed => {
                // 人工处理后再给一次机会：同阶段新尝试，上次失败原因带进提示词。
                let run = PipelineRuns::update_status(
                    pool,
                    run.id,
                    PipelineRunStatus::Running,
                    Some(latest.stage_key),
                )
                .await?;
                self.enter_stage(&run, &template, latest.stage_key, latest.error.clone())
                    .await?;
            }
            other => {
                return Err(PipelineError::Conflict(format!(
                    "最后一个阶段状态是 {}，无法继续",
                    other.as_str()
                )));
            }
        }
        self.view_for_run_id(run_id).await
    }

    pub async fn cancel(&self, run_id: Uuid) -> Result<IssuePipelineView, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();
        let run = self.load_run(run_id).await?;
        if run.status.is_finished() {
            return Err(PipelineError::Conflict("流水线已结束".to_string()));
        }
        let mut was_running = false;
        if let Some(latest) = PipelineStageRuns::find_latest_for_run(pool, run.id).await?
            && matches!(
                latest.status,
                PipelineStageStatus::Pending
                    | PipelineStageStatus::Running
                    | PipelineStageStatus::WaitingGate
            )
        {
            was_running = latest.status == PipelineStageStatus::Running;
            PipelineStageRuns::set_status(
                pool,
                latest.id,
                PipelineStageStatus::Skipped,
                None,
                Some("流水线已取消"),
            )
            .await?;
        }
        PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Cancelled, None).await?;
        if was_running && let Some(workspace_id) = run.workspace_id {
            self.inner.launcher.stop_workspace(workspace_id).await;
        }
        self.view_for_run_id(run_id).await
    }

    /// 服务启动时调用：上次没跑完的阶段（进程已随服务退出）记失败，运行转人工。
    /// 返回处理的尝试数。
    pub async fn recover_interrupted(&self) -> Result<usize, PipelineError> {
        let _guard = self.inner.lock.lock().await;
        let pool = self.pool();
        let running = PipelineStageRuns::list_running(pool).await?;
        let count = running.len();
        for stage_run in running {
            let run = self.load_run(stage_run.run_id).await?;
            if run.status.is_finished() {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Skipped,
                    None,
                    None,
                )
                .await?;
                continue;
            }
            self.fail_run(&run, &stage_run, "服务重启，阶段执行被中断")
                .await?;
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // 内部：应用转移、进入阶段、启动
    // ------------------------------------------------------------------

    async fn apply(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
        next: Transition,
        closing: PipelineStageStatus,
        summary: Option<String>,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        match next {
            Transition::WaitHuman => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::WaitingGate,
                    summary.as_deref(),
                    None,
                )
                .await?;
                if run.status != PipelineRunStatus::Paused {
                    PipelineRuns::update_status(pool, run.id, PipelineRunStatus::WaitingGate, None)
                        .await?;
                }
            }
            Transition::Advance { to } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Passed,
                    summary.as_deref(),
                    None,
                )
                .await?;
                self.enter_stage(run, template, to, None).await?;
            }
            Transition::Complete => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Passed,
                    summary.as_deref(),
                    None,
                )
                .await?;
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Completed, None)
                    .await?;
                self.move_issue(run.issue_id, StageType::Done).await;
            }
            Transition::Rerun { stage, feedback } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    closing,
                    summary.as_deref(),
                    Some(&feedback),
                )
                .await?;
                self.enter_stage(run, template, stage, Some(feedback)).await?;
            }
            Transition::FailRun { reason } => {
                PipelineStageRuns::set_status(
                    pool,
                    stage_run.id,
                    PipelineStageStatus::Failed,
                    summary.as_deref(),
                    Some(&reason),
                )
                .await?;
                PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
            }
        }
        Ok(())
    }

    /// 建阶段的新一次尝试并移动需求列；运行未暂停时立即启动，暂停中则留 pending。
    async fn enter_stage(
        &self,
        run: &PipelineRun,
        template: &PipelineTemplate,
        key: PipelineStageKey,
        feedback: Option<String>,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let Some(stage) = template.stage(key) else {
            PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
            return Ok(());
        };
        let paused = self.load_run(run.id).await?.status == PipelineRunStatus::Paused;
        let stage_run = PipelineStageRuns::create(
            pool,
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: key,
                gate_kind: stage.gate.kind(),
                status: if paused {
                    PipelineStageStatus::Pending
                } else {
                    PipelineStageStatus::Running
                },
                feedback,
            },
        )
        .await?;
        let run = PipelineRuns::update_status(
            pool,
            run.id,
            if paused {
                PipelineRunStatus::Paused
            } else {
                PipelineRunStatus::Running
            },
            Some(key),
        )
        .await?;
        self.move_issue(run.issue_id, transition::stage_column(key))
            .await;
        if !paused {
            self.launch(&run, &stage_run, template).await;
        }
        Ok(())
    }

    /// 启动一次尝试；失败时把尝试与运行记为 failed（不向上抛，调用方照常返回视图）。
    async fn launch(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
    ) {
        if let Err(e) = self.try_launch(run, stage_run, template).await {
            tracing::warn!(
                run_id = %run.id,
                stage = stage_run.stage_key.as_str(),
                "流水线阶段启动失败: {e}"
            );
            if let Err(db_error) = self.fail_run(run, stage_run, &e.to_string()).await {
                tracing::error!("记录阶段启动失败时出错: {db_error}");
            }
        }
    }

    async fn try_launch(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        template: &PipelineTemplate,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let workspace_id = run
            .workspace_id
            .ok_or_else(|| PipelineError::Conflict("流水线没有关联工作区".to_string()))?;
        let stage = template.stage(stage_run.stage_key).ok_or_else(|| {
            PipelineError::Conflict(format!("模板里没有阶段 {}", stage_run.stage_key.as_str()))
        })?;
        let issue = self.load_issue(run.issue_id).await?;
        let internals = PipelineRuns::internals(pool, run.id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("流水线 {}", run.id)))?;
        let executor_config: ExecutorConfig =
            serde_json::from_str(&internals.executor_config_json)
                .map_err(|e| PipelineError::Conflict(format!("执行器配置损坏：{e}")))?;

        let root = self.inner.launcher.prepare_workspace(workspace_id).await?;
        let dir = artifacts::artifacts_dir(&root, &issue.simple_id);
        tokio::fs::create_dir_all(&dir).await?;
        let existing = artifacts::list_existing(&dir);
        let feedback = PipelineStageRuns::feedback(pool, stage_run.id).await?;
        let prompt = prompt::build_stage_prompt(&StagePromptInput {
            skill: &stage.skill,
            simple_id: &issue.simple_id,
            title: &issue.title,
            artifacts_dir: &dir,
            required: &stage.artifacts,
            existing: &existing,
            feedback: feedback.as_deref(),
        });
        let run_setup = !PipelineStageRuns::any_launched(pool, run.id).await?;
        let step = self
            .inner
            .launcher
            .start_agent_session(workspace_id, &executor_config, prompt, run_setup)
            .await?;
        PipelineStageRuns::mark_started(pool, stage_run.id, step.session_id, step.execution_process_id)
            .await?;
        Ok(())
    }

    async fn fail_run(
        &self,
        run: &PipelineRun,
        stage_run: &PipelineStageRun,
        reason: &str,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        PipelineStageRuns::set_status(
            pool,
            stage_run.id,
            PipelineStageStatus::Failed,
            None,
            Some(reason),
        )
        .await?;
        PipelineRuns::update_status(pool, run.id, PipelineRunStatus::Failed, None).await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // 内部：读取
    // ------------------------------------------------------------------

    /// 运行启动时存下的模板快照；损坏时退回内置模板。
    async fn template_for(&self, run: &PipelineRun) -> Result<PipelineTemplate, PipelineError> {
        let internals = PipelineRuns::internals(self.pool(), run.id).await?;
        Ok(internals
            .and_then(|internals| serde_json::from_str(&internals.template_json).ok())
            .unwrap_or_else(template::builtin_template))
    }

    async fn load_run(&self, run_id: Uuid) -> Result<PipelineRun, PipelineError> {
        PipelineRuns::find_by_id(self.pool(), run_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("流水线 {run_id}")))
    }

    async fn load_issue(&self, issue_id: Uuid) -> Result<Issue, PipelineError> {
        Issues::find_by_id(self.pool(), issue_id)
            .await?
            .ok_or_else(|| PipelineError::NotFound(format!("需求 {issue_id}")))
    }

    async fn workspace_root(&self, run: &PipelineRun) -> Result<Option<PathBuf>, PipelineError> {
        let Some(workspace_id) = run.workspace_id else {
            return Ok(None);
        };
        Ok(Workspace::find_by_id(self.pool(), workspace_id)
            .await?
            .and_then(|workspace| workspace.container_ref)
            .map(PathBuf::from))
    }

    async fn move_issue(&self, issue_id: Uuid, column: StageType) {
        if let Err(e) = Issues::move_to_stage(self.pool(), issue_id, column).await {
            tracing::warn!(
                "流水线移动需求 {issue_id} 到 {} 列失败: {e}",
                column.as_str()
            );
        }
    }
}

fn run_reason_label(run_reason: &ExecutionProcessRunReason) -> &'static str {
    match run_reason {
        ExecutionProcessRunReason::SetupScript => "setup 脚本",
        ExecutionProcessRunReason::CleanupScript => "cleanup 脚本",
        ExecutionProcessRunReason::ArchiveScript => "归档脚本",
        ExecutionProcessRunReason::CodingAgent => "编码智能体",
        ExecutionProcessRunReason::DevServer => "开发服务器",
        ExecutionProcessRunReason::PipelineStep => "检查脚本",
    }
}

fn describe_process_failure(process: &FinishedProcess) -> String {
    format!(
        "{} 进程未成功结束（状态 {:?}，退出码 {}）",
        run_reason_label(&process.run_reason),
        process.status,
        process
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "无".to_string())
    )
}

fn summarize(verdict: &StageVerdict) -> String {
    match verdict {
        StageVerdict::WaitHuman => "产出齐全，等待人工确认".to_string(),
        StageVerdict::Pass => "自动判定通过".to_string(),
        StageVerdict::Fail { reason } => reason.clone(),
        StageVerdict::FailBackTo { stage, reason } => {
            format!("{reason}（退回 {}）", stage.as_str())
        }
    }
}
```

`mod.rs` 最终形态：

```rust
//! 交付流水线引擎（设计 docs/superpowers/specs/2026-09-18-personal-pipeline-design.md §6）。
//! 对外只暴露 [`PipelineService`]；其余子模块是它的零件。

pub mod artifacts;
pub mod engine;
pub mod gates;
pub mod launcher;
pub mod prompt;
pub mod template;
pub mod transition;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use engine::{PipelineError, PipelineService, StartPipelineInput};
pub use launcher::{ContainerStageLauncher, LaunchedStep, NoopStageLauncher, StageLauncher};
pub use transition::{FinishedProcess, PipelineExitEvent};
```

并新建空的 `crates/services/src/services/pipeline/tests.rs`（任务 16 填内容）：

```rust
//! 引擎集成测试（任务 16）。
```

- [ ] **Step 2: 编译与 lint**

Run: `cargo check -p services --all-targets`
Expected: `Finished`，无 error。

Run: `cargo clippy -p services --all-targets --features qa-mode -- -D warnings`
Expected: 无 warning（若提示 `test_support` 里 `准备工作区` 未使用，属于任务 16 之前的中间态，可先忽略这一条，任务 16 会用到它）。

- [ ] **Step 3: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：引擎 PipelineService（启动、回调、关卡、暂停继续取消、重启恢复）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 16: 引擎集成测试（黄金路径 + 异常路径）

**这就是「qa-mode 七阶段黄金路径」的 Rust 层集成测试。** 它用真 SQLite（`TestDb`）、真引擎、真模板/关卡/转移/提示词/产出物入库代码，以及与 qa-mode 模拟器**同一份**的产出物生成函数（`executors::pipeline_prompt::write_mock_artifacts`）；只把「开会话、起进程」换成假启动器，并由测试直接调用 `on_process_finished` 代替容器退出钩子。容器侧（`start_new_session`、退出钩子的 `chain_continues`）没有可用的测试夹具（见任务 14 说明），由计划 B 的 e2e 在 `--features qa-mode` 的真后端上覆盖；`is_stage_terminal` 的各种进程组合已在任务 9 单测。

**Files:**
- Modify: `crates/services/src/services/pipeline/test_support.rs`（追加假启动器）
- Modify: `crates/services/src/services/pipeline/tests.rs`

- [ ] **Step 1: 追加假启动器**

在 `test_support.rs` 顶部 `use` 区追加：

```rust
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use executors::{
    pipeline_prompt::{parse_pipeline_prompt, write_mock_artifacts},
    profile::ExecutorConfig,
};
use sqlx::SqlitePool;

use super::{
    engine::PipelineError,
    launcher::{LaunchedStep, StageLauncher},
};
```

文件末尾追加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FakeLaunchKind {
    Agent,
    Checks,
}

#[derive(Debug, Clone)]
pub(crate) struct FakeLaunch {
    pub kind: FakeLaunchKind,
    pub session_id: Uuid,
    pub execution_process_id: Uuid,
    /// 智能体是提示词，检查是脚本。
    pub prompt: String,
    pub run_setup: bool,
}

/// 假启动器：不起进程，只记账；智能体启动时按提示词写 qa-mode 同款占位产出物。
pub(crate) struct FakeLauncher {
    pool: SqlitePool,
    root: PathBuf,
    launches: Mutex<Vec<FakeLaunch>>,
    pub(crate) stopped: Mutex<Vec<Uuid>>,
    /// 关掉后智能体「什么都不产出」，用来测缺产出物。
    pub(crate) write_artifacts: AtomicBool,
    /// 打开后所有启动都失败。
    pub(crate) fail_start: AtomicBool,
}

impl FakeLauncher {
    pub(crate) fn new(pool: SqlitePool, root: PathBuf) -> Self {
        Self {
            pool,
            root,
            launches: Mutex::new(Vec::new()),
            stopped: Mutex::new(Vec::new()),
            write_artifacts: AtomicBool::new(true),
            fail_start: AtomicBool::new(false),
        }
    }

    pub(crate) fn launches(&self) -> Vec<FakeLaunch> {
        self.launches.lock().unwrap().clone()
    }

    pub(crate) fn last(&self) -> FakeLaunch {
        self.launches().last().cloned().expect("还没有任何启动")
    }

    fn record(&self, launch: FakeLaunch) -> LaunchedStep {
        let step = LaunchedStep {
            session_id: launch.session_id,
            execution_process_id: launch.execution_process_id,
        };
        self.launches.lock().unwrap().push(launch);
        step
    }
}

#[async_trait]
impl StageLauncher for FakeLauncher {
    async fn prepare_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, PipelineError> {
        if self.fail_start.load(Ordering::SeqCst) {
            return Err(PipelineError::Launch("模拟启动失败".to_string()));
        }
        std::fs::create_dir_all(&self.root)?;
        Workspace::update_container_ref(&self.pool, workspace_id, &self.root.to_string_lossy())
            .await?;
        Ok(self.root.clone())
    }

    async fn start_agent_session(
        &self,
        _workspace_id: Uuid,
        _executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
    ) -> Result<LaunchedStep, PipelineError> {
        if self.write_artifacts.load(Ordering::SeqCst)
            && let Some(parsed) = parse_pipeline_prompt(&prompt)
        {
            write_mock_artifacts(&parsed)?;
        }
        Ok(self.record(FakeLaunch {
            kind: FakeLaunchKind::Agent,
            session_id: Uuid::new_v4(),
            execution_process_id: Uuid::new_v4(),
            prompt,
            run_setup,
        }))
    }

    async fn start_checks(
        &self,
        _workspace_id: Uuid,
        session_id: Uuid,
        script: String,
    ) -> Result<LaunchedStep, PipelineError> {
        Ok(self.record(FakeLaunch {
            kind: FakeLaunchKind::Checks,
            session_id,
            execution_process_id: Uuid::new_v4(),
            prompt: script,
            run_setup: false,
        }))
    }

    async fn stop_workspace(&self, workspace_id: Uuid) {
        self.stopped.lock().unwrap().push(workspace_id);
    }
}
```

- [ ] **Step 2: 写测试**

把 `tests.rs` 整个替换为：

```rust
//! 引擎集成测试：真 SQLite + 真引擎 + qa-mode 同款产出物，假启动器代替容器。

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use api_types::issue::Issue;
use db::{
    models::{
        execution_process::{ExecutionProcessRunReason, ExecutionProcessStatus},
        issue::Issues,
        local_project::DEFAULT_USER_ID,
        local_project_status::{ProjectStatuses, StageType},
        pipeline::{
            ArtifactKind, GateDecisionKind, GateDecisionRequest, IssuePipelineView,
            PipelineRunStatus, PipelineRuns, PipelineStageKey, PipelineStageStatus,
        },
    },
    test_support::TestDb,
};
use executors::{executors::BaseCodingAgent, profile::ExecutorConfig};
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    FinishedProcess, PipelineError, PipelineService, StartPipelineInput,
    test_support::{FakeLaunch, FakeLaunchKind, FakeLauncher, 准备工作区, 准备需求},
    transition::stage_column,
};

use PipelineStageKey::{Deliver, Develop, Requirement, Review, Spec, Test, TestDesign};
use PipelineStageStatus::{Failed, Passed, Pending, Rejected, Running, Skipped, WaitingGate};

struct 场景 {
    test_db: TestDb,
    dir: TempDir,
    launcher: Arc<FakeLauncher>,
    service: PipelineService,
    issue: Issue,
    workspace_id: Uuid,
}

impl 场景 {
    async fn 新建(title: &str) -> Self {
        let test_db = TestDb::new().await;
        let dir = tempfile::tempdir().unwrap();
        let issue = 准备需求(&test_db, title).await;
        let workspace_id = 准备工作区(&test_db).await;
        let launcher = Arc::new(FakeLauncher::new(
            test_db.pool().clone(),
            dir.path().join("workspace"),
        ));
        let service = PipelineService::new(test_db.db.clone(), launcher.clone());
        Self {
            test_db,
            dir,
            launcher,
            service,
            issue,
            workspace_id,
        }
    }

    fn 仓库目录(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn 写仓库模板(&self, yaml: &str) {
        std::fs::create_dir_all(self.仓库目录().join(".vibe")).unwrap();
        std::fs::write(self.仓库目录().join(".vibe/pipeline.yaml"), yaml).unwrap();
    }

    async fn 启动(&self) -> Result<IssuePipelineView, PipelineError> {
        self.service
            .start(StartPipelineInput {
                issue: self.issue.clone(),
                workspace_id: self.workspace_id,
                repo_root: self.仓库目录(),
                executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
                template_key: None,
            })
            .await
    }

    async fn 视图(&self) -> IssuePipelineView {
        self.service
            .view_for_issue(self.issue.id)
            .await
            .unwrap()
            .expect("应有运行")
    }

    async fn 当前(&self) -> (PipelineStageKey, PipelineStageStatus) {
        let view = self.视图().await;
        let last = view.stages.last().expect("至少一条尝试");
        (last.stage_key, last.status)
    }

    async fn 结束(&self, launch: &FakeLaunch, ok: bool) {
        self.service
            .on_process_finished(FinishedProcess {
                execution_process_id: launch.execution_process_id,
                session_id: launch.session_id,
                run_reason: match launch.kind {
                    FakeLaunchKind::Agent => ExecutionProcessRunReason::CodingAgent,
                    FakeLaunchKind::Checks => ExecutionProcessRunReason::PipelineStep,
                },
                status: if ok {
                    ExecutionProcessStatus::Completed
                } else {
                    ExecutionProcessStatus::Failed
                },
                exit_code: Some(if ok { 0 } else { 1 }),
                has_next_action: false,
                chain_continues: false,
            })
            .await
            .unwrap();
    }

    async fn 结束最近一次启动(&self, ok: bool) {
        let launch = self.launcher.last();
        self.结束(&launch, ok).await;
    }

    async fn 等待确认的阶段(&self) -> Uuid {
        self.视图()
            .await
            .stages
            .iter()
            .rev()
            .find(|stage| stage.status == WaitingGate)
            .expect("应有等待确认的阶段")
            .id
    }

    async fn 通过关卡(&self) {
        let id = self.等待确认的阶段().await;
        self.service
            .decide_gate(
                id,
                GateDecisionRequest {
                    decision: GateDecisionKind::Approve,
                    comment: None,
                },
                Some(DEFAULT_USER_ID),
            )
            .await
            .unwrap();
    }

    async fn 推进到(&self, target: PipelineStageKey) {
        for _ in 0..30 {
            match self.当前().await {
                (key, Running) if key == target => return,
                (_, Running) => self.结束最近一次启动(true).await,
                (_, WaitingGate) => self.通过关卡().await,
                other => panic!("推进途中遇到 {other:?}"),
            }
        }
        panic!("30 步内没有走到 {target:?}");
    }

    async fn 需求所在列(&self) -> StageType {
        let pool = self.test_db.pool();
        let issue = Issues::find_by_id(pool, self.issue.id).await.unwrap().unwrap();
        for column in [
            StageType::Backlog,
            StageType::Todo,
            StageType::Dev,
            StageType::Review,
            StageType::Test,
            StageType::Done,
        ] {
            let status = ProjectStatuses::find_stage(pool, issue.project_id, column)
                .await
                .unwrap()
                .unwrap();
            if status.id == issue.status_id {
                return column;
            }
        }
        panic!("需求不在任何阶段列里");
    }

    fn 尝试(view: &IssuePipelineView, key: PipelineStageKey) -> Vec<(i64, PipelineStageStatus)> {
        view.stages
            .iter()
            .filter(|stage| stage.stage_key == key)
            .map(|stage| (stage.attempt, stage.status))
            .collect()
    }
}

#[tokio::test]
async fn 黄金路径七个阶段依次跑完() {
    let s = 场景::新建("用户可以导出报表").await;
    let view = s.启动().await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(view.template.key, "standard");
    assert_eq!(view.template.stages.len(), 7);

    for key in PipelineStageKey::ALL {
        assert_eq!(s.当前().await, (key, Running), "应轮到 {key:?}");
        assert_eq!(s.需求所在列().await, stage_column(key), "{key:?} 阶段需求应在对应列");
        s.结束最近一次启动(true).await;
        if matches!(key, Requirement | Spec | TestDesign) {
            assert_eq!(s.当前().await, (key, WaitingGate));
            assert_eq!(s.视图().await.run.status, PipelineRunStatus::WaitingGate);
            let pending = PipelineRuns::list_pending(s.test_db.pool(), None).await.unwrap();
            assert_eq!(pending.len(), 1, "等人工时出现在待处理列表");
            assert_eq!(pending[0].stage_run.stage_key, key);
            s.通过关卡().await;
        }
    }

    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Completed);
    assert!(view.run.finished_at.is_some());
    assert_eq!(view.stages.len(), 7);
    assert!(
        view.stages
            .iter()
            .all(|stage| stage.status == Passed && stage.attempt == 1)
    );
    assert_eq!(view.decisions.len(), 3);
    assert!(
        view.decisions
            .iter()
            .all(|decision| decision.decision == GateDecisionKind::Approve)
    );
    for kind in [
        ArtifactKind::Requirement,
        ArtifactKind::Spec,
        ArtifactKind::Plan,
        ArtifactKind::TestCases,
        ArtifactKind::TraceMatrix,
        ArtifactKind::Review,
        ArtifactKind::TestReport,
        ArtifactKind::DeliveryReport,
    ] {
        assert!(
            view.artifacts.iter().any(|artifact| artifact.kind == kind),
            "缺产出物 {kind:?}"
        );
    }
    assert_eq!(s.需求所在列().await, StageType::Done);
    assert!(
        PipelineRuns::list_pending(s.test_db.pool(), None)
            .await
            .unwrap()
            .is_empty()
    );

    let launches = s.launcher.launches();
    assert_eq!(launches.len(), 7, "每个阶段开一个新会话");
    assert!(launches[0].run_setup, "首个会话跑 setup 脚本");
    assert!(launches[1..].iter().all(|l| !l.run_setup), "后续会话不跑 setup");
    assert!(launches[0].prompt.contains(&format!(
        "需求：{} 用户可以导出报表",
        s.issue.simple_id
    )));
    assert!(launches[0].prompt.contains("必须产出：requirement.md"));
    assert!(launches[1].prompt.contains("上一阶段产出：requirement.md"));
    assert!(launches[3].prompt.contains("必须产出：无"), "开发阶段不产出文件");
}

#[tokio::test]
async fn 人工打回后同阶段重跑且意见进提示词() {
    let s = 场景::新建("打回").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    let view = s
        .service
        .decide_gate(
            waiting,
            GateDecisionRequest {
                decision: GateDecisionKind::Reject,
                comment: Some("  补充验收标准 ".to_string()),
            },
            Some(DEFAULT_USER_ID),
        )
        .await
        .unwrap();
    assert_eq!(场景::尝试(&view, Requirement), vec![(1, Rejected), (2, Running)]);
    assert_eq!(view.stages[0].error.as_deref(), Some("补充验收标准"));
    assert_eq!(view.decisions[0].comment.as_deref(), Some("补充验收标准"));
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert!(
        s.launcher
            .last()
            .prompt
            .contains("上一次被打回的意见：补充验收标准")
    );
}

#[tokio::test]
async fn 关卡决策的错误映射() {
    let s = 场景::新建("决策错误").await;
    let view = s.启动().await.unwrap();
    let running = view.stages[0].id;
    let approve = GateDecisionRequest {
        decision: GateDecisionKind::Approve,
        comment: None,
    };
    assert!(matches!(
        s.service.decide_gate(running, approve.clone(), None).await,
        Err(PipelineError::Conflict(_))
    ));
    s.结束最近一次启动(true).await;
    let waiting = s.等待确认的阶段().await;
    assert!(matches!(
        s.service
            .decide_gate(
                waiting,
                GateDecisionRequest {
                    decision: GateDecisionKind::Reject,
                    comment: Some("   ".to_string()),
                },
                None,
            )
            .await,
        Err(PipelineError::BadRequest(_))
    ));
    assert!(matches!(
        s.service.decide_gate(Uuid::new_v4(), approve, None).await,
        Err(PipelineError::NotFound(_))
    ));
}

#[tokio::test]
async fn 评审阻断一次后重跑通过() {
    let s = 场景::新建("评审 [qa:review-blocker-once]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    s.结束最近一次启动(true).await;

    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Review), vec![(1, Failed), (2, Running)]);
    let first = view.stages.iter().find(|x| x.stage_key == Review).unwrap();
    assert!(first.error.as_deref().unwrap().contains("评审发现 1 个阻断项"));
    assert!(
        s.launcher
            .last()
            .prompt
            .contains("上一次被打回的意见：评审发现 1 个阻断项")
    );
    assert_eq!(s.需求所在列().await, StageType::Review);

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Test, Running));
}

#[tokio::test]
async fn 评审始终阻断时用尽轮次转人工() {
    let s = 场景::新建("评审 [qa:always-fail-review]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    for _ in 0..3 {
        s.结束最近一次启动(true).await;
    }
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(
        场景::尝试(&view, Review),
        vec![(1, Failed), (2, Failed), (3, Failed)]
    );
    assert!(
        view.stages
            .last()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("已累计失败 3 次")
    );
    let pending = PipelineRuns::list_pending(s.test_db.pool(), None).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stage_run.stage_key, Review);
    assert_eq!(pending[0].stage_run.status, Failed);
}

#[tokio::test]
async fn 失败的运行可以继续再给一次机会() {
    let s = 场景::新建("评审 [qa:always-fail-review]").await;
    s.启动().await.unwrap();
    s.推进到(Review).await;
    for _ in 0..3 {
        s.结束最近一次启动(true).await;
    }
    let launches_before = s.launcher.launches().len();
    let view = s.service.resume(s.视图().await.run.id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(场景::尝试(&view, Review).last(), Some(&(4, Running)));
    assert_eq!(s.launcher.launches().len(), launches_before + 1);
    assert!(s.launcher.last().prompt.contains("已累计失败 3 次"));
}

#[tokio::test]
async fn 测试归因代码缺陷回到开发() {
    let s = 场景::新建("测试 [qa:test-fail-code-once]").await;
    s.启动().await.unwrap();
    s.推进到(Test).await;
    s.结束最近一次启动(true).await;

    assert_eq!(s.当前().await, (Develop, Running));
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Develop), vec![(1, Passed), (2, Running)]);
    assert_eq!(s.需求所在列().await, StageType::Dev);
    assert!(s.launcher.last().prompt.contains("测试失败 1 条"));

    s.推进到(Deliver).await;
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Test), vec![(1, Failed), (2, Passed)]);
    assert_eq!(场景::尝试(&view, Review), vec![(1, Passed), (2, Passed)]);
}

#[tokio::test]
async fn 测试归因用例问题回到用例设计并重新走人工关卡() {
    let s = 场景::新建("测试 [qa:test-fail-case-once]").await;
    s.启动().await.unwrap();
    s.推进到(Test).await;
    s.结束最近一次启动(true).await;

    assert_eq!(s.当前().await, (TestDesign, Running));
    assert_eq!(s.需求所在列().await, StageType::Todo);
    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (TestDesign, WaitingGate), "改过的用例要人重新确认");
    assert_eq!(s.视图().await.run.status, PipelineRunStatus::WaitingGate);
}

#[tokio::test]
async fn 智能体进程失败按轮次重跑并回喂原因() {
    let s = 场景::新建("进程失败").await;
    s.启动().await.unwrap();
    s.结束最近一次启动(false).await;
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Requirement), vec![(1, Failed), (2, Running)]);
    assert!(
        view.stages[0]
            .error
            .as_deref()
            .unwrap()
            .contains("编码智能体 进程未成功结束")
    );
}

#[tokio::test]
async fn 缺产出物按阶段失败处理() {
    let s = 场景::新建("缺产出物").await;
    s.launcher.write_artifacts.store(false, Ordering::SeqCst);
    s.启动().await.unwrap();
    s.结束最近一次启动(true).await;
    let view = s.视图().await;
    assert_eq!(view.stages[0].status, Failed);
    assert_eq!(
        view.stages[0].error.as_deref(),
        Some("技能没有产出 requirement.md")
    );
    assert_eq!(s.当前().await, (Requirement, Running));
}

#[tokio::test]
async fn 暂停后阶段结束不再调度_继续后启动() {
    let s = 场景::新建("暂停").await;
    let run_id = s.启动().await.unwrap().run.id;
    let view = s.service.pause(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    assert!(matches!(
        s.service.pause(run_id).await,
        Err(PipelineError::Conflict(_))
    ));

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Requirement, WaitingGate));
    assert_eq!(s.视图().await.run.status, PipelineRunStatus::Paused, "暂停中保持 paused");

    s.通过关卡().await;
    let view = s.视图().await;
    assert_eq!(s.当前().await, (Spec, Pending));
    assert_eq!(view.run.current_stage_key, Spec);
    assert_eq!(view.run.status, PipelineRunStatus::Paused);
    assert_eq!(s.launcher.launches().len(), 1, "暂停中不启动新阶段");

    let view = s.service.resume(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Running);
    assert_eq!(s.当前().await, (Spec, Running));
    assert_eq!(s.launcher.launches().len(), 2);
}

#[tokio::test]
async fn 取消后回调被忽略且可以重新启动() {
    let s = 场景::新建("取消").await;
    let run_id = s.启动().await.unwrap().run.id;
    let view = s.service.cancel(run_id).await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Cancelled);
    assert_eq!(view.stages[0].status, Skipped);
    assert_eq!(*s.launcher.stopped.lock().unwrap(), vec![s.workspace_id]);

    s.结束最近一次启动(true).await;
    let view = s.视图().await;
    assert_eq!(view.stages.len(), 1, "取消后的回调不再推进");
    assert!(matches!(
        s.service.cancel(run_id).await,
        Err(PipelineError::Conflict(_))
    ));
    assert!(matches!(
        s.service.resume(run_id).await,
        Err(PipelineError::Conflict(_))
    ));

    let again = s.启动().await.unwrap();
    assert_ne!(again.run.id, run_id, "取消后可以重新开一条");
}

#[tokio::test]
async fn 同一需求重复启动返回冲突() {
    let s = 场景::新建("重复").await;
    s.启动().await.unwrap();
    assert!(matches!(s.启动().await, Err(PipelineError::Conflict(_))));
}

#[tokio::test]
async fn 未知模板返回请求无效() {
    let s = 场景::新建("模板").await;
    let result = s
        .service
        .start(StartPipelineInput {
            issue: s.issue.clone(),
            workspace_id: s.workspace_id,
            repo_root: s.仓库目录(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            template_key: Some("fancy".to_string()),
        })
        .await;
    assert!(matches!(result, Err(PipelineError::BadRequest(_))));
}

#[tokio::test]
async fn 启动失败记为阶段失败且修好后可以继续() {
    let s = 场景::新建("启动失败").await;
    s.launcher.fail_start.store(true, Ordering::SeqCst);
    let view = s.启动().await.unwrap();
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(view.stages[0].status, Failed);
    assert!(view.stages[0].error.as_deref().unwrap().contains("启动执行失败"));

    s.launcher.fail_start.store(false, Ordering::SeqCst);
    let view = s.service.resume(view.run.id).await.unwrap();
    assert_eq!(场景::尝试(&view, Requirement), vec![(1, Failed), (2, Running)]);
}

#[tokio::test]
async fn 服务重启后中断的阶段记失败且旧回调被忽略() {
    let s = 场景::新建("重启").await;
    s.启动().await.unwrap();
    assert_eq!(s.service.recover_interrupted().await.unwrap(), 1);
    let view = s.视图().await;
    assert_eq!(view.run.status, PipelineRunStatus::Failed);
    assert_eq!(view.stages[0].error.as_deref(), Some("服务重启，阶段执行被中断"));

    s.结束最近一次启动(true).await;
    assert_eq!(s.当前().await, (Requirement, Failed));
}

#[tokio::test]
async fn 非终点事件与无关会话被忽略() {
    let s = 场景::新建("忽略").await;
    s.启动().await.unwrap();
    let launch = s.launcher.last();
    s.service
        .on_process_finished(FinishedProcess {
            execution_process_id: launch.execution_process_id,
            session_id: launch.session_id,
            run_reason: ExecutionProcessRunReason::CodingAgent,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action: true,
            chain_continues: true,
        })
        .await
        .unwrap();
    s.service
        .on_process_finished(FinishedProcess {
            execution_process_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_reason: ExecutionProcessRunReason::CodingAgent,
            status: ExecutionProcessStatus::Completed,
            exit_code: Some(0),
            has_next_action: false,
            chain_continues: false,
        })
        .await
        .unwrap();
    assert_eq!(s.当前().await, (Requirement, Running));
}

#[tokio::test]
async fn 开发阶段有检查命令时先跑检查再判关卡() {
    let s = 场景::新建("检查").await;
    s.写仓库模板(
        "version: 1\nstages:\n  - key: develop\n    skill: vk-develop\n    checks: [\"cargo test\"]\n    gate: { auto: checks_passed }\n  - key: review\n    skill: vk-review\n    artifacts: [review.json]\n    gate: { auto: no_blocking_findings }\n",
    );
    let view = s.启动().await.unwrap();
    assert_eq!(view.template.key, "repo");
    assert_eq!(s.需求所在列().await, StageType::Dev);

    let agent = s.launcher.last();
    s.结束(&agent, true).await;
    let checks = s.launcher.last();
    assert_eq!(checks.kind, FakeLaunchKind::Checks);
    assert_eq!(checks.session_id, agent.session_id, "检查脚本在同一会话里跑");
    assert_eq!(checks.prompt, "set -e\ncargo test\n");
    assert_eq!(s.当前().await, (Develop, Running));
    assert_eq!(
        s.视图().await.stages[0].execution_process_id,
        Some(checks.execution_process_id)
    );

    s.结束(&checks, false).await;
    let view = s.视图().await;
    assert_eq!(场景::尝试(&view, Develop), vec![(1, Failed), (2, Running)]);
    assert!(
        view.stages[0]
            .error
            .as_deref()
            .unwrap()
            .contains("检查脚本 进程未成功结束")
    );

    s.结束最近一次启动(true).await;
    let checks = s.launcher.last();
    assert_eq!(checks.kind, FakeLaunchKind::Checks);
    s.结束(&checks, true).await;
    assert_eq!(s.当前().await, (Review, Running));
}

#[tokio::test]
async fn 仓库模板损坏时回落内置模板并记警告() {
    let s = 场景::新建("坏模板").await;
    s.写仓库模板("version: 9\nstages: []\n");
    let view = s.启动().await.unwrap();
    assert_eq!(view.template.key, "standard");
    let internals = PipelineRuns::internals(s.test_db.pool(), view.run.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        internals
            .template_warning
            .as_deref()
            .unwrap()
            .contains(".vibe/pipeline.yaml")
    );
}
```

- [ ] **Step 3: 运行，确认全部通过**

Run: `cargo test -p services pipeline::tests`
Expected: `test result: ok. 19 passed; 0 failed`

若有失败，按 `superpowers:systematic-debugging` 排查：先看失败断言对应 §1.3 行为总表的哪一行，再对照 `engine.rs` 的 `apply` / `enter_stage`；**不要改测试去迁就实现**，除非能证明测试与 §1.3 矛盾（那种情况同时改 §1.3）。

- [ ] **Step 4: 故意破坏一次，确认测试能抓住**

把 `engine.rs` 里 `apply` 的 `Transition::WaitHuman` 分支中 `if run.status != PipelineRunStatus::Paused` 临时改成 `if true`。

Run: `cargo test -p services pipeline::tests::暂停后阶段结束不再调度_继续后启动`
Expected: FAIL，`暂停中保持 paused`。改回原样后重跑通过。

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/
git commit -m "$(cat <<'EOF'
流水线：引擎集成测试覆盖七阶段黄金路径与打回、回流、轮次用尽、暂停、取消、重启恢复

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 17: 退出钩子与 deployment 接线

**Files:**
- Modify: `crates/local-deployment/src/container.rs`：`:1-7`（`std` 导入）、`:42-53`（services 导入）、`:69-88`（结构体字段）、`:92-133`（`new`）、`:549-812`（`spawn_exit_monitor` 内部）
- Modify: `crates/deployment/src/lib.rs:16-31`（导入）、`:105` 附近（trait 方法）
- Modify: `crates/local-deployment/src/lib.rs:18-35`（导入）、`:56-87`（字段）、`:282-296` 之后（装配）、`:328-357`（构造）、`:398-400` 附近（trait 实现）

**测试说明：** 这三处都是装配代码，仓库里没有能构造 `LocalContainerService` / `LocalDeployment` 的夹具（§2.4 最后一条）。引擎一侧的回调语义已在任务 9（`is_stage_terminal`）与任务 16 覆盖；这里用编译、clippy 与既有测试兜底，真实链路由任务 20 的人工冒烟与计划 B 的 e2e 验证。

- [ ] **Step 1: 容器——字段、构造、设置方法**

`crates/local-deployment/src/container.rs:1-7` 的 `std` 导入里把 `sync::Arc,` 改为 `sync::{Arc, OnceLock},`。

`:42-53` 的 `use services::services::{ ... }` 里，在 `notification::NotificationService,` 之后加：

```rust
    pipeline::PipelineExitEvent,
```

结构体 `LocalContainerService`（:69-88）在 `remote_client: Option<RemoteClient>,` 之后加字段：

```rust
    /// 流水线引擎的退出通知通道；部署装配时设置一次（`set_pipeline_exit_notifier`）。
    pipeline_exit_tx: Arc<OnceLock<tokio::sync::mpsc::UnboundedSender<PipelineExitEvent>>>,
```

`new()` 里构造结构体处（:111-128），在 `remote_client,` 之后加：

```rust
            pipeline_exit_tx: Arc::new(OnceLock::new()),
```

在 `new()` 函数之后（`fn map_workspace_manager_error` 之前）加方法：

```rust
    /// 接上流水线引擎：之后每个执行进程收尾都会发一条 [`PipelineExitEvent`]。
    /// 只能设置一次；重复设置记警告并忽略。
    pub fn set_pipeline_exit_notifier(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<PipelineExitEvent>,
    ) {
        if self.pipeline_exit_tx.set(tx).is_err() {
            tracing::warn!("流水线退出通知已经接过，忽略重复设置");
        }
    }
```

- [ ] **Step 2: 容器——退出钩子记录「链是否继续」并通知引擎**

在 `spawn_exit_monitor` 的 async 块里，`if let Ok(ctx) = ExecutionProcess::load_context(&db.pool, exec_id).await {`（:549）**之前**加：

```rust
            // 这个进程收尾时是否接着起了同会话的下一个进程（next_action 或排队续聊）。
            // 流水线引擎靠它判断「阶段链是否走完」，见 services::pipeline::transition::is_stage_terminal。
            let mut chain_continues = false;
```

把 `:595-600` 的：

```rust
                    if should_start_next {
                        // If the process exited successfully, start the next action
                        if let Err(e) = container.try_start_next_action(&ctx).await {
                            tracing::error!("Failed to start next action after completion: {}", e);
                        }
                    } else {
```

改为：

```rust
                    if should_start_next {
                        let has_next_action = ctx
                            .execution_process
                            .executor_action()
                            .ok()
                            .and_then(|action| action.next_action())
                            .is_some();
                        // If the process exited successfully, start the next action
                        match container.try_start_next_action(&ctx).await {
                            Ok(()) => chain_continues = has_next_action,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to start next action after completion: {}",
                                    e
                                );
                            }
                        }
                    } else {
```

把 `:660` 的 `started_queued_follow_up = true;` 改为：

```rust
                                started_queued_follow_up = true;
                                chain_continues = true;
```

把 `:729-737`（并行 setup 结束后消费排队消息）的：

```rust
                        if let Err(e) = container
                            .start_queued_follow_up(&ctx, &queued_msg.data)
                            .await
                        {
                            tracing::error!(
                                "Failed to start queued follow-up from setup script completion: {}",
                                e
                            );
                        }
```

改为：

```rust
                        match container
                            .start_queued_follow_up(&ctx, &queued_msg.data)
                            .await
                        {
                            Ok(_) => chain_continues = true,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to start queued follow-up from setup script completion: {}",
                                    e
                                );
                            }
                        }
```

在 async 块最后一行 `child_store.write().await.remove(&exec_id);`（:811）之后加：

```rust

            // 通知流水线引擎（设计 §6.2 第 4 步）。放在最后：提交、next_action、
            // HEAD 记录、日志落盘都已完成，引擎读到的是最终状态。
            if let Some(tx) = container.pipeline_exit_tx.get() {
                let _ = tx.send(PipelineExitEvent {
                    execution_process_id: exec_id,
                    chain_continues,
                });
            }
```

Run: `cargo check -p local-deployment`
Expected: `Finished`，无 error。（`crates/local-deployment/Cargo.toml` 没有 `[features]`，已核实，所以这里不带 `--features qa-mode`；qa-mode 组合在 Step 5 用 `--workspace` 检查。）

- [ ] **Step 3: Deployment trait 加 `pipeline()`**

`crates/deployment/src/lib.rs` 的 `use services::services::{ ... }` 里，在 `local_auth::runtime::LocalAuthRuntime,` 之后加：

```rust
    pipeline::PipelineService,
```

trait 里 `fn events(&self) -> &EventService;` 之后加：

```rust
    /// 交付流水线引擎（设计 §6）。
    fn pipeline(&self) -> &PipelineService;
```

- [ ] **Step 4: LocalDeployment 装配**

`crates/local-deployment/src/lib.rs` 的 `use services::services::{ ... }` 里，在 `oauth_handoff::{...},` 之后加：

```rust
    pipeline::{ContainerStageLauncher, PipelineExitEvent, PipelineService},
```

结构体 `LocalDeployment` 在 `events: EventService,` 之后加字段：

```rust
    pipeline: PipelineService,
```

在 `let container = LocalContainerService::new( ... ).await;`（:283-296）之后插入：

```rust
        // 交付流水线：容器在每个执行进程收尾后发退出事件，引擎在后台逐条处理。
        let (pipeline_exit_tx, mut pipeline_exit_rx) =
            tokio::sync::mpsc::unbounded_channel::<PipelineExitEvent>();
        container.set_pipeline_exit_notifier(pipeline_exit_tx);
        let pipeline = PipelineService::new(
            db.clone(),
            Arc::new(ContainerStageLauncher::new(container.clone())),
        );
        match pipeline.recover_interrupted().await {
            Ok(0) => {}
            Ok(n) => tracing::warn!("流水线：{n} 个阶段因服务重启被中断，已转人工处理"),
            Err(e) => tracing::warn!("流水线：恢复中断阶段失败: {e}"),
        }
        {
            let pipeline = pipeline.clone();
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        event = pipeline_exit_rx.recv() => match event {
                            Some(event) => pipeline.handle_exit_event(event).await,
                            None => break,
                        },
                        _ = shutdown.cancelled() => break,
                    }
                }
                tracing::debug!("流水线退出事件处理任务已退出");
            });
        }
```

构造 `let deployment = Self { ... }`（:328）里 `events,` 之后加 `pipeline,`。

`impl Deployment for LocalDeployment` 里 `fn events(&self) -> &EventService { &self.events }` 之后加：

```rust
    fn pipeline(&self) -> &PipelineService {
        &self.pipeline
    }
```

- [ ] **Step 5: 编译、lint、既有测试**

Run: `cargo check --workspace --features qa-mode`
Expected: `Finished`，无 error。

Run: `cargo clippy --workspace --all-targets --features qa-mode -- -D warnings`
Expected: 无 warning（与 `package.json:31` 的 `backend:lint` 同一条命令；`local-deployment`、`deployment`、`db` 都没有 `[features]`，不能对它们单独带 `--features qa-mode`）。

Run: `cargo test -p local-deployment -p services`
Expected: 全部通过。

- [ ] **Step 6: 提交**

```bash
git add crates/local-deployment/src/container.rs crates/local-deployment/src/lib.rs crates/deployment/src/lib.rs
git commit -m "$(cat <<'EOF'
接线：执行进程收尾时通知流水线引擎，PipelineService 挂到 Deployment

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 18: 路由与 `create.rs` 抽取

**Files:**
- Modify: `crates/server/src/routes/workspaces/create.rs:215-248`（抽出 `create_workspace_with_repos`）
- Create: `crates/server/src/routes/local_projects/pipeline.rs`
- Modify: `crates/server/src/routes/local_projects/issues.rs:181-192`（`router()` 登记 `/{id}/pipeline`）
- Modify: `crates/server/src/routes/local_projects/mod.rs:5-9`（`pub mod pipeline;`）、`:141-151`（`.merge(pipeline::router())`）

**路由落点说明（契约修正 C4）：** `/api/local/issues/{id}/pipeline` 挂进 `issues.rs` 已有的 `LocalRoutes::new("/issues")`，而不是在 `pipeline.rs` 里再 `nest("/issues")`——同一前缀 nest 两次、以及同一位置参数名不同（`{id}` 对 `{issue_id}`）在 axum 0.8 / matchit 下有冲突风险，仓库里也没有先例。handler 本身仍写在 `pipeline.rs`。

- [ ] **Step 1: 抽出 `create_workspace_with_repos`**

在 `crates/server/src/routes/workspaces/create.rs` 顶部 `use` 区追加：

```rust
use db::models::requests::WorkspaceRepoInput;
use workspace_manager::ManagedWorkspace;
```

在 `create_workspace_record` 之后加：

```rust
/// 建工作区记录并挂上仓库（不建 worktree、不启动执行）。
/// `create_and_start_workspace` 与流水线启动接口共用。
pub(crate) async fn create_workspace_with_repos(
    deployment: &DeploymentImpl,
    name: Option<String>,
    repos: &[WorkspaceRepoInput],
    created_by_user_id: Uuid,
) -> Result<ManagedWorkspace, ApiError> {
    let mut managed_workspace = deployment
        .workspace_manager()
        .load_managed_workspace(create_workspace_record(deployment, name, created_by_user_id).await?)
        .await?;

    for repo in repos {
        managed_workspace
            .add_repository(repo, deployment.git())
            .await
            .map_err(ApiError::from)?;
    }

    Ok(managed_workspace)
}
```

把 `create_and_start_workspace` 里：

```rust
    let mut managed_workspace = deployment
        .workspace_manager()
        .load_managed_workspace(create_workspace_record(&deployment, name, current_user.id).await?)
        .await?;

    for repo in &repos {
        managed_workspace
            .add_repository(repo, deployment.git())
            .await
            .map_err(ApiError::from)?;
    }
```

替换为：

```rust
    let managed_workspace =
        create_workspace_with_repos(&deployment, name, &repos, current_user.id).await?;
```

Run: `cargo test -p server routes::workspaces::create`
Expected: 全部通过（行为不变）。

- [ ] **Step 2: 写失败测试**

新建 `crates/server/src/routes/local_projects/pipeline.rs`，先只放测试；`mod.rs` 的模块声明区加 `pub mod pipeline;`：

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use api_types::{
        issue::{CreateIssueRequest, Issue},
        project::CreateProjectRequest,
    };
    use db::{
        models::{
            issue::Issues,
            local_project::{DEFAULT_ORGANIZATION_ID, DEFAULT_USER_ID, LocalProjects},
            local_project_status::{ProjectStatuses, StageType},
            pipeline::{
                ArtifactKind, CreatePipelineRun, CreateStageRun, GateDecisionKind,
                GateDecisionRequest, GateKind, IssueArtifacts, PipelineRun, PipelineRunStatus,
                PipelineRuns, PipelineStageKey, PipelineStageRun, PipelineStageRuns,
                PipelineStageStatus, StartPipelineRepo, StartPipelineRequest,
            },
        },
        test_support::TestDb,
    };
    use executors::{executors::BaseCodingAgent, profile::ExecutorConfig};
    use services::services::pipeline::{NoopStageLauncher, PipelineService};
    use uuid::Uuid;

    use super::*;

    async fn 准备需求(test_db: &TestDb) -> Issue {
        let project = LocalProjects::create(
            test_db.pool(),
            &CreateProjectRequest {
                id: None,
                organization_id: DEFAULT_ORGANIZATION_ID,
                name: "流水线接口".to_string(),
                color: "#6366f1".to_string(),
            },
        )
        .await
        .unwrap();
        let backlog = ProjectStatuses::find_stage(test_db.pool(), project.id, StageType::Backlog)
            .await
            .unwrap()
            .unwrap();
        Issues::create(
            test_db.pool(),
            &CreateIssueRequest {
                id: None,
                project_id: project.id,
                status_id: backlog.id,
                title: "接口测试".to_string(),
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
            DEFAULT_USER_ID,
        )
        .await
        .unwrap()
    }

    /// 直接落一条运行 + 一条尝试（不经引擎，模板快照故意写 "{}"，引擎会退回内置模板）。
    async fn 准备运行(
        test_db: &TestDb,
        issue: &Issue,
        stage_status: PipelineStageStatus,
    ) -> (PipelineRun, PipelineStageRun) {
        let run = PipelineRuns::create(
            test_db.pool(),
            &CreatePipelineRun {
                issue_id: issue.id,
                project_id: issue.project_id,
                workspace_id: None,
                template_key: "standard".to_string(),
                template_version: 1,
                template_json: "{}".to_string(),
                template_warning: None,
                executor_config_json: "{}".to_string(),
                first_stage: PipelineStageKey::Requirement,
            },
        )
        .await
        .unwrap();
        let stage = PipelineStageRuns::create(
            test_db.pool(),
            &CreateStageRun {
                run_id: run.id,
                project_id: run.project_id,
                stage_key: PipelineStageKey::Requirement,
                gate_kind: GateKind::Human,
                status: PipelineStageStatus::Running,
                feedback: None,
            },
        )
        .await
        .unwrap();
        let stage = if stage_status == PipelineStageStatus::Running {
            stage
        } else {
            PipelineStageRuns::set_status(test_db.pool(), stage.id, stage_status, None, None)
                .await
                .unwrap()
        };
        (run, stage)
    }

    fn 引擎(test_db: &TestDb) -> PipelineService {
        PipelineService::new(test_db.db.clone(), Arc::new(NoopStageLauncher))
    }

    fn 启动请求(repos: Vec<StartPipelineRepo>) -> StartPipelineRequest {
        StartPipelineRequest {
            repos,
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            template_key: None,
        }
    }

    fn 一个仓库() -> Vec<StartPipelineRepo> {
        vec![StartPipelineRepo {
            repo_id: Uuid::new_v4(),
            target_branch: "main".to_string(),
        }]
    }

    #[tokio::test]
    async fn 启动前校验的三种错误() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;

        let missing = validate_start(test_db.pool(), Uuid::new_v4(), &启动请求(一个仓库())).await;
        assert!(matches!(missing, Err(ApiError::NotFound)));

        let no_repo = validate_start(test_db.pool(), issue.id, &启动请求(vec![])).await;
        assert!(matches!(no_repo, Err(ApiError::BadRequest(_))));

        assert_eq!(
            validate_start(test_db.pool(), issue.id, &启动请求(一个仓库()))
                .await
                .unwrap()
                .id,
            issue.id
        );

        准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let active = validate_start(test_db.pool(), issue.id, &启动请求(一个仓库())).await;
        assert!(matches!(active, Err(ApiError::Conflict(_))));
    }

    #[tokio::test]
    async fn 查询需求流水线() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;

        assert!(matches!(
            handle_get_issue_pipeline(test_db.pool(), &pipeline, Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
        assert!(
            handle_get_issue_pipeline(test_db.pool(), &pipeline, issue.id)
                .await
                .unwrap()
                .is_none()
        );

        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let view = handle_get_issue_pipeline(test_db.pool(), &pipeline, issue.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view.run.id, run.id);
        assert_eq!(view.stages.len(), 1);
        assert_eq!(view.template.stages.len(), 7, "模板快照损坏时退回内置模板");
    }

    #[tokio::test]
    async fn 关卡决策的错误码() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (_, running) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let approve = GateDecisionRequest {
            decision: GateDecisionKind::Approve,
            comment: None,
        };

        assert!(matches!(
            handle_gate(&pipeline, Uuid::new_v4(), approve.clone(), DEFAULT_USER_ID).await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            handle_gate(&pipeline, running.id, approve, DEFAULT_USER_ID).await,
            Err(ApiError::Conflict(_))
        ));

        PipelineStageRuns::set_status(
            test_db.pool(),
            running.id,
            PipelineStageStatus::WaitingGate,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            handle_gate(
                &pipeline,
                running.id,
                GateDecisionRequest {
                    decision: GateDecisionKind::Reject,
                    comment: None,
                },
                DEFAULT_USER_ID
            )
            .await,
            Err(ApiError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn 暂停中通过关卡只建待启动阶段() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (run, waiting) = 准备运行(&test_db, &issue, PipelineStageStatus::WaitingGate).await;
        PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::Paused, None)
            .await
            .unwrap();

        let view = handle_gate(
            &pipeline,
            waiting.id,
            GateDecisionRequest {
                decision: GateDecisionKind::Approve,
                comment: None,
            },
            DEFAULT_USER_ID,
        )
        .await
        .unwrap();
        assert_eq!(view.run.status, PipelineRunStatus::Paused);
        assert_eq!(view.run.current_stage_key, PipelineStageKey::Spec);
        assert_eq!(
            view.stages
                .iter()
                .map(|s| (s.stage_key, s.status))
                .collect::<Vec<_>>(),
            vec![
                (PipelineStageKey::Requirement, PipelineStageStatus::Passed),
                (PipelineStageKey::Spec, PipelineStageStatus::Pending),
            ]
        );
        assert_eq!(view.decisions[0].decided_by, Some(DEFAULT_USER_ID));
    }

    #[tokio::test]
    async fn 暂停继续取消的状态约束() {
        let test_db = TestDb::new().await;
        let pipeline = 引擎(&test_db);
        let issue = 准备需求(&test_db).await;
        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;

        assert!(matches!(
            handle_resume(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert_eq!(
            handle_pause(&pipeline, run.id).await.unwrap().run.status,
            PipelineRunStatus::Paused
        );
        assert!(matches!(
            handle_pause(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert_eq!(
            handle_resume(&pipeline, run.id).await.unwrap().run.status,
            PipelineRunStatus::Running
        );
        let cancelled = handle_cancel(&pipeline, run.id).await.unwrap();
        assert_eq!(cancelled.run.status, PipelineRunStatus::Cancelled);
        assert_eq!(cancelled.stages[0].status, PipelineStageStatus::Skipped);
        assert!(matches!(
            handle_cancel(&pipeline, run.id).await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            handle_pause(&pipeline, Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
    }

    #[tokio::test]
    async fn 待处理列表与两个快照接口() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;
        let (run, _) = 准备运行(&test_db, &issue, PipelineStageStatus::WaitingGate).await;
        PipelineRuns::update_status(test_db.pool(), run.id, PipelineRunStatus::WaitingGate, None)
            .await
            .unwrap();

        let all = handle_pending(test_db.pool(), None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].issue_title, "接口测试");
        assert!(
            handle_pending(test_db.pool(), Some(Uuid::new_v4()))
                .await
                .unwrap()
                .is_empty()
        );

        let runs = handle_list_runs(test_db.pool(), issue.project_id).await.unwrap();
        assert_eq!(runs.0["pipeline_runs"].as_array().unwrap().len(), 1);
        let stages = handle_list_stage_runs(test_db.pool(), issue.project_id)
            .await
            .unwrap();
        assert_eq!(stages.0["pipeline_stage_runs"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn 取单个产出物() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db).await;
        let (_, stage) = 准备运行(&test_db, &issue, PipelineStageStatus::Running).await;
        let summary = IssueArtifacts::insert_version(
            test_db.pool(),
            issue.id,
            stage.id,
            ArtifactKind::Requirement,
            ".vk/runs/X/requirement.md",
            "# 需求",
        )
        .await
        .unwrap();

        let artifact = handle_get_artifact(test_db.pool(), summary.id).await.unwrap();
        assert_eq!(artifact.content, "# 需求");
        assert!(matches!(
            handle_get_artifact(test_db.pool(), Uuid::new_v4()).await,
            Err(ApiError::NotFound)
        ));
    }

    #[test]
    fn 流水线端点都已登记且路由不冲突() {
        // 构造 router() 同时验证 axum 路由表没有冲突（冲突会直接 panic）。
        let _ = crate::routes::local_projects::router();
        let registered = crate::routes::local_projects::registered_endpoints();
        for (path, method) in [
            ("/api/local/issues/{id}/pipeline", "GET"),
            ("/api/local/issues/{id}/pipeline", "POST"),
            ("/api/local/pipeline/artifacts/{id}", "GET"),
            ("/api/local/pipeline/stage-runs/{id}/gate", "POST"),
            ("/api/local/pipeline/runs/{id}/pause", "POST"),
            ("/api/local/pipeline/runs/{id}/resume", "POST"),
            ("/api/local/pipeline/runs/{id}/cancel", "POST"),
            ("/api/local/pipeline/pending", "GET"),
            ("/api/local/pipeline_runs", "GET"),
            ("/api/local/pipeline_stage_runs", "GET"),
        ] {
            assert!(
                registered
                    .get(path)
                    .is_some_and(|methods| methods.contains(method)),
                "{method} {path} 未登记，已登记：{registered:#?}"
            );
        }
    }
}
```

- [ ] **Step 3: 运行，确认失败**

Run: `cargo test -p server routes::local_projects::pipeline`
Expected: 编译失败，`cannot find function \`validate_start\``等。

- [ ] **Step 4: 实现**

在 `pipeline.rs` 测试模块之前写入：

```rust
//! 交付流水线接口（契约 §2、§3），挂在 /api/local 下。
//!
//! `/issues/{id}/pipeline` 的两个 handler 在这里，但登记在 `issues.rs` 的路由表里
//! （与 `/issues/{id}` 共用一个 nest，见计划 A 任务 18 说明）。
//! §2 的接口统一包 `ApiResponse`；§3 的两个快照接口按本地集合约定返回 `{ "<表名>": [...] }`。

use api_types::issue::Issue;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use db::models::{
    issue::Issues,
    pipeline::{
        GateDecisionRequest, IssueArtifact, IssueArtifacts, IssuePipelineView,
        PendingPipelineItem, PipelineRuns, PipelineStageRuns, StartPipelineRequest,
    },
    repo::Repo,
    requests::WorkspaceRepoInput,
    workspace::Workspace,
};
use deployment::Deployment;
use serde::Deserialize;
use serde_json::Value;
use services::services::pipeline::{PipelineError, PipelineService, StartPipelineInput};
use sqlx::SqlitePool;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::{LocalRoutes, ProjectScopedQuery, snapshot};
use crate::{
    DeploymentImpl, error::ApiError, middleware::local_session::CurrentUser,
    routes::workspaces::create::create_workspace_with_repos,
};

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    pub project_id: Option<Uuid>,
}

pub(crate) fn map_pipeline_error(error: PipelineError) -> ApiError {
    match error {
        PipelineError::NotFound(_) => ApiError::NotFound,
        PipelineError::Conflict(message) => ApiError::Conflict(message),
        PipelineError::BadRequest(message) => ApiError::BadRequest(message),
        PipelineError::Database(error) => ApiError::Database(error),
        PipelineError::Io(error) => ApiError::Io(error),
        PipelineError::Launch(message) => ApiError::Conflict(message),
    }
}

// ---------------------------------------------------------------------------
// 可直接单测的 handler 主体
// ---------------------------------------------------------------------------

/// 启动前校验：404 需求不存在；409 已有未结束的运行；400 没选仓库。
pub(crate) async fn validate_start(
    pool: &SqlitePool,
    issue_id: Uuid,
    payload: &StartPipelineRequest,
) -> Result<Issue, ApiError> {
    let issue = Issues::find_by_id(pool, issue_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if PipelineRuns::has_active_for_issue(pool, issue_id).await? {
        return Err(ApiError::Conflict("该需求已有未结束的流水线".to_string()));
    }
    if payload.repos.is_empty() {
        return Err(ApiError::BadRequest("至少选择一个仓库".to_string()));
    }
    Ok(issue)
}

pub(crate) async fn handle_get_issue_pipeline(
    pool: &SqlitePool,
    pipeline: &PipelineService,
    issue_id: Uuid,
) -> Result<Option<IssuePipelineView>, ApiError> {
    if Issues::find_by_id(pool, issue_id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    pipeline
        .view_for_issue(issue_id)
        .await
        .map_err(map_pipeline_error)
}

pub(crate) async fn handle_get_artifact(
    pool: &SqlitePool,
    artifact_id: Uuid,
) -> Result<IssueArtifact, ApiError> {
    IssueArtifacts::find_by_id(pool, artifact_id)
        .await?
        .ok_or(ApiError::NotFound)
}

pub(crate) async fn handle_gate(
    pipeline: &PipelineService,
    stage_run_id: Uuid,
    payload: GateDecisionRequest,
    decided_by: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline
        .decide_gate(stage_run_id, payload, Some(decided_by))
        .await
        .map_err(map_pipeline_error)
}

pub(crate) async fn handle_pause(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.pause(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_resume(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.resume(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_cancel(
    pipeline: &PipelineService,
    run_id: Uuid,
) -> Result<IssuePipelineView, ApiError> {
    pipeline.cancel(run_id).await.map_err(map_pipeline_error)
}

pub(crate) async fn handle_pending(
    pool: &SqlitePool,
    project_id: Option<Uuid>,
) -> Result<Vec<PendingPipelineItem>, ApiError> {
    Ok(PipelineRuns::list_pending(pool, project_id).await?)
}

pub(crate) async fn handle_list_runs(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "pipeline_runs",
        PipelineRuns::list_by_project(pool, project_id).await?,
    ))
}

pub(crate) async fn handle_list_stage_runs(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<Json<Value>, ApiError> {
    Ok(snapshot(
        "pipeline_stage_runs",
        PipelineStageRuns::list_by_project(pool, project_id).await?,
    ))
}

// ---------------------------------------------------------------------------
// axum handler
// ---------------------------------------------------------------------------

pub(super) async fn get_issue_pipeline(
    State(deployment): State<DeploymentImpl>,
    Path(issue_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Option<IssuePipelineView>>>, ApiError> {
    let view =
        handle_get_issue_pipeline(&deployment.db().pool, deployment.pipeline(), issue_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

pub(super) async fn start_pipeline(
    State(deployment): State<DeploymentImpl>,
    current_user: CurrentUser,
    Path(issue_id): Path<Uuid>,
    Json(payload): Json<StartPipelineRequest>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let pool = &deployment.db().pool;
    let issue = validate_start(pool, issue_id, &payload).await?;
    let repos: Vec<WorkspaceRepoInput> = payload
        .repos
        .iter()
        .map(|repo| WorkspaceRepoInput {
            repo_id: repo.repo_id,
            target_branch: repo.target_branch.clone(),
        })
        .collect();
    let first_repo = Repo::find_by_id(pool, repos[0].repo_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let managed = create_workspace_with_repos(
        &deployment,
        Some(format!("{} {}", issue.simple_id, issue.title)),
        &repos,
        current_user.id,
    )
    .await?;
    Workspace::set_issue_id(pool, managed.workspace.id, Some(issue.id)).await?;

    let view = deployment
        .pipeline()
        .start(StartPipelineInput {
            issue,
            workspace_id: managed.workspace.id,
            repo_root: first_repo.path.clone(),
            executor_config: payload.executor_config,
            template_key: payload.template_key,
        })
        .await
        .map_err(map_pipeline_error)?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn get_artifact(
    State(deployment): State<DeploymentImpl>,
    Path(artifact_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssueArtifact>>, ApiError> {
    let artifact = handle_get_artifact(&deployment.db().pool, artifact_id).await?;
    Ok(ResponseJson(ApiResponse::success(artifact)))
}

async fn decide_gate(
    State(deployment): State<DeploymentImpl>,
    current_user: CurrentUser,
    Path(stage_run_id): Path<Uuid>,
    Json(payload): Json<GateDecisionRequest>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_gate(deployment.pipeline(), stage_run_id, payload, current_user.id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn pause(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_pause(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn resume(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_resume(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn cancel(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssuePipelineView>>, ApiError> {
    let view = handle_cancel(deployment.pipeline(), run_id).await?;
    Ok(ResponseJson(ApiResponse::success(view)))
}

async fn pending(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<PendingQuery>,
) -> Result<ResponseJson<ApiResponse<Vec<PendingPipelineItem>>>, ApiError> {
    let items = handle_pending(&deployment.db().pool, query.project_id).await?;
    Ok(ResponseJson(ApiResponse::success(items)))
}

async fn list_runs(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list_runs(&deployment.db().pool, query.project_id).await
}

async fn list_stage_runs(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedQuery>,
) -> Result<Json<Value>, ApiError> {
    handle_list_stage_runs(&deployment.db().pool, query.project_id).await
}

pub fn router() -> Router<DeploymentImpl> {
    let pipeline = LocalRoutes::new("/pipeline")
        .route("/artifacts/{id}", &["GET"], get(get_artifact))
        .route("/stage-runs/{id}/gate", &["POST"], post(decide_gate))
        .route("/runs/{id}/pause", &["POST"], post(pause))
        .route("/runs/{id}/resume", &["POST"], post(resume))
        .route("/runs/{id}/cancel", &["POST"], post(cancel))
        .route("/pending", &["GET"], get(pending))
        .into_router();
    let runs = LocalRoutes::new("/pipeline_runs")
        .route("/", &["GET"], get(list_runs))
        .into_router();
    let stage_runs = LocalRoutes::new("/pipeline_stage_runs")
        .route("/", &["GET"], get(list_stage_runs))
        .into_router();
    Router::new().merge(pipeline).merge(runs).merge(stage_runs)
}
```

`crates/server/src/routes/local_projects/issues.rs` 的 `router()`（:181-192）改为：

```rust
pub fn router() -> Router<DeploymentImpl> {
    LocalRoutes::new("/issues")
        .route("/", &["GET", "POST"], get(list).post(create))
        .route("/bulk", &["POST"], post(bulk_update))
        .route("/search", &["POST"], post(search))
        .route(
            "/{id}",
            &["GET", "PATCH", "DELETE"],
            get(get_one).patch(update).delete(delete),
        )
        // 流水线（契约 §2）。handler 在 pipeline.rs；与 /{id} 共用同一个 nest 与参数名。
        .route(
            "/{id}/pipeline",
            &["GET", "POST"],
            get(super::pipeline::get_issue_pipeline).post(super::pipeline::start_pipeline),
        )
        .into_router()
}
```

`crates/server/src/routes/local_projects/mod.rs` 的 `router()` 改为：

```rust
pub fn router() -> Router<DeploymentImpl> {
    Router::new().nest(
        "/local",
        Router::new()
            .merge(projects::router())
            .merge(statuses::router())
            .merge(issues::router())
            .merge(side::router())
            .merge(projections::router())
            .merge(pipeline::router()),
    )
}
```

- [ ] **Step 5: 运行，确认通过**

Run: `cargo test -p server routes::local_projects`
Expected: 新增 8 个测试与原有路由契约测试全部通过（`前端用到的本地端点都挂上了路由` 仍通过）。

- [ ] **Step 6: 提交**

```bash
git add crates/server/src/routes/
git commit -m "$(cat <<'EOF'
接口：流水线启动、查询、关卡、暂停继续取消、待处理与两个集合快照

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 19: 类型生成

**Files:**
- Modify: `crates/server/src/bin/generate_types.rs:188-189` 之后
- Regenerate: `shared/types.ts`

- [ ] **Step 1: 注册类型**

在 `crates/server/src/bin/generate_types.rs` 的 `db::models::requests::CreateAndStartWorkspaceResponse::decl(),` 之后插入：

```rust
        db::models::pipeline::PipelineStageKey::decl(),
        db::models::pipeline::PipelineRunStatus::decl(),
        db::models::pipeline::PipelineStageStatus::decl(),
        db::models::pipeline::GateKind::decl(),
        db::models::pipeline::GateDecisionKind::decl(),
        db::models::pipeline::ArtifactKind::decl(),
        db::models::pipeline::PipelineRun::decl(),
        db::models::pipeline::PipelineStageRun::decl(),
        db::models::pipeline::PipelineGateDecision::decl(),
        db::models::pipeline::IssueArtifactSummary::decl(),
        db::models::pipeline::IssueArtifact::decl(),
        db::models::pipeline::PipelineTemplateStageView::decl(),
        db::models::pipeline::PipelineTemplateView::decl(),
        db::models::pipeline::IssuePipelineView::decl(),
        db::models::pipeline::StartPipelineRepo::decl(),
        db::models::pipeline::StartPipelineRequest::decl(),
        db::models::pipeline::GateDecisionRequest::decl(),
        db::models::pipeline::PendingPipelineItem::decl(),
```

- [ ] **Step 2: 生成并检查输出**

Run: `pnpm run generate-types`
Expected: 成功退出，`git diff --stat shared/types.ts` 显示有改动。

Run:

```bash
grep -n 'export type PipelineStageKey\|export type ExecutionProcessRunReason\|export type ScriptContext\|export type PipelineRun =\|export type IssueArtifact =\|export type GateKind' shared/types.ts
```

Expected（逐条核对）：
- `export type PipelineStageKey = "requirement" | "spec" | "test_design" | "develop" | "review" | "test" | "deliver";`
- `export type ExecutionProcessRunReason = "setupscript" | "cleanupscript" | "archivescript" | "codingagent" | "devserver" | "pipelinestep";`
- `ScriptContext` 含 `"PipelineCheck"`
- `export type GateKind = "human" | "auto" | "none";`
- `PipelineRun` 里 `template_version: number`（不是 `bigint`，契约修正 C2）
- `IssueArtifact` 是平铺的（形如 `{ content: string, } & IssueArtifactSummary` 或把 `id`、`kind` 等字段直接列出，两种写法 ts-rs 版本不同而异，前端用法相同），**不是** `{ summary: IssueArtifactSummary, ... }`（契约修正 C8，已核实 ts-rs 分支会读 `#[serde(flatten)]`）。

- [ ] **Step 3: 前端类型检查不被新联合类型打断**

Run: `pnpm run check`
Expected: 全部通过（前端对 `run_reason` / `ScriptContext` 的 switch 都有 `default`，见 §2.3）。

- [ ] **Step 4: 提交**

```bash
git add crates/server/src/bin/generate_types.rs shared/types.ts crates/db/src/models/pipeline.rs
git commit -m "$(cat <<'EOF'
类型：流水线契约类型生成到 shared/types.ts

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 20: 收尾校验

**Files:** 无新增；只修校验暴露的问题。

- [ ] **Step 1: 格式化**

Run: `pnpm run format`
Expected: 成功退出；若有文件被改动，`git diff` 只应是空白/换行。

- [ ] **Step 2: 类型检查**

Run: `pnpm run check`
Expected: 成功退出（含 `cargo check --workspace` 与 `crates/remote` 的检查；本计划没有改 remote）。

- [ ] **Step 3: lint（含 qa-mode 的 clippy）**

Run: `pnpm run lint`
Expected: 成功退出。它包含 `cargo clippy --workspace --all-targets --features qa-mode -- -D warnings`（`package.json:31`）与 `check-unused-i18n-keys`（本计划不加前端文案，不受影响）。

- [ ] **Step 4: 全量测试**

Run: `cargo test --workspace`
Expected: `test result: ok.`，无失败。

Run: `cargo test -p executors --features qa-mode`
Expected: `test result: ok.`（覆盖只在 qa-mode 下编译的 `qa_mock` 测试）。

- [ ] **Step 5: CI 同款检查**

Run: `pnpm run generate-types:check`
Expected: 通过（`shared/types.ts` 已是最新）。

Run: `pnpm run prepare-db:check`
Expected: 通过。**待核实**：预期无差异（见 §2.2）；若报 `.sqlx` 过期，执行 `pnpm run prepare-db`，确认 diff 只涉及 `execution_processes` 相关查询的描述信息后一并提交。

- [ ] **Step 6: 人工冒烟（可选，需要一个本地 git 仓库）**

Run: `pnpm run dev:qa`
然后在浏览器里用现有界面建项目、登记一个本地仓库、建一条需求，再在另一个终端：

```bash
curl -s -X POST "http://localhost:${BACKEND_PORT}/api/local/issues/<需求id>/pipeline" \
  -H 'Content-Type: application/json' \
  -d '{"repos":[{"repo_id":"<仓库id>","target_branch":"main"}],"executor_config":{"executor":"CLAUDE_CODE"},"template_key":null}' | jq '.data.run.status'
```

Expected: `"running"`；约 2 秒后 `GET /api/local/issues/<需求id>/pipeline` 的 `data.stages[0].status` 为 `"waiting_gate"`，工作区目录下 `.vk/runs/<simple_id>/requirement.md` 存在。个人版默认免登录；团队版（`VK_MODE=team`）需带会话 Cookie 与 `X-VK-CSRF`。**待核实**：`BACKEND_PORT` 取 `dev:qa` 启动日志里打印的端口；`executor` 字段取值以 `shared/types.ts` 的 `BaseCodingAgent` 为准（qa-mode 下任何取值都走模拟器）。

- [ ] **Step 7: 提交格式化改动（如有）**

```bash
git status --short
git add -u
git commit -m "$(cat <<'EOF'
收尾：格式化与校验

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

（`git status --short` 为空则跳过本步。）

---

## 7. 待核实清单（汇总）

| # | 位置 | 内容 | 如何核实 / 各分支处理 |
|---|---|---|---|
| V1 | 任务 7 Step 1 | 执行时能否联网拉 `serde_yaml`（写计划时已能联网，标了 deprecated） | 能联网照做；断网改用 `.vibe/pipeline.json` + `serde_json`，记入遗留 |
| V2 | 任务 20 Step 5 | `prepare-db:check` 预期无差异 | 有差异就 `pnpm run prepare-db` 并检查 diff 范围 |
| V3 | 任务 2 Step 4 | `sqlx::migrate!` 感知新迁移 | `touch crates/db/src/lib.rs`，仍不行就 `cargo clean -p db` |
| V4 | 任务 20 Step 6 | 冒烟用端口与执行器取值 | 见步骤说明 |

## 8. 遗留（不在本计划范围，交给计划 B 或后续）

1. 前端集合 `pipeline_runs` / `pipeline_stage_runs` 的接入、`ISSUE_STREAM_TABLES` 与 `REST_RESOURCE` 登记（计划 B；本计划已按契约 C1 提供快照接口，路由契约测试会在计划 B 登记后自动覆盖）。
2. 容器真实链路（`start_new_session`、退出钩子 `chain_continues`、qa-mode 模拟器写产出物）的端到端验证：计划 B 的 Playwright e2e。
3. 每个阶段结束都会发一次「Workspace Complete」系统通知（`crates/services/src/services/container.rs:238-270`），流水线运行时略吵，后续可在 `finalize_task` 里对流水线会话降噪。
4. 交付阶段自动提 PR（设计 §9.2）、预算熔断（U5）。
5. `template_warning` 目前只落库与打日志，界面不可见；需要展示时再在契约里加字段。
