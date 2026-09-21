# 个人版界面重做与交付流水线设计（U1 + U2）

> 状态：已确认方向，待实施｜日期：2026-09-18
> 上级方案：`docs/superpowers/specs/2026-09-15-vibe-kanban-optimization-roadmap.md`
> 前置：`docs/superpowers/specs/2026-09-16-personal-issue-flow-design.md`、`docs/superpowers/specs/2026-09-16-team-mode-and-ui-design.md`
> 方案页（含界面草图）：https://claude.ai/artifact/SjDULXZBVY1Hw38xUtdPEK

## 1. 目标

把个人版从「聊天框 + 看板」改成一条会自己跑的交付流水线：

```
一句话 / PRD → 需求 → 设计规格 → 用例设计 → 开发 → 评审 → 测试 → 交付
                 ↑人工确认   ↑人工确认    ↑人工确认   └──── 全自动 ────┘
```

本设计覆盖两期：

- **U1 界面骨架**：信息架构重排、工作台、看板与需求详情重做、清理上游遗留物、端到端测试框架。
- **U2 流水线引擎**：流程模板、阶段状态机、关卡、回流与轮次、产出物存储、实时推送。

技能接入（U3）、`prd2testcase` 与 `atp-run`（U4）、交付与度量（U5）不在本设计的实施范围内，但数据模型与接口按它们的需要预留，本文档在 §9 写明预留点。

## 2. 已确认的决策

| 事项 | 决策 |
|---|---|
| 人工关卡 | 三道：需求确认、设计规格确认、测试用例设计确认 |
| 其余阶段 | 开发、评审、测试、交付自动；交付自动提 PR，**不自动合并** |
| 用例回流 | 测试失败若归因为「用例问题」，改后的用例要人重新确认 |
| 每阶段一个技能 | 优先使用量高的第三方技能，平台包一层 `vk-*`（见 §9.1） |
| 用例生成 | 新建 `prd2testcase` 技能（用户确认网上没有现成的） |
| 需求详情 | 全屏页，不是抽屉 |
| 前端落点 | 在 `packages/web-core` 上重排，个人版与团队版共用一套页面 |
| 交付节奏 | U1 + U2 一起验收 |

## 3. 现状要点（已核对代码）

| 事实 | 位置 | 影响 |
|---|---|---|
| 外壳、左侧图标栏、停服横幅都在一处 | `web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx:321-386`、`packages/ui/src/components/AppBar.tsx` | 信息架构改造集中在两个文件 |
| 分组标题 Local / Remote / Projects / Export 是硬编码英文 | `AppBar.tsx:248-354` | 要走国际化 |
| 停服横幅由 `isExportActive || (isSignedIn && isProjectDestination)` 控制 | `SharedAppLayout.tsx:185,330-333` | 个人版直接不渲染 |
| 列头的「同名徽标」是阶段名 | `packages/ui/src/components/KanbanColumnHeader.tsx:62-66` | 由 `KanbanColumn.tsx:109-117` 的 `showStageBadge` 控制，可关 |
| 每列的「新建需求」来自空态组件 | `KanbanColumnEmptyState.tsx:96` | 改为只有第一列有新建入口 |
| Team/Personal 切换与「新问题」在筛选栏 | `packages/ui/src/components/KanbanFilterBar.tsx:194-207,286-293` | 个人版隐藏切换；文案统一成「需求」 |
| 需求详情是右侧面板，三个页签 | `pages/kanban/KanbanIssuePanelContainer.tsx:1083-1156`、`issuePanelTabs.ts:8` | 新增全屏路由，页签扩展到六个 |
| 前端类型主要来自 `shared/remote-types.ts`，`ProjectStatus` 没有 `stage_type` | `features/kanban/model/stageType.ts` | 流水线新类型走 `shared/types.ts`（ts-rs 生成） |
| 本地数据集合先 REST 快照再 WS 增量 | `shared/lib/local/localCollections.ts:214`、`localEndpoints.ts` | 流水线状态复用同一机制 |
| 变更钩子白名单是 `HookTables` 枚举 | `crates/services/src/services/events/types.rs:22`、`events.rs:86-250`、`events/streams.rs:320-381` | 新表要登记，且 `id` 必须是第 0 列 |
| 需求阶段流转已有两处 | `routes/workspaces/create.rs:257-283`、`services/issue_flow.rs:30` | 流水线接管后要避免双写 |
| 执行进程退出后有统一钩子 | `crates/local-deployment/src/container.rs:480-612` | 流水线在这里推进下一阶段 |
| `ExecutionProcessRunReason` 有库内 CHECK 约束 | `crates/db/src/models/execution_process.rs:53`、迁移 `20260203000000_*` | 新增取值要写迁移 |
| qa-mode 是编译期硬切换的模拟执行器 | `crates/executors/src/executors/qa_mock.rs`、`actions/coding_agent_initial.rs:53-58` | 端到端测试用它，不花真实额度 |
| `dev:qa` 调用了不存在的脚本 | `package.json:36` 引用 `backend:dev:watch:qa` | 顺手补上 |
| debug 构建数据目录写死 `dev_assets` | `crates/utils/src/assets.rs:6` | 端到端测试要能指定临时目录 |
| 工作区目录下每个仓库是一个子目录 | `crates/workspace-manager/src/workspace_manager.rs:312` | 产出物放工作区目录下的 `.vk/`，天然在 git 之外 |
| 数据库测试统一用 `TestDb` | `crates/db/src/test_support.rs:14` | 新模型沿用 |
| 路由层测试直接调 `handle_xxx` | `routes/local_projects/issues.rs:194` | 新接口沿用 |
| `/local` 已在受保护路由里 | `crates/server/src/routes/mod.rs:54,92-103` | 新接口挂进去即自动鉴权 |

## 4. 架构

```
浏览器
  ├─ 工作台 / 需求流水线 / 需求详情（全屏） / 测试中心 / 文档 / 设置
  └─ localCollections：REST 快照 + WS JSON Patch
        │
本地服务（crates/server）
  ├─ /api/local/*                    项目、需求、状态列（现有）
  ├─ /api/local/pipeline/*           流水线：启动、查询、关卡决策、暂停/继续（新增）
  └─ /api/issues/streams/ws          需求 + 流水线状态实时推送（扩展）
        │
流水线引擎（crates/services/src/services/pipeline/）
  ├─ template.rs   模板加载（内置模板 + 仓库 .vibe/pipeline.yaml）
  ├─ engine.rs     状态机：开始 → 运行 → 等关卡 → 通过 / 回流 / 失败 / 完成
  ├─ steps.rs      技能步骤（编码智能体会话）、脚本步骤
  ├─ artifacts.rs  产出物读盘与入库
  └─ gates.rs      人工关卡与自动判定
        │
执行层（现有）：工作区（git worktree）→ 会话 → 执行进程 → 退出钩子回调引擎
        │
SQLite：pipeline_runs / pipeline_stage_runs / pipeline_gate_decisions / issue_artifacts
```

引擎不自己起进程，所有执行都走现有的容器服务与执行进程，因此日志、审批、实时推送全部复用。

## 5. 数据模型

新增迁移 `20260918000000_add_pipeline.sql`。四张表，**`id` 一律第 0 列**（变更钩子约定）。

| 表 | 关键字段 |
|---|---|
| `pipeline_runs` | id、issue_id、project_id（冗余，供推送过滤）、workspace_id（可空，第一个技能步骤时创建）、template_key、template_version、status、current_stage_key、created_at、updated_at、finished_at |
| `pipeline_stage_runs` | id、run_id、project_id（冗余）、stage_key、attempt（从 1 开始）、status、gate_kind（human/auto/none）、session_id、execution_process_id、started_at、finished_at、summary、error |
| `pipeline_gate_decisions` | id、stage_run_id、decision（approve/reject）、comment、decided_by、decided_at |
| `issue_artifacts` | id、issue_id、stage_run_id、kind、rel_path、content、version、created_at；`(issue_id, kind, version)` 唯一 |

状态取值：

- `pipeline_runs.status`：`running` / `waiting_gate` / `paused` / `failed` / `completed` / `cancelled`
- `pipeline_stage_runs.status`：`pending` / `running` / `waiting_gate` / `passed` / `rejected` / `failed` / `skipped`
- `issue_artifacts.kind`：`requirement` / `spec` / `plan` / `test_cases` / `trace_matrix` / `review` / `test_report` / `delivery_report`

产出物同时落两处：工作区目录下的文件（技能读写的真实位置）与 `issue_artifacts`（界面展示、历史留痕）。文本超过 256 KB 时库里只存前 256 KB 加截断标记，完整内容仍在磁盘。

`ExecutionProcessRunReason` 新增 `PipelineStep`，同时写迁移放开 CHECK 约束。

## 6. 流水线引擎

### 6.1 模板

内置「标准七阶段」模板写在 Rust 里（`template.rs`），仓库存在 `.vibe/pipeline.yaml` 时以仓库为准，解析失败则回落内置模板并在运行记录里写警告。字段：

```yaml
version: 1
stages:
  - key: requirement        # 阶段标识，前端按它取中文名
    skill: vk-requirement   # U3 前不存在，引擎只负责把名字写进提示词
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
    checks: [lint_script, test_script]
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
```

自动判定只有四种，都在 `gates.rs` 里实现，不做表达式引擎（YAGNI）：`checks_passed`、`no_blocking_findings`、`all_cases_passed`、`artifacts_present`。

### 6.2 阶段执行

1. 引擎建 `pipeline_stage_runs` 记录，状态 `running`。
2. 首个技能步骤时创建工作区（复用 `create_and_start_workspace` 的内部路径），之后各阶段共用同一个工作区；每个阶段开新会话，避免上下文无限增长。
3. 提示词由引擎拼装，固定结构：

```
使用技能 {skill}。
需求：{simple_id} {title}
产出物目录（绝对路径）：{artifacts_dir}
必须产出：{artifacts}
上一阶段产出：{已有产出物清单}
{打回时附带：上一次被打回的意见}
这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。
```

4. 执行进程退出后，`local-deployment` 的退出钩子调用 `PipelineService::on_execution_finished`。
5. 引擎读产出物目录，入库，然后判关卡。

`artifacts_dir` = 工作区目录下的 `.vk/runs/<simple_id>/`。工作区目录里每个仓库是子目录，所以这个位置不在任何 git 仓库内，产出物不会混进提交。

### 6.3 关卡与回流

| 情况 | 行为 |
|---|---|
| 人工关卡 | 阶段置 `waiting_gate`，运行置 `waiting_gate`；界面出现在工作台「需要你确认」 |
| 人工通过 | 记一条 `approve`，进入下一阶段 |
| 人工打回 | 记一条 `reject` 带意见，同一阶段 `attempt + 1` 重跑，意见进提示词 |
| 自动判定通过 | 直接进入下一阶段 |
| 自动判定不通过 | 同阶段重跑，把失败信息回喂；达到 `max_rounds` 后运行置 `failed`，进工作台「需要你处理」 |
| 测试归因为代码缺陷 | 回到 `develop` 阶段重跑 |
| 测试归因为用例问题 | 回到 `test_design` 阶段，**重新走人工关卡** |
| 暂停 | 运行置 `paused`，不再调度；可继续或取消 |

阶段回退时不删历史记录，`attempt` 递增，界面按阶段分组显示每次尝试。

### 6.4 与需求状态的关系

引擎是唯一写需求阶段的地方（流水线启动后）。阶段与看板列的映射：

| 流水线阶段 | `stage_type` | 看板列 |
|---|---|---|
| requirement | backlog | 需求 |
| spec、test_design | todo | 规格 · 用例 |
| develop | dev | 开发 |
| review | review | 评审 |
| test | test | 测试 |
| deliver | done | 交付 |

`workspaces/create.rs:257-283` 与 `issue_flow.rs` 的自动流转保留给「不走流水线」的手工工作区；需求已有运行中的流水线时跳过，避免双写。

## 7. 接口

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/local/issues/{id}/pipeline` | 启动流水线（参数：仓库、分支、执行器配置、模板 key） |
| GET | `/api/local/issues/{id}/pipeline` | 当前运行 + 各阶段 + 产出物清单 |
| GET | `/api/local/pipeline/artifacts/{id}` | 取单个产出物内容（按产出物 id；契约修订 M6，原先写的 `runs/{id}/artifacts/{kind}` 作废） |
| POST | `/api/local/pipeline/stage-runs/{id}/gate` | 关卡决策：`{decision, comment}` |
| POST | `/api/local/pipeline/runs/{id}/pause` \| `/resume` \| `/cancel` | 暂停、继续、取消 |
| GET | `/api/local/pipeline/pending` | 所有等待人工确认与失败的运行（工作台用），可带 `project_id` |
| GET | `/api/local/pipeline_runs?project_id=` | 集合快照：`{ "pipeline_runs": [...] }`，**不包 `ApiResponse`** |
| GET | `/api/local/pipeline_stage_runs?project_id=` | 集合快照：`{ "pipeline_stage_runs": [...] }`，**不包 `ApiResponse`** |

上表前六行的响应统一包 `ApiResponse`；最后两行是给前端本地集合机制用的快照接口，按本地约定返回 `{ "<表名>": [...] }`（契约 C1）。路径参数在后端路由表里一律登记为 `{id}`（契约 C4）。

全部挂在 `local_projects::router()` 的 `/local` 下，自动落进受保护路由。类型定义在 `crates/db/src/models/pipeline.rs`（与模型同文件），注册进 `generate_types.rs`，前端从 `shared/types.ts` 取。

**阶段失败原因会被广播**：检查脚本失败时，引擎把它的输出末尾（最多 50 行、4KB，超出从头截断并注明）写进 `pipeline_stage_runs.error`，这一行经变更钩子推给订阅该项目的所有客户端。因此 `.vibe/pipeline.yaml` 里的 `checks` 命令不要打印密钥、令牌等敏感信息。

推送：`pipeline_runs`、`pipeline_stage_runs` 加进 `HookTables` 与 issues 流的路径前缀白名单，按 `project_id` 过滤，前端用现有集合机制订阅；产出物内容不推送，由界面按需拉取。

## 8. 界面

### 8.1 信息架构

左侧导航固定五项：工作台、需求流水线、测试中心、文档、设置；项目切换改为顶部下拉（颜色块 + 全名）。移除：停服横幅、GitHub / Discord 徽标、Remote 与 Export 分组（导出功能移进设置）、个人版里的 Team / Personal 切换。工作区不再是一级入口，进入需求详情的「开发」页签查看。

### 8.2 工作台（新页面）

- 顶部输入框：「描述一个需求，或粘贴 PRD / 飞书文档链接」，下面选仓库、模板、智能体，提交即建需求并启动流水线。
- 三栏：**需要你确认**（三类关卡卡片 + 失败转人工）、**正在自动跑**（阶段徽标 + 七格进度条）、**最近交付**（含本周统计）。

### 8.3 看板

- 六列：需求、规格 · 用例、开发、评审、测试、交付；列头只写一次名字（关掉阶段徽标），显示计数与 WIP 提示。
- 卡片底部七格进度条：已完成绿、进行中蓝、等人工橙、失败红；运行中的状态标签带呼吸点。
- 新建入口只保留顶部一个；空列写明「为什么空」。
- 拖拽：只允许往回拖（打回），往前推进由关卡决定；非法拖拽给一次提示而不是静默失败。

### 8.4 需求详情（全屏路由）

新增 `/projects/$projectId/issues/$issueId/detail`（保留原右侧面板路由不动，避免影响团队版）。

- 顶部：面包屑 + 标题 + 当前阶段徽标 + 「暂停自动化」。
- 阶段步进条：七个阶段，已完成打勾，当前高亮，失败标红。
- 左主栏页签：需求与验收标准 / 规格 / 用例 / 代码变更 / 测试 / 交付报告，内容来自产出物。
- 右栏：时间线（AI 与人的每一步、耗时）、产出物列表、用量。
- 底部关卡条：等人工时显示「确认并继续」「打回并说明」；自动关卡显示判定条件与当前进度。

### 8.5 设计规范

沿用现有令牌（`text-high`、`bg-primary`、`brand`）；阶段语义色固定：灰=需求/规格、蓝=开发、紫=评审、琥珀=测试、绿=完成、红=失败。状态三重编码（颜色 + 文字 + 进度格），色弱可辨。默认密度从 12px 放宽到 13px，可切紧凑。所有新文案走国际化（7 种语言都要补 key，`check-unused-i18n-keys` 会拦截只加中文的情况），等宽字体只用于编号、耗时、计数、提交哈希。

## 9. 为后续几期预留

### 9.1 技能（U3、U4）

平台技能放仓库 `.claude/skills/`，第三方技能拷贝进来并在 `.vibe/skills.lock` 记录来源仓库、提交号、许可证：

| 阶段 | 平台技能 | 基于 |
|---|---|---|
| 需求 | `vk-requirement` | superpowers/brainstorming |
| 设计规格 | `vk-spec` | mattpocock/to-spec + superpowers/writing-plans + gstack/plan-eng-review |
| 用例设计 | `prd2testcase` | 新建（三步法 + 追踪矩阵，输出 atp 旧 CSV） |
| 开发 | `vk-develop` | superpowers/test-driven-development |
| 评审 | `vk-review` | superpowers/requesting-code-review + 内置 `/security-review` |
| 测试 | `atp-run` | 新建，归因流程取 superpowers/systematic-debugging |
| 交付 | `vk-deliver` | superpowers/finishing-a-development-branch + gstack/document-release |

U2 阶段这些技能都不存在，引擎只把技能名写进提示词；qa-mode 下由模拟执行器直接产出产出物文件，流程一样跑通。

### 9.2 其它预留

- `test-report.json` 的结构按 atp `results.json` 对齐，含每条用例的归因字段。
- 交付阶段的 PR 创建走现有 `workspaces/pr.rs`，U5 接多 Git 服务器。
- 预算（累计时长、总轮次上限）U2 只落库不启用，U5 做界面与熔断。

## 10. 端到端测试

新增 `packages/e2e`（Playwright），跑真后端 + 真前端，不打桩网络层：

1. 后端用 `--features qa-mode` 编译，模拟执行器按提示词里的产出物清单生成文件，因此流水线七个阶段都能自动跑完。
2. 每个用例独立数据目录：`asset_dir()` 增加 `VK_ASSET_DIR` 环境变量覆盖（默认行为不变）。
3. 黄金路径：输入一句话 → 需求确认 → 规格确认 → 用例确认 → 开发 → 评审 → 测试 → 交付，断言看板列、进度格、时间线与产出物页签。
4. 异常路径：人工打回后重跑、自动判定失败用尽轮次转人工、测试归因用例问题退回并重新确认、暂停与继续、刷新后状态恢复。
5. 视觉回归：工作台 / 看板 / 详情三页 × 亮暗主题 × 1280 与 1440 宽度。
6. CI 增加 `e2e` 任务，失败上传 trace 与截图；本地 `pnpm run e2e`。

单元测试照旧：Rust 用 `TestDb` 测模型与 handler（模板解析、状态机转移、关卡判定、归因分派都要有用例），前端纯函数用 Vitest（进度格计算、阶段映射、关卡文案）。

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| 引擎与既有自动流转双写需求状态 | 需求存在运行中的流水线时，`issue_flow` 与 `create.rs` 的流转短路；加单测钉住 |
| 智能体不产出约定文件 | 缺产出物按阶段失败处理，进入重试；重试仍缺则转人工，并在界面显示「技能没有产出 X」 |
| 阶段重跑导致工作区状态混乱 | 同一运行共用工作区，重跑前记录当前提交，失败时不自动回滚，交人工判断 |
| 产出物过大撑爆库 | 库里截断到 256 KB，完整内容留磁盘 |
| 模拟执行器让端到端测试失真 | 模拟器只负责产出文件，状态机、关卡、推送、界面都是真实路径；关键断言放在引擎与接口层 |
| 全屏详情页与既有右侧面板重复维护 | 两者共用同一批容器组件，全屏页只负责布局 |
| 新表推送漏登记 | 迁移与 `HookTables` 在同一个任务里改，并加一条推送用的集成测试 |

## 12. 验收标准

1. 个人版进入后默认是工作台；左侧只有五个中文入口；无停服横幅、无 GitHub/Discord 徽标、无 Team/Personal 切换。
2. 在工作台输入一句话可建需求并启动流水线；三道人工关卡依次出现在「需要你确认」。
3. 看板六列与卡片进度格正确反映阶段；等人工的卡片是橙色格子。
4. 需求详情全屏页能看到七段步进条、各阶段产出物与时间线。
5. 打回、失败转人工、暂停继续、测试归因回退四条异常路径行为与 §6.3 一致。
6. `pnpm run format`、`pnpm run check`、`pnpm run lint`、`cargo test --workspace`、`pnpm run e2e` 全绿。
7. 团队版（`VK_MODE=team`）行为不回退：登录、成员管理、工作区删除审批、第三方账号绑定照常。
