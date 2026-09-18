# 个人版界面重做与端到端测试 实施计划（计划 B）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **假设你对本仓库零了解。** 每一步都写了精确的文件路径、行号依据、命令与预期输出。凡是标注「**待核实**」的地方，先按给出的命令核实，再按对应分支处理，不要按计划里的猜测硬写。

**Goal:** 把个人版的外壳、工作台、需求流水线看板、需求详情全屏页按设计文档 §8 重做，接上计划 A 的流水线接口与实时推送，并新增 `packages/e2e`（Playwright）跑通 §10 的黄金路径、四条异常路径与视觉回归。

**Architecture:** 界面全部落在 `packages/web-core`（页面 / 容器 / 纯函数）与 `packages/ui`（无状态视图），路由文件在 `packages/local-web/src/routes`。流水线数据走两条路：`pipeline_runs` / `pipeline_stage_runs` 两个本地集合（REST 快照 + 需求流 WebSocket 增量，复用 `createLocalShapeCollection`），以及按需拉取的 `IssuePipelineView` / 产出物（react-query，集合变化时失效重拉）。所有判定逻辑抽成纯函数放在 `entities/pipeline/model`、`features/kanban/model`、`pages/*/…Model.ts`，用 Vitest（node 环境）测；界面行为只靠端到端测试。团队版（`VK_MODE=team`）一律走原代码路径。

**Tech Stack:** React 18 + TypeScript、TanStack Router（文件路由）、TanStack DB（`useShape`）、@tanstack/react-query 5、Tailwind（`tailwind.new.config.js` 令牌）、i18next、Vitest 3、Playwright（自带无头 Chromium）、Rust（本计划不改 Rust 代码；`VK_ASSET_DIR` 等由计划 A 提供）。

**设计文档：** `docs/superpowers/specs/2026-09-18-personal-pipeline-design.md`（重点 §8、§10、§12）
**前后端契约：** `docs/superpowers/plans/2026-09-18-pipeline-contract.md`（类型名、接口、推送路径、qa 标记一律以它为准；已按其修订记录 C1–C9 对齐，见 §3 决策 12）
**计划 A（后端）：** `docs/superpowers/plans/2026-09-18-pipeline-engine.md`（本计划依赖它的产物，见 Task 1）
**界面草图：** 方案页「04 新界面草图」（工作台 / 需求流水线看板 / 需求详情全屏），信息结构与文案照它落地，样式用本仓库令牌。

---

## 0. 阅读顺序（执行前必读）

1. 本计划第 1～6 节（约定、已核实事实、决策、待核实、团队版回归点、文件清单）。
2. 设计文档 §8（界面）、§10（端到端）、§12（验收）。
3. 契约全文（184 行）。
4. `CLAUDE.md`、`packages/local-web/AGENTS.md`（View/Container 分层、令牌、间距 `half`/`base`/`double`）。

**行号约定：** 本计划所有「文件:行号」指**改动前**的原文件。同一文件多处修改时，按从下往上的顺序改，前面的行号就不会漂移；或按引号里给出的原文定位。

**执行顺序硬约束：** Task 1 通过之前不要开始任何后续任务：Task 2～11 是词条 + 纯函数，但**依赖 `shared/types.ts` 里的流水线类型**；Task 12 起还依赖计划 A 的后端路由与推送。另：Task 15 要求先做 Task 17 的 Step 1～3（见 Task 15 开头）。

---

## 1. 跨任务共享约定（逐字一致）

### 1.1 新增模块与导出名

| 名称 | 位置 | 说明 |
|---|---|---|
| `PIPELINE_STAGE_KEYS`、`PipelineBoardStage`、`boardStageOf`、`isPipelineStageKey`、`pipelineStageLabelKey`、`fallbackGateLabelKey`、`autoConditionLabelKey`、`pipelineStageTone`、`PipelineStageTone` | `packages/web-core/src/entities/pipeline/model/stages.ts` | 阶段元数据 |
| `toMillis`、`toNumber`、`stageKeysOf`、`latestAttempts`、`pipelineProgress`、`pipelineStatus`、`pipelineStatusText`、`PipelineCellState`、`PipelineCell`、`PipelineTone`、`PipelineStatusInfo`、`Translate` | `…/entities/pipeline/model/progress.ts` | 进度格与状态文案 |
| `buildPipelineCardInfo`、`latestRunByIssue`、`groupStagesByRun`、`PipelineCardInfo`、`PipelineCardCell`、`cellLabelKey` | `…/entities/pipeline/model/cardInfo.ts` | 卡片底部信息 |
| `durationText`、`relativeTimeText`、`TextSpec` | `…/entities/pipeline/model/time.ts` | 耗时 / 相对时间 |
| `pipelineSignature` | `…/entities/pipeline/model/signature.ts` | 集合变化 → 失效重拉的指纹 |
| `PIPELINE_RUNS_SHAPE`、`PIPELINE_STAGE_RUNS_SHAPE` | `…/entities/pipeline/api/pipelineShapes.ts` | 集合形状 |
| `PIPELINE_API_PATHS`、`PipelineApiError`、`getIssuePipeline`、`startPipeline`、`getArtifact`、`decideGate`、`runPipelineAction`、`listPendingPipelines`、`createIssue`、`createIssueAndStartPipeline`、`PipelineRunAction` | `…/entities/pipeline/api/pipelineApi.ts` | 接口封装 |
| `pipelineQueryKeys`、`usePipelineRuns`、`usePipelineStageRuns`、`useIssuePipeline`、`usePendingPipelines`、`useArtifact`、`useGateDecision`、`usePipelineRunAction`、`useStartPipeline` | `…/entities/pipeline/model/hooks/usePipelineData.ts` | hooks |
| `pipelineColumnTitleKey`、`pipelineColumnEmptyKey`、`pipelineColumnHintKey`、`LEGACY_DEFAULT_STATUS_NAMES` | `…/features/kanban/model/pipelineColumns.ts` | 看板列文案 |
| `canDropIssue`、`DropDecision` | `…/features/kanban/model/pipelineDrag.ts` | 拖拽判定 |
| `buildTimeline`、`timelineTextKey`、`TimelineEvent` | `…/pages/issue-detail/timeline.ts` | 时间线 |
| `parseCsv`、`parseReview`、`parseTestReport` | `…/pages/issue-detail/artifactParsers.ts` | 产出物解析 |
| `ISSUE_DETAIL_TABS`、`IssueDetailTab`、`issueDetailTabLabelKey`、`tabArtifactKinds`、`tabOfArtifactKind`、`defaultIssueDetailTab`、`latestArtifactOfKind`、`latestArtifactsByKind`、`artifactKindLabelKey`、`artifactRenderMode`、`gateBarState`、`GateBarState` | `…/pages/issue-detail/issueDetailModel.ts` | 详情页签与关卡条 |
| `runningRuns`、`recentDelivered`、`weeklyStats`、`stagesFinishedToday`、`greetingKey`、`pickBacklogStatusId`、`buildCreateIssueRequest`、`pickDefaultBranch`、`pickWorkbenchProjectId` | `…/pages/workbench/workbenchModel.ts` | 工作台 |
| `PERSONAL_ROUTES`、`personalNavActiveKey` | `…/shared/lib/routes/personalRoutes.ts` | 个人版路由路径 |

**为什么流水线公共逻辑放 `entities/` 而不是简报说的 `features/*/model`：** `packages/local-web/.eslintrc.cjs` 的分层规则（`features/**` 不得 import `@/features/**`，`entities/**` 只能 import shared）里，看板（`features/kanban`）、工作台（`pages/workbench`）、详情（`pages/issue-detail`）三处都要用进度格计算；放在 `features/pipeline` 会让 `features/kanban` 跨 feature 引用。`entities` 目录目前在 web-core 不存在，但 eslint 规则与 `scripts/check-legacy-frontend-paths.sh:43` 都承认它是合法层。只被单个页面用的逻辑（时间线、解析、工作台）放在该页面目录下，沿用 `pages/kanban/issuePanelTabs.ts` 的先例。

### 1.2 路由路径

| 路径 | 路由文件（`packages/local-web/src/routes/`） | 页面组件 |
|---|---|---|
| `/home` | `_app.home.tsx`（新建） | `pages/workbench/WorkbenchPage.tsx` |
| `/docs` | `_app.docs.tsx`（新建） | `pages/docs/DocsPlaceholderPage.tsx` |
| `/testing` | `_app.testing.tsx`（已有，不改） | `pages/testing/TestingPlaceholderPage.tsx` |
| `/projects/$projectId/issues/$issueId/detail` | `_app.projects.$projectId_.issues.$issueId_.detail.tsx`（新建） | `pages/issue-detail/IssueDetailPage.tsx` |
| `/projects/$projectId` | 已有，不改 | 看板 |

**决定：工作台用新路由 `/home`，并改根跳转**：个人版（`isLocalPersonalMode()`）进 `/` 直接 `replace` 到 `/home`；团队版与云端构建沿用 `RootRedirectPage` 原逻辑。`/home`、`/docs`、详情路由都在 `beforeLoad` 里对非个人版 `redirect({ to: '/' })`（照 `_app.members.tsx:13-17` 的写法），团队版看不到这三页。

### 1.3 `data-testid` 约定（端到端测试靠它定位）

| testid | 所在组件 | 附加属性 |
|---|---|---|
| `personal-sidebar` | `PersonalSidebar` | — |
| `workbench-greeting` | `WorkbenchPage` | — |
| `workbench-column-confirm` / `-running` / `-delivered` | `WorkbenchColumn` | — |
| `workbench-card` | `WorkbenchCard` | `data-issue-id` |
| `pipeline-progress` | `PipelineProgressBar` | `data-cells="done,gate,pending,…"` |
| `pipeline-status-tag` | `PipelineStatusTag` | `data-tone` |
| `kanban-column-header` | `KanbanColumnHeader` | `data-stage` |
| `kanban-card-pipeline` | `KanbanColumn` | `data-issue-id` |
| `kanban-drag-hint` | `KanbanContainer` | — |
| `pipeline-stepper` | `PipelineStepper` | `data-current`；每个 `li` 带 `data-state` |
| `gate-bar` | `PipelineGateBar` | `data-kind` |
| `pipeline-timeline` / `timeline-item` | `PipelineTimeline` | `data-actor` |
| `artifact-section` | `ArtifactSection` | `data-kind` |
| `csv-table` | `CsvTableView` | — |
| `relative-time` | 所有相对时间文本 | — |
| `nav-footer` | `PersonalSidebar` 底部版本号 | — |
| `docs-empty` | `DocsPlaceholderPage` | — |

### 1.4 i18n

- 新词条全部放 `common` 命名空间（`packages/web-core/src/i18n/locales/<lang>/common.json`），只有设置页的导出卡片放 `settings` 命名空间。
- 七种语言 key 必须齐（`scripts/check-i18n.sh` 的 `check_key_consistency` 会拦）；es / fr / ja / ko 先用英文值。
- 代码里引用 key 一律写**字面量映射表**，不用模板字符串拼 key（`scripts/check-unused-i18n-keys.mjs` 抓不到拼出来的 key）。
- `check-unused-i18n-keys` 在 Task 2 之后到 Task 21 完成之前会报「未引用」，**这是预期的**；中间任务只跑 `web-core:test` 与 `tsc`，Task 28 才跑全量 `lint`。

### 1.5 提交信息

中文，一行摘要 + 可选正文，末尾固定两行：

```
<摘要>

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
```

---

## 2. 已核实的现状（文件:行号）

### 2.1 外壳

| 事实 | 位置 |
|---|---|
| 停服横幅条件 `isExportActive \|\| (isSignedIn && isProjectDestination(currentDestination))` | `packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx:185-186` |
| 桌面 grid：有横幅时三行 | 同上 `:320-325` |
| 桌面横幅渲染 | 同上 `:330-334`；移动端横幅 `:424-426` |
| 顶栏 `NavbarContainer` | 同上 `:342-345` |
| 左侧 `AppBar`（含 GitHub/Discord 图标路径） | 同上 `:347-386` |
| 移动端抽屉（Workspaces / Export / 项目列表 / 新建项目） | 同上 `:439-567` |
| GitHub 星数、Discord 在线数的请求 | 同上 `:76-77`（`useDiscordOnlineCount` / `useGitHubStars`，两个 hook 在 `shared/hooks/useDiscordOnlineCount.ts:29-39`、`shared/hooks/useGitHubStars.ts:27-37`，**没有 enabled 参数**） |
| `useParams({ strict: false })` 只取了 `hostId` | 同上 `:81` |
| 选中项目写入偏好 store | 同上 `:197-205`（`setSelectedProjectId`），store 字段 `shared/stores/useUiPreferencesStore.ts:381,517,887` |
| 「测试」导航用 `navigate({ to: '/testing' })` | 同上 `:211-214` |
| `AppBar` 分组 `Local` / `Remote` / `Projects` / `Export` 写死英文 | `packages/ui/src/components/AppBar.tsx:266,272,346,354`；`const { t } = useTranslation('common')` 在 `:238` |
| 分组标题组件 | `AppBar.tsx:111-117`，渲染在 `:540-554` |
| GitHub / Discord 徽标 | `AppBar.tsx:560-580` |
| 运行模式：`isLocalPersonalMode()` / `isLocalTeamMode()` | `packages/web-core/src/shared/lib/local/runtimeMode.ts:63-65`、`:74-76` |
| 根跳转 | `packages/web-core/src/pages/root/RootRedirectPage.tsx:14-58`（个人版无项目时落到建工作区页） |
| 路由用 TanStack 文件路由，`routeTree.gen.ts` 由 vite 插件生成 | `packages/local-web/vite.config.ts` `tanstackRouter({...})`；`packages/local-web/src/routeTree.gen.ts` |
| 非嵌套路由写法 `$projectId_` / `$issueId_` | 例：`packages/local-web/src/routes/_app.projects.$projectId_.issues.$issueId_.workspaces.$workspaceId.tsx` |
| 个人版挡路由的写法 | `packages/local-web/src/routes/_app.members.tsx:13-17` |
| 设置对话框 section 注册表 | `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx:27-35,58-68,95-135`；`case 'general'` 在 `:101-102`，没传 `onClose`；`relay` 分支 `:123-129` 传了 |
| 常规设置最后一张卡片「Safety」与保存条 | `…/settings/GeneralSettingsSection.tsx:817-837`、`:839` |
| 导出页走 `appNavigation.goToExport()` | `SharedAppLayout.tsx:216-218`；`packages/local-web/src/app/navigation/AppNavigation.ts:343` |

### 2.2 看板

| 事实 | 位置 |
|---|---|
| 列组件（内部零 hook） | `packages/web-core/src/features/kanban/ui/KanbanColumn.tsx`（243 行）：props `:30-66`，列头 `:109-117`，卡片 `:137-224`，空态 `:227-238` |
| 列头徽标由 `stageLabelKey` 控制 | `packages/ui/src/components/KanbanColumnHeader.tsx:62-66`；新建按钮 `:68-75` |
| 空态「新建需求」 | `packages/ui/src/components/KanbanColumnEmptyState.tsx:94-98`；标题 `:91-93` |
| Team / Personal 切换 | `packages/ui/src/components/KanbanFilterBar.tsx:194-207`；顶部「新问题」按钮 `:286-307` |
| 阶段类型与顺序 | `packages/web-core/src/features/kanban/model/stageType.ts:11,17-24` |
| 个人版默认列名（待规划 / 待开发 / 开发中 / 待评审 / 测试中 / 已完成） | `crates/db/src/models/local_project.rs:14-21` |
| 看板容器：`showStageBadge` `:629-632`、`showAssignees` `:641`、密度 `:644-649`、拖拽 `handleDragEnd` `:779-876`、筛选栏 `:1190-1222`、整板空提示 `:1226-1229`、`KanbanBoardView` `:1239-1268` | `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx` |
| `ProjectKanban` → `KanbanContainer` / `KanbanIssuePanelContainer` **也被 remote-web 引用** | `packages/remote-web/src/pages/RemoteProjectKanbanShell.tsx:1` |
| 卡片组件 `KanbanCard` 有 `className`，没有 testid | `packages/ui/src/components/KanbanBoard.tsx:66-94` |
| 卡片标题字号是 `text-base` | `packages/ui/src/components/KanbanCardContent.tsx:272` 附近（`<span className="text-base text-normal truncate">{title}</span>`） |

### 2.3 详情

| 事实 | 位置 |
|---|---|
| 右侧面板容器 | `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx`（1159 行），渲染 `KanbanIssuePanel` 在 `:1082-1158`，`onMoreActions` 在 `:1129` |
| 面板头部按钮区（复制链接 / 更多 / 关闭） | `packages/ui/src/components/KanbanIssuePanel.tsx:293-326`；props `:141-146`；解构 `:202-203`；`t` 来自 `useTranslation('common')` `:221` |
| 三段式页签 | `packages/web-core/src/pages/kanban/issuePanelTabs.ts:8` |
| 测试占位 | `packages/ui/src/components/IssueTestingPanel.tsx`、`packages/web-core/src/pages/testing/TestingPlaceholderPage.tsx` |
| Markdown 渲染 | `packages/web-core/src/shared/components/MarkdownPreview.tsx:10-28`（props：`content`、`theme: 'light' \| 'dark'`、`className`） |
| 主题解析 | `packages/web-core/src/shared/hooks/useTheme.ts:29-36`（`getResolvedTheme`）；`ThemeProvider` 给 `<html>` 加 `light`/`dark` 类：`packages/local-web/src/app/providers/ThemeProvider.tsx:21-38` |

### 2.4 数据层

| 事实 | 位置 |
|---|---|
| 本地集合：REST 快照 + WS JSON Patch | `packages/web-core/src/shared/lib/local/localCollections.ts:214-363` |
| 快照响应必须是 `{ "<表名>": [...] }` | `localCollections.ts:264`（`extractFallbackRows(json, shape.table)`）→ `shared/lib/electric/rows.ts:23-37`；后端对应 `crates/server/src/routes/local_projects/mod.rs:93-96`（`snapshot()`） |
| 表 → REST 资源、允许的过滤参数、需求流覆盖的表 | `packages/web-core/src/shared/lib/local/localEndpoints.ts:11-15,18-27,30-39` |
| **Rust 路由契约测试会解析 `localEndpoints.ts` 的 `REST_RESOURCE`**，要求每个表都有 `GET /api/local/<表>` | `crates/server/src/routes/local_projects/mod.rs:211-215,263-316` |
| local 数据源下 `createShapeCollection` 直接走本地集合，不读 `url` | `packages/web-core/src/shared/lib/electric/collections.ts:741-751` |
| `useShape` 签名、`enabled=false` 时不建集合 | `packages/web-core/src/shared/integrations/electric/hooks.ts:86-98,136-151` |
| `ShapeDefinition` 结构（`table/params/url/fallbackUrl`） | `shared/remote-types.ts:187-203`；`defineShape` 未导出 |
| 本地请求唯一收口 `makeLocalApiRequest`（写方法自动带 CSRF） | `packages/web-core/src/shared/lib/localApiTransport.ts:161-190`；测试替身 `setLocalApiTransport` `:147-149` |
| 信封解析写法样板 | `packages/web-core/src/shared/lib/local/adminApi.ts:60-90`（`requestLocalEnvelope` 钉死 `hostScope: 'none'`，**流水线接口不能用它**——它们应跟随当前 host，和看板集合一致） |
| 失败响应取 `message` | `packages/web-core/src/shared/lib/local/apiEnvelope.ts:14-32` |
| 建需求接口返回 `{ txid }`，不是信封；支持前端自带 `id` | `crates/server/src/routes/local_projects/mod.rs:83-91`；`crates/api-types/src/issue.rs:60-77`；TS 类型 `shared/remote-types.ts:113-118` |
| `ApiResponse<T>` 的 TS 形状 `{ success, data: T \| null, error_data, message }` | `shared/types.ts:217` |
| **ts-rs 把 `i64` 生成成 `bigint`**（运行时 JSON 仍是 number） | `shared/types.ts:17,37,169,191` |
| 仓库列表 / 分支接口 | `packages/web-core/src/shared/lib/api.ts:835-900`（`repoApi.list`、`repoApi.getBranches`） |
| 执行器选择 hook（纯 props 驱动，可复用） | `packages/web-core/src/shared/hooks/useExecutorConfig.ts:180-215` |

### 2.5 可复用性结论（简报要求核实）

- `CreateChatBoxContainer`（`shared/components/CreateChatBoxContainer.tsx:39-63`）**不能直接复用**：它绑死 `useCreateMode()` 的草稿上下文（需要 `CreateModeProvider`，后者拉工作区列表与 scratch 草稿，`features/create-mode/model/CreateModeProvider.tsx:17-48`），提交时调的是 `createWorkspace`（`:221-272`），不是建需求。
- `CreateModeRepoPickerBar`（`shared/components/CreateModeRepoPickerBar.tsx:16`）同样依赖 `useCreateMode()`，**不复用**。
- `useExecutorConfig` 是纯 props 驱动的 hook，**复用**；`AgentIcon` 可复用但工作台不需要图标，**不引入**。
- `ModelSelectorContainer`（`shared/components/ModelSelectorContainer.tsx:60-74`）需要 presets / overrides 等一整套状态，工作台只要选智能体，**不复用**（YAGNI）。
- 结论：工作台输入框用新的无状态视图 `WorkbenchComposer` + 容器 `WorkbenchComposerContainer`，数据来自 `repoApi` + `useExecutorConfig`。

### 2.6 设计令牌

| 事实 | 位置 |
|---|---|
| 亮色令牌 `:root` | `packages/web-core/src/app/styles/new/index.css:15-88`（`--merged` 在 `:83`） |
| 暗色令牌 **是 `.dark` 类** | 同上 `:90-165`（`--merged` 在 `:142`）；Tailwind `darkMode: ["class"]`（`packages/local-web/tailwind.new.config.js:23`） |
| Tailwind 颜色令牌 | `tailwind.new.config.js:104-126`（`merged` 在 `:118`） |
| 字号令牌用 rem：`xs=0.75rem`、`sm=0.875rem`、`base=1rem` | `tailwind.new.config.js:3-10,90-96` |
| `<html>` 没有设置根字号（除移动端 `--mobile-font-scale`） | `index.css:8-10,470-472` |
| 密度模型只管间距与描述行，不管字号 | `packages/web-core/src/features/kanban/model/density.ts:31-51` |

### 2.7 端到端相关

| 事实 | 位置 |
|---|---|
| qa-mode 特性 | `crates/server/Cargo.toml:78`（`qa-mode = ["services/qa-mode", "executors/qa-mode"]`）；`server` 是 `default-run` |
| qa-mode 只影响「扫描本机 git 仓库」这两个入口，返回固定的两个 GitHub QA 仓库 | `crates/services/src/services/filesystem.rs:101-105,176-181`；`crates/services/src/services/qa_repos.rs:15-18`。**`POST /api/repos` 注册任意路径不受影响**（`crates/server/src/routes/repo.rs:50-64,370`），所以 fixtures 走注册接口，不碰扫描接口，避免触发 `git clone github.com` |
| 模拟执行器原来每行日志 `sleep 1`、共 10 行；**契约 C9：流水线模式下每行 0.1 秒、只写产出物不改文件** → 每个阶段约 1～3 秒（含工作区 / 会话调度） | `crates/executors/src/executors/qa_mock.rs:60-66,202-420`（计划 A 修改） |
| debug 构建不自动开浏览器 | `crates/server/src/main.rs:168` |
| 端口与监听地址 | `crates/server/src/main.rs:96-117`（`BACKEND_PORT`/`PORT`，`HOST` 默认 `127.0.0.1`） |
| 数据库文件在 `asset_dir()` 下 | `crates/db/src/lib.rs:134-138`；`asset_dir()` 在 `crates/utils/src/assets.rs:6-22`（debug 写死 `dev_assets`） |
| 前端 dev 代理：`/api` → `http://localhost:${BACKEND_PORT}`，`ws: true` | `packages/local-web/vite.config.ts` `server.proxy` |
| 配置接口 `GET /api/info`、`PUT /api/config` | `crates/server/src/routes/config.rs:45-46,106-110,180-183`；`Config` 字段 `crates/services/src/services/config/versions/v8.rs:36-60`，默认 `theme: System`、`remote_onboarding_acknowledged: false` |
| 首次进看板会弹「项目指南」，靠 `showcases.seen_features` 包含 `projects-guide` 跳过 | `packages/web-core/src/pages/kanban/LocalProjectKanban.tsx:7-27` |
| 建项目接口（前端自带 id，个人版组织 id 固定为 `…0001`） | `crates/api-types/src/project.rs:20-28`；`crates/db/src/models/local_project.rs:9` |
| `dev:qa` 调用了不存在的 `backend:dev:watch:qa` | 根 `package.json` `scripts.dev:qa` |
| CI 前端检查 / 后端测试 job | `.github/workflows/test.yml:162-195`（frontend-checks）、`:312-338`（backend-test）；路径过滤 `:40-161` |
| pnpm workspace 已包含 `packages/*` | `pnpm-workspace.yaml:1-2` |

---

## 3. 决策（含对简报 / 设计文档的偏离，均需在汇报里说明）

1. **流水线界面只在个人版（`isLocalPersonalMode()`）启用。** 团队版的看板、面板、外壳全部走原路径；新路由对团队版 redirect 到 `/`。
2. **新左侧栏只在个人版渲染**（`PersonalSidebar` 替换 `AppBar`）。团队版保留 `AppBar`，只把四个分组英文标题走 i18n。
3. **导出入口移进设置**：常规设置末尾加「数据导出」卡片，仅个人版显示，按钮关闭对话框后 `appNavigation.goToExport()`；`/export` 路由不动。
4. **GitHub / Discord 的两个外部请求**在个人版关掉（给两个 hook 加可选 `enabled`），既去掉徽标也不再发外网请求（端到端测试也因此不依赖外网）。
5. **看板列名**：个人版下，状态列名**等于默认名**（`local_project.rs:14-21`）时显示流水线列名（需求 / 规格 · 用例 / …）；用户改过名就尊重用户的名字。
6. **新建入口**：个人版只保留筛选栏上的「新建需求」一个入口，列头 `+` 与空列里的新建按钮都不渲染；空列显示「为什么空」。
7. **拖拽**：设计文档要求「只允许往回拖」，但契约**没有「打回到指定阶段」的接口**。本计划的规则：没有流水线的需求 → 任意拖（手工需求保持旧行为）；流水线在跑（`running` / `waiting_gate`）→ 任何跨列拖拽都拦下并提示「请先暂停再打回」；流水线已停（`paused` / `failed` / `completed` / `cancelled`）→ 只允许往回拖，往后拖提示「推进由关卡决定」。往回拖只改列（沿用 `bulkUpdateIssues`），**不改流水线状态**——这一点列入对契约的疑问。
8. **13px 默认密度**：设计文档 §8.5 与草图说「默认从 12px 放宽到 13px」，但代码里 `text-base = 1rem`、`<html>` 没改根字号，卡片标题实际是 16px（见 §2.6）。「12px」出自 `packages/local-web/AGENTS.md` 里过时的注释。**本计划默认不改字号**，Task 3 第一步实测；实测若确为 12px，按 Task 3 的分支 A 加 `text-comfy`（13px）。这一条列入待核实与汇报。
9. **i64 / 时间字段**：契约 C2 已定为 TS `number` 与 ISO 字符串；前端仍统一经 `toNumber()` / `toMillis()` 读（防御性，零成本），万一生成类型漂移也不会在算术处出错。
10. **全屏详情页**与右侧面板共用数据 hook，但**不共用布局组件**：详情页是新的一组视图（步进条、产出物、时间线、关卡条）；面板只加一个「全屏查看」按钮（仅个人版）。
11. **视觉回归**只在 Linux 上比对（CI），基线由 `workflow_dispatch` 带 `update_snapshots=true` 生成后下载提交；macOS 本地跑会 `skip`。理由：字体渲染跨平台差异会让基线失真。
12. **与契约修订 C1–C9 对齐**：C1 快照形状 `{ "<表名>": [...] }` 不包信封（Task 12 按此实现）；C2 数值 / 时间类型（决策 9）；C3 内置模板 develop 无 checks，判定只看智能体退出码（原 V7 取消）；C4 后端路由表参数名统一 `{id}`，前端拼的 URL 不变（本计划无影响）；C5 `pause` 只在 `running` / `waiting_gate`，`resume` 只在 `paused` / `failed`（详情页按钮显隐与之一致，Task 21），`failed` 算未结束；C6 `stages` 按创建顺序、`pending` 排最后（`latestAttempts` 按 `attempt` 比较，不依赖数组顺序）；C9 模拟器每阶段约 1～3 秒（暂停用例改为在人工关卡处暂停，Task 25）。另据计划 A：人工关卡阶段 `max_rounds` 也默认 3，但人工打回不计入失败轮次，所以状态文案只对**自动关卡阶段**显示「第 n/m 轮」（Task 5）；等待人工时阶段不写 `finished_at`（计划 A 测试 `assert!(waiting.finished_at.is_none())`），工作台卡片的等待时间用阶段开始时间近似；`VK_ASSET_DIR` 与 `backend:dev:watch:qa` 由计划 A Task 1 实现（本计划 Task 22 只核对）。

---

## 4. 待核实清单（执行到对应任务时先核实）

| # | 事项 | 怎么核实 | 分支处理 |
|---|---|---|---|
| V1 | 计划 A 已生成流水线类型（契约 C2：i64 为 `number`、时间为 `string`） | Task 1 Step 1 | 缺任何一个 → 停，等计划 A；声明若与 C2 不符也不影响本计划（统一 `toNumber`/`toMillis`），但要反馈给计划 A |
| V2 | 快照接口响应是 `{ "pipeline_runs": [...] }`（契约 C1 已约定） | Task 1 Step 3 的 curl | 不符 → 集合会报 `Fallback response missing "pipeline_runs" array`；按 C1 请计划 A 修正，**前端不做兼容** |
| V3 | 模板里三个人工关卡的 `gate_label` 恰为「需求确认」「设计规格确认」「用例设计确认」 | 启动一条流水线后 `GET /api/local/issues/<id>/pipeline`，看 `template.stages[*].gate_label` | 不同 → 改 `packages/e2e/support/pages.ts` 的 `GATE_LABELS` 常量，界面不用改（界面直接显示模板值） |
| V4 | 根字号与卡片标题实际像素 | Task 3 Step 1 | 见 Task 3 |
| V5 | vite 代理 `localhost` 在 Node ≥ 17 可能解析到 `::1`，而后端只听 `127.0.0.1` | e2e 首次跑若前端日志出现 `ECONNREFUSED ::1:4310` | 本计划已在前端 webServer 环境里加 `NODE_OPTIONS=--dns-result-order=ipv4first`；仍失败则把后端 `HOST` 改为 `localhost` |
| V6 | ~~pause / resume 的允许状态~~ **已由契约 C5 确定** | — | 界面按 C5 渲染；e2e 在人工关卡（`waiting_gate`）处暂停，时序确定 |
| V7 | ~~develop 的 checks~~ **已由契约 C3 确定**：内置模板 checks 为空，只看智能体退出码 | — | fixture 仓库不需要任何脚本 |
| V8 | CI 上后端做 git 提交需要身份 | e2e 首跑日志 | 已在 CI 步骤里 `git config --global user.name/email` |
| V9 | `/api/health` 不需要 relay 签名即可访问 | `curl -i http://127.0.0.1:$BACKEND_PORT/api/health` | 若非 2xx，把 webServer 的就绪 URL 改成 `/api/local-auth/bootstrap` |
| V10 | `VK_ASSET_DIR` 与 `backend:dev:watch:qa` 已由计划 A Task 1 实现 | Task 22 | 缺 → 回到计划 A Task 1，本计划不重复实现（避免两份实现冲突） |
| V11 | 个人版 `CreateRemoteProjectDialog` 能建本地项目 | 个人版点侧栏项目下拉「新建项目」 | 能 → 无事；不能 → 工作台无项目时的提示改成「请在看板页新建项目」，不在本计划修 |
| V12 | `PROJECTS_SHAPE` 在 local 数据源下传空参数可用 | `resolveLocalShapeEndpoint(PROJECTS_SHAPE, {})` 已有单测（`localEndpoints.test.ts:43-49`） | 已核实可用 |

---

## 5. 团队版不回退：受影响路径与回归检查点

| 改动点 | 团队版表现 | 回归检查 |
|---|---|---|
| `SharedAppLayout` 左栏 / 横幅 / 抽屉 | 条件 `isLocalPersonalMode()` 为假 → 原 `AppBar`、原横幅条件、原抽屉 | `VK_MODE=team` 登录后左侧仍是图标栏，Local/Remote/Projects 分组可见（标题变为 i18n 文案） |
| `AppBar` 分组标题 | 文案来源从常量换成 `t()`，英文界面字面相同 | 英文界面标题仍是 Local / Remote / Projects / Export |
| `useGitHubStars` / `useDiscordOnlineCount` 新增 `enabled` | 团队版传 `true`，行为不变 | 徽标仍显示 |
| `KanbanFilterBar.showViewSwitch` | 默认 `true` | Team / Personal 切换仍在 |
| `KanbanColumnHeader` / `KanbanColumnEmptyState` 新可选 props | 默认值等于旧行为 | 列头 `+`、空列新建按钮仍在 |
| `KanbanColumn` / `KanbanContainer` | `isPipelineBoard=false`，流水线集合 `enabled=false`，拖拽不拦 | 任意跨列拖拽成功；卡片底部没有进度格 |
| `KanbanIssuePanel.onOpenFullscreen` | 团队版不传，按钮不渲染 | 面板头部按钮与改造前一致 |
| `GeneralSettingsSection` 导出卡片 | 团队版不渲染 | 设置 → 常规末尾仍是「Safety」 |
| `RootRedirectPage` | 团队版走原逻辑 | 登录后进入上次项目 |
| 新路由 `/home`、`/docs`、`…/detail` | `beforeLoad` 重定向到 `/` | 手输 URL 被送回首页 |
| i18n 值改动：`kanban.newIssue` / `kanban.createNewIssue` 中文改为「新建需求」 | 团队版中文界面按钮文案从「新问题」变「新建需求」（术语统一，设计文档 §8.5） | 仅文案变化 |
| remote-web 编译 | `KanbanContainer` / `KanbanIssuePanelContainer` 被 remote-web 引用；新代码不用带类型的 `navigate({to})` 指向 local-web 独有路由，改用 `router.history.push(路径字符串)` | `pnpm run remote-web:check` 通过 |

团队版人工验收（照设计文档 §12.7）：登录、成员管理、工作区删除审批、第三方账号绑定照常。可参考记忆中的「团队版人工验收待办」清单。

---

## 6. 文件清单

**新建（web-core）**

```
packages/web-core/src/entities/pipeline/model/stages.ts            (+ stages.test.ts)
packages/web-core/src/entities/pipeline/model/progress.ts          (+ progress.test.ts)
packages/web-core/src/entities/pipeline/model/cardInfo.ts          (+ cardInfo.test.ts)
packages/web-core/src/entities/pipeline/model/time.ts              (+ time.test.ts)
packages/web-core/src/entities/pipeline/model/signature.ts         (+ signature.test.ts)
packages/web-core/src/entities/pipeline/model/__fixtures__/pipeline.ts
packages/web-core/src/entities/pipeline/model/hooks/usePipelineData.ts
packages/web-core/src/entities/pipeline/api/pipelineShapes.ts
packages/web-core/src/entities/pipeline/api/pipelineApi.ts         (+ pipelineApi.test.ts)
packages/web-core/src/features/kanban/model/pipelineColumns.ts     (+ pipelineColumns.test.ts)
packages/web-core/src/features/kanban/model/pipelineDrag.ts        (+ pipelineDrag.test.ts)
packages/web-core/src/pages/issue-detail/timeline.ts               (+ timeline.test.ts)
packages/web-core/src/pages/issue-detail/artifactParsers.ts        (+ artifactParsers.test.ts)
packages/web-core/src/pages/issue-detail/issueDetailModel.ts       (+ issueDetailModel.test.ts)
packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx
packages/web-core/src/pages/issue-detail/ArtifactSection.tsx
packages/web-core/src/pages/issue-detail/GateBarContainer.tsx
packages/web-core/src/pages/workbench/workbenchModel.ts            (+ workbenchModel.test.ts)
packages/web-core/src/pages/workbench/WorkbenchPage.tsx
packages/web-core/src/pages/workbench/WorkbenchComposerContainer.tsx
packages/web-core/src/pages/docs/DocsPlaceholderPage.tsx
packages/web-core/src/shared/lib/routes/personalRoutes.ts          (+ personalRoutes.test.ts)
```

**新建（ui，无状态视图）**

```
packages/ui/src/components/PersonalSidebar.tsx
packages/ui/src/components/PipelineProgressBar.tsx
packages/ui/src/components/PipelineStatusTag.tsx
packages/ui/src/components/PipelineStageBadge.tsx
packages/ui/src/components/PipelineStepper.tsx
packages/ui/src/components/PipelineGateBar.tsx
packages/ui/src/components/PipelineTimeline.tsx
packages/ui/src/components/CsvTableView.tsx
packages/ui/src/components/WorkbenchComposer.tsx
packages/ui/src/components/WorkbenchColumn.tsx
packages/ui/src/components/WorkbenchCard.tsx
```

**新建（路由 / e2e / CI）**

```
packages/local-web/src/routes/_app.home.tsx
packages/local-web/src/routes/_app.docs.tsx
packages/local-web/src/routes/_app.projects.$projectId_.issues.$issueId_.detail.tsx
packages/e2e/package.json
packages/e2e/tsconfig.json
packages/e2e/playwright.config.ts
packages/e2e/.gitignore
packages/e2e/support/env.ts
packages/e2e/support/api.ts
packages/e2e/support/fixtures.ts
packages/e2e/support/pages.ts
packages/e2e/tests/golden-path.spec.ts
packages/e2e/tests/exceptions.spec.ts
packages/e2e/tests/visual.spec.ts
```

**修改**

```
packages/web-core/src/i18n/locales/{en,zh-Hans,zh-Hant,es,fr,ja,ko}/common.json
packages/web-core/src/i18n/locales/{en,zh-Hans,zh-Hant,es,fr,ja,ko}/settings.json
packages/web-core/src/app/styles/new/index.css
packages/local-web/tailwind.new.config.js
packages/web-core/src/shared/lib/local/localEndpoints.ts           (+ localEndpoints.test.ts)
packages/web-core/src/shared/hooks/useGitHubStars.ts
packages/web-core/src/shared/hooks/useDiscordOnlineCount.ts
packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx
packages/web-core/src/pages/root/RootRedirectPage.tsx
packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx
packages/web-core/src/shared/dialogs/settings/settings/GeneralSettingsSection.tsx
packages/web-core/src/features/kanban/ui/KanbanColumn.tsx
packages/web-core/src/features/kanban/ui/KanbanContainer.tsx
packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx
packages/ui/src/components/AppBar.tsx
packages/ui/src/components/KanbanColumnHeader.tsx
packages/ui/src/components/KanbanColumnEmptyState.tsx
packages/ui/src/components/KanbanFilterBar.tsx
packages/ui/src/components/KanbanIssuePanel.tsx
packages/local-web/src/routeTree.gen.ts                            (vite 插件生成，不手改)
package.json
.github/workflows/test.yml
```

---

## 7. 任务总览

| # | 标题 | 步骤数 |
|---|---|---|
| 1 | 前置核对：计划 A 的产物 | 4 |
| 2 | i18n 词条（七种语言） | 6 |
| 3 | 设计令牌：阶段语义色（+ 字号核实） | 6 |
| 4 | 阶段元数据与测试夹具 | 5 |
| 5 | 进度格、状态文案、卡片信息 | 8 |
| 6 | 耗时 / 相对时间 / 指纹 | 5 |
| 7 | 看板列文案与拖拽判定 | 6 |
| 8 | 时间线 | 5 |
| 9 | 产出物解析（CSV / 评审 / 测试报告） | 5 |
| 10 | 详情页签与关卡条状态 | 5 |
| 11 | 工作台纯函数 | 5 |
| 12 | 数据层：集合、接口封装、hooks | 10 |
| 13 | 外壳：AppBar 标题国际化 + 外网徽标开关 | 5 |
| 14 | 外壳：个人版侧栏视图与导航纯函数 | 6 |
| 15 | 外壳：SharedAppLayout 按模式切换 | 6 |
| 16 | 设置：导出入口 | 4 |
| 17 | 路由：工作台 / 文档 / 详情 + 根跳转 | 7 |
| 18 | 流水线视图组件（ui） | 6 |
| 19 | 看板改造 | 9 |
| 20 | 工作台页面 | 6 |
| 21 | 需求详情全屏页 + 面板入口 | 8 |
| 22 | 后端测试支撑核对（`VK_ASSET_DIR`、`dev:qa`，计划 A 实现） | 2 |
| 23 | e2e 包骨架与 fixtures | 9 |
| 24 | e2e 黄金路径 | 4 |
| 25 | e2e 异常路径（打回、轮次用尽、用例回流、暂停继续、刷新恢复） | 6 |
| 26 | 视觉回归 | 5 |
| 27 | CI：e2e 任务 | 4 |
| 28 | 收尾：format / check / lint / e2e / 团队版回归 | 6 |

---

## Task 1: 前置核对——计划 A 的产物

**Files:** 无改动（只读核对）

- [ ] **Step 1: 类型已生成（V1）**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
grep -nE "export type (PipelineRun|PipelineStageRun|PipelineGateDecision|IssueArtifactSummary|IssueArtifact|PipelineTemplateStageView|PipelineTemplateView|IssuePipelineView|StartPipelineRepo|StartPipelineRequest|GateDecisionRequest|PendingPipelineItem|PipelineStageKey|PipelineRunStatus|PipelineStageStatus|GateKind|GateDecisionKind|ArtifactKind) =" shared/types.ts | wc -l
```
Expected: `18`。少于 18 → 停止，等计划 A 完成 `generate_types.rs` 注册。

再看一眼字段类型（只记录，不影响后续）：
```bash
grep -n "export type PipelineStageRun =" shared/types.ts
```
Expected（契约 C2）: 一行，含 `attempt: number` 与 `started_at: string | null`。若仍是 `bigint` / `Date`，本计划照样能跑（统一经 `toNumber()` / `toMillis()` 读），但要反馈给计划 A。

- [ ] **Step 2: 后端路由已挂（契约 §2、§3）**

Run:
```bash
grep -rnE "pipeline_runs|pipeline_stage_runs|stage-runs|/pipeline/pending|/pipeline/artifacts" crates/server/src/routes/local_projects/ | wc -l
```
Expected: 大于等于 `6`（快照两条 + 接口若干）。为 0 → 停止，等计划 A。

- [ ] **Step 3: 快照响应格式（V2）**

在一个终端起后端（计划 A 完成后即可）：
```bash
BACKEND_PORT=4399 cargo run --bin server
```
另一个终端：
```bash
PID=$(curl -s http://127.0.0.1:4399/api/local/projects | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const p=JSON.parse(s).projects;console.log(p[0]?.id??"")})')
curl -s "http://127.0.0.1:4399/api/local/pipeline_runs?project_id=$PID" | head -c 200; echo
curl -s "http://127.0.0.1:4399/api/local/pipeline_stage_runs?project_id=$PID" | head -c 200; echo
```
Expected: 两行分别以 `{"pipeline_runs":[` 与 `{"pipeline_stage_runs":[` 开头。若不是 → 按 §4 V2 请计划 A 修正，本计划不做兼容。停掉后端（Ctrl+C）。

- [ ] **Step 4: 条件项（V10）记录**

Run:
```bash
grep -n "VK_ASSET_DIR" crates/utils/src/assets.rs; grep -n '"backend:dev:watch:qa"' package.json
```
两条都应有输出（计划 A Task 1 的产物）；Task 22 会再核对一次。本任务不提交。

---

## Task 2: i18n 词条（七种语言）

**Files:**
- Modify: `packages/web-core/src/i18n/locales/{en,zh-Hans,zh-Hant,es,fr,ja,ko}/common.json`
- Modify: `packages/web-core/src/i18n/locales/{en,zh-Hans,zh-Hant,es,fr,ja,ko}/settings.json`

本任务一次性加齐后续所有任务用到的词条（后续任务不再改 locale 文件）。es / fr / ja / ko 用英文值。

- [ ] **Step 1: 写英文片段**

Create `/tmp/vk-i18n/en.common.json`:
```json
{
  "appBar": {
    "sections": {
      "local": "Local",
      "remote": "Remote",
      "projects": "Projects",
      "export": "Export"
    }
  },
  "nav": {
    "workbench": "Workbench",
    "pipeline": "Pipeline",
    "testing": "Test center",
    "docs": "Docs",
    "settings": "Settings",
    "projectSwitcher": "Switch project",
    "createProject": "New project",
    "noProject": "No project yet",
    "footer": "Personal · local v{{version}}"
  },
  "pipeline": {
    "stage": {
      "requirement": "Requirement",
      "spec": "Spec",
      "test_design": "Test design",
      "develop": "Develop",
      "review": "Review",
      "test": "Test",
      "deliver": "Deliver"
    },
    "gateLabel": {
      "requirement": "Requirement review",
      "spec": "Spec review",
      "test_design": "Test design review"
    },
    "column": {
      "backlog": "Requirements",
      "todo": "Spec · Cases",
      "dev": "Develop",
      "review": "Review",
      "test": "Test",
      "done": "Deliver",
      "humanGate": "Human gate",
      "auto": "Auto"
    },
    "columnEmpty": {
      "backlog": "Describe a requirement on the workbench and it shows up here",
      "todo": "Once the requirement is confirmed, AI writes the spec and test cases here",
      "dev": "Development starts automatically once test cases are confirmed",
      "review": "AI reviews automatically, usually in under 3 minutes",
      "test": "Tests run automatically after the review passes",
      "done": "Delivered automatically once every test passes"
    },
    "status": {
      "waitingGate": "Waiting for you · {{gate}}",
      "running": "{{stage}} · in progress",
      "runningRound": "{{stage}} · round {{attempt}}/{{max}}",
      "paused": "Paused",
      "failed": "Needs your attention",
      "completed": "Delivered",
      "cancelled": "Cancelled"
    },
    "cell": {
      "done": "Done",
      "running": "In progress",
      "gate": "Waiting for you",
      "failed": "Failed",
      "pending": "Not started",
      "paused": "Paused",
      "cancelled": "Cancelled"
    },
    "progressLabel": "Pipeline progress: {{summary}}",
    "drag": {
      "forwardBlocked": "Cards can't be moved forward: gates decide progress",
      "pauseFirst": "The pipeline is running. Pause it before sending the card back"
    },
    "gate": {
      "confirm": "Confirm and continue",
      "reject": "Send back with notes",
      "rejectPlaceholder": "Describe what needs to change (required)",
      "rejectSubmit": "Send back",
      "cancel": "Cancel",
      "pause": "Pause automation",
      "resume": "Resume",
      "autoGate": "Auto gate",
      "runningHint": "AI is working on: {{stage}}",
      "round": "Round {{attempt}}/{{max}}",
      "failedHint": "Automatic fix rounds are used up. Please take over",
      "pausedHint": "Automation is paused",
      "completedHint": "Delivered. The PR is not merged automatically",
      "cancelledHint": "This pipeline was cancelled",
      "error": "Action failed: {{message}}",
      "autoCondition": {
        "checksPassed": "Moves on automatically once lint and unit tests pass",
        "noBlockingFindings": "Moves on automatically once the review has no blocking findings",
        "allCasesPassed": "Delivers automatically once every case passes",
        "artifactsPresent": "Completes once the delivery report is generated"
      }
    },
    "duration": {
      "seconds": "{{s}}s",
      "minutesSeconds": "{{m}}m {{s}}s",
      "hoursMinutes": "{{h}}h {{m}}m"
    },
    "time": {
      "justNow": "just now",
      "minutesAgo": "{{n}} min ago",
      "hoursAgo": "{{n}} h ago",
      "daysAgo": "{{n}} d ago"
    }
  },
  "workbench": {
    "greeting": {
      "morning": "Good morning",
      "afternoon": "Good afternoon",
      "evening": "Good evening"
    },
    "summary": "{{stages}} stages finished automatically today, {{pending}} items waiting for you",
    "composer": {
      "label": "Describe a requirement",
      "placeholder": "Describe a requirement, or paste a PRD / Feishu doc link…",
      "example": "e.g. Add a minimum order amount check to the order API: below 5 USDT return 400 with ORDER_MIN_NOTIONAL",
      "repo": "Repository",
      "branch": "Branch",
      "template": "Template",
      "templateStandard": "Standard seven stages",
      "agent": "Agent",
      "submit": "Start",
      "submitting": "Starting…",
      "noRepo": "Add a repository in Settings → Repositories first",
      "noProject": "Create a project first",
      "error": "Failed to start: {{message}}"
    },
    "columns": {
      "confirm": "Needs your confirmation",
      "running": "Running automatically",
      "delivered": "Recently delivered"
    },
    "empty": {
      "confirm": "Nothing is waiting for you",
      "running": "No pipeline is running",
      "delivered": "Nothing delivered yet"
    },
    "card": {
      "open": "Open"
    },
    "weekly": "This week: {{delivered}} delivered · avg cycle {{avg}} min · first-pass rate {{rate}}%",
    "weeklyEmpty": "This week: nothing delivered yet"
  },
  "docs": {
    "title": "Docs",
    "emptyTitle": "Specs and requirement documents will show up here",
    "emptyHint": "Once a pipeline finishes the requirement and spec stages, its documents are filed here by requirement."
  },
  "issueDetail": {
    "breadcrumb": "Pipeline",
    "fullscreen": "Open full screen",
    "notFound": "This requirement could not be found",
    "noPipeline": "No pipeline has been started for this requirement. Start one from the workbench.",
    "noArtifact": "This stage has no artifact yet",
    "truncated": "Content is too long; only the first 256 KB is shown",
    "version": "v{{version}}",
    "loading": "Loading…",
    "openWorkspace": "Open workspace",
    "noWorkspace": "A workspace is created when development starts",
    "testSummary": "{{passed}} passed · {{failed}} failed · {{total}} total",
    "reviewEmpty": "No review findings",
    "parseError": "This artifact could not be parsed; showing the raw content",
    "tab": {
      "requirement": "Requirement & acceptance",
      "spec": "Spec",
      "cases": "Test cases",
      "code": "Code changes",
      "test": "Tests",
      "delivery": "Delivery report"
    },
    "side": {
      "timeline": "Timeline",
      "artifacts": "Artifacts",
      "usage": "Usage",
      "usageNotReady": "Usage statistics arrive in a later release"
    },
    "artifactKind": {
      "requirement": "Requirement",
      "spec": "Spec",
      "plan": "Plan",
      "test_cases": "Test cases",
      "trace_matrix": "Trace matrix",
      "review": "Review",
      "test_report": "Test report",
      "delivery_report": "Delivery report"
    },
    "testCols": {
      "case": "Case",
      "status": "Result",
      "attribution": "Attribution",
      "message": "Notes"
    },
    "attribution": {
      "code": "Code defect",
      "case": "Test case issue"
    },
    "caseStatus": {
      "passed": "Passed",
      "failed": "Failed",
      "unknown": "Unknown"
    },
    "severity": {
      "blocker": "Blocker",
      "major": "Major",
      "minor": "Minor",
      "unknown": "Other"
    },
    "timeline": {
      "stageStarted": "{{stage}} started (attempt {{attempt}})",
      "stagePassed": "{{stage}} finished",
      "stageFailed": "{{stage}} failed",
      "stageWaiting": "{{stage}} is waiting for confirmation",
      "stageRejected": "{{stage}} was sent back",
      "gateApproved": "You confirmed {{stage}}",
      "gateRejected": "You sent back {{stage}}",
      "actorAi": "AI",
      "actorHuman": "You",
      "empty": "Nothing has happened yet"
    }
  }
}
```

Create `/tmp/vk-i18n/en.settings.json`:
```json
{
  "settings": {
    "general": {
      "export": {
        "title": "Data export",
        "description": "Export project data from the cloud service (available when signed in to a cloud account)",
        "button": "Open export page"
      }
    }
  }
}
```

- [ ] **Step 2: 写简体中文片段**

Create `/tmp/vk-i18n/zh-Hans.common.json`:
```json
{
  "appBar": {
    "sections": {
      "local": "本地",
      "remote": "远程",
      "projects": "项目",
      "export": "导出"
    }
  },
  "kanban": {
    "newIssue": "新建需求",
    "createNewIssue": "新建需求"
  },
  "nav": {
    "workbench": "工作台",
    "pipeline": "需求流水线",
    "testing": "测试中心",
    "docs": "文档",
    "settings": "设置",
    "projectSwitcher": "切换项目",
    "createProject": "新建项目",
    "noProject": "还没有项目",
    "footer": "个人版 · 本机 v{{version}}"
  },
  "pipeline": {
    "stage": {
      "requirement": "需求",
      "spec": "设计规格",
      "test_design": "用例设计",
      "develop": "开发",
      "review": "评审",
      "test": "测试",
      "deliver": "交付"
    },
    "gateLabel": {
      "requirement": "需求确认",
      "spec": "设计规格确认",
      "test_design": "用例设计确认"
    },
    "column": {
      "backlog": "需求",
      "todo": "规格 · 用例",
      "dev": "开发",
      "review": "评审",
      "test": "测试",
      "done": "交付",
      "humanGate": "人工确认",
      "auto": "自动"
    },
    "columnEmpty": {
      "backlog": "在工作台描述一个需求，它会出现在这里",
      "todo": "需求确认后，AI 会在这里写规格与用例",
      "dev": "用例确认后自动进入开发",
      "review": "评审由 AI 自动完成，一般停留不到 3 分钟",
      "test": "评审通过后自动跑测试",
      "done": "测试全部通过后自动交付"
    },
    "status": {
      "waitingGate": "等你确认 · {{gate}}",
      "running": "{{stage}} · 进行中",
      "runningRound": "{{stage}} · 第 {{attempt}}/{{max}} 轮",
      "paused": "已暂停",
      "failed": "需要你处理",
      "completed": "已交付",
      "cancelled": "已取消"
    },
    "cell": {
      "done": "已完成",
      "running": "进行中",
      "gate": "等人工",
      "failed": "失败",
      "pending": "未开始",
      "paused": "已暂停",
      "cancelled": "已取消"
    },
    "progressLabel": "流水线进度：{{summary}}",
    "drag": {
      "forwardBlocked": "不能往后拖：推进由关卡决定",
      "pauseFirst": "流水线正在运行，请先暂停再打回"
    },
    "gate": {
      "confirm": "确认并继续",
      "reject": "打回并说明",
      "rejectPlaceholder": "写清需要改什么（必填）",
      "rejectSubmit": "提交打回",
      "cancel": "取消",
      "pause": "暂停自动化",
      "resume": "继续",
      "autoGate": "自动关卡",
      "runningHint": "AI 正在处理：{{stage}}",
      "round": "第 {{attempt}}/{{max}} 轮",
      "failedHint": "自动修复轮次已用尽，需要你处理",
      "pausedHint": "自动化已暂停",
      "completedHint": "已交付，PR 不会自动合并",
      "cancelledHint": "这条流水线已取消",
      "error": "操作失败：{{message}}",
      "autoCondition": {
        "checksPassed": "lint 与单测全部通过后自动进入下一阶段",
        "noBlockingFindings": "评审没有阻断问题后自动进入下一阶段",
        "allCasesPassed": "用例全部通过后自动交付",
        "artifactsPresent": "交付报告生成后完成"
      }
    },
    "duration": {
      "seconds": "{{s}} 秒",
      "minutesSeconds": "{{m}} 分 {{s}} 秒",
      "hoursMinutes": "{{h}} 小时 {{m}} 分"
    },
    "time": {
      "justNow": "刚刚",
      "minutesAgo": "{{n}} 分钟前",
      "hoursAgo": "{{n}} 小时前",
      "daysAgo": "{{n}} 天前"
    }
  },
  "workbench": {
    "greeting": {
      "morning": "上午好",
      "afternoon": "下午好",
      "evening": "晚上好"
    },
    "summary": "今天自动完成 {{stages}} 个阶段，{{pending}} 件事等你确认",
    "composer": {
      "label": "描述一个需求",
      "placeholder": "描述一个需求，或粘贴 PRD / 飞书文档链接……",
      "example": "例：下单接口增加最小下单金额校验，低于 5 USDT 返回 400 与错误码 ORDER_MIN_NOTIONAL",
      "repo": "仓库",
      "branch": "分支",
      "template": "流程模板",
      "templateStandard": "标准七阶段",
      "agent": "智能体",
      "submit": "开始",
      "submitting": "正在启动……",
      "noRepo": "先在 设置 → 仓库 里添加一个仓库",
      "noProject": "先新建一个项目",
      "error": "启动失败：{{message}}"
    },
    "columns": {
      "confirm": "需要你确认",
      "running": "正在自动跑",
      "delivered": "最近交付"
    },
    "empty": {
      "confirm": "没有等你确认的事",
      "running": "没有正在跑的流水线",
      "delivered": "还没有交付记录"
    },
    "card": {
      "open": "查看"
    },
    "weekly": "本周：交付 {{delivered}} · 平均周期 {{avg}} 分 · 一次通过率 {{rate}}%",
    "weeklyEmpty": "本周：还没有交付"
  },
  "docs": {
    "title": "文档",
    "emptyTitle": "规格与需求文档会出现在这里",
    "emptyHint": "流水线跑完需求与设计规格阶段后，产出的文档会按需求归档到这一页。"
  },
  "issueDetail": {
    "breadcrumb": "需求流水线",
    "fullscreen": "全屏查看",
    "notFound": "找不到这个需求",
    "noPipeline": "这个需求还没有启动流水线，请在工作台启动。",
    "noArtifact": "这个阶段还没有产出物",
    "truncated": "内容过长，只显示前 256 KB",
    "version": "第 {{version}} 版",
    "loading": "加载中……",
    "openWorkspace": "打开工作区",
    "noWorkspace": "开发阶段开始后会创建工作区",
    "testSummary": "{{passed}} 通过 · {{failed}} 失败 · 共 {{total}}",
    "reviewEmpty": "没有评审意见",
    "parseError": "产出物无法解析，下面显示原文",
    "tab": {
      "requirement": "需求与验收标准",
      "spec": "规格",
      "cases": "用例",
      "code": "代码变更",
      "test": "测试",
      "delivery": "交付报告"
    },
    "side": {
      "timeline": "时间线",
      "artifacts": "产出物",
      "usage": "用量",
      "usageNotReady": "用量统计将在后续版本提供"
    },
    "artifactKind": {
      "requirement": "需求文档",
      "spec": "设计规格",
      "plan": "实施计划",
      "test_cases": "测试用例",
      "trace_matrix": "追踪矩阵",
      "review": "评审结果",
      "test_report": "测试报告",
      "delivery_report": "交付报告"
    },
    "testCols": {
      "case": "用例",
      "status": "结果",
      "attribution": "归因",
      "message": "说明"
    },
    "attribution": {
      "code": "代码缺陷",
      "case": "用例问题"
    },
    "caseStatus": {
      "passed": "通过",
      "failed": "失败",
      "unknown": "未知"
    },
    "severity": {
      "blocker": "阻断",
      "major": "重要",
      "minor": "次要",
      "unknown": "其它"
    },
    "timeline": {
      "stageStarted": "{{stage}}开始（第 {{attempt}} 次）",
      "stagePassed": "{{stage}}完成",
      "stageFailed": "{{stage}}失败",
      "stageWaiting": "{{stage}}等待确认",
      "stageRejected": "{{stage}}被打回",
      "gateApproved": "你确认了{{stage}}",
      "gateRejected": "你打回了{{stage}}",
      "actorAi": "AI",
      "actorHuman": "你",
      "empty": "还没有动静"
    }
  }
}
```

Create `/tmp/vk-i18n/zh-Hans.settings.json`:
```json
{
  "settings": {
    "general": {
      "export": {
        "title": "数据导出",
        "description": "导出云端服务里的项目数据（登录云端账号时可用）",
        "button": "打开导出页"
      }
    }
  }
}
```

- [ ] **Step 3: 写繁体中文片段**

Create `/tmp/vk-i18n/zh-Hant.common.json`:
```json
{
  "appBar": {
    "sections": {
      "local": "本機",
      "remote": "遠端",
      "projects": "專案",
      "export": "匯出"
    }
  },
  "kanban": {
    "newIssue": "新增需求",
    "createNewIssue": "新增需求"
  },
  "nav": {
    "workbench": "工作台",
    "pipeline": "需求流水線",
    "testing": "測試中心",
    "docs": "文件",
    "settings": "設定",
    "projectSwitcher": "切換專案",
    "createProject": "新增專案",
    "noProject": "還沒有專案",
    "footer": "個人版 · 本機 v{{version}}"
  },
  "pipeline": {
    "stage": {
      "requirement": "需求",
      "spec": "設計規格",
      "test_design": "用例設計",
      "develop": "開發",
      "review": "評審",
      "test": "測試",
      "deliver": "交付"
    },
    "gateLabel": {
      "requirement": "需求確認",
      "spec": "設計規格確認",
      "test_design": "用例設計確認"
    },
    "column": {
      "backlog": "需求",
      "todo": "規格 · 用例",
      "dev": "開發",
      "review": "評審",
      "test": "測試",
      "done": "交付",
      "humanGate": "人工確認",
      "auto": "自動"
    },
    "columnEmpty": {
      "backlog": "在工作台描述一個需求，它會出現在這裡",
      "todo": "需求確認後，AI 會在這裡寫規格與用例",
      "dev": "用例確認後自動進入開發",
      "review": "評審由 AI 自動完成，一般停留不到 3 分鐘",
      "test": "評審通過後自動跑測試",
      "done": "測試全部通過後自動交付"
    },
    "status": {
      "waitingGate": "等你確認 · {{gate}}",
      "running": "{{stage}} · 進行中",
      "runningRound": "{{stage}} · 第 {{attempt}}/{{max}} 輪",
      "paused": "已暫停",
      "failed": "需要你處理",
      "completed": "已交付",
      "cancelled": "已取消"
    },
    "cell": {
      "done": "已完成",
      "running": "進行中",
      "gate": "等人工",
      "failed": "失敗",
      "pending": "未開始",
      "paused": "已暫停",
      "cancelled": "已取消"
    },
    "progressLabel": "流水線進度：{{summary}}",
    "drag": {
      "forwardBlocked": "不能往後拖：推進由關卡決定",
      "pauseFirst": "流水線正在執行，請先暫停再退回"
    },
    "gate": {
      "confirm": "確認並繼續",
      "reject": "退回並說明",
      "rejectPlaceholder": "寫清需要改什麼（必填）",
      "rejectSubmit": "提交退回",
      "cancel": "取消",
      "pause": "暫停自動化",
      "resume": "繼續",
      "autoGate": "自動關卡",
      "runningHint": "AI 正在處理：{{stage}}",
      "round": "第 {{attempt}}/{{max}} 輪",
      "failedHint": "自動修復輪次已用盡，需要你處理",
      "pausedHint": "自動化已暫停",
      "completedHint": "已交付，PR 不會自動合併",
      "cancelledHint": "這條流水線已取消",
      "error": "操作失敗：{{message}}",
      "autoCondition": {
        "checksPassed": "lint 與單元測試全部通過後自動進入下一階段",
        "noBlockingFindings": "評審沒有阻斷問題後自動進入下一階段",
        "allCasesPassed": "用例全部通過後自動交付",
        "artifactsPresent": "交付報告產生後完成"
      }
    },
    "duration": {
      "seconds": "{{s}} 秒",
      "minutesSeconds": "{{m}} 分 {{s}} 秒",
      "hoursMinutes": "{{h}} 小時 {{m}} 分"
    },
    "time": {
      "justNow": "剛剛",
      "minutesAgo": "{{n}} 分鐘前",
      "hoursAgo": "{{n}} 小時前",
      "daysAgo": "{{n}} 天前"
    }
  },
  "workbench": {
    "greeting": {
      "morning": "早安",
      "afternoon": "午安",
      "evening": "晚安"
    },
    "summary": "今天自動完成 {{stages}} 個階段，{{pending}} 件事等你確認",
    "composer": {
      "label": "描述一個需求",
      "placeholder": "描述一個需求，或貼上 PRD / 飛書文件連結……",
      "example": "例：下單介面增加最小下單金額校驗，低於 5 USDT 回傳 400 與錯誤碼 ORDER_MIN_NOTIONAL",
      "repo": "儲存庫",
      "branch": "分支",
      "template": "流程範本",
      "templateStandard": "標準七階段",
      "agent": "智慧體",
      "submit": "開始",
      "submitting": "正在啟動……",
      "noRepo": "先在 設定 → 儲存庫 裡新增一個儲存庫",
      "noProject": "先新增一個專案",
      "error": "啟動失敗：{{message}}"
    },
    "columns": {
      "confirm": "需要你確認",
      "running": "正在自動執行",
      "delivered": "最近交付"
    },
    "empty": {
      "confirm": "沒有等你確認的事",
      "running": "沒有正在執行的流水線",
      "delivered": "還沒有交付紀錄"
    },
    "card": {
      "open": "檢視"
    },
    "weekly": "本週：交付 {{delivered}} · 平均週期 {{avg}} 分 · 一次通過率 {{rate}}%",
    "weeklyEmpty": "本週：還沒有交付"
  },
  "docs": {
    "title": "文件",
    "emptyTitle": "規格與需求文件會出現在這裡",
    "emptyHint": "流水線跑完需求與設計規格階段後，產出的文件會按需求歸檔到這一頁。"
  },
  "issueDetail": {
    "breadcrumb": "需求流水線",
    "fullscreen": "全螢幕檢視",
    "notFound": "找不到這個需求",
    "noPipeline": "這個需求還沒有啟動流水線，請在工作台啟動。",
    "noArtifact": "這個階段還沒有產出物",
    "truncated": "內容過長，只顯示前 256 KB",
    "version": "第 {{version}} 版",
    "loading": "載入中……",
    "openWorkspace": "開啟工作區",
    "noWorkspace": "開發階段開始後會建立工作區",
    "testSummary": "{{passed}} 通過 · {{failed}} 失敗 · 共 {{total}}",
    "reviewEmpty": "沒有評審意見",
    "parseError": "產出物無法解析，下面顯示原文",
    "tab": {
      "requirement": "需求與驗收標準",
      "spec": "規格",
      "cases": "用例",
      "code": "程式碼變更",
      "test": "測試",
      "delivery": "交付報告"
    },
    "side": {
      "timeline": "時間線",
      "artifacts": "產出物",
      "usage": "用量",
      "usageNotReady": "用量統計將在後續版本提供"
    },
    "artifactKind": {
      "requirement": "需求文件",
      "spec": "設計規格",
      "plan": "實施計畫",
      "test_cases": "測試用例",
      "trace_matrix": "追蹤矩陣",
      "review": "評審結果",
      "test_report": "測試報告",
      "delivery_report": "交付報告"
    },
    "testCols": {
      "case": "用例",
      "status": "結果",
      "attribution": "歸因",
      "message": "說明"
    },
    "attribution": {
      "code": "程式缺陷",
      "case": "用例問題"
    },
    "caseStatus": {
      "passed": "通過",
      "failed": "失敗",
      "unknown": "未知"
    },
    "severity": {
      "blocker": "阻斷",
      "major": "重要",
      "minor": "次要",
      "unknown": "其他"
    },
    "timeline": {
      "stageStarted": "{{stage}}開始（第 {{attempt}} 次）",
      "stagePassed": "{{stage}}完成",
      "stageFailed": "{{stage}}失敗",
      "stageWaiting": "{{stage}}等待確認",
      "stageRejected": "{{stage}}被退回",
      "gateApproved": "你確認了{{stage}}",
      "gateRejected": "你退回了{{stage}}",
      "actorAi": "AI",
      "actorHuman": "你",
      "empty": "還沒有動靜"
    }
  }
}
```

Create `/tmp/vk-i18n/zh-Hant.settings.json`:
```json
{
  "settings": {
    "general": {
      "export": {
        "title": "資料匯出",
        "description": "匯出雲端服務裡的專案資料（登入雲端帳號時可用）",
        "button": "開啟匯出頁"
      }
    }
  }
}
```

- [ ] **Step 4: 深合并进七种语言**

Run（仓库根目录）:
```bash
node --input-type=module <<'EOF'
import fs from 'node:fs';
const root = 'packages/web-core/src/i18n/locales';
const source = { en: 'en', 'zh-Hans': 'zh-Hans', 'zh-Hant': 'zh-Hant', es: 'en', fr: 'en', ja: 'en', ko: 'en' };
const isObj = (v) => v && typeof v === 'object' && !Array.isArray(v);
function merge(target, patch) {
  for (const [key, value] of Object.entries(patch)) {
    target[key] = isObj(value) ? merge(isObj(target[key]) ? target[key] : {}, value) : value;
  }
  return target;
}
for (const [lang, src] of Object.entries(source)) {
  for (const ns of ['common', 'settings']) {
    const file = `${root}/${lang}/${ns}.json`;
    const data = JSON.parse(fs.readFileSync(file, 'utf8'));
    const patch = JSON.parse(fs.readFileSync(`/tmp/vk-i18n/${src}.${ns}.json`, 'utf8'));
    fs.writeFileSync(file, JSON.stringify(merge(data, patch), null, 2) + '\n');
  }
}
console.log('merged');
EOF
pnpm --filter @vibe/web-core exec prettier --write "src/i18n/locales/**/*.json"
```
Expected: 打印 `merged`，prettier 列出 14 个文件。

> 注意：en 片段不含 `kanban.newIssue`，所以 es/fr/ja/ko 与 en 的 `kanban.newIssue` 不变；只有简繁中文改成「新建需求 / 新增需求」。

- [ ] **Step 5: 校验 key 一致**

Run:
```bash
GITHUB_BASE_REF=main ./scripts/check-i18n.sh 2>&1 | tail -6
```
Expected: 最后两行含 `✅ No duplicate keys found in JSON files.` 与 `✅ Translation keys are consistent across locales.`（字面量统计那一段本任务不影响）。

- [ ] **Step 6: 提交**

```bash
git add packages/web-core/src/i18n/locales
git commit -m "$(cat <<'EOF'
界面：补齐个人版流水线界面的七语言词条

新增导航、流水线、工作台、文档、需求详情与设置导出卡片的词条；
简繁中文把「新问题」统一为「新建需求」。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 设计令牌——阶段语义色（+ 字号核实）

**Files:**
- Modify: `packages/web-core/src/app/styles/new/index.css:83`（亮色）、`:142`（暗色）
- Modify: `packages/local-web/tailwind.new.config.js:118`

- [ ] **Step 1: 实测字号（V4）**

`pnpm run dev` 起开发环境，浏览器打开任一项目看板，开发者工具控制台执行：
```js
[getComputedStyle(document.documentElement).fontSize, getComputedStyle(document.querySelector('.text-base')).fontSize]
```
Expected（按代码推断）: `['16px', '16px']`。

- 结果是 `16px`（分支 B，默认）：设计文档「12px → 13px」的前提不成立，**本任务只加颜色令牌，不动字号**；在汇报里说明。跳过 Step 4 的字号部分。
- 结果是 `12px`（分支 A）：Step 4 额外加 `comfy`（13px）/`dense`（12px）字号令牌，并在 Task 19 Step 6 把卡片标题的 `text-base` 换成密度字号（Task 19 有对应分支说明）。

- [ ] **Step 2: 加亮色阶段令牌**

Modify `packages/web-core/src/app/styles/new/index.css`，在 `:83` 的 `--merged: 271 81% 46%;` 之后插入：
```css

    /* Pipeline stage semantic colors (light) — 设计文档 §8.5：
       灰=需求/规格、蓝=开发、紫=评审、琥珀=测试、绿=完成、红=失败。
       等人工一律用 --brand（橙）。 */
    --stage-neutral: 220 9% 58%;
    --stage-dev: 217 91% 52%;
    --stage-review: 271 70% 56%;
    --stage-test: 38 92% 45%;
    --stage-done: 142 64% 36%;
    --stage-failed: 0 72% 50%;
```

- [ ] **Step 3: 加暗色阶段令牌**

同文件 `.dark` 块内，在 `--merged: 271 81% 66%;`（原 `:142`，插入 Step 2 后行号 +9）之后插入：
```css

    /* Pipeline stage semantic colors (dark) */
    --stage-neutral: 220 9% 52%;
    --stage-dev: 217 91% 64%;
    --stage-review: 271 75% 70%;
    --stage-test: 38 92% 56%;
    --stage-done: 142 55% 48%;
    --stage-failed: 0 75% 62%;
```

- [ ] **Step 4: Tailwind 颜色（分支 A 另加字号）**

Modify `packages/local-web/tailwind.new.config.js`，在 `:118` 的 `merged: "hsl(var(--merged))",` 之后插入：
```js
        // Pipeline stage semantic colors (设计文档 §8.5)
        'stage-neutral': "hsl(var(--stage-neutral))",
        'stage-dev': "hsl(var(--stage-dev))",
        'stage-review': "hsl(var(--stage-review))",
        'stage-test': "hsl(var(--stage-test))",
        'stage-done': "hsl(var(--stage-done))",
        'stage-failed': "hsl(var(--stage-failed))",
```

**仅分支 A**：在 `fontSize` 块（`:90-96`）的 `cta:` 行之前插入：
```js
        dense: ['0.75rem', { lineHeight: '1.125rem' }],   // 12px（紧凑）
        comfy: ['0.8125rem', { lineHeight: '1.25rem' }],  // 13px（舒适，默认）
```

- [ ] **Step 5: 验证编译**

Run:
```bash
cd packages/local-web && pnpm exec vite build --logLevel error >/dev/null && grep -o "stage-failed" dist/assets/*.css | head -1; cd ../..
```
Expected: 构建成功；此时还没有类使用这些颜色，`grep` 无输出是正常的（Task 18 之后再构建会有）。只要构建不报错即可。

- [ ] **Step 6: 提交**

```bash
git add packages/web-core/src/app/styles/new/index.css packages/local-web/tailwind.new.config.js
git commit -m "$(cat <<'EOF'
界面：新增流水线阶段语义色令牌（亮暗两套）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: 阶段元数据与测试夹具

**Files:**
- Create: `packages/web-core/src/entities/pipeline/model/stages.ts`
- Create: `packages/web-core/src/entities/pipeline/model/__fixtures__/pipeline.ts`
- Test: `packages/web-core/src/entities/pipeline/model/stages.test.ts`

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/entities/pipeline/model/stages.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  PIPELINE_STAGE_KEYS,
  autoConditionLabelKey,
  boardStageOf,
  fallbackGateLabelKey,
  isPipelineStageKey,
  pipelineStageLabelKey,
  pipelineStageTone,
} from './stages';

describe('PIPELINE_STAGE_KEYS', () => {
  it('是标准七阶段，顺序固定', () => {
    expect(PIPELINE_STAGE_KEYS).toEqual([
      'requirement',
      'spec',
      'test_design',
      'develop',
      'review',
      'test',
      'deliver',
    ]);
  });
});

describe('boardStageOf（设计文档 §6.4 阶段与看板列映射）', () => {
  it('规格与用例设计合并到同一列', () => {
    expect(boardStageOf('requirement')).toBe('backlog');
    expect(boardStageOf('spec')).toBe('todo');
    expect(boardStageOf('test_design')).toBe('todo');
    expect(boardStageOf('develop')).toBe('dev');
    expect(boardStageOf('review')).toBe('review');
    expect(boardStageOf('test')).toBe('test');
    expect(boardStageOf('deliver')).toBe('done');
  });
});

describe('isPipelineStageKey', () => {
  it('只认七个阶段', () => {
    expect(isPipelineStageKey('develop')).toBe(true);
    expect(isPipelineStageKey('dev')).toBe(false);
    expect(isPipelineStageKey(null)).toBe(false);
  });
});

describe('i18n key', () => {
  it('阶段名 key 是字面量', () => {
    expect(pipelineStageLabelKey('test_design')).toBe(
      'pipeline.stage.test_design'
    );
  });

  it('人工关卡有专名，其它阶段回落到阶段名', () => {
    expect(fallbackGateLabelKey('spec')).toBe('pipeline.gateLabel.spec');
    expect(fallbackGateLabelKey('develop')).toBe('pipeline.stage.develop');
  });

  it('只有四个自动阶段有判定条件文案', () => {
    expect(autoConditionLabelKey('develop')).toBe(
      'pipeline.gate.autoCondition.checksPassed'
    );
    expect(autoConditionLabelKey('review')).toBe(
      'pipeline.gate.autoCondition.noBlockingFindings'
    );
    expect(autoConditionLabelKey('test')).toBe(
      'pipeline.gate.autoCondition.allCasesPassed'
    );
    expect(autoConditionLabelKey('deliver')).toBe(
      'pipeline.gate.autoCondition.artifactsPresent'
    );
    expect(autoConditionLabelKey('requirement')).toBeNull();
  });
});

describe('pipelineStageTone（设计文档 §8.5 阶段语义色）', () => {
  it('灰=需求/规格/用例，蓝=开发，紫=评审，琥珀=测试，绿=交付', () => {
    expect(PIPELINE_STAGE_KEYS.map(pipelineStageTone)).toEqual([
      'neutral',
      'neutral',
      'neutral',
      'dev',
      'review',
      'test',
      'done',
    ]);
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/stages.test.ts`
Expected: FAIL，`Failed to resolve import "./stages"`。

- [ ] **Step 3: 最小实现**

Create `packages/web-core/src/entities/pipeline/model/stages.ts`:
```ts
import type { PipelineStageKey } from 'shared/types';

/**
 * 标准七阶段的固定顺序（设计文档 §6.1）。
 *
 * 拿得到模板（`PipelineTemplateView.stages`）时以模板为准；看板卡片只有
 * 集合数据、没有模板，就用这份默认顺序。
 */
export const PIPELINE_STAGE_KEYS: readonly PipelineStageKey[] = [
  'requirement',
  'spec',
  'test_design',
  'develop',
  'review',
  'test',
  'deliver',
];

/**
 * 看板列对应的流程阶段，与 `features/kanban/model/stageType.ts` 的
 * `StageType` 逐字一致。entities 层不能反向依赖 features，所以这里重复
 * 声明一次，由 `features/kanban/model/pipelineColumns.test.ts` 钉住两边一致。
 */
export type PipelineBoardStage =
  | 'backlog'
  | 'todo'
  | 'dev'
  | 'review'
  | 'test'
  | 'done';

const STAGE_TO_BOARD: Record<PipelineStageKey, PipelineBoardStage> = {
  requirement: 'backlog',
  spec: 'todo',
  test_design: 'todo',
  develop: 'dev',
  review: 'review',
  test: 'test',
  deliver: 'done',
};

/** 流水线阶段落在看板哪一列（设计文档 §6.4）。 */
export function boardStageOf(key: PipelineStageKey): PipelineBoardStage {
  return STAGE_TO_BOARD[key];
}

export function isPipelineStageKey(value: unknown): value is PipelineStageKey {
  return (
    typeof value === 'string' &&
    (PIPELINE_STAGE_KEYS as readonly string[]).includes(value)
  );
}

const STAGE_LABEL_KEYS: Record<PipelineStageKey, string> = {
  requirement: 'pipeline.stage.requirement',
  spec: 'pipeline.stage.spec',
  test_design: 'pipeline.stage.test_design',
  develop: 'pipeline.stage.develop',
  review: 'pipeline.stage.review',
  test: 'pipeline.stage.test',
  deliver: 'pipeline.stage.deliver',
};

/** 阶段名的 i18n key（common 命名空间）。 */
export function pipelineStageLabelKey(key: PipelineStageKey): string {
  return STAGE_LABEL_KEYS[key];
}

const GATE_LABEL_KEYS: Record<PipelineStageKey, string | null> = {
  requirement: 'pipeline.gateLabel.requirement',
  spec: 'pipeline.gateLabel.spec',
  test_design: 'pipeline.gateLabel.test_design',
  develop: null,
  review: null,
  test: null,
  deliver: null,
};

/**
 * 模板没给 `gate_label` 时用的关卡名 key：三道人工关卡有专名，
 * 其它阶段回落到阶段名。
 */
export function fallbackGateLabelKey(key: PipelineStageKey): string {
  return GATE_LABEL_KEYS[key] ?? STAGE_LABEL_KEYS[key];
}

/**
 * 自动关卡的判定条件文案（设计文档 §6.1 的四种判定）。
 *
 * 契约里 `PipelineTemplateStageView` 没有带判定条件名，只能按阶段 key
 * 对应——模板改了判定条件这里要同步（已列入对契约的疑问）。
 */
const AUTO_CONDITION_KEYS: Record<PipelineStageKey, string | null> = {
  requirement: null,
  spec: null,
  test_design: null,
  develop: 'pipeline.gate.autoCondition.checksPassed',
  review: 'pipeline.gate.autoCondition.noBlockingFindings',
  test: 'pipeline.gate.autoCondition.allCasesPassed',
  deliver: 'pipeline.gate.autoCondition.artifactsPresent',
};

export function autoConditionLabelKey(key: PipelineStageKey): string | null {
  return AUTO_CONDITION_KEYS[key];
}

/** 阶段语义色（设计文档 §8.5）。渲染方映射成 `stage-*` 颜色令牌。 */
export type PipelineStageTone = 'neutral' | 'dev' | 'review' | 'test' | 'done';

const STAGE_TONES: Record<PipelineStageKey, PipelineStageTone> = {
  requirement: 'neutral',
  spec: 'neutral',
  test_design: 'neutral',
  develop: 'dev',
  review: 'review',
  test: 'test',
  deliver: 'done',
};

export function pipelineStageTone(key: PipelineStageKey): PipelineStageTone {
  return STAGE_TONES[key];
}
```

- [ ] **Step 4: 通过 + 写测试夹具**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/stages.test.ts`
Expected: PASS（7 tests）。

Create `packages/web-core/src/entities/pipeline/model/__fixtures__/pipeline.ts`（后续任务的测试共用；它不是 `.test.ts`，会被 `tsc` 检查，但不会被业务代码 import）：
```ts
import type {
  IssueArtifactSummary,
  IssuePipelineView,
  PipelineGateDecision,
  PipelineRun,
  PipelineStageRun,
  PipelineTemplateView,
} from 'shared/types';

/**
 * 流水线测试夹具。
 *
 * 按**运行时真实形状**构造（时间是 ISO 字符串、i64 是 number，契约 C2），
 * 再 `as unknown as` 成生成类型，避免每个夹具都要写满全部字段。
 */
const T0 = Date.parse('2026-09-18T02:00:00.000Z');

/** 以 T0 为零点的第 n 分钟，ISO 字符串。 */
export function at(minutes: number): string {
  return new Date(T0 + minutes * 60_000).toISOString();
}

export const T0_MS = T0;

type Loose<T> = { [K in keyof T]?: unknown };

export function makeRun(overrides: Loose<PipelineRun> = {}): PipelineRun {
  return {
    id: 'run-1',
    issue_id: 'issue-1',
    project_id: 'project-1',
    workspace_id: null,
    template_key: 'standard',
    template_version: 1,
    status: 'running',
    current_stage_key: 'requirement',
    created_at: at(0),
    updated_at: at(0),
    finished_at: null,
    ...overrides,
  } as unknown as PipelineRun;
}

export function makeStage(
  overrides: Loose<PipelineStageRun> = {}
): PipelineStageRun {
  return {
    id: 'stage-1',
    run_id: 'run-1',
    project_id: 'project-1',
    stage_key: 'requirement',
    attempt: 1,
    status: 'running',
    gate_kind: 'human',
    session_id: null,
    execution_process_id: null,
    started_at: at(0),
    finished_at: null,
    summary: null,
    error: null,
    ...overrides,
  } as unknown as PipelineStageRun;
}

export function makeDecision(
  overrides: Loose<PipelineGateDecision> = {}
): PipelineGateDecision {
  return {
    id: 'decision-1',
    stage_run_id: 'stage-1',
    decision: 'approve',
    comment: null,
    decided_by: null,
    decided_at: at(1),
    ...overrides,
  } as unknown as PipelineGateDecision;
}

export function makeArtifact(
  overrides: Loose<IssueArtifactSummary> = {}
): IssueArtifactSummary {
  return {
    id: 'artifact-1',
    issue_id: 'issue-1',
    stage_run_id: 'stage-1',
    kind: 'requirement',
    rel_path: 'requirement.md',
    version: 1,
    truncated: false,
    created_at: at(1),
    ...overrides,
  } as unknown as IssueArtifactSummary;
}

export const STANDARD_TEMPLATE = {
  key: 'standard',
  version: 1,
  stages: [
    { key: 'requirement', skill: 'vk-requirement', gate_kind: 'human', gate_label: '需求确认', max_rounds: 3 },
    { key: 'spec', skill: 'vk-spec', gate_kind: 'human', gate_label: '设计规格确认', max_rounds: 3 },
    { key: 'test_design', skill: 'prd2testcase', gate_kind: 'human', gate_label: '用例设计确认', max_rounds: 3 },
    { key: 'develop', skill: 'vk-develop', gate_kind: 'auto', gate_label: null, max_rounds: 3 },
    { key: 'review', skill: 'vk-review', gate_kind: 'auto', gate_label: null, max_rounds: 3 },
    { key: 'test', skill: 'atp-run', gate_kind: 'auto', gate_label: null, max_rounds: 3 },
    { key: 'deliver', skill: 'vk-deliver', gate_kind: 'auto', gate_label: null, max_rounds: 3 },
  ],
} as unknown as PipelineTemplateView;

export function makeView(
  overrides: Partial<IssuePipelineView> = {}
): IssuePipelineView {
  return {
    run: makeRun(),
    stages: [makeStage()],
    decisions: [],
    artifacts: [],
    template: STANDARD_TEMPLATE,
    ...overrides,
  };
}
```

Run: `pnpm --filter @vibe/web-core run check`
Expected: 通过（0 error）。若 `STANDARD_TEMPLATE` 行被 prettier 以外的规则报长，不影响；Step 5 前跑一次 `pnpm --filter @vibe/web-core run format`。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/entities/pipeline/model
git commit -m "$(cat <<'EOF'
界面：流水线阶段元数据与测试夹具

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: 进度格、状态文案、卡片信息

**Files:**
- Create: `packages/web-core/src/entities/pipeline/model/progress.ts`
- Create: `packages/web-core/src/entities/pipeline/model/cardInfo.ts`
- Test: `packages/web-core/src/entities/pipeline/model/progress.test.ts`
- Test: `packages/web-core/src/entities/pipeline/model/cardInfo.test.ts`

规则（设计文档 §8.3 + §6.3）：
- 运行 `completed` → 七格全绿。
- 当前阶段之前的格子一律「已完成」（回流时前面的阶段都已通过）；之后的一律「未开始」（回流后要重跑，哪怕以前跑过）。
- 当前阶段的格子看运行状态：`failed` 红、`waiting_gate` 橙、`paused` 暂停、`cancelled` 取消、`running` 时再看该阶段最新一次尝试（`waiting_gate` 橙、`failed` 红、其它蓝）。
- 同一阶段多次尝试取 `attempt` 最大的一次；并列取 `started_at` 更晚的。

- [ ] **Step 1: 写 progress 的失败测试**

Create `packages/web-core/src/entities/pipeline/model/progress.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  latestAttempts,
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  toNumber,
  type Translate,
} from './progress';
import {
  STANDARD_TEMPLATE,
  at,
  makeRun,
  makeStage,
} from './__fixtures__/pipeline';

const states = (cells: { state: string }[]) => cells.map((cell) => cell.state);

/** 假翻译：key + 参数原样拼出来，断言时一眼看出用了哪个 key。 */
const t: Translate = (key, params) =>
  params ? `${key}${JSON.stringify(params)}` : key;

describe('toMillis / toNumber', () => {
  it('兼容字符串、Date、bigint、空值与非法值', () => {
    expect(toMillis('2026-09-18T02:00:00.000Z')).toBe(
      Date.parse('2026-09-18T02:00:00.000Z')
    );
    expect(toMillis(new Date(5))).toBe(5);
    expect(toMillis(null)).toBeNull();
    expect(toMillis('not a date')).toBeNull();
    expect(toNumber(3)).toBe(3);
    expect(toNumber(BigInt(4))).toBe(4);
    expect(toNumber(undefined)).toBeNull();
  });
});

describe('latestAttempts', () => {
  it('取 attempt 最大的一次', () => {
    const latest = latestAttempts([
      makeStage({ id: 'a', stage_key: 'develop', attempt: 1 }),
      makeStage({ id: 'b', stage_key: 'develop', attempt: 2 }),
    ]);
    expect(latest.get('develop')?.id).toBe('b');
  });

  it('attempt 相同取 started_at 更晚的', () => {
    const latest = latestAttempts([
      makeStage({ id: 'a', stage_key: 'spec', attempt: 1, started_at: at(5) }),
      makeStage({ id: 'b', stage_key: 'spec', attempt: 1, started_at: at(3) }),
    ]);
    expect(latest.get('spec')?.id).toBe('a');
  });
});

describe('pipelineProgress', () => {
  it('刚启动：第一格蓝，其余未开始', () => {
    const run = makeRun();
    expect(states(pipelineProgress([makeStage()], null, run))).toEqual([
      'running',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('停在规格确认：第一格绿、第二格橙', () => {
    const run = makeRun({ status: 'waiting_gate', current_stage_key: 'spec' });
    const stages = [
      makeStage({ id: 's1', status: 'passed', finished_at: at(1) }),
      makeStage({ id: 's2', stage_key: 'spec', status: 'waiting_gate' }),
    ];
    expect(states(pipelineProgress(stages, STANDARD_TEMPLATE, run))).toEqual([
      'done',
      'gate',
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('测试归因代码缺陷回到开发：后面的评审、测试重新算未开始', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const stages = [
      makeStage({ id: 'd1', stage_key: 'develop', attempt: 1, status: 'passed' }),
      makeStage({ id: 'r1', stage_key: 'review', attempt: 1, status: 'passed' }),
      makeStage({ id: 't1', stage_key: 'test', attempt: 1, status: 'failed' }),
      makeStage({ id: 'd2', stage_key: 'develop', attempt: 2, status: 'running' }),
    ];
    expect(states(pipelineProgress(stages, null, run))).toEqual([
      'done',
      'done',
      'done',
      'running',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('评审轮次用尽：第五格红', () => {
    const run = makeRun({ status: 'failed', current_stage_key: 'review' });
    expect(states(pipelineProgress([], null, run))[4]).toBe('failed');
  });

  it('完成：七格全绿', () => {
    const run = makeRun({ status: 'completed', current_stage_key: 'deliver' });
    expect(new Set(states(pipelineProgress([], null, run)))).toEqual(
      new Set(['done'])
    );
  });

  it('暂停与取消各有自己的格子状态', () => {
    const paused = makeRun({ status: 'paused', current_stage_key: 'develop' });
    const cancelled = makeRun({ status: 'cancelled', current_stage_key: 'test' });
    expect(states(pipelineProgress([], null, paused))[3]).toBe('paused');
    expect(states(pipelineProgress([], null, cancelled))[5]).toBe('cancelled');
  });

  it('运行中但当前阶段最新尝试在等关卡：显示橙', () => {
    const run = makeRun({ status: 'running' });
    const stages = [makeStage({ status: 'waiting_gate' })];
    expect(states(pipelineProgress(stages, null, run))[0]).toBe('gate');
  });

  it('只看本运行的阶段记录', () => {
    const run = makeRun({ id: 'run-2' });
    const other = makeStage({ run_id: 'run-1', status: 'waiting_gate' });
    expect(states(pipelineProgress([other], null, run))[0]).toBe('running');
  });

  it('模板顺序优先于默认顺序', () => {
    const template = {
      ...STANDARD_TEMPLATE,
      stages: STANDARD_TEMPLATE.stages.slice(0, 2),
    };
    const cells = pipelineProgress([], template, makeRun());
    expect(cells.map((cell) => cell.key)).toEqual(['requirement', 'spec']);
  });
});

describe('pipelineStatus + pipelineStatusText', () => {
  it('等人工：用模板里的关卡名', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = pipelineStatus(
      [makeStage({ status: 'waiting_gate' })],
      STANDARD_TEMPLATE,
      run
    );
    expect(info.tone).toBe('gate');
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.waitingGate{"gate":"需求确认"}'
    );
  });

  it('等人工但没有模板：回落到内置关卡名 key', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = pipelineStatus([], null, run);
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.waitingGate{"gate":"pipeline.gateLabel.requirement"}'
    );
  });

  it('自动阶段第 2 轮：显示轮次', () => {
    const run = makeRun({ current_stage_key: 'develop' });
    const info = pipelineStatus(
      [makeStage({ stage_key: 'develop', attempt: 2, gate_kind: 'auto' })],
      STANDARD_TEMPLATE,
      run
    );
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.runningRound{"stage":"pipeline.stage.develop","attempt":2,"max":3}'
    );
  });

  it('人工阶段被打回后重跑不显示轮次（打回不计入失败轮次）', () => {
    const info = pipelineStatus(
      [makeStage({ attempt: 2 })],
      STANDARD_TEMPLATE,
      makeRun()
    );
    expect(info.maxRounds).toBeNull();
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.running{"stage":"pipeline.stage.requirement"}'
    );
  });

  it('第 1 轮不显示轮次', () => {
    const info = pipelineStatus([makeStage()], STANDARD_TEMPLATE, makeRun());
    expect(pipelineStatusText(info, t)).toBe(
      'pipeline.status.running{"stage":"pipeline.stage.requirement"}'
    );
  });

  it('失败、暂停、完成、取消各用自己的 key', () => {
    const text = (status: string) =>
      pipelineStatusText(pipelineStatus([], null, makeRun({ status })), t);
    expect(text('failed')).toBe('pipeline.status.failed');
    expect(text('paused')).toBe('pipeline.status.paused');
    expect(text('completed')).toBe('pipeline.status.completed');
    expect(text('cancelled')).toBe('pipeline.status.cancelled');
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/progress.test.ts`
Expected: FAIL，`Failed to resolve import "./progress"`。

- [ ] **Step 3: 实现 progress.ts**

Create `packages/web-core/src/entities/pipeline/model/progress.ts`:
```ts
import type {
  PipelineRun,
  PipelineRunStatus,
  PipelineStageKey,
  PipelineStageRun,
  PipelineStageStatus,
  PipelineTemplateView,
} from 'shared/types';
import {
  PIPELINE_STAGE_KEYS,
  fallbackGateLabelKey,
  pipelineStageLabelKey,
} from './stages';

/** 进度格状态（设计文档 §8.3：已完成绿、进行中蓝、等人工橙、失败红）。 */
export type PipelineCellState =
  | 'done'
  | 'running'
  | 'gate'
  | 'failed'
  | 'pending'
  | 'paused'
  | 'cancelled';

export interface PipelineCell {
  key: PipelineStageKey;
  state: PipelineCellState;
  /** 该阶段最新一次尝试的序号；还没跑过为 null。 */
  attempt: number | null;
}

/** 运行整体的状态色调（= 当前阶段格子的状态，永远不是 pending）。 */
export type PipelineTone = Exclude<PipelineCellState, 'pending'>;

type TemplateLike = Pick<PipelineTemplateView, 'stages'> | null | undefined;

/**
 * 时间字段一律用它读：ts-rs 可能把 `DateTime<Utc>` 声明成 `Date` 或 `string`，
 * 运行时 JSON 一定是 ISO 字符串。非法值返回 null，绝不抛错。
 */
export function toMillis(value: unknown): number | null {
  if (value instanceof Date) {
    const ms = value.getTime();
    return Number.isFinite(ms) ? ms : null;
  }
  if (typeof value === 'string') {
    const ms = Date.parse(value);
    return Number.isFinite(ms) ? ms : null;
  }
  return null;
}

/**
 * i64 字段一律用它读：ts-rs 默认把 i64 声明成 `bigint`（`shared/types.ts:17`），
 * 运行时 JSON 是 number。
 */
export function toNumber(value: unknown): number | null {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value === 'bigint') return Number(value);
  return null;
}

export function stageKeysOf(template: TemplateLike): PipelineStageKey[] {
  const keys = template?.stages?.map((stage) => stage.key) ?? [];
  return keys.length > 0 ? keys : [...PIPELINE_STAGE_KEYS];
}

function isNewer(candidate: PipelineStageRun, current: PipelineStageRun) {
  const a = toNumber(candidate.attempt) ?? 0;
  const b = toNumber(current.attempt) ?? 0;
  if (a !== b) return a > b;
  return (toMillis(candidate.started_at) ?? -1) > (toMillis(current.started_at) ?? -1);
}

/** 每个阶段取「最新一次尝试」。 */
export function latestAttempts(
  stages: readonly PipelineStageRun[]
): Map<PipelineStageKey, PipelineStageRun> {
  const latest = new Map<PipelineStageKey, PipelineStageRun>();
  for (const stage of stages) {
    const current = latest.get(stage.stage_key);
    if (!current || isNewer(stage, current)) {
      latest.set(stage.stage_key, stage);
    }
  }
  return latest;
}

function currentCellState(
  runStatus: PipelineRunStatus,
  stageStatus: PipelineStageStatus | null
): PipelineTone {
  switch (runStatus) {
    case 'completed':
      return 'done';
    case 'failed':
      return 'failed';
    case 'waiting_gate':
      return 'gate';
    case 'paused':
      return 'paused';
    case 'cancelled':
      return 'cancelled';
    case 'running':
      if (stageStatus === 'waiting_gate') return 'gate';
      if (stageStatus === 'failed') return 'failed';
      return 'running';
  }
}

/**
 * 七格进度条（设计文档 §8.3）。只看 `run.id` 这一次运行的阶段记录。
 */
export function pipelineProgress(
  stages: readonly PipelineStageRun[],
  template: TemplateLike,
  run: PipelineRun
): PipelineCell[] {
  const keys = stageKeysOf(template);
  const latest = latestAttempts(stages.filter((s) => s.run_id === run.id));
  const currentIndex = keys.indexOf(run.current_stage_key);

  return keys.map((key, index): PipelineCell => {
    const attempt = latest.get(key) ?? null;
    const attemptNo = attempt ? toNumber(attempt.attempt) : null;

    if (run.status === 'completed') {
      return { key, state: 'done', attempt: attemptNo };
    }
    if (currentIndex === -1) {
      const passed =
        attempt?.status === 'passed' || attempt?.status === 'skipped';
      return { key, state: passed ? 'done' : 'pending', attempt: attemptNo };
    }
    if (index < currentIndex) {
      return { key, state: 'done', attempt: attemptNo };
    }
    if (index > currentIndex) {
      return { key, state: 'pending', attempt: attemptNo };
    }
    return {
      key,
      state: currentCellState(run.status, attempt?.status ?? null),
      attempt: attemptNo,
    };
  });
}

export interface PipelineStatusInfo {
  tone: PipelineTone;
  stageKey: PipelineStageKey;
  /** 模板里的人工关卡名（如「需求确认」）；没有模板或不是人工关卡为 null。 */
  gateLabel: string | null;
  /** 当前阶段最新一次尝试的序号，至少为 1。 */
  attempt: number;
  /**
   * 当前阶段最多轮次；没有模板、或不是自动关卡阶段时为 null。
   * 人工关卡阶段的 max_rounds 只管「缺产出物 / 进程失败」的重试，人工打回
   * 不计入（计划 A），显示「第 2/3 轮」会误导，所以只给自动阶段。
   */
  maxRounds: number | null;
}

export function pipelineStatus(
  stages: readonly PipelineStageRun[],
  template: TemplateLike,
  run: PipelineRun
): PipelineStatusInfo {
  const latest =
    latestAttempts(stages.filter((s) => s.run_id === run.id)).get(
      run.current_stage_key
    ) ?? null;
  const templateStage =
    template?.stages?.find((stage) => stage.key === run.current_stage_key) ??
    null;

  return {
    tone: currentCellState(run.status, latest?.status ?? null),
    stageKey: run.current_stage_key,
    gateLabel: templateStage?.gate_label ?? null,
    attempt: Math.max(1, toNumber(latest?.attempt) ?? 1),
    maxRounds:
      templateStage && templateStage.gate_kind === 'auto'
        ? toNumber(templateStage.max_rounds)
        : null,
  };
}

/** 翻译函数的最小形状，方便在纯函数里注入与在测试里替身。 */
export type Translate = (
  key: string,
  params?: Record<string, string | number>
) => string;

const FIXED_STATUS_KEYS: Record<
  Exclude<PipelineTone, 'gate' | 'running'>,
  string
> = {
  failed: 'pipeline.status.failed',
  paused: 'pipeline.status.paused',
  done: 'pipeline.status.completed',
  cancelled: 'pipeline.status.cancelled',
};

/** 状态标签文案（卡片、工作台、详情头部共用）。 */
export function pipelineStatusText(
  info: PipelineStatusInfo,
  t: Translate
): string {
  const stage = t(pipelineStageLabelKey(info.stageKey));
  switch (info.tone) {
    case 'gate':
      return t('pipeline.status.waitingGate', {
        gate: info.gateLabel ?? t(fallbackGateLabelKey(info.stageKey)),
      });
    case 'running':
      if (info.attempt > 1 && info.maxRounds !== null && info.maxRounds > 1) {
        return t('pipeline.status.runningRound', {
          stage,
          attempt: info.attempt,
          max: info.maxRounds,
        });
      }
      return t('pipeline.status.running', { stage });
    default:
      return t(FIXED_STATUS_KEYS[info.tone]);
  }
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/progress.test.ts`
Expected: PASS（18 tests）。

- [ ] **Step 5: 写 cardInfo 的失败测试**

Create `packages/web-core/src/entities/pipeline/model/cardInfo.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  buildPipelineCardInfo,
  cellLabelKey,
  groupStagesByRun,
  latestRunByIssue,
} from './cardInfo';
import type { Translate } from './progress';
import { at, makeRun, makeStage } from './__fixtures__/pipeline';

const t: Translate = (key, params) =>
  params ? `${key}${JSON.stringify(params)}` : key;

describe('latestRunByIssue', () => {
  it('同一需求多次运行取最新创建的那次', () => {
    const map = latestRunByIssue([
      makeRun({ id: 'old', created_at: at(0) }),
      makeRun({ id: 'new', created_at: at(10) }),
      makeRun({ id: 'other', issue_id: 'issue-2' }),
    ]);
    expect(map.get('issue-1')?.id).toBe('new');
    expect(map.get('issue-2')?.id).toBe('other');
  });
});

describe('groupStagesByRun', () => {
  it('按 run_id 分组', () => {
    const groups = groupStagesByRun([
      makeStage({ id: 'a', run_id: 'r1' }),
      makeStage({ id: 'b', run_id: 'r2' }),
      makeStage({ id: 'c', run_id: 'r1' }),
    ]);
    expect(groups.get('r1')?.map((s) => s.id)).toEqual(['a', 'c']);
    expect(groups.get('r2')?.map((s) => s.id)).toEqual(['b']);
  });
});

describe('buildPipelineCardInfo', () => {
  it('七格 + 状态文案 + 无障碍标签', () => {
    const run = makeRun({ status: 'waiting_gate' });
    const info = buildPipelineCardInfo(
      run,
      [makeStage({ status: 'waiting_gate' })],
      t
    );
    expect(info.runId).toBe('run-1');
    expect(info.cells).toHaveLength(7);
    expect(info.cells[0]).toEqual({
      key: 'requirement',
      state: 'gate',
      label: 'pipeline.stage.requirement: pipeline.cell.gate',
    });
    expect(info.tone).toBe('gate');
    expect(info.statusText).toBe(
      'pipeline.status.waitingGate{"gate":"pipeline.gateLabel.requirement"}'
    );
    expect(info.progressLabel.startsWith('pipeline.progressLabel')).toBe(true);
    expect(info.isFailed).toBe(false);
  });

  it('失败时 isFailed 为真（卡片加红边框）', () => {
    const run = makeRun({ status: 'failed', current_stage_key: 'review' });
    expect(buildPipelineCardInfo(run, [], t).isFailed).toBe(true);
  });

  it('格子状态的 key 是字面量', () => {
    expect(cellLabelKey('pending')).toBe('pipeline.cell.pending');
  });
});
```

- [ ] **Step 6: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/cardInfo.test.ts`
Expected: FAIL，`Failed to resolve import "./cardInfo"`。

- [ ] **Step 7: 实现 cardInfo.ts 并通过**

Create `packages/web-core/src/entities/pipeline/model/cardInfo.ts`:
```ts
import type { PipelineRun, PipelineStageRun } from 'shared/types';
import {
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  type PipelineCellState,
  type PipelineTone,
  type Translate,
} from './progress';
import { pipelineStageLabelKey } from './stages';

export interface PipelineCardCell {
  key: string;
  state: PipelineCellState;
  /** 已翻译：「开发: 进行中」，用于格子的 title 与无障碍说明。 */
  label: string;
}

/** 看板卡片 / 工作台卡片底部要的全部信息（已翻译）。 */
export interface PipelineCardInfo {
  runId: string;
  cells: PipelineCardCell[];
  tone: PipelineTone;
  statusText: string;
  progressLabel: string;
  /** 失败时卡片加红边框（设计文档 §8.5 三重编码）。 */
  isFailed: boolean;
}

const CELL_LABEL_KEYS: Record<PipelineCellState, string> = {
  done: 'pipeline.cell.done',
  running: 'pipeline.cell.running',
  gate: 'pipeline.cell.gate',
  failed: 'pipeline.cell.failed',
  pending: 'pipeline.cell.pending',
  paused: 'pipeline.cell.paused',
  cancelled: 'pipeline.cell.cancelled',
};

export function cellLabelKey(state: PipelineCellState): string {
  return CELL_LABEL_KEYS[state];
}

/** 每个需求取最新创建的一次运行。 */
export function latestRunByIssue(
  runs: readonly PipelineRun[]
): Map<string, PipelineRun> {
  const map = new Map<string, PipelineRun>();
  for (const run of runs) {
    const current = map.get(run.issue_id);
    if (
      !current ||
      (toMillis(run.created_at) ?? 0) > (toMillis(current.created_at) ?? 0)
    ) {
      map.set(run.issue_id, run);
    }
  }
  return map;
}

export function groupStagesByRun(
  stages: readonly PipelineStageRun[]
): Map<string, PipelineStageRun[]> {
  const map = new Map<string, PipelineStageRun[]>();
  for (const stage of stages) {
    const list = map.get(stage.run_id);
    if (list) {
      list.push(stage);
    } else {
      map.set(stage.run_id, [stage]);
    }
  }
  return map;
}

/**
 * 卡片信息。集合里没有模板，所以用默认七阶段顺序、不显示关卡专名
 * （回落到内置关卡名 key）。
 */
export function buildPipelineCardInfo(
  run: PipelineRun,
  stages: readonly PipelineStageRun[],
  t: Translate
): PipelineCardInfo {
  const cells = pipelineProgress(stages, null, run).map((cell) => ({
    key: cell.key,
    state: cell.state,
    label: `${t(pipelineStageLabelKey(cell.key))}: ${t(CELL_LABEL_KEYS[cell.state])}`,
  }));
  const info = pipelineStatus(stages, null, run);
  return {
    runId: run.id,
    cells,
    tone: info.tone,
    statusText: pipelineStatusText(info, t),
    progressLabel: t('pipeline.progressLabel', {
      summary: cells.map((cell) => cell.label).join(', '),
    }),
    isFailed: info.tone === 'failed',
  };
}
```

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/`
Expected: PASS（stages 7 + progress 18 + cardInfo 5）。

- [ ] **Step 8: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/entities/pipeline/model
git commit -m "$(cat <<'EOF'
界面：流水线七格进度与状态文案纯函数

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: 耗时 / 相对时间 / 集合指纹

**Files:**
- Create: `packages/web-core/src/entities/pipeline/model/time.ts`
- Create: `packages/web-core/src/entities/pipeline/model/signature.ts`
- Test: `packages/web-core/src/entities/pipeline/model/time.test.ts`
- Test: `packages/web-core/src/entities/pipeline/model/signature.test.ts`

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/entities/pipeline/model/time.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { durationText, relativeTimeText } from './time';

describe('durationText', () => {
  it('不足一分钟按秒', () => {
    expect(durationText(12_400)).toEqual({
      key: 'pipeline.duration.seconds',
      params: { s: 12 },
    });
  });
  it('一小时内按分秒', () => {
    expect(durationText(72_000)).toEqual({
      key: 'pipeline.duration.minutesSeconds',
      params: { m: 1, s: 12 },
    });
  });
  it('超过一小时按时分', () => {
    expect(durationText(3_780_000)).toEqual({
      key: 'pipeline.duration.hoursMinutes',
      params: { h: 1, m: 3 },
    });
  });
  it('负数当 0', () => {
    expect(durationText(-5).params).toEqual({ s: 0 });
  });
});

describe('relativeTimeText', () => {
  const now = Date.parse('2026-09-18T10:00:00Z');
  it('一分钟内是刚刚', () => {
    expect(relativeTimeText(now - 30_000, now).key).toBe('pipeline.time.justNow');
  });
  it('分钟 / 小时 / 天', () => {
    expect(relativeTimeText(now - 3 * 60_000, now)).toEqual({
      key: 'pipeline.time.minutesAgo',
      params: { n: 3 },
    });
    expect(relativeTimeText(now - 5 * 3_600_000, now)).toEqual({
      key: 'pipeline.time.hoursAgo',
      params: { n: 5 },
    });
    expect(relativeTimeText(now - 2 * 86_400_000, now)).toEqual({
      key: 'pipeline.time.daysAgo',
      params: { n: 2 },
    });
  });
  it('未来时间当刚刚', () => {
    expect(relativeTimeText(now + 60_000, now).key).toBe('pipeline.time.justNow');
  });
});
```

Create `packages/web-core/src/entities/pipeline/model/signature.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { pipelineSignature } from './signature';
import { makeRun, makeStage } from './__fixtures__/pipeline';

describe('pipelineSignature', () => {
  it('与数组顺序无关', () => {
    const runs = [makeRun({ id: 'a' }), makeRun({ id: 'b' })];
    const stages = [
      makeStage({ id: 's1', run_id: 'a' }),
      makeStage({ id: 's2', run_id: 'b' }),
    ];
    expect(pipelineSignature(runs, stages)).toBe(
      pipelineSignature([...runs].reverse(), [...stages].reverse())
    );
  });

  it('阶段状态或尝试次数变化时指纹变化', () => {
    const runs = [makeRun()];
    const before = pipelineSignature(runs, [makeStage()]);
    expect(pipelineSignature(runs, [makeStage({ status: 'passed' })])).not.toBe(before);
    expect(pipelineSignature(runs, [makeStage({ attempt: 2 })])).not.toBe(before);
  });

  it('不属于这些运行的阶段不计入', () => {
    const runs = [makeRun({ id: 'a' })];
    expect(pipelineSignature(runs, [makeStage({ run_id: 'zzz' })])).toBe(
      pipelineSignature(runs, [])
    );
  });

  it('没有运行时是空串', () => {
    expect(pipelineSignature([], [])).toBe('');
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/time.test.ts src/entities/pipeline/model/signature.test.ts`
Expected: FAIL，两个文件都报 `Failed to resolve import`。

- [ ] **Step 3: 实现**

Create `packages/web-core/src/entities/pipeline/model/time.ts`:
```ts
/** 需要翻译的文本：key + 参数，渲染方 `t(spec.key, spec.params)`。 */
export interface TextSpec {
  key: string;
  params: Record<string, number>;
}

/** 耗时（设计文档 §8.4 时间线「耗时」）。 */
export function durationText(ms: number): TextSpec {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) {
    return { key: 'pipeline.duration.seconds', params: { s: totalSeconds } };
  }
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return {
      key: 'pipeline.duration.minutesSeconds',
      params: { m: totalMinutes, s: totalSeconds % 60 },
    };
  }
  return {
    key: 'pipeline.duration.hoursMinutes',
    params: { h: Math.floor(totalMinutes / 60), m: totalMinutes % 60 },
  };
}

/**
 * 相对时间。现有 `shared/lib/date.ts:17 formatRelativeTime` 写死英文，
 * 这里换成 i18n key。
 */
export function relativeTimeText(at: number, now: number): TextSpec {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return { key: 'pipeline.time.justNow', params: {} };
  if (minutes < 60) return { key: 'pipeline.time.minutesAgo', params: { n: minutes } };
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return { key: 'pipeline.time.hoursAgo', params: { n: hours } };
  return { key: 'pipeline.time.daysAgo', params: { n: Math.floor(hours / 24) } };
}
```

Create `packages/web-core/src/entities/pipeline/model/signature.ts`:
```ts
import type { PipelineRun, PipelineStageRun } from 'shared/types';

/**
 * 集合数据的指纹：运行状态 / 当前阶段 / 阶段状态 / 尝试次数任一变化就变。
 *
 * 产出物与关卡决策不推送（契约 §3），前端靠这个指纹变化去失效
 * `GET /api/local/issues/{id}/pipeline` 与 pending 查询。
 */
export function pipelineSignature(
  runs: readonly PipelineRun[],
  stages: readonly PipelineStageRun[]
): string {
  const runIds = new Set(runs.map((run) => run.id));
  const parts = [
    ...runs.map((run) => `r:${run.id}:${run.status}:${run.current_stage_key}`),
    ...stages
      .filter((stage) => runIds.has(stage.run_id))
      .map((stage) => `s:${stage.id}:${stage.status}:${String(stage.attempt)}`),
  ];
  return parts.sort().join('|');
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/model/time.test.ts src/entities/pipeline/model/signature.test.ts`
Expected: PASS（time 7 + signature 4）。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/entities/pipeline/model
git commit -m "$(cat <<'EOF'
界面：流水线耗时、相对时间与集合指纹

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: 看板列文案与拖拽判定

**Files:**
- Create: `packages/web-core/src/features/kanban/model/pipelineColumns.ts`
- Create: `packages/web-core/src/features/kanban/model/pipelineDrag.ts`
- Test: `packages/web-core/src/features/kanban/model/pipelineColumns.test.ts`
- Test: `packages/web-core/src/features/kanban/model/pipelineDrag.test.ts`

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/features/kanban/model/pipelineColumns.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  PIPELINE_STAGE_KEYS,
  boardStageOf,
} from '@/entities/pipeline/model/stages';
import {
  LEGACY_DEFAULT_STATUS_NAMES,
  pipelineColumnEmptyKey,
  pipelineColumnHintKey,
  pipelineColumnTitleKey,
} from './pipelineColumns';
import { STAGE_ORDER, isStageType } from './stageType';

describe('entities 与 features 的阶段类型一致', () => {
  it('流水线阶段映射出的列都是看板认识的阶段', () => {
    for (const key of PIPELINE_STAGE_KEYS) {
      expect(isStageType(boardStageOf(key))).toBe(true);
    }
  });

  it('默认列名表覆盖全部六个看板阶段', () => {
    expect(Object.keys(LEGACY_DEFAULT_STATUS_NAMES).sort()).toEqual(
      Object.keys(STAGE_ORDER).sort()
    );
  });
});

describe('pipelineColumnTitleKey', () => {
  it('没改过名的默认列显示流水线列名', () => {
    expect(pipelineColumnTitleKey('待规划', 'backlog')).toBe(
      'pipeline.column.backlog'
    );
    expect(pipelineColumnTitleKey(' 待开发 ', 'todo')).toBe(
      'pipeline.column.todo'
    );
    expect(pipelineColumnTitleKey('已完成', 'done')).toBe(
      'pipeline.column.done'
    );
  });

  it('用户改过名就返回 null（显示用户的名字）', () => {
    expect(pipelineColumnTitleKey('等上线', 'done')).toBeNull();
  });
});

describe('列提示与空列说明', () => {
  it('前两列是人工确认，交付列是自动，其它没有提示', () => {
    expect(pipelineColumnHintKey('backlog')).toBe('pipeline.column.humanGate');
    expect(pipelineColumnHintKey('todo')).toBe('pipeline.column.humanGate');
    expect(pipelineColumnHintKey('done')).toBe('pipeline.column.auto');
    expect(pipelineColumnHintKey('dev')).toBeNull();
  });

  it('每列都有「为什么空」', () => {
    expect(pipelineColumnEmptyKey('review')).toBe('pipeline.columnEmpty.review');
    expect(pipelineColumnEmptyKey('backlog')).toBe(
      'pipeline.columnEmpty.backlog'
    );
  });
});
```

Create `packages/web-core/src/features/kanban/model/pipelineDrag.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { canDropIssue } from './pipelineDrag';

describe('canDropIssue（设计文档 §8.3：只允许往回拖）', () => {
  it('同列内调整顺序永远允许', () => {
    expect(
      canDropIssue({ fromStage: 'dev', toStage: 'dev', runStatus: 'running' })
    ).toEqual({ allowed: true });
  });

  it('没有流水线的手工需求：前后都能拖', () => {
    expect(
      canDropIssue({ fromStage: 'todo', toStage: 'dev', runStatus: null })
    ).toEqual({ allowed: true });
    expect(
      canDropIssue({ fromStage: 'dev', toStage: 'todo', runStatus: null })
    ).toEqual({ allowed: true });
  });

  it('流水线在跑：任何跨列都拦，提示先暂停', () => {
    for (const runStatus of ['running', 'waiting_gate'] as const) {
      expect(
        canDropIssue({ fromStage: 'test', toStage: 'dev', runStatus })
      ).toEqual({ allowed: false, reasonKey: 'pipeline.drag.pauseFirst' });
    }
  });

  it('流水线已停：往回允许，往后拦', () => {
    for (const runStatus of ['paused', 'failed', 'completed', 'cancelled'] as const) {
      expect(
        canDropIssue({ fromStage: 'review', toStage: 'dev', runStatus })
      ).toEqual({ allowed: true });
      expect(
        canDropIssue({ fromStage: 'review', toStage: 'test', runStatus })
      ).toEqual({ allowed: false, reasonKey: 'pipeline.drag.forwardBlocked' });
    }
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/features/kanban/model/pipelineColumns.test.ts src/features/kanban/model/pipelineDrag.test.ts`
Expected: FAIL，`Failed to resolve import "./pipelineColumns"` / `"./pipelineDrag"`。

- [ ] **Step 3: 实现 pipelineColumns.ts**

Create `packages/web-core/src/features/kanban/model/pipelineColumns.ts`:
```ts
import type { StageType } from './stageType';

/**
 * 个人版新建项目时自动创建的列名（`crates/db/src/models/local_project.rs:14-21`
 * 的 `DEFAULT_STATUSES`）。列名等于它，说明用户没改过名，可以换成流水线列名。
 */
export const LEGACY_DEFAULT_STATUS_NAMES: Record<StageType, string> = {
  backlog: '待规划',
  todo: '待开发',
  dev: '开发中',
  review: '待评审',
  test: '测试中',
  done: '已完成',
};

const COLUMN_TITLE_KEYS: Record<StageType, string> = {
  backlog: 'pipeline.column.backlog',
  todo: 'pipeline.column.todo',
  dev: 'pipeline.column.dev',
  review: 'pipeline.column.review',
  test: 'pipeline.column.test',
  done: 'pipeline.column.done',
};

/** 个人版列头标题：默认列名 → 流水线列名 key；用户改过名 → null（显示原名）。 */
export function pipelineColumnTitleKey(
  statusName: string,
  stage: StageType
): string | null {
  return statusName.trim() === LEGACY_DEFAULT_STATUS_NAMES[stage]
    ? COLUMN_TITLE_KEYS[stage]
    : null;
}

const COLUMN_EMPTY_KEYS: Record<StageType, string> = {
  backlog: 'pipeline.columnEmpty.backlog',
  todo: 'pipeline.columnEmpty.todo',
  dev: 'pipeline.columnEmpty.dev',
  review: 'pipeline.columnEmpty.review',
  test: 'pipeline.columnEmpty.test',
  done: 'pipeline.columnEmpty.done',
};

/** 空列写明「为什么空」（设计文档 §8.3）。 */
export function pipelineColumnEmptyKey(stage: StageType): string {
  return COLUMN_EMPTY_KEYS[stage];
}

const COLUMN_HINT_KEYS: Record<StageType, string | null> = {
  backlog: 'pipeline.column.humanGate',
  todo: 'pipeline.column.humanGate',
  dev: null,
  review: null,
  test: null,
  done: 'pipeline.column.auto',
};

/** 列名旁的小提示（草图：「需求 · 人工确认」「交付 · 自动」）。 */
export function pipelineColumnHintKey(stage: StageType): string | null {
  return COLUMN_HINT_KEYS[stage];
}
```

- [ ] **Step 4: 实现 pipelineDrag.ts 并通过**

Create `packages/web-core/src/features/kanban/model/pipelineDrag.ts`:
```ts
import type { PipelineRunStatus } from 'shared/types';
import { STAGE_ORDER, type StageType } from './stageType';

export type DropDecision =
  | { allowed: true }
  | { allowed: false; reasonKey: string };

const ACTIVE_STATUSES: ReadonlySet<PipelineRunStatus> = new Set([
  'running',
  'waiting_gate',
]);

/**
 * 个人版流水线看板的拖拽规则（设计文档 §8.3 + 计划 §3 决策 7）。
 *
 * - 没有流水线的手工需求：不限制（保持旧行为）。
 * - 流水线在跑：跨列一律拦下——契约没有「打回到指定阶段」的接口，
 *   只改列会和引擎写的阶段打架。
 * - 流水线已停：只允许往回拖。
 */
export function canDropIssue(args: {
  fromStage: StageType;
  toStage: StageType;
  runStatus: PipelineRunStatus | null;
}): DropDecision {
  const { fromStage, toStage, runStatus } = args;
  if (fromStage === toStage || runStatus === null) {
    return { allowed: true };
  }
  if (ACTIVE_STATUSES.has(runStatus)) {
    return { allowed: false, reasonKey: 'pipeline.drag.pauseFirst' };
  }
  if (STAGE_ORDER[toStage] > STAGE_ORDER[fromStage]) {
    return { allowed: false, reasonKey: 'pipeline.drag.forwardBlocked' };
  }
  return { allowed: true };
}
```

Run: `pnpm --filter @vibe/web-core exec vitest run src/features/kanban/model/pipelineColumns.test.ts src/features/kanban/model/pipelineDrag.test.ts`
Expected: PASS（columns 6 + drag 4）。

- [ ] **Step 5: 全量单测回归**

Run: `pnpm --filter @vibe/web-core run test 2>&1 | tail -4`
Expected: `Test Files  N passed`，无失败。

- [ ] **Step 6: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/features/kanban/model
git commit -m "$(cat <<'EOF'
界面：看板流水线列文案与只许往回拖的判定

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: 时间线

**Files:**
- Create: `packages/web-core/src/pages/issue-detail/timeline.ts`
- Test: `packages/web-core/src/pages/issue-detail/timeline.test.ts`

规则：每次阶段尝试产生「开始」事件（AI）与一个结束事件（完成 / 失败 / 等待确认 / 被打回，AI）；每条关卡决策产生「确认」或「打回」事件（人）。阶段状态是 `rejected` 且已有对应的打回决策时，不再重复出「被打回」。按时间升序，同一时刻先阶段事件后人工事件。

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/pages/issue-detail/timeline.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { buildTimeline, timelineTextKey } from './timeline';
import {
  at,
  makeDecision,
  makeStage,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('buildTimeline', () => {
  it('需求被打回后重跑再确认：事件顺序与演员', () => {
    const stages = [
      makeStage({
        id: 'r1',
        attempt: 1,
        status: 'rejected',
        started_at: at(0),
        finished_at: at(2),
        summary: '5 条验收标准',
      }),
      makeStage({
        id: 'r2',
        attempt: 2,
        status: 'passed',
        started_at: at(4),
        finished_at: at(6),
        summary: '补充了边界',
      }),
    ];
    const decisions = [
      makeDecision({ id: 'd1', stage_run_id: 'r1', decision: 'reject', comment: '缺边界', decided_at: at(3) }),
      makeDecision({ id: 'd2', stage_run_id: 'r2', decision: 'approve', decided_at: at(7) }),
    ];

    const events = buildTimeline(stages, decisions);
    expect(events.map((e) => [e.kind, e.actor, e.attempt])).toEqual([
      ['stage_started', 'ai', 1],
      ['gate_rejected', 'human', 1],
      ['stage_started', 'ai', 2],
      ['stage_passed', 'ai', 2],
      ['gate_approved', 'human', 2],
    ]);
    expect(events[1].text).toBe('缺边界');
    expect(events[3].durationMs).toBe(2 * 60_000);
    expect(events[3].text).toBe('补充了边界');
  });

  it('没有打回决策的 rejected 阶段仍然出「被打回」', () => {
    const events = buildTimeline(
      [makeStage({ status: 'rejected', finished_at: at(1) })],
      []
    );
    expect(events.map((e) => e.kind)).toEqual(['stage_started', 'stage_rejected']);
  });

  it('失败事件的说明优先用 error', () => {
    const events = buildTimeline(
      [
        makeStage({
          stage_key: 'review',
          status: 'failed',
          finished_at: at(3),
          error: '3 轮后仍有 blocker',
          summary: '评审完成',
        }),
      ],
      []
    );
    expect(events[1]).toMatchObject({ kind: 'stage_failed', text: '3 轮后仍有 blocker' });
  });

  it('等待确认且没有 finished_at：与开始同一时刻，排在开始之后', () => {
    const events = buildTimeline([makeStage({ status: 'waiting_gate' })], []);
    expect(events.map((e) => e.kind)).toEqual(['stage_started', 'stage_waiting']);
  });

  it('还没开始的阶段与找不到阶段的决策都忽略', () => {
    const events = buildTimeline(
      [makeStage({ status: 'pending', started_at: null })],
      [makeDecision({ stage_run_id: 'nope' })]
    );
    expect(events).toEqual([]);
  });

  it('文案 key 是字面量', () => {
    expect(timelineTextKey('gate_approved')).toBe('issueDetail.timeline.gateApproved');
    expect(timelineTextKey('stage_started')).toBe('issueDetail.timeline.stageStarted');
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/timeline.test.ts`
Expected: FAIL，`Failed to resolve import "./timeline"`。

- [ ] **Step 3: 实现**

Create `packages/web-core/src/pages/issue-detail/timeline.ts`:
```ts
import type {
  PipelineGateDecision,
  PipelineStageKey,
  PipelineStageRun,
} from 'shared/types';
import { toMillis, toNumber } from '@/entities/pipeline/model/progress';

export type TimelineKind =
  | 'stage_started'
  | 'stage_passed'
  | 'stage_failed'
  | 'stage_waiting'
  | 'stage_rejected'
  | 'gate_approved'
  | 'gate_rejected';

export interface TimelineEvent {
  id: string;
  at: number;
  kind: TimelineKind;
  stageKey: PipelineStageKey;
  attempt: number;
  actor: 'ai' | 'human';
  durationMs: number | null;
  /** 阶段摘要 / 错误 / 打回意见，原样展示。 */
  text: string | null;
}

/** 同一时刻的先后：开始 < 结束 < 人工决策。 */
const KIND_ORDER: Record<TimelineKind, number> = {
  stage_started: 0,
  stage_passed: 1,
  stage_failed: 1,
  stage_waiting: 1,
  stage_rejected: 1,
  gate_approved: 2,
  gate_rejected: 2,
};

const END_KIND: Partial<Record<PipelineStageRun['status'], TimelineKind>> = {
  passed: 'stage_passed',
  failed: 'stage_failed',
  waiting_gate: 'stage_waiting',
  rejected: 'stage_rejected',
};

/** 由阶段记录与关卡决策合成时间线（设计文档 §8.4 右栏）。 */
export function buildTimeline(
  stages: readonly PipelineStageRun[],
  decisions: readonly PipelineGateDecision[]
): TimelineEvent[] {
  const stageById = new Map(stages.map((stage) => [stage.id, stage]));
  const rejectedByDecision = new Set(
    decisions
      .filter((decision) => decision.decision === 'reject')
      .map((decision) => decision.stage_run_id)
  );
  const events: TimelineEvent[] = [];

  for (const stage of stages) {
    const attempt = toNumber(stage.attempt) ?? 1;
    const started = toMillis(stage.started_at);
    const finished = toMillis(stage.finished_at);
    if (started !== null) {
      events.push({
        id: `${stage.id}:started`,
        at: started,
        kind: 'stage_started',
        stageKey: stage.stage_key,
        attempt,
        actor: 'ai',
        durationMs: null,
        text: null,
      });
    }

    const endKind = END_KIND[stage.status];
    const endAt = finished ?? started;
    if (!endKind || endAt === null) continue;
    if (endKind === 'stage_rejected' && rejectedByDecision.has(stage.id)) continue;

    events.push({
      id: `${stage.id}:${endKind}`,
      at: endAt,
      kind: endKind,
      stageKey: stage.stage_key,
      attempt,
      actor: 'ai',
      durationMs:
        started !== null && finished !== null ? Math.max(0, finished - started) : null,
      text: endKind === 'stage_failed' ? (stage.error ?? stage.summary) : stage.summary,
    });
  }

  for (const decision of decisions) {
    const stage = stageById.get(decision.stage_run_id);
    const decidedAt = toMillis(decision.decided_at);
    if (!stage || decidedAt === null) continue;
    events.push({
      id: decision.id,
      at: decidedAt,
      kind: decision.decision === 'approve' ? 'gate_approved' : 'gate_rejected',
      stageKey: stage.stage_key,
      attempt: toNumber(stage.attempt) ?? 1,
      actor: 'human',
      durationMs: null,
      text: decision.comment,
    });
  }

  return events.sort(
    (a, b) => a.at - b.at || KIND_ORDER[a.kind] - KIND_ORDER[b.kind]
  );
}

const TIMELINE_TEXT_KEYS: Record<TimelineKind, string> = {
  stage_started: 'issueDetail.timeline.stageStarted',
  stage_passed: 'issueDetail.timeline.stagePassed',
  stage_failed: 'issueDetail.timeline.stageFailed',
  stage_waiting: 'issueDetail.timeline.stageWaiting',
  stage_rejected: 'issueDetail.timeline.stageRejected',
  gate_approved: 'issueDetail.timeline.gateApproved',
  gate_rejected: 'issueDetail.timeline.gateRejected',
};

export function timelineTextKey(kind: TimelineKind): string {
  return TIMELINE_TEXT_KEYS[kind];
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/timeline.test.ts`
Expected: PASS（6 tests）。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/pages/issue-detail
git commit -m "$(cat <<'EOF'
界面：需求详情时间线由阶段记录与关卡决策合成

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: 产出物解析（CSV / 评审 / 测试报告）

**Files:**
- Create: `packages/web-core/src/pages/issue-detail/artifactParsers.ts`
- Test: `packages/web-core/src/pages/issue-detail/artifactParsers.test.ts`

契约 §4：`review.json` = `{"findings":[{"severity","file","line","message"}]}`；`test-report.json` = `{"total","passed","failed","cases":[{"id","status","attribution","message"}]}`。CSV 是 atp 旧格式，按 RFC 4180 解析（引号、转义引号、引号内逗号与换行、CRLF、BOM）。

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/pages/issue-detail/artifactParsers.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { parseCsv, parseReview, parseTestReport } from './artifactParsers';

describe('parseCsv', () => {
  it('第一行是表头', () => {
    expect(parseCsv('用例编号,标题\nTC-1,下单\nTC-2,撤单\n')).toEqual({
      header: ['用例编号', '标题'],
      rows: [
        ['TC-1', '下单'],
        ['TC-2', '撤单'],
      ],
    });
  });

  it('引号内的逗号、换行与转义引号', () => {
    const csv = 'a,b\n"x, y","第一行\n第二行"\n"say ""hi""",z';
    expect(parseCsv(csv).rows).toEqual([
      ['x, y', '第一行\n第二行'],
      ['say "hi"', 'z'],
    ]);
  });

  it('CRLF 与 BOM', () => {
    expect(parseCsv('﻿a,b\r\n1,2\r\n')).toEqual({
      header: ['a', 'b'],
      rows: [['1', '2']],
    });
  });

  it('空字段保留，空行跳过', () => {
    expect(parseCsv('a,b,c\n1,,3\n\n')).toEqual({
      header: ['a', 'b', 'c'],
      rows: [['1', '', '3']],
    });
  });

  it('空输入', () => {
    expect(parseCsv('')).toEqual({ header: [], rows: [] });
  });
});

describe('parseReview', () => {
  it('解析契约里的最小结构', () => {
    const result = parseReview(
      '{"findings":[{"severity":"blocker","file":"src/a.rs","line":3,"message":"未处理错误"}]}'
    );
    expect(result).toEqual({
      ok: true,
      value: [
        { severity: 'blocker', file: 'src/a.rs', line: 3, message: '未处理错误' },
      ],
    });
  });

  it('不认识的严重度归为 unknown，缺字段给默认值', () => {
    const result = parseReview('{"findings":[{"severity":"nit"}]}');
    expect(result).toEqual({
      ok: true,
      value: [{ severity: 'unknown', file: null, line: null, message: '' }],
    });
  });

  it('不是 JSON 或结构不对', () => {
    expect(parseReview('oops')).toEqual({ ok: false });
    expect(parseReview('{"items":[]}')).toEqual({ ok: false });
  });
});

describe('parseTestReport', () => {
  it('解析契约里的最小结构', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 2,
        passed: 1,
        failed: 1,
        cases: [
          { id: 'kline_021', status: 'failed', attribution: 'code', message: 'next_cursor 应为空' },
          { id: 'kline_020', status: 'passed', attribution: null, message: '' },
        ],
      })
    );
    expect(result).toEqual({
      ok: true,
      value: {
        total: 2,
        passed: 1,
        failed: 1,
        cases: [
          { id: 'kline_021', status: 'failed', attribution: 'code', message: 'next_cursor 应为空' },
          { id: 'kline_020', status: 'passed', attribution: null, message: '' },
        ],
      },
    });
  });

  it('缺计数时按用例现算', () => {
    const result = parseTestReport(
      '{"cases":[{"id":"a","status":"passed"},{"id":"b","status":"failed","attribution":"case"}]}'
    );
    expect(result.ok && result.value).toMatchObject({ total: 2, passed: 1, failed: 1 });
  });

  it('结构不对', () => {
    expect(parseTestReport('[]')).toEqual({ ok: false });
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/artifactParsers.test.ts`
Expected: FAIL，`Failed to resolve import "./artifactParsers"`。

- [ ] **Step 3: 实现**

Create `packages/web-core/src/pages/issue-detail/artifactParsers.ts`:
```ts
export interface CsvTable {
  header: string[];
  rows: string[][];
}

/** RFC 4180 CSV 解析（atp 旧 CSV 用例，契约 §4 `test-cases.csv`）。 */
export function parseCsv(text: string): CsvTable {
  const input = text.startsWith('﻿') ? text.slice(1) : text;
  const records: string[][] = [];
  let record: string[] = [];
  let field = '';
  let inQuotes = false;
  let i = 0;

  while (i < input.length) {
    const ch = input[i];
    if (inQuotes) {
      if (ch === '"') {
        if (input[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i += 1;
        continue;
      }
      field += ch;
      i += 1;
      continue;
    }
    if (ch === '"' && field === '') {
      inQuotes = true;
      i += 1;
      continue;
    }
    if (ch === ',') {
      record.push(field);
      field = '';
      i += 1;
      continue;
    }
    if (ch === '\r' || ch === '\n') {
      record.push(field);
      records.push(record);
      record = [];
      field = '';
      i += ch === '\r' && input[i + 1] === '\n' ? 2 : 1;
      continue;
    }
    field += ch;
    i += 1;
  }
  if (field !== '' || record.length > 0) {
    record.push(field);
    records.push(record);
  }

  const nonEmpty = records.filter((r) => !(r.length === 1 && r[0] === ''));
  const [header = [], ...rows] = nonEmpty;
  return { header, rows };
}

export type ParseResult<T> = { ok: true; value: T } | { ok: false };

function parseJson(content: string): unknown {
  try {
    return JSON.parse(content);
  } catch {
    return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

const str = (value: unknown): string | null =>
  typeof value === 'string' ? value : null;
const num = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) ? value : null;

export type ReviewSeverity = 'blocker' | 'major' | 'minor' | 'unknown';

export interface ReviewFinding {
  severity: ReviewSeverity;
  file: string | null;
  line: number | null;
  message: string;
}

function normalizeSeverity(value: unknown): ReviewSeverity {
  return value === 'blocker' || value === 'major' || value === 'minor'
    ? value
    : 'unknown';
}

/** 评审结果（契约 §4 `review.json`）。 */
export function parseReview(content: string): ParseResult<ReviewFinding[]> {
  const data = parseJson(content);
  if (!isRecord(data) || !Array.isArray(data.findings)) return { ok: false };
  return {
    ok: true,
    value: data.findings.filter(isRecord).map((finding) => ({
      severity: normalizeSeverity(finding.severity),
      file: str(finding.file),
      line: num(finding.line),
      message: str(finding.message) ?? '',
    })),
  };
}

export type TestCaseStatus = 'passed' | 'failed' | 'unknown';
export type TestAttribution = 'code' | 'case' | null;

export interface TestCaseResult {
  id: string;
  status: TestCaseStatus;
  attribution: TestAttribution;
  message: string;
}

export interface TestReport {
  total: number;
  passed: number;
  failed: number;
  cases: TestCaseResult[];
}

/** 测试报告（契约 §4 `test-report.json`）。 */
export function parseTestReport(content: string): ParseResult<TestReport> {
  const data = parseJson(content);
  if (!isRecord(data) || !Array.isArray(data.cases)) return { ok: false };
  const cases: TestCaseResult[] = data.cases.filter(isRecord).map((c) => ({
    id: str(c.id) ?? '',
    status: c.status === 'passed' || c.status === 'failed' ? c.status : 'unknown',
    attribution: c.attribution === 'code' || c.attribution === 'case' ? c.attribution : null,
    message: str(c.message) ?? '',
  }));
  return {
    ok: true,
    value: {
      total: num(data.total) ?? cases.length,
      passed: num(data.passed) ?? cases.filter((c) => c.status === 'passed').length,
      failed: num(data.failed) ?? cases.filter((c) => c.status === 'failed').length,
      cases,
    },
  };
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/artifactParsers.test.ts`
Expected: PASS（11 tests）。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/pages/issue-detail
git commit -m "$(cat <<'EOF'
界面：产出物解析（旧 CSV 用例、评审结果、测试报告）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: 详情页签与关卡条状态

**Files:**
- Create: `packages/web-core/src/pages/issue-detail/issueDetailModel.ts`
- Test: `packages/web-core/src/pages/issue-detail/issueDetailModel.test.ts`

页签与产出物（设计文档 §8.4 + 契约 §4）：需求与验收标准 ← `requirement`；规格 ← `spec`、`plan`；用例 ← `test_cases`、`trace_matrix`；代码变更 ← `review`（外加工作区入口）；测试 ← `test_report`；交付报告 ← `delivery_report`。

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/pages/issue-detail/issueDetailModel.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  ISSUE_DETAIL_TABS,
  artifactKindLabelKey,
  artifactRenderMode,
  defaultIssueDetailTab,
  gateBarState,
  isIssueDetailTab,
  issueDetailTabLabelKey,
  latestArtifactOfKind,
  latestArtifactsByKind,
  tabArtifactKinds,
  tabOfArtifactKind,
} from './issueDetailModel';
import {
  at,
  makeArtifact,
  makeRun,
  makeStage,
  makeView,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('页签', () => {
  it('六个页签，顺序固定', () => {
    expect(ISSUE_DETAIL_TABS).toEqual([
      'requirement',
      'spec',
      'cases',
      'code',
      'test',
      'delivery',
    ]);
    expect(issueDetailTabLabelKey('cases')).toBe('issueDetail.tab.cases');
    expect(isIssueDetailTab('code')).toBe(true);
    expect(isIssueDetailTab('overview')).toBe(false);
  });

  it('页签与产出物种类互相对应', () => {
    expect(tabArtifactKinds('spec')).toEqual(['spec', 'plan']);
    expect(tabArtifactKinds('cases')).toEqual(['test_cases', 'trace_matrix']);
    expect(tabOfArtifactKind('plan')).toBe('spec');
    expect(tabOfArtifactKind('review')).toBe('code');
    expect(tabOfArtifactKind('delivery_report')).toBe('delivery');
  });

  it('默认页签跟着当前阶段走', () => {
    expect(defaultIssueDetailTab(null)).toBe('requirement');
    expect(defaultIssueDetailTab('test_design')).toBe('cases');
    expect(defaultIssueDetailTab('review')).toBe('code');
    expect(defaultIssueDetailTab('deliver')).toBe('delivery');
  });
});

describe('产出物', () => {
  it('同一种类取版本最大的', () => {
    const artifacts = [
      makeArtifact({ id: 'v1', kind: 'spec', version: 1 }),
      makeArtifact({ id: 'v2', kind: 'spec', version: 2 }),
      makeArtifact({ id: 'req', kind: 'requirement' }),
    ];
    expect(latestArtifactOfKind(artifacts, 'spec')?.id).toBe('v2');
    expect(latestArtifactOfKind(artifacts, 'review')).toBeNull();
    expect(latestArtifactsByKind(artifacts).map((a) => a.id)).toEqual(['req', 'v2']);
  });

  it('渲染方式', () => {
    expect(artifactRenderMode('test_cases')).toBe('csv');
    expect(artifactRenderMode('review')).toBe('review');
    expect(artifactRenderMode('test_report')).toBe('test_report');
    expect(artifactRenderMode('trace_matrix')).toBe('markdown');
    expect(artifactKindLabelKey('trace_matrix')).toBe('issueDetail.artifactKind.trace_matrix');
  });
});

describe('gateBarState', () => {
  it('没有流水线', () => {
    expect(gateBarState(null)).toEqual({ kind: 'none' });
  });

  it('人工关卡等待中', () => {
    const view = makeView({
      run: makeRun({ status: 'waiting_gate', current_stage_key: 'spec' }),
      stages: [makeStage({ id: 'sp', stage_key: 'spec', status: 'waiting_gate', gate_kind: 'human' })],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'human',
      runId: 'run-1',
      stageRunId: 'sp',
      stageKey: 'spec',
      gateLabel: '设计规格确认',
    });
  });

  it('人工阶段还在跑（AI 在写需求）', () => {
    expect(gateBarState(makeView())).toEqual({
      kind: 'running',
      runId: 'run-1',
      stageKey: 'requirement',
    });
  });

  it('自动阶段：带轮次', () => {
    const view = makeView({
      run: makeRun({ current_stage_key: 'develop' }),
      stages: [makeStage({ stage_key: 'develop', attempt: 2, gate_kind: 'auto' })],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'auto',
      runId: 'run-1',
      stageKey: 'develop',
      attempt: 2,
      maxRounds: 3,
    });
  });

  it('暂停 / 失败 / 完成 / 取消', () => {
    const kind = (status: string) =>
      gateBarState(makeView({ run: makeRun({ status, current_stage_key: 'review' }) })).kind;
    expect(kind('paused')).toBe('paused');
    expect(kind('failed')).toBe('failed');
    expect(kind('completed')).toBe('completed');
    expect(kind('cancelled')).toBe('cancelled');
  });

  it('失败时带上轮次，便于显示「第 3/3 轮」', () => {
    const view = makeView({
      run: makeRun({ status: 'failed', current_stage_key: 'review', updated_at: at(9) }),
      stages: [makeStage({ stage_key: 'review', attempt: 3, status: 'failed', gate_kind: 'auto' })],
    });
    expect(gateBarState(view)).toEqual({
      kind: 'failed',
      runId: 'run-1',
      stageKey: 'review',
      attempt: 3,
      maxRounds: 3,
    });
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/issueDetailModel.test.ts`
Expected: FAIL，`Failed to resolve import "./issueDetailModel"`。

- [ ] **Step 3: 实现**

Create `packages/web-core/src/pages/issue-detail/issueDetailModel.ts`:
```ts
import type {
  ArtifactKind,
  IssueArtifactSummary,
  IssuePipelineView,
  PipelineStageKey,
} from 'shared/types';
import {
  latestAttempts,
  toMillis,
  toNumber,
} from '@/entities/pipeline/model/progress';

export type IssueDetailTab =
  | 'requirement'
  | 'spec'
  | 'cases'
  | 'code'
  | 'test'
  | 'delivery';

export const ISSUE_DETAIL_TABS: readonly IssueDetailTab[] = [
  'requirement',
  'spec',
  'cases',
  'code',
  'test',
  'delivery',
];

export function isIssueDetailTab(value: unknown): value is IssueDetailTab {
  return (
    typeof value === 'string' &&
    (ISSUE_DETAIL_TABS as readonly string[]).includes(value)
  );
}

const TAB_LABEL_KEYS: Record<IssueDetailTab, string> = {
  requirement: 'issueDetail.tab.requirement',
  spec: 'issueDetail.tab.spec',
  cases: 'issueDetail.tab.cases',
  code: 'issueDetail.tab.code',
  test: 'issueDetail.tab.test',
  delivery: 'issueDetail.tab.delivery',
};

export function issueDetailTabLabelKey(tab: IssueDetailTab): string {
  return TAB_LABEL_KEYS[tab];
}

const TAB_ARTIFACT_KINDS: Record<IssueDetailTab, readonly ArtifactKind[]> = {
  requirement: ['requirement'],
  spec: ['spec', 'plan'],
  cases: ['test_cases', 'trace_matrix'],
  code: ['review'],
  test: ['test_report'],
  delivery: ['delivery_report'],
};

export function tabArtifactKinds(tab: IssueDetailTab): readonly ArtifactKind[] {
  return TAB_ARTIFACT_KINDS[tab];
}

export function tabOfArtifactKind(kind: ArtifactKind): IssueDetailTab {
  return (
    ISSUE_DETAIL_TABS.find((tab) => TAB_ARTIFACT_KINDS[tab].includes(kind)) ??
    'requirement'
  );
}

const STAGE_DEFAULT_TAB: Record<PipelineStageKey, IssueDetailTab> = {
  requirement: 'requirement',
  spec: 'spec',
  test_design: 'cases',
  develop: 'code',
  review: 'code',
  test: 'test',
  deliver: 'delivery',
};

/** 打开详情时默认落在当前阶段对应的页签。 */
export function defaultIssueDetailTab(
  stage: PipelineStageKey | null
): IssueDetailTab {
  return stage ? STAGE_DEFAULT_TAB[stage] : 'requirement';
}

function isNewerArtifact(a: IssueArtifactSummary, b: IssueArtifactSummary) {
  const va = toNumber(a.version) ?? 0;
  const vb = toNumber(b.version) ?? 0;
  if (va !== vb) return va > vb;
  return (toMillis(a.created_at) ?? 0) > (toMillis(b.created_at) ?? 0);
}

export function latestArtifactOfKind(
  artifacts: readonly IssueArtifactSummary[],
  kind: ArtifactKind
): IssueArtifactSummary | null {
  let latest: IssueArtifactSummary | null = null;
  for (const artifact of artifacts) {
    if (artifact.kind !== kind) continue;
    if (!latest || isNewerArtifact(artifact, latest)) latest = artifact;
  }
  return latest;
}

/** 右栏「产出物」列表：每种取最新版，按页签顺序排。 */
export function latestArtifactsByKind(
  artifacts: readonly IssueArtifactSummary[]
): IssueArtifactSummary[] {
  return ISSUE_DETAIL_TABS.flatMap((tab) => TAB_ARTIFACT_KINDS[tab])
    .map((kind) => latestArtifactOfKind(artifacts, kind))
    .filter((artifact): artifact is IssueArtifactSummary => artifact !== null);
}

const ARTIFACT_KIND_LABEL_KEYS: Record<ArtifactKind, string> = {
  requirement: 'issueDetail.artifactKind.requirement',
  spec: 'issueDetail.artifactKind.spec',
  plan: 'issueDetail.artifactKind.plan',
  test_cases: 'issueDetail.artifactKind.test_cases',
  trace_matrix: 'issueDetail.artifactKind.trace_matrix',
  review: 'issueDetail.artifactKind.review',
  test_report: 'issueDetail.artifactKind.test_report',
  delivery_report: 'issueDetail.artifactKind.delivery_report',
};

export function artifactKindLabelKey(kind: ArtifactKind): string {
  return ARTIFACT_KIND_LABEL_KEYS[kind];
}

export type ArtifactRenderMode = 'markdown' | 'csv' | 'review' | 'test_report';

export function artifactRenderMode(kind: ArtifactKind): ArtifactRenderMode {
  switch (kind) {
    case 'test_cases':
      return 'csv';
    case 'review':
      return 'review';
    case 'test_report':
      return 'test_report';
    default:
      return 'markdown';
  }
}

export type GateBarState =
  | { kind: 'none' }
  | { kind: 'completed' }
  | { kind: 'cancelled' }
  | { kind: 'paused'; runId: string; stageKey: PipelineStageKey }
  | {
      kind: 'failed';
      runId: string;
      stageKey: PipelineStageKey;
      attempt: number;
      maxRounds: number | null;
    }
  | {
      kind: 'human';
      runId: string;
      stageRunId: string;
      stageKey: PipelineStageKey;
      gateLabel: string | null;
    }
  | {
      kind: 'auto';
      runId: string;
      stageKey: PipelineStageKey;
      attempt: number;
      maxRounds: number | null;
    }
  | { kind: 'running'; runId: string; stageKey: PipelineStageKey };

/** 底部关卡条显示什么（设计文档 §8.4）。 */
export function gateBarState(view: IssuePipelineView | null): GateBarState {
  if (!view) return { kind: 'none' };
  const { run } = view;
  const stageKey = run.current_stage_key;
  const current =
    latestAttempts(view.stages.filter((s) => s.run_id === run.id)).get(
      stageKey
    ) ?? null;
  const templateStage =
    view.template.stages.find((stage) => stage.key === stageKey) ?? null;
  const attempt = Math.max(1, toNumber(current?.attempt) ?? 1);
  const maxRounds = templateStage ? toNumber(templateStage.max_rounds) : null;

  switch (run.status) {
    case 'completed':
      return { kind: 'completed' };
    case 'cancelled':
      return { kind: 'cancelled' };
    case 'paused':
      return { kind: 'paused', runId: run.id, stageKey };
    case 'failed':
      return { kind: 'failed', runId: run.id, stageKey, attempt, maxRounds };
    case 'waiting_gate':
    case 'running':
      if (
        current &&
        current.status === 'waiting_gate' &&
        current.gate_kind === 'human'
      ) {
        return {
          kind: 'human',
          runId: run.id,
          stageRunId: current.id,
          stageKey,
          gateLabel: templateStage?.gate_label ?? null,
        };
      }
      if (templateStage?.gate_kind === 'auto') {
        return { kind: 'auto', runId: run.id, stageKey, attempt, maxRounds };
      }
      return { kind: 'running', runId: run.id, stageKey };
  }
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/issue-detail/issueDetailModel.test.ts`
Expected: PASS（11 tests）。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/pages/issue-detail
git commit -m "$(cat <<'EOF'
界面：需求详情六个页签与关卡条状态纯函数

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: 工作台纯函数

**Files:**
- Create: `packages/web-core/src/pages/workbench/workbenchModel.ts`
- Test: `packages/web-core/src/pages/workbench/workbenchModel.test.ts`

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/pages/workbench/workbenchModel.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import {
  buildCreateIssueRequest,
  greetingKey,
  pickBacklogStatusId,
  pickDefaultBranch,
  pickWorkbenchProjectId,
  recentDelivered,
  runningRuns,
  stagesFinishedToday,
  weeklyStats,
} from './workbenchModel';
import {
  T0_MS,
  at,
  makeRun,
  makeStage,
} from '@/entities/pipeline/model/__fixtures__/pipeline';

describe('三栏分桶', () => {
  const runs = [
    makeRun({ id: 'a', status: 'running', updated_at: at(1) }),
    makeRun({ id: 'b', status: 'paused', updated_at: at(5) }),
    makeRun({ id: 'c', status: 'waiting_gate' }),
    makeRun({ id: 'd', status: 'completed', finished_at: at(10) }),
    makeRun({ id: 'e', status: 'cancelled', finished_at: at(20) }),
    makeRun({ id: 'f', status: 'failed' }),
  ];

  it('正在自动跑 = running + paused，最近更新在前', () => {
    expect(runningRuns(runs).map((r) => r.id)).toEqual(['b', 'a']);
  });

  it('最近交付 = completed + cancelled，最近结束在前，可限条数', () => {
    expect(recentDelivered(runs).map((r) => r.id)).toEqual(['e', 'd']);
    expect(recentDelivered(runs, 1).map((r) => r.id)).toEqual(['e']);
  });
});

describe('weeklyStats', () => {
  const now = T0_MS + 60 * 60_000;

  it('只算最近 7 天完成的；平均周期按分钟取整；一次通过率', () => {
    const runs = [
      makeRun({ id: 'r1', status: 'completed', created_at: at(0), finished_at: at(40) }),
      makeRun({ id: 'r2', status: 'completed', created_at: at(0), finished_at: at(60) }),
      makeRun({ id: 'old', status: 'completed', created_at: at(-20_000), finished_at: at(-19_000) }),
    ];
    const stages = [
      makeStage({ run_id: 'r1', attempt: 1, status: 'passed' }),
      makeStage({ id: 's2', run_id: 'r2', stage_key: 'review', attempt: 2, status: 'passed' }),
    ];
    expect(weeklyStats(runs, stages, now)).toEqual({
      delivered: 2,
      avgCycleMinutes: 50,
      firstPassRate: 50,
    });
  });

  it('本周没有交付', () => {
    expect(weeklyStats([], [], now)).toEqual({
      delivered: 0,
      avgCycleMinutes: null,
      firstPassRate: null,
    });
  });
});

describe('stagesFinishedToday', () => {
  it('只数今天（本地日期）通过的阶段', () => {
    const now = new Date(2026, 8, 18, 15, 0).getTime();
    const today = new Date(2026, 8, 18, 9, 0).toISOString();
    const yesterday = new Date(2026, 8, 17, 23, 0).toISOString();
    const stages = [
      makeStage({ id: 'a', status: 'passed', finished_at: today }),
      makeStage({ id: 'b', status: 'passed', finished_at: yesterday }),
      makeStage({ id: 'c', status: 'failed', finished_at: today }),
    ];
    expect(stagesFinishedToday(stages, now)).toBe(1);
  });
});

describe('greetingKey', () => {
  it('上午 / 下午 / 晚上', () => {
    expect(greetingKey(9)).toBe('workbench.greeting.morning');
    expect(greetingKey(14)).toBe('workbench.greeting.afternoon');
    expect(greetingKey(20)).toBe('workbench.greeting.evening');
  });
});

describe('建需求', () => {
  it('优先挑 stage_type = backlog 的列', () => {
    const statuses = [
      { id: 'todo', sort_order: 0, stage_type: 'todo' },
      { id: 'backlog', sort_order: 1, stage_type: 'backlog' },
    ];
    expect(pickBacklogStatusId(statuses)).toBe('backlog');
  });

  it('没有 stage_type 时取排序第一列；没有列返回 null', () => {
    expect(
      pickBacklogStatusId([
        { id: 'b', sort_order: 2 },
        { id: 'a', sort_order: 1 },
      ])
    ).toBe('a');
    expect(pickBacklogStatusId([])).toBeNull();
  });

  it('请求体：第一行做标题，其余做描述，id 用传入的', () => {
    expect(
      buildCreateIssueRequest({
        id: 'i1',
        projectId: 'p1',
        statusId: 's1',
        prompt: '  下单最小金额校验\n低于 5 USDT 返回 400  ',
      })
    ).toEqual({
      id: 'i1',
      project_id: 'p1',
      status_id: 's1',
      title: '下单最小金额校验',
      description: '低于 5 USDT 返回 400',
      priority: null,
      start_date: null,
      target_date: null,
      completed_at: null,
      sort_order: 0,
      parent_issue_id: null,
      parent_issue_sort_order: null,
      extension_metadata: {},
    });
  });
});

describe('pickDefaultBranch', () => {
  const branches = [
    { name: 'origin/main', is_current: false, is_remote: true },
    { name: 'dev', is_current: true, is_remote: false },
    { name: 'main', is_current: false, is_remote: false },
  ];

  it('仓库设置了默认目标分支且存在：用它', () => {
    expect(pickDefaultBranch({ default_target_branch: 'main' }, branches)).toBe('main');
  });

  it('否则用当前分支；再否则第一个本地分支；都没有返回 null', () => {
    expect(pickDefaultBranch({ default_target_branch: 'nope' }, branches)).toBe('dev');
    expect(
      pickDefaultBranch({ default_target_branch: null }, [
        { name: 'x', is_current: false, is_remote: false },
      ])
    ).toBe('x');
    expect(pickDefaultBranch({ default_target_branch: null }, [])).toBeNull();
  });
});

describe('pickWorkbenchProjectId', () => {
  it('选中的项目还在就用它，否则第一个', () => {
    const projects = [{ id: 'a' }, { id: 'b' }];
    expect(pickWorkbenchProjectId(projects, 'b')).toBe('b');
    expect(pickWorkbenchProjectId(projects, 'gone')).toBe('a');
    expect(pickWorkbenchProjectId([], null)).toBeNull();
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/workbench/workbenchModel.test.ts`
Expected: FAIL，`Failed to resolve import "./workbenchModel"`。

- [ ] **Step 3: 实现**

Create `packages/web-core/src/pages/workbench/workbenchModel.ts`:
```ts
import type { PipelineRun, PipelineStageRun } from 'shared/types';
import type { CreateIssueRequest } from 'shared/remote-types';
import { toMillis, toNumber } from '@/entities/pipeline/model/progress';
import { splitMessageToTitleDescription } from '@/shared/lib/string';

const byDesc = (value: (run: PipelineRun) => unknown) => (a: PipelineRun, b: PipelineRun) =>
  (toMillis(value(b)) ?? 0) - (toMillis(value(a)) ?? 0);

/** 「正在自动跑」：running + paused（等人工与失败走 pending 接口，进「需要你确认」）。 */
export function runningRuns(runs: readonly PipelineRun[]): PipelineRun[] {
  return runs
    .filter((run) => run.status === 'running' || run.status === 'paused')
    .sort(byDesc((run) => run.updated_at));
}

/** 「最近交付」：completed + cancelled，最近结束在前。 */
export function recentDelivered(
  runs: readonly PipelineRun[],
  limit = 5
): PipelineRun[] {
  return runs
    .filter((run) => run.status === 'completed' || run.status === 'cancelled')
    .sort(byDesc((run) => run.finished_at))
    .slice(0, limit);
}

export interface WeeklyStats {
  delivered: number;
  avgCycleMinutes: number | null;
  /** 百分比整数；本周没有交付为 null。 */
  firstPassRate: number | null;
}

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * 本周统计（草图：交付 7 · 平均周期 52 分 · 一次通过率 71%）。
 * 「一次通过」= 这次运行的所有阶段都只跑了一次。
 */
export function weeklyStats(
  runs: readonly PipelineRun[],
  stages: readonly PipelineStageRun[],
  now: number
): WeeklyStats {
  const delivered = runs.filter((run) => {
    const finished = toMillis(run.finished_at);
    return run.status === 'completed' && finished !== null && now - finished <= WEEK_MS;
  });
  if (delivered.length === 0) {
    return { delivered: 0, avgCycleMinutes: null, firstPassRate: null };
  }
  const cycles = delivered.map(
    (run) => ((toMillis(run.finished_at) ?? 0) - (toMillis(run.created_at) ?? 0)) / 60_000
  );
  const firstPass = delivered.filter((run) =>
    stages
      .filter((stage) => stage.run_id === run.id)
      .every((stage) => (toNumber(stage.attempt) ?? 1) <= 1)
  ).length;
  return {
    delivered: delivered.length,
    avgCycleMinutes: Math.round(cycles.reduce((sum, c) => sum + c, 0) / cycles.length),
    firstPassRate: Math.round((firstPass / delivered.length) * 100),
  };
}

function sameLocalDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}

/** 「今天自动完成 N 个阶段」。 */
export function stagesFinishedToday(
  stages: readonly PipelineStageRun[],
  now: number
): number {
  return stages.filter((stage) => {
    const finished = toMillis(stage.finished_at);
    return stage.status === 'passed' && finished !== null && sameLocalDay(finished, now);
  }).length;
}

export function greetingKey(hour: number): string {
  if (hour < 12) return 'workbench.greeting.morning';
  if (hour < 18) return 'workbench.greeting.afternoon';
  return 'workbench.greeting.evening';
}

/** 新需求落在哪一列：优先 `stage_type = backlog`，否则排序第一列。 */
export function pickBacklogStatusId(
  statuses: readonly { id: string; sort_order: number; stage_type?: unknown }[]
): string | null {
  const backlog = statuses.find((status) => status.stage_type === 'backlog');
  if (backlog) return backlog.id;
  const sorted = [...statuses].sort((a, b) => a.sort_order - b.sort_order);
  return sorted[0]?.id ?? null;
}

/** 工作台输入框 → 建需求请求体（第一行做标题，沿用 `splitMessageToTitleDescription`）。 */
export function buildCreateIssueRequest(args: {
  id: string;
  projectId: string;
  statusId: string;
  prompt: string;
}): CreateIssueRequest & { id: string } {
  const { title, description } = splitMessageToTitleDescription(args.prompt);
  return {
    id: args.id,
    project_id: args.projectId,
    status_id: args.statusId,
    title,
    description,
    priority: null,
    start_date: null,
    target_date: null,
    completed_at: null,
    sort_order: 0,
    parent_issue_id: null,
    parent_issue_sort_order: null,
    extension_metadata: {},
  };
}

/** 目标分支默认值：仓库默认目标分支 → 当前分支 → 第一个本地分支。 */
export function pickDefaultBranch(
  repo: { default_target_branch: string | null },
  branches: readonly { name: string; is_current: boolean; is_remote: boolean }[]
): string | null {
  const local = branches.filter((branch) => !branch.is_remote);
  if (
    repo.default_target_branch &&
    local.some((branch) => branch.name === repo.default_target_branch)
  ) {
    return repo.default_target_branch;
  }
  return local.find((branch) => branch.is_current)?.name ?? local[0]?.name ?? null;
}

export function pickWorkbenchProjectId(
  projects: readonly { id: string }[],
  selectedProjectId: string | null
): string | null {
  if (selectedProjectId && projects.some((p) => p.id === selectedProjectId)) {
    return selectedProjectId;
  }
  return projects[0]?.id ?? null;
}
```

- [ ] **Step 4: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/pages/workbench/workbenchModel.test.ts`
Expected: PASS（12 tests）。

> `splitMessageToTitleDescription`（`shared/lib/string.ts:45-80`）会先 `trim()` 再按第一行切分，所以测试里首尾空白被去掉。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/pages/workbench
git commit -m "$(cat <<'EOF'
界面：工作台三栏分桶、本周统计与建需求请求体纯函数

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: 数据层——集合、接口封装、hooks

**Files:**
- Create: `packages/web-core/src/entities/pipeline/api/pipelineShapes.ts`
- Create: `packages/web-core/src/entities/pipeline/api/pipelineApi.ts`
- Create: `packages/web-core/src/entities/pipeline/model/hooks/usePipelineData.ts`
- Modify: `packages/web-core/src/shared/lib/local/localEndpoints.ts:11-15,18-27,30-39`
- Test: `packages/web-core/src/shared/lib/local/localEndpoints.test.ts`（追加）
- Test: `packages/web-core/src/entities/pipeline/api/pipelineApi.test.ts`

- [ ] **Step 1: 写集合的失败测试**

在 `packages/web-core/src/shared/lib/local/localEndpoints.test.ts` 末尾追加：
```ts
import {
  PIPELINE_RUNS_SHAPE,
  PIPELINE_STAGE_RUNS_SHAPE,
} from '@/entities/pipeline/api/pipelineShapes';
import { patchToWrites } from '@/shared/lib/local/localCollections';

describe('流水线集合（契约 §3）', () => {
  it('pipeline_runs：REST 快照 + 订阅需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PIPELINE_RUNS_SHAPE, { project_id: 'p1' })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/pipeline_runs?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('pipeline_stage_runs：同一条需求流', () => {
    expect(
      resolveLocalShapeEndpoint(PIPELINE_STAGE_RUNS_SHAPE, { project_id: 'p1' })
    ).toEqual({
      kind: 'rest',
      path: '/api/local/pipeline_stage_runs?project_id=p1',
      wsPath: '/api/issues/streams/ws?project_id=p1',
    });
  });

  it('缺 project_id 时是空集合', () => {
    expect(resolveLocalShapeEndpoint(PIPELINE_RUNS_SHAPE, {})).toEqual({
      kind: 'empty',
      table: 'pipeline_runs',
    });
  });

  it('推送路径 /pipeline_runs/{id} 能落成集合写操作', () => {
    expect(
      patchToWrites(
        [{ op: 'replace', path: '/pipeline_runs/r1', value: { id: 'r1' } }],
        'pipeline_runs'
      )
    ).toEqual([{ type: 'update', value: { id: 'r1' } }]);
  });
});
```
（`import` 放到文件顶部已有 import 之后。）

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/shared/lib/local/localEndpoints.test.ts`
Expected: FAIL，`Failed to resolve import "@/entities/pipeline/api/pipelineShapes"`。

- [ ] **Step 3: 形状定义 + 端点表**

Create `packages/web-core/src/entities/pipeline/api/pipelineShapes.ts`:
```ts
import type { PipelineRun, PipelineStageRun } from 'shared/types';
import type { ShapeDefinition } from 'shared/remote-types';

/**
 * 流水线两张表的集合形状（契约 §3）。
 *
 * 云端（`shared/remote-types.ts`）没有这两张表，`defineShape` 也没导出，
 * 这里手写同样的结构。`url` / `fallbackUrl` 只在云端数据源下被读，
 * 本地数据源下 `createShapeCollection` 直接走 `createLocalShapeCollection`
 * （`shared/lib/electric/collections.ts:741-751`），读的是 `localEndpoints.ts`。
 * 使用方（`usePipelineData.ts`）只在个人版启用这两个集合。
 */
export const PIPELINE_RUNS_SHAPE = {
  table: 'pipeline_runs',
  params: ['project_id'],
  url: '',
  fallbackUrl: '',
} as unknown as ShapeDefinition<PipelineRun>;

export const PIPELINE_STAGE_RUNS_SHAPE = {
  table: 'pipeline_stage_runs',
  params: ['project_id'],
  url: '',
  fallbackUrl: '',
} as unknown as ShapeDefinition<PipelineStageRun>;
```

Modify `packages/web-core/src/shared/lib/local/localEndpoints.ts`：

`:11-15` 改为：
```ts
const ISSUE_STREAM_TABLES = new Set([
  'issues',
  'project_statuses',
  'issue_comments',
  // 流水线状态与需求走同一条流（契约 §3）
  'pipeline_runs',
  'pipeline_stage_runs',
]);
```

`:18-27` 的 `REST_RESOURCE` 在 `pull_requests: 'pull_requests',` 之后加两行：
```ts
  pipeline_runs: 'pipeline_runs',
  pipeline_stage_runs: 'pipeline_stage_runs',
```

`:30-39` 的 `ALLOWED_PARAM` 在 `pull_requests: 'project_id',` 之后加两行：
```ts
  pipeline_runs: 'project_id',
  pipeline_stage_runs: 'project_id',
```

> `REST_RESOURCE` 的改动会让 Rust 路由契约测试（`crates/server/src/routes/local_projects/mod.rs:263-316`）要求后端挂 `GET /api/local/pipeline_runs` 与 `GET /api/local/pipeline_stage_runs`——这正是契约 §3 的快照接口，由计划 A 提供。

- [ ] **Step 4: 通过（前端 + Rust 契约测试）**

Run:
```bash
pnpm --filter @vibe/web-core exec vitest run src/shared/lib/local/localEndpoints.test.ts
cargo test -p server --lib 前端用到的本地端点都挂上了路由 2>&1 | tail -3
```
Expected: vitest PASS（原有用例 + 新增 4 个）；cargo 输出 `test result: ok. 1 passed`。cargo 若报 `前端会调用但后端没挂的端点：["GET /api/local/pipeline_runs（路径未挂载）", …]` → 计划 A 尚未挂快照接口，回到 Task 1。

- [ ] **Step 5: 写接口封装的失败测试**

Create `packages/web-core/src/entities/pipeline/api/pipelineApi.test.ts`:
```ts
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { StartPipelineRequest } from 'shared/types';
import {
  PIPELINE_API_PATHS,
  createIssueAndStartPipeline,
  decideGate,
  getIssuePipeline,
  listPendingPipelines,
  runPipelineAction,
} from './pipelineApi';
import {
  setLocalApiTransport,
  type LocalApiRequestOptions,
} from '@/shared/lib/localApiTransport';
import { resetRuntimeModeForTests } from '@/shared/lib/local/runtimeMode';

interface RecordedCall {
  path: string;
  init: LocalApiRequestOptions;
}

let calls: RecordedCall[] = [];
let responses: Response[] = [];

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
const envelope = (data: unknown) =>
  json({ success: true, data, error_data: null, message: null });

const PIPELINE_REQUEST = {
  repos: [{ repo_id: 'repo-1', target_branch: 'main' }],
  executor_config: { executor: 'CLAUDE_CODE' },
  template_key: null,
} as unknown as StartPipelineRequest;

const ISSUE_REQUEST = {
  id: 'i1',
  project_id: 'p1',
  status_id: 's1',
  title: '下单最小金额校验',
  description: null,
  priority: null,
  start_date: null,
  target_date: null,
  completed_at: null,
  sort_order: 0,
  parent_issue_id: null,
  parent_issue_sort_order: null,
  extension_metadata: {},
};

beforeEach(() => {
  calls = [];
  responses = [];
  resetRuntimeModeForTests();
  setLocalApiTransport({
    request: (path, init = {}) => {
      calls.push({ path, init });
      return Promise.resolve(
        responses.shift() ?? new Response(null, { status: 500 })
      );
    },
    openWebSocket: () => {
      throw new Error('not used');
    },
  });
});

afterEach(() => {
  setLocalApiTransport(null);
});

describe('PIPELINE_API_PATHS（契约 §2）', () => {
  it('路径逐字对上契约', () => {
    expect(PIPELINE_API_PATHS.issuePipeline('i1')).toBe('/api/local/issues/i1/pipeline');
    expect(PIPELINE_API_PATHS.artifact('a1')).toBe('/api/local/pipeline/artifacts/a1');
    expect(PIPELINE_API_PATHS.gate('s1')).toBe('/api/local/pipeline/stage-runs/s1/gate');
    expect(PIPELINE_API_PATHS.runAction('r1', 'resume')).toBe('/api/local/pipeline/runs/r1/resume');
    expect(PIPELINE_API_PATHS.pending('p 1')).toBe('/api/local/pipeline/pending?project_id=p%201');
    expect(PIPELINE_API_PATHS.pending(null)).toBe('/api/local/pipeline/pending');
  });
});

describe('信封解析', () => {
  it('没有运行时 data 为 null', async () => {
    responses.push(envelope(null));
    await expect(getIssuePipeline('i1')).resolves.toBeNull();
    expect(calls[0].path).toBe('/api/local/issues/i1/pipeline');
  });

  it('409 抛 PipelineApiError，带后端 message', async () => {
    responses.push(
      json({ success: false, data: null, error_data: null, message: '该阶段不在等待确认' }, 409)
    );
    await expect(
      decideGate('s1', { decision: 'approve', comment: null })
    ).rejects.toMatchObject({ name: 'PipelineApiError', status: 409, message: '该阶段不在等待确认' });
  });

  it('HTTP 200 但 success=false 也算失败', async () => {
    responses.push(json({ success: false, data: null, error_data: null, message: '不行' }));
    await expect(listPendingPipelines('p1')).rejects.toMatchObject({ status: 200, message: '不行' });
  });

  it('写操作是 POST，带 JSON 体', async () => {
    responses.push(envelope({ run: { id: 'r1' } }));
    await decideGate('s1', { decision: 'reject', comment: '缺边界' });
    expect(calls[0].init.method).toBe('POST');
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ decision: 'reject', comment: '缺边界' });
  });

  it('暂停 / 继续 / 取消都是无体 POST', async () => {
    responses.push(envelope({ run: { id: 'r1' } }));
    await runPipelineAction('r1', 'pause');
    expect(calls[0]).toMatchObject({ path: '/api/local/pipeline/runs/r1/pause', init: { method: 'POST' } });
    expect(calls[0].init.body).toBeUndefined();
  });
});

describe('createIssueAndStartPipeline', () => {
  it('先建需求再启动流水线，需求 id 贯穿两步', async () => {
    responses.push(json({ txid: 0 }), envelope({ run: { id: 'r1', issue_id: 'i1' } }));
    await createIssueAndStartPipeline({ issue: ISSUE_REQUEST, pipeline: PIPELINE_REQUEST });
    expect(calls.map((c) => [c.init.method, c.path])).toEqual([
      ['POST', '/api/local/issues'],
      ['POST', '/api/local/issues/i1/pipeline'],
    ]);
    expect(JSON.parse(String(calls[0].init.body)).id).toBe('i1');
    expect(JSON.parse(String(calls[1].init.body))).toEqual(PIPELINE_REQUEST);
  });

  it('建需求失败就不启动流水线', async () => {
    responses.push(json({ message: '标题不能为空' }, 400));
    await expect(
      createIssueAndStartPipeline({ issue: ISSUE_REQUEST, pipeline: PIPELINE_REQUEST })
    ).rejects.toMatchObject({ status: 400, message: '标题不能为空' });
    expect(calls).toHaveLength(1);
  });
});
```

- [ ] **Step 6: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/api/pipelineApi.test.ts`
Expected: FAIL，`Failed to resolve import "./pipelineApi"`。

- [ ] **Step 7: 实现 pipelineApi.ts**

Create `packages/web-core/src/entities/pipeline/api/pipelineApi.ts`:
```ts
import type {
  ApiResponse,
  GateDecisionRequest,
  IssueArtifact,
  IssuePipelineView,
  PendingPipelineItem,
  StartPipelineRequest,
} from 'shared/types';
import type { CreateIssueRequest } from 'shared/remote-types';
import { makeLocalApiRequest } from '@/shared/lib/localApiTransport';
import { parseEnvelopeError } from '@/shared/lib/local/apiEnvelope';

export type PipelineRunAction = 'pause' | 'resume' | 'cancel';

/** 契约 §2 的全部路径。改名先改契约。 */
export const PIPELINE_API_PATHS = {
  issues: '/api/local/issues',
  issuePipeline: (issueId: string) => `/api/local/issues/${issueId}/pipeline`,
  artifact: (artifactId: string) => `/api/local/pipeline/artifacts/${artifactId}`,
  gate: (stageRunId: string) => `/api/local/pipeline/stage-runs/${stageRunId}/gate`,
  runAction: (runId: string, action: PipelineRunAction) =>
    `/api/local/pipeline/runs/${runId}/${action}`,
  pending: (projectId: string | null) =>
    projectId
      ? `/api/local/pipeline/pending?project_id=${encodeURIComponent(projectId)}`
      : '/api/local/pipeline/pending',
} as const;

export class PipelineApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = 'PipelineApiError';
  }
}

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const;

function post(body?: unknown): RequestInit {
  return body === undefined
    ? { method: 'POST' }
    : { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) };
}

/**
 * 走 `ApiResponse<T>` 信封的请求。
 *
 * 和 `adminApi.ts:60 requestLocalEnvelope` 同样的错误语义（400/404/409 的
 * message 可以直接展示），但**不钉死 `hostScope: 'none'`**：流水线数据和看板
 * 集合一样跟随当前 host（`localApiTransport.ts:111-129`）。
 */
async function requestEnvelope<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await makeLocalApiRequest(path, init);
  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new PipelineApiError(
      response.status,
      message ?? `${path} failed with status ${response.status}`
    );
  }
  let envelope: ApiResponse<T> | null = null;
  try {
    envelope = (await response.json()) as ApiResponse<T>;
  } catch {
    envelope = null;
  }
  if (!envelope || !envelope.success) {
    throw new PipelineApiError(
      response.status,
      envelope?.message ?? `${path} returned an unsuccessful response`
    );
  }
  return envelope.data as T;
}

export function getIssuePipeline(issueId: string): Promise<IssuePipelineView | null> {
  return requestEnvelope(PIPELINE_API_PATHS.issuePipeline(issueId));
}

export function startPipeline(
  issueId: string,
  payload: StartPipelineRequest
): Promise<IssuePipelineView> {
  return requestEnvelope(PIPELINE_API_PATHS.issuePipeline(issueId), post(payload));
}

export function getArtifact(artifactId: string): Promise<IssueArtifact> {
  return requestEnvelope(PIPELINE_API_PATHS.artifact(artifactId));
}

export function decideGate(
  stageRunId: string,
  payload: GateDecisionRequest
): Promise<IssuePipelineView> {
  return requestEnvelope(PIPELINE_API_PATHS.gate(stageRunId), post(payload));
}

export function runPipelineAction(
  runId: string,
  action: PipelineRunAction
): Promise<IssuePipelineView> {
  return requestEnvelope(PIPELINE_API_PATHS.runAction(runId, action), post());
}

export function listPendingPipelines(
  projectId: string | null
): Promise<PendingPipelineItem[]> {
  return requestEnvelope(PIPELINE_API_PATHS.pending(projectId));
}

/**
 * 建需求。本地写接口返回 `{ txid }` 而不是信封
 * （`crates/server/src/routes/local_projects/mod.rs:83-91`），只看状态码。
 */
export async function createIssue(payload: CreateIssueRequest): Promise<void> {
  const response = await makeLocalApiRequest(PIPELINE_API_PATHS.issues, post(payload));
  if (!response.ok) {
    const { message } = await parseEnvelopeError(response);
    throw new PipelineApiError(
      response.status,
      message ?? `create issue failed with status ${response.status}`
    );
  }
}

/** 工作台「开始」：先建需求（前端生成 id），再启动流水线。 */
export async function createIssueAndStartPipeline(args: {
  issue: CreateIssueRequest & { id: string };
  pipeline: StartPipelineRequest;
}): Promise<IssuePipelineView> {
  await createIssue(args.issue);
  return startPipeline(args.issue.id, args.pipeline);
}
```

- [ ] **Step 8: 通过**

Run: `pnpm --filter @vibe/web-core exec vitest run src/entities/pipeline/api/pipelineApi.test.ts`
Expected: PASS（8 tests）。

- [ ] **Step 9: hooks（无单测，靠类型检查）**

Create `packages/web-core/src/entities/pipeline/model/hooks/usePipelineData.ts`:
```ts
import { useEffect, useMemo, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { GateDecisionRequest } from 'shared/types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import {
  PIPELINE_RUNS_SHAPE,
  PIPELINE_STAGE_RUNS_SHAPE,
} from '../../api/pipelineShapes';
import {
  createIssueAndStartPipeline,
  decideGate,
  getArtifact,
  getIssuePipeline,
  listPendingPipelines,
  runPipelineAction,
  type PipelineRunAction,
} from '../../api/pipelineApi';
import { pipelineSignature } from '../signature';

export const pipelineQueryKeys = {
  all: ['pipeline'] as const,
  issue: (issueId: string) => ['pipeline', 'issue', issueId] as const,
  artifact: (artifactId: string) => ['pipeline', 'artifact', artifactId] as const,
  pendingAll: ['pipeline', 'pending'] as const,
  pending: (projectId: string | null) =>
    ['pipeline', 'pending', projectId ?? 'all'] as const,
};

/** 个人版且有项目时才开集合（团队版一律不开，保证行为不变）。 */
function collectionEnabled(projectId: string | null): boolean {
  return isLocalPersonalMode() && !!projectId;
}

export function usePipelineRuns(projectId: string | null) {
  const { data, isLoading } = useShape(
    PIPELINE_RUNS_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: collectionEnabled(projectId) }
  );
  return { runs: data, isLoading };
}

export function usePipelineStageRuns(projectId: string | null) {
  const { data, isLoading } = useShape(
    PIPELINE_STAGE_RUNS_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: collectionEnabled(projectId) }
  );
  return { stages: data, isLoading };
}

/** 指纹变化（首次除外）时失效给定查询。 */
function useInvalidateOnChange(signature: string, queryKey: readonly unknown[] | null) {
  const queryClient = useQueryClient();
  const previous = useRef<string | null>(null);
  const keyString = queryKey ? JSON.stringify(queryKey) : null;
  useEffect(() => {
    if (keyString && previous.current !== null && previous.current !== signature) {
      void queryClient.invalidateQueries({ queryKey: JSON.parse(keyString) as unknown[] });
    }
    previous.current = signature;
  }, [keyString, queryClient, signature]);
}

/**
 * 单个需求的流水线全貌。产出物与决策不推送（契约 §3），所以集合里
 * 这条需求的运行 / 阶段一有变化就重拉。
 */
export function useIssuePipeline(projectId: string | null, issueId: string | null) {
  const query = useQuery({
    queryKey: pipelineQueryKeys.issue(issueId ?? ''),
    queryFn: () => getIssuePipeline(issueId as string),
    enabled: !!issueId,
  });
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const signature = useMemo(
    () => pipelineSignature(runs.filter((run) => run.issue_id === issueId), stages),
    [runs, stages, issueId]
  );
  useInvalidateOnChange(signature, issueId ? pipelineQueryKeys.issue(issueId) : null);
  return query;
}

/** 工作台「需要你确认」（等人工 + 失败）。 */
export function usePendingPipelines(projectId: string | null) {
  const query = useQuery({
    queryKey: pipelineQueryKeys.pending(projectId),
    queryFn: () => listPendingPipelines(projectId),
    enabled: collectionEnabled(projectId),
  });
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const signature = useMemo(() => pipelineSignature(runs, stages), [runs, stages]);
  useInvalidateOnChange(signature, projectId ? pipelineQueryKeys.pending(projectId) : null);
  return query;
}

/** 产出物按版本不可变，拉一次就够。 */
export function useArtifact(artifactId: string | null) {
  return useQuery({
    queryKey: pipelineQueryKeys.artifact(artifactId ?? ''),
    queryFn: () => getArtifact(artifactId as string),
    enabled: !!artifactId,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useGateDecision() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      issueId: string;
      stageRunId: string;
      request: GateDecisionRequest;
    }) => decideGate(args.stageRunId, args.request),
    onSuccess: (view, args) => {
      queryClient.setQueryData(pipelineQueryKeys.issue(args.issueId), view);
      void queryClient.invalidateQueries({ queryKey: pipelineQueryKeys.pendingAll });
    },
  });
}

export function usePipelineRunAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: { issueId: string; runId: string; action: PipelineRunAction }) =>
      runPipelineAction(args.runId, args.action),
    onSuccess: (view, args) => {
      queryClient.setQueryData(pipelineQueryKeys.issue(args.issueId), view);
      void queryClient.invalidateQueries({ queryKey: pipelineQueryKeys.pendingAll });
    },
  });
}

export function useStartPipeline() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createIssueAndStartPipeline,
    onSuccess: (view) => {
      queryClient.setQueryData(pipelineQueryKeys.issue(view.run.issue_id), view);
      void queryClient.invalidateQueries({ queryKey: pipelineQueryKeys.pendingAll });
    },
  });
}
```

Run:
```bash
pnpm --filter @vibe/web-core run check && pnpm --filter @vibe/web-core run test 2>&1 | tail -3
```
Expected: `tsc` 0 error；vitest 全部通过。

- [ ] **Step 10: 提交**

```bash
pnpm --filter @vibe/web-core run format >/dev/null
git add packages/web-core/src/entities/pipeline packages/web-core/src/shared/lib/local/localEndpoints.ts packages/web-core/src/shared/lib/local/localEndpoints.test.ts
git commit -m "$(cat <<'EOF'
界面：流水线集合、接口封装与数据 hooks

pipeline_runs / pipeline_stage_runs 两个集合走 REST 快照 + 需求流增量；
接口按契约 §2 封装，集合指纹变化时失效详情与待处理查询。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: 外壳——AppBar 分组标题国际化 + 外网徽标开关

**Files:**
- Modify: `packages/ui/src/components/AppBar.tsx:266,272,346,354`
- Modify: `packages/web-core/src/shared/hooks/useGitHubStars.ts:27-37`
- Modify: `packages/web-core/src/shared/hooks/useDiscordOnlineCount.ts:29-39`

本任务是文案替换与 hook 加开关，没有可单测的逻辑；正确性靠类型检查 + Task 28 团队版回归。

- [ ] **Step 1: 分组标题走 i18n**

Modify `packages/ui/src/components/AppBar.tsx`（`t` 已在 `:238` 定义）：

`:266`
```tsx
    sections.push({ key: 'local', label: 'Local', items: localItems });
```
改为
```tsx
    sections.push({
      key: 'local',
      label: t('appBar.sections.local'),
      items: localItems,
    });
```

`:272` 的 `label: 'Remote',` 改为 `label: t('appBar.sections.remote'),`

`:346` 的 `label: 'Projects',` 改为 `label: t('appBar.sections.projects'),`

`:354` 的 `label: 'Export',` 改为 `label: t('appBar.sections.export'),`

- [ ] **Step 2: 两个外网请求 hook 加可选开关**

Modify `packages/web-core/src/shared/hooks/useGitHubStars.ts:27-37`，整段替换为：
```ts
/**
 * 个人版不显示 GitHub 徽标（设计文档 §8.1），调用方传 `enabled: false`
 * 连请求也不发。默认开启，团队版与云端构建行为不变。
 */
export function useGitHubStars(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ['github-stars'],
    queryFn: fetchGitHubStars,
    enabled: options.enabled ?? true,
    refetchInterval: 10 * 60 * 1000,
    staleTime: 10 * 60 * 1000,
    retry: false,
    refetchOnMount: false,
    placeholderData: (previousData) => previousData,
  });
}
```

Modify `packages/web-core/src/shared/hooks/useDiscordOnlineCount.ts:29-39`，整段替换为：
```ts
/** 同 `useGitHubStars`：个人版传 `enabled: false`。 */
export function useDiscordOnlineCount(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ['discord-online-count'],
    queryFn: fetchDiscordOnlineCount,
    enabled: options.enabled ?? true,
    refetchInterval: 10 * 60 * 1000,
    staleTime: 10 * 60 * 1000,
    retry: false,
    refetchOnMount: false,
    placeholderData: (previousData) => previousData,
  });
}
```

- [ ] **Step 3: 确认没有别的调用方被破坏**

Run:
```bash
grep -rn "useGitHubStars(\|useDiscordOnlineCount(" packages/web-core/src packages/local-web/src packages/remote-web/src | grep -v "export function"
```
Expected: 只有 `SharedAppLayout.tsx:76-77` 两处（Task 15 会改）。若 remote-web 有调用，参数可选，不受影响。

- [ ] **Step 4: 类型检查**

Run: `pnpm run ui:check && pnpm run web-core:check`
Expected: 两个都 0 error。

- [ ] **Step 5: 提交**

```bash
pnpm run ui:format >/dev/null && pnpm run web-core:format >/dev/null
git add packages/ui/src/components/AppBar.tsx packages/web-core/src/shared/hooks/useGitHubStars.ts packages/web-core/src/shared/hooks/useDiscordOnlineCount.ts
git commit -m "$(cat <<'EOF'
界面：侧栏分组标题走国际化，外网徽标请求可关闭

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: 外壳——个人版侧栏视图与导航纯函数

**Files:**
- Create: `packages/web-core/src/shared/lib/routes/personalRoutes.ts`
- Test: `packages/web-core/src/shared/lib/routes/personalRoutes.test.ts`
- Create: `packages/ui/src/components/PersonalSidebar.tsx`

- [ ] **Step 1: 写失败测试**

Create `packages/web-core/src/shared/lib/routes/personalRoutes.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { PERSONAL_ROUTES, personalNavActiveKey } from './personalRoutes';

describe('PERSONAL_ROUTES', () => {
  it('固定路径与带参路径', () => {
    expect(PERSONAL_ROUTES.workbench).toBe('/home');
    expect(PERSONAL_ROUTES.docs).toBe('/docs');
    expect(PERSONAL_ROUTES.testing).toBe('/testing');
    expect(PERSONAL_ROUTES.project('p1')).toBe('/projects/p1');
    expect(PERSONAL_ROUTES.issueDetail('p1', 'i 1')).toBe(
      '/projects/p1/issues/i%201/detail'
    );
  });
});

describe('personalNavActiveKey', () => {
  it('按路径前缀判断当前导航项', () => {
    expect(personalNavActiveKey('/home')).toBe('workbench');
    expect(personalNavActiveKey('/projects/p1')).toBe('pipeline');
    expect(personalNavActiveKey('/projects/p1/issues/i1/detail')).toBe('pipeline');
    expect(personalNavActiveKey('/testing')).toBe('testing');
    expect(personalNavActiveKey('/docs')).toBe('docs');
  });

  it('其它页面没有高亮项；前缀不能误伤', () => {
    expect(personalNavActiveKey('/workspaces')).toBeNull();
    expect(personalNavActiveKey('/homepage')).toBeNull();
    expect(personalNavActiveKey('/')).toBeNull();
  });
});
```

- [ ] **Step 2: 看它失败**

Run: `pnpm --filter @vibe/web-core exec vitest run src/shared/lib/routes/personalRoutes.test.ts`
Expected: FAIL，`Failed to resolve import "./personalRoutes"`。

- [ ] **Step 3: 实现并通过**

Create `packages/web-core/src/shared/lib/routes/personalRoutes.ts`:
```ts
/**
 * 个人版的路由路径（计划 §1.2）。
 *
 * `KanbanContainer` / `KanbanIssuePanelContainer` 也被 remote-web 编译，
 * 那里没有这些路由，所以共享代码里用 `router.history.push(路径字符串)`
 * 而不是带类型的 `navigate({ to })`；只在 local-web 独有的代码里用后者。
 */
export const PERSONAL_ROUTES = {
  workbench: '/home',
  docs: '/docs',
  testing: '/testing',
  project: (projectId: string) => `/projects/${encodeURIComponent(projectId)}`,
  issueDetail: (projectId: string, issueId: string) =>
    `/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/detail`,
} as const;

/** 与 `@vibe/ui/components/PersonalSidebar` 的同名类型逐字一致。 */
export type PersonalNavKey =
  | 'workbench'
  | 'pipeline'
  | 'testing'
  | 'docs'
  | 'settings';

function matches(pathname: string, prefix: string): boolean {
  return pathname === prefix || pathname.startsWith(`${prefix}/`);
}

/** 当前路径高亮哪个导航项（设置是对话框，永远不高亮）。 */
export function personalNavActiveKey(pathname: string): PersonalNavKey | null {
  if (matches(pathname, '/home')) return 'workbench';
  if (matches(pathname, '/projects')) return 'pipeline';
  if (matches(pathname, '/testing')) return 'testing';
  if (matches(pathname, '/docs')) return 'docs';
  return null;
}
```

Run: `pnpm --filter @vibe/web-core exec vitest run src/shared/lib/routes/personalRoutes.test.ts`
Expected: PASS（3 tests）。

- [ ] **Step 4: 侧栏视图**

Create `packages/ui/src/components/PersonalSidebar.tsx`:
```tsx
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  BookOpenIcon,
  CaretDownIcon,
  FlaskIcon,
  GearIcon,
  HouseIcon,
  KanbanIcon,
  PlusIcon,
  type Icon,
} from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './Dropdown';

/** 与 `web-core/src/shared/lib/routes/personalRoutes.ts` 的同名类型逐字一致。 */
export type PersonalNavKey =
  | 'workbench'
  | 'pipeline'
  | 'testing'
  | 'docs'
  | 'settings';

export interface PersonalSidebarProject {
  id: string;
  name: string;
  /** HSL 三元组字符串，与 `projects.color` 一致。 */
  color: string;
}

export interface PersonalSidebarProps {
  projects: PersonalSidebarProject[];
  activeProjectId: string | null;
  onProjectSelect: (projectId: string) => void;
  onCreateProject: () => void;
  activeKey: PersonalNavKey | null;
  onNavigate: (key: PersonalNavKey) => void;
  /** 「工作台」旁的待处理数（等人工 + 失败）。 */
  pendingCount: number;
  appVersion: string | null;
  /** 用户菜单等底部附加内容。 */
  userSlot?: ReactNode;
  className?: string;
}

const NAV_ITEMS: { key: PersonalNavKey; labelKey: string; icon: Icon }[] = [
  { key: 'workbench', labelKey: 'nav.workbench', icon: HouseIcon },
  { key: 'pipeline', labelKey: 'nav.pipeline', icon: KanbanIcon },
  { key: 'testing', labelKey: 'nav.testing', icon: FlaskIcon },
  { key: 'docs', labelKey: 'nav.docs', icon: BookOpenIcon },
  { key: 'settings', labelKey: 'nav.settings', icon: GearIcon },
];

/**
 * 个人版左侧栏（设计文档 §8.1 + 草图①左栏）：顶部项目下拉（颜色块 + 全名），
 * 五个中文入口，底部版本号。无状态：数据与回调全部走 props。
 *
 * 选中态用 `bg-primary text-high`，**不用品牌橙**——设计文档 §8.5：
 * 橙只用于主操作与「等你处理」（这里的待处理计数）。
 */
export function PersonalSidebar({
  projects,
  activeProjectId,
  onProjectSelect,
  onCreateProject,
  activeKey,
  onNavigate,
  pendingCount,
  appVersion,
  userSlot,
  className,
}: PersonalSidebarProps) {
  const { t } = useTranslation('common');
  const activeProject =
    projects.find((project) => project.id === activeProjectId) ?? null;

  return (
    <div
      data-testid="personal-sidebar"
      className={cn(
        'flex h-full min-h-0 w-52 flex-col gap-base overflow-y-auto',
        'border-r border-border bg-secondary p-base',
        className
      )}
    >
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={t('nav.projectSwitcher')}
            className={cn(
              'flex w-full items-center gap-half rounded-sm px-half py-half',
              'text-left text-sm text-high hover:bg-primary',
              'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand'
            )}
          >
            <span
              aria-hidden="true"
              className="size-3 shrink-0 rounded-sm bg-panel"
              style={
                activeProject
                  ? { backgroundColor: `hsl(${activeProject.color})` }
                  : undefined
              }
            />
            <span className="min-w-0 flex-1 truncate">
              {activeProject?.name ?? t('nav.noProject')}
            </span>
            <CaretDownIcon className="size-icon-xs shrink-0 text-low" weight="bold" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {projects.map((project) => (
            <DropdownMenuItem
              key={project.id}
              onSelect={() => onProjectSelect(project.id)}
            >
              <span className="flex min-w-0 items-center gap-half">
                <span
                  aria-hidden="true"
                  className="size-3 shrink-0 rounded-sm"
                  style={{ backgroundColor: `hsl(${project.color})` }}
                />
                <span className="truncate">{project.name}</span>
              </span>
            </DropdownMenuItem>
          ))}
          {projects.length > 0 && <DropdownMenuSeparator />}
          <DropdownMenuItem icon={PlusIcon} onSelect={onCreateProject}>
            {t('nav.createProject')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {NAV_ITEMS.map((item) => {
          const isActive = item.key === activeKey;
          return (
            <li key={item.key}>
              <button
                type="button"
                onClick={() => onNavigate(item.key)}
                aria-current={isActive ? 'page' : undefined}
                className={cn(
                  'flex w-full items-center gap-half rounded-sm px-half py-half',
                  'text-sm transition-colors',
                  'focus:outline-none focus-visible:ring-1 focus-visible:ring-brand',
                  isActive
                    ? 'bg-primary font-medium text-high'
                    : 'text-normal hover:bg-primary'
                )}
              >
                <item.icon
                  className={cn(
                    'size-icon-sm shrink-0',
                    isActive ? 'text-high' : 'text-low'
                  )}
                  weight="bold"
                />
                <span className="flex-1 truncate text-left">
                  {t(item.labelKey)}
                </span>
                {item.key === 'workbench' && pendingCount > 0 && (
                  <span className="rounded-sm bg-brand px-1 font-ibm-plex-mono text-xs text-on-brand">
                    {pendingCount}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>

      <div className="mt-auto flex flex-col gap-half">
        {userSlot}
        {appVersion && (
          <p data-testid="nav-footer" className="m-0 truncate text-xs text-low">
            {t('nav.footer', { version: appVersion })}
          </p>
        )}
      </div>
    </div>
  );
}
```

Run: `pnpm run ui:check`
Expected: 0 error。若报 `DropdownMenuItem` 不接受 `onSelect`：它是 Radix `DropdownMenu.Item` 的封装（`packages/ui/src/components/Dropdown.tsx:181-230`），`onSelect` 是 Radix 原生 prop，应当通过；若确实不通过，改用 `onClick`。

- [ ] **Step 5: 词条引用自检**

Run:
```bash
node scripts/check-unused-i18n-keys.mjs --list 2>/dev/null | grep -E "^\s*common:nav\." || echo "nav.* 全部已引用"
```
Expected: 输出 `nav.* 全部已引用`。

- [ ] **Step 6: 提交**

```bash
pnpm run ui:format >/dev/null && pnpm run web-core:format >/dev/null
git add packages/web-core/src/shared/lib/routes packages/ui/src/components/PersonalSidebar.tsx
git commit -m "$(cat <<'EOF'
界面：个人版左侧栏视图（项目下拉 + 五个中文入口）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: 外壳——SharedAppLayout 按模式切换

**Files:**
- Modify: `packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx`（`:1-62` import、`:64-83`、`:185-186`、`:197-205` 之后、`:347-386`、`:439-567`）

**顺序要求：先完成 Task 17 的 Step 1～3（三个路由文件 + 生成 `routeTree.gen.ts`），再做本任务。** 否则本任务里带类型的 `navigate({ to: '/home' })` 在 `local-web:check` 过不去。

每处条件都以 `isPersonalShell = isLocalPersonalMode()` 为准（`runtimeMode.ts:63-65`：本地数据源 + 不需要登录）。团队版（`isLocalTeamMode()`）与云端构建时它为 `false`，走原代码。

- [ ] **Step 1: import 与模式常量**

在 `:62`（`import { CloudShutdownExportBanner } …`）之后追加：
```ts
import {
  PersonalSidebar,
  type PersonalNavKey,
} from '@vibe/ui/components/PersonalSidebar';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import {
  PERSONAL_ROUTES,
  personalNavActiveKey,
} from '@/shared/lib/routes/personalRoutes';
import { usePendingPipelines } from '@/entities/pipeline/model/hooks/usePipelineData';
```

> 分层说明：shared 层按 eslint 规则不该 import `@/entities`，但本文件已有 `@/pages/workspaces/*` 的先例（`:58-59`），且 web-core 目录不在任何 eslint 范围内（根 `lint` 只跑 local-web 与 ui）。这是外壳唯一需要流水线数据的地方（工作台待处理计数），接受这一处例外。

`:65`（`const appNavigation = useAppNavigation();`）之前插入：
```ts
  // 个人版外壳（设计文档 §8.1）。运行时模式在入口 bootstrap 后就固定，
  // 整个进程生命周期内不变（见 local-web App.tsx 的 AuthBoundary 注释）。
  const isPersonalShell = isLocalPersonalMode();
```

- [ ] **Step 2: 外网徽标请求、路由参数、停服横幅**

`:76-77` 改为：
```ts
  const { data: onlineCount } = useDiscordOnlineCount({
    enabled: !isPersonalShell,
  });
  const { data: starCount } = useGitHubStars({ enabled: !isPersonalShell });
```

`:81` 改为：
```ts
  const { hostId: routeHostId, projectId: routeProjectId } = useParams({
    strict: false,
  });
```

`:185-186` 改为：
```ts
  // 个人版不渲染停服横幅（设计文档 §8.1）；团队版 / 云端构建条件不变。
  const showCloudShutdownBanner =
    !isPersonalShell &&
    (isExportActive ||
      (isSignedIn && isProjectDestination(currentDestination)));
```

- [ ] **Step 3: 个人版导航状态与回调**

在 `:205`（「Persist last selected project」那个 `useEffect` 的结束 `}, [activeProjectId, setSelectedProjectId]);`）之后插入：
```ts

  // ---- 个人版左侧栏（设计文档 §8.1）----
  const selectedProjectId = useUiPreferencesStore((s) => s.selectedProjectId);
  // 详情全屏页不属于 AppDestination 体系，项目 id 从路由参数拿。
  const personalProjectId =
    activeProjectId ??
    routeProjectId ??
    selectedProjectId ??
    orderedProjects[0]?.id ??
    null;
  const personalActiveKey = personalNavActiveKey(location.pathname);
  const { data: pendingItems } = usePendingPipelines(
    isPersonalShell ? personalProjectId : null
  );

  const handlePersonalNavigate = useCallback(
    (key: PersonalNavKey) => {
      switch (key) {
        case 'workbench':
          void navigate({ to: PERSONAL_ROUTES.workbench });
          return;
        case 'pipeline':
          if (personalProjectId) {
            appNavigation.goToProject(personalProjectId);
          } else {
            void navigate({ to: PERSONAL_ROUTES.workbench });
          }
          return;
        case 'testing':
          void navigate({ to: PERSONAL_ROUTES.testing });
          return;
        case 'docs':
          void navigate({ to: PERSONAL_ROUTES.docs });
          return;
        case 'settings':
          void SettingsDialog.show();
          return;
      }
    },
    [appNavigation, navigate, personalProjectId]
  );

  const handlePersonalProjectSelect = useCallback(
    (projectId: string) => {
      setSelectedProjectId(projectId);
      if (personalActiveKey === 'pipeline') {
        appNavigation.goToProject(projectId);
      }
    },
    [appNavigation, personalActiveKey, setSelectedProjectId]
  );
```

- [ ] **Step 4: 桌面左栏按模式渲染**

`AppBar` 那一段（原 `:347` 的 `<AppBar` 到 `:386` 的 `/>`）**一字不改**，只在它前后各插一段，让它成为条件的 else 分支。

在原 `:346`（`{/* Desktop AppBar sidebar. */}`）之后、`:347`（`<AppBar`）之前插入：
```tsx
            {isPersonalShell ? (
              <PersonalSidebar
                projects={orderedProjects}
                activeProjectId={personalProjectId}
                onProjectSelect={handlePersonalProjectSelect}
                onCreateProject={handleCreateProject}
                activeKey={personalActiveKey}
                onNavigate={handlePersonalNavigate}
                pendingCount={pendingItems?.length ?? 0}
                appVersion={appVersion}
                userSlot={
                  <AppBarUserPopoverContainer
                    organizations={organizations}
                    selectedOrgId={selectedOrgId ?? ''}
                    onOrgSelect={setSelectedOrgId}
                  />
                }
              />
            ) : (
```

在原 `:386`（`AppBar` 的结束 `/>`）之后插入：
```tsx
            )}
```

`PersonalSidebar` 的 `projects` 需要 `{ id, name, color }`，`orderedProjects` 是 `RemoteProject[]`（`shared/remote-types.ts:7`，含这三个字段），结构兼容。

- [ ] **Step 5: 移动端抽屉按模式渲染**

抽屉原内容（原 `:443` 的 `<div className="flex flex-col h-full">` 到 `:566` 与之配对的 `</div>`）**一字不改**，前后各插一段。

在原 `:442`（`>`，即 `<MobileDrawer …>` 开标签结束）之后、`:443` 之前插入：
```tsx
          {isPersonalShell ? (
            <PersonalSidebar
              className="w-full border-r-0"
              projects={orderedProjects}
              activeProjectId={personalProjectId}
              onProjectSelect={(projectId) => {
                handlePersonalProjectSelect(projectId);
                setIsDrawerOpen(false);
              }}
              onCreateProject={() => {
                void handleCreateProject();
                setIsDrawerOpen(false);
              }}
              activeKey={personalActiveKey}
              onNavigate={(key) => {
                handlePersonalNavigate(key);
                setIsDrawerOpen(false);
              }}
              pendingCount={pendingItems?.length ?? 0}
              appVersion={appVersion}
            />
          ) : (
```

在原 `:566` 的 `</div>`（`:567` 是 `</MobileDrawer>`）之后插入：
```tsx
          )}
```

- [ ] **Step 6: 验证与提交**

Run:
```bash
pnpm run web-core:check && pnpm run local-web:check && pnpm run remote-web:check
```
Expected: 三个都 0 error。（`local-web:check` 依赖 `/home`、`/docs` 已在 `routeTree.gen.ts` 里——见本任务开头的顺序要求。）

然后手动冒烟：`pnpm run dev`，浏览器打开前端：左侧是项目下拉 + 工作台 / 需求流水线 / 测试中心 / 文档 / 设置；无停服横幅、无 GitHub / Discord 徽标。

```bash
pnpm run web-core:format >/dev/null
git add packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx
git commit -m "$(cat <<'EOF'
界面：个人版外壳换成五项中文导航，去掉停服横幅与外网徽标

团队版与云端构建仍渲染原 AppBar 与原横幅条件。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: 设置——导出入口

**Files:**
- Modify: `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx:101-102`
- Modify: `packages/web-core/src/shared/dialogs/settings/settings/GeneralSettingsSection.tsx:57`、`:817-837` 之后

- [ ] **Step 1: 常规设置拿到 onClose**

Modify `settingsRegistry.tsx:101-102`：
```tsx
    case 'general':
      return <GeneralSettingsSection />;
```
改为
```tsx
    case 'general':
      return <GeneralSettingsSection onClose={onClose} />;
```

- [ ] **Step 2: 常规设置末尾加导出卡片（仅个人版）**

Modify `GeneralSettingsSection.tsx`：

文件顶部 import 区追加：
```ts
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
```

`:57` 的
```tsx
export function GeneralSettingsSection() {
```
改为
```tsx
export function GeneralSettingsSection({
  onClose,
}: { onClose?: () => void } = {}) {
```

在函数体第一行（`const { t } = useTranslation(['settings', 'common']);`）之后插入：
```tsx
  const appNavigation = useAppNavigation();
  // 个人版把导出入口从左侧栏挪到这里（设计文档 §8.1）。
  const handleOpenExport = useCallback(() => {
    onClose?.();
    appNavigation.goToExport();
  }, [appNavigation, onClose]);
```
（`useCallback` 已在 `:1` 导入。）

在 Safety 卡片（`:817-837`）之后、`<SettingsSaveBar`（`:839`）之前插入：
```tsx
      {/* 数据导出（仅个人版；团队版导出仍在左侧 AppBar） */}
      {isLocalPersonalMode() && (
        <SettingsCard
          title={t('settings.general.export.title')}
          description={t('settings.general.export.description')}
        >
          <div className="flex justify-end">
            <PrimaryButton
              variant="tertiary"
              value={t('settings.general.export.button')}
              onClick={handleOpenExport}
            />
          </div>
        </SettingsCard>
      )}
```

- [ ] **Step 3: 验证**

Run: `pnpm run web-core:check && pnpm run remote-web:check`
Expected: 0 error（`GeneralSettingsSection` 被 remote-web 编译时 `onClose` 可选，不受影响）。

- [ ] **Step 4: 提交**

```bash
pnpm run web-core:format >/dev/null
git add packages/web-core/src/shared/dialogs/settings/settings
git commit -m "$(cat <<'EOF'
界面：个人版把数据导出入口挪进设置

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: 路由——工作台 / 文档 / 详情 + 根跳转

**Files:**
- Create: `packages/local-web/src/routes/_app.home.tsx`
- Create: `packages/local-web/src/routes/_app.docs.tsx`
- Create: `packages/local-web/src/routes/_app.projects.$projectId_.issues.$issueId_.detail.tsx`
- Create: `packages/web-core/src/pages/docs/DocsPlaceholderPage.tsx`
- Create（占位，Task 20 / 21 覆盖）: `packages/web-core/src/pages/workbench/WorkbenchPage.tsx`、`packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx`
- Modify: `packages/web-core/src/pages/root/RootRedirectPage.tsx:1-58`
- Generated: `packages/local-web/src/routeTree.gen.ts`

> 若已按 Task 15 的顺序要求提前做过 Step 1～3，这里从 Step 4 开始。

- [ ] **Step 1: 页面占位与文档页**

Create `packages/web-core/src/pages/workbench/WorkbenchPage.tsx`（Task 20 整体替换）：
```tsx
export function WorkbenchPage() {
  return null;
}
```

Create `packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx`（Task 21 整体替换）：
```tsx
export function IssueDetailPage(_props: { projectId: string; issueId: string }) {
  return null;
}
```

Create `packages/web-core/src/pages/docs/DocsPlaceholderPage.tsx`:
```tsx
import { useTranslation } from 'react-i18next';
import { BookOpenIcon } from '@phosphor-icons/react';

/**
 * 「文档」导航项的空态占位（设计文档 §8.1）。
 * 下一期把各需求的 requirement / spec / plan 产出物归档到这里。
 */
export function DocsPlaceholderPage() {
  const { t } = useTranslation('common');

  return (
    <div className="h-full overflow-y-auto bg-primary px-double py-double">
      <div className="mx-auto flex max-w-3xl flex-col gap-base">
        <div className="flex items-center gap-half">
          <BookOpenIcon className="size-icon-base text-low" weight="bold" />
          <h2 className="m-0 text-xl font-medium text-high">{t('docs.title')}</h2>
        </div>
        <div
          data-testid="docs-empty"
          className="rounded-sm border border-dashed border-border p-double text-center"
        >
          <p className="m-0 text-base text-normal">{t('docs.emptyTitle')}</p>
          <p className="m-0 mt-half text-sm text-low">{t('docs.emptyHint')}</p>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 三个路由文件**

Create `packages/local-web/src/routes/_app.home.tsx`:
```tsx
import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { WorkbenchPage } from '@/pages/workbench/WorkbenchPage';

/** 工作台（设计文档 §8.2）。只在个人版存在，团队版送回首页。 */
export const Route = createFileRoute('/_app/home')({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: WorkbenchPage,
});
```

Create `packages/local-web/src/routes/_app.docs.tsx`:
```tsx
import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { DocsPlaceholderPage } from '@/pages/docs/DocsPlaceholderPage';

export const Route = createFileRoute('/_app/docs')({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: DocsPlaceholderPage,
});
```

Create `packages/local-web/src/routes/_app.projects.$projectId_.issues.$issueId_.detail.tsx`:
```tsx
import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { IssueDetailPage } from '@/pages/issue-detail/IssueDetailPage';

function IssueDetailRouteComponent() {
  const { projectId, issueId } = Route.useParams();
  return <IssueDetailPage projectId={projectId} issueId={issueId} />;
}

/**
 * 需求详情全屏页（设计文档 §8.4）。`$projectId_` / `$issueId_` 表示不嵌套在
 * 看板路由下（否则会被 `LocalProjectKanban` 包住）。原右侧面板路由不动。
 */
export const Route = createFileRoute(
  '/_app/projects/$projectId_/issues/$issueId_/detail'
)({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: IssueDetailRouteComponent,
});
```

- [ ] **Step 3: 生成 routeTree**

`routeTree.gen.ts` 由 vite 的 `tanstackRouter` 插件生成（`packages/local-web/vite.config.ts`），不要手改。`build` 脚本是 `tsc && vite build`，tsc 会先因为新路由不在树里而失败，所以直接跑 vite：
```bash
cd packages/local-web && pnpm exec vite build --logLevel error >/dev/null; cd ../..
grep -c "_app/home\|_app/docs\|issues/\$issueId_/detail" packages/local-web/src/routeTree.gen.ts
```
Expected: 计数大于等于 `3`。

- [ ] **Step 4: 根跳转（个人版 → 工作台）**

Modify `packages/web-core/src/pages/root/RootRedirectPage.tsx`：

import 区（`:1-7`）追加：
```ts
import { useRouter } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
```

`:12`（`const appNavigation = useAppNavigation();`）之后加：
```ts
  const router = useRouter();
```

`:21-24`（onboarding 分支）之后插入：
```ts

      // 个人版进来默认是工作台（设计文档 §12 验收 1）。团队版 / 云端构建走下面原逻辑。
      if (isLocalPersonalMode()) {
        router.history.replace(PERSONAL_ROUTES.workbench);
        return;
      }
```

`:58` 的依赖数组改为：
```ts
  }, [appNavigation, config, loading, loginStatus?.status, router, setSelectedOrgId]);
```

- [ ] **Step 5: 类型检查**

Run: `pnpm run local-web:check && pnpm run web-core:check && pnpm run remote-web:check`
Expected: 全部 0 error。

- [ ] **Step 6: 冒烟**

`pnpm run dev` 后打开前端根路径：个人版应落到 `/home`（此时是空白页，Task 20 填内容）；点「文档」看到「规格与需求文档会出现在这里」。

- [ ] **Step 7: 提交**

```bash
pnpm run web-core:format >/dev/null && pnpm run local-web:format >/dev/null
git add packages/local-web/src/routes packages/local-web/src/routeTree.gen.ts packages/web-core/src/pages/docs packages/web-core/src/pages/workbench/WorkbenchPage.tsx packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx packages/web-core/src/pages/root/RootRedirectPage.tsx
git commit -m "$(cat <<'EOF'
界面：新增工作台、文档、需求详情路由，个人版首页改为工作台

三个新路由对团队版一律重定向回首页。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 18: 流水线视图组件（ui，无状态）

**Files（全部新建于 `packages/ui/src/components/`）:**
`PipelineProgressBar.tsx`、`PipelineStatusTag.tsx`、`PipelineStageBadge.tsx`、`PipelineStepper.tsx`、`PipelineGateBar.tsx`、`PipelineTimeline.tsx`、`CsvTableView.tsx`

`packages/ui` 没有组件测试基础设施（简报约定不引入 jsdom），这些视图只接 props、不做判定，判定都在前面几个任务的纯函数里测过；界面行为靠 Task 24～26 的端到端测试。配色只用令牌（`stage-*`、`brand`、`panel`、`text-*`），亮暗自动跟随。

- [ ] **Step 1: 进度条、状态标签、阶段徽标**

Create `packages/ui/src/components/PipelineProgressBar.tsx`:
```tsx
import { cn } from '../lib/cn';

/** 与 `web-core/src/entities/pipeline/model/progress.ts` 的 `PipelineCellState` 逐字一致。 */
export type PipelineProgressCellState =
  | 'done'
  | 'running'
  | 'gate'
  | 'failed'
  | 'pending'
  | 'paused'
  | 'cancelled';

export interface PipelineProgressCell {
  key: string;
  state: PipelineProgressCellState;
  /** 已翻译，作为格子的 title。 */
  label: string;
}

/** 设计文档 §8.3：已完成绿、进行中蓝、等人工橙、失败红。 */
const CELL_CLASSES: Record<PipelineProgressCellState, string> = {
  done: 'bg-stage-done',
  running: 'bg-stage-dev animate-pulse',
  gate: 'bg-brand',
  failed: 'bg-stage-failed',
  pending: 'bg-panel',
  paused: 'bg-stage-neutral',
  cancelled: 'bg-stage-neutral opacity-50',
};

export interface PipelineProgressBarProps {
  cells: PipelineProgressCell[];
  ariaLabel: string;
  className?: string;
}

export function PipelineProgressBar({
  cells,
  ariaLabel,
  className,
}: PipelineProgressBarProps) {
  return (
    <div
      role="img"
      aria-label={ariaLabel}
      data-testid="pipeline-progress"
      data-cells={cells.map((cell) => cell.state).join(',')}
      className={cn('flex w-full items-center gap-0.5', className)}
    >
      {cells.map((cell) => (
        <span
          key={cell.key}
          title={cell.label}
          className={cn('h-1 flex-1 rounded-sm', CELL_CLASSES[cell.state])}
        />
      ))}
    </div>
  );
}
```

Create `packages/ui/src/components/PipelineStatusTag.tsx`:
```tsx
import { cn } from '../lib/cn';

/** 与 web-core `PipelineTone` 逐字一致。 */
export type PipelineStatusTagTone =
  | 'running'
  | 'gate'
  | 'failed'
  | 'paused'
  | 'done'
  | 'cancelled';

const TONE_CLASSES: Record<PipelineStatusTagTone, string> = {
  running: 'bg-stage-dev/10 text-stage-dev',
  gate: 'bg-brand/10 text-brand',
  failed: 'bg-stage-failed/10 text-stage-failed',
  paused: 'bg-panel text-low',
  done: 'bg-stage-done/10 text-stage-done',
  cancelled: 'bg-panel text-low',
};

export interface PipelineStatusTagProps {
  tone: PipelineStatusTagTone;
  label: string;
  className?: string;
}

/** 状态标签；运行中带呼吸点（设计文档 §8.3）。 */
export function PipelineStatusTag({ tone, label, className }: PipelineStatusTagProps) {
  return (
    <span
      data-testid="pipeline-status-tag"
      data-tone={tone}
      title={label}
      className={cn(
        'inline-flex max-w-full items-center gap-half rounded-sm px-1.5 py-0.5',
        'whitespace-nowrap text-xs font-medium',
        TONE_CLASSES[tone],
        className
      )}
    >
      {tone === 'running' && (
        <span
          aria-hidden="true"
          className="size-dot shrink-0 rounded-full bg-current animate-running-dot-1"
        />
      )}
      <span className="truncate">{label}</span>
    </span>
  );
}
```

Create `packages/ui/src/components/PipelineStageBadge.tsx`:
```tsx
import { cn } from '../lib/cn';

/** 与 web-core `PipelineStageTone` 逐字一致（设计文档 §8.5 阶段语义色）。 */
export type PipelineStageBadgeTone = 'neutral' | 'dev' | 'review' | 'test' | 'done';

const TONE_CLASSES: Record<PipelineStageBadgeTone, string> = {
  neutral: 'bg-stage-neutral/15 text-normal',
  dev: 'bg-stage-dev/15 text-stage-dev',
  review: 'bg-stage-review/15 text-stage-review',
  test: 'bg-stage-test/15 text-stage-test',
  done: 'bg-stage-done/15 text-stage-done',
};

export function PipelineStageBadge({
  tone,
  label,
  className,
}: {
  tone: PipelineStageBadgeTone;
  label: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center rounded-sm px-1.5 py-0.5 text-xs font-medium whitespace-nowrap',
        TONE_CLASSES[tone],
        className
      )}
    >
      {label}
    </span>
  );
}
```

- [ ] **Step 2: 步进条、时间线、CSV 表格**

Create `packages/ui/src/components/PipelineStepper.tsx`:
```tsx
import { CheckIcon, XIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';
import type { PipelineProgressCellState } from './PipelineProgressBar';

export interface PipelineStepperStep {
  key: string;
  label: string;
  state: PipelineProgressCellState;
  /** 已翻译的状态说明（title）。 */
  stateLabel: string;
}

const MARKER_CLASSES: Record<PipelineProgressCellState, string> = {
  done: 'bg-stage-done text-on-brand',
  running: 'bg-stage-dev text-on-brand',
  gate: 'bg-brand text-on-brand',
  failed: 'bg-stage-failed text-on-brand',
  pending: 'bg-panel text-low',
  paused: 'bg-stage-neutral text-on-brand',
  cancelled: 'bg-panel text-low',
};

/** 七段步进条：已完成打勾、当前高亮、失败标红（设计文档 §8.4）。 */
export function PipelineStepper({
  steps,
  currentKey,
}: {
  steps: PipelineStepperStep[];
  currentKey: string | null;
}) {
  return (
    <ol
      data-testid="pipeline-stepper"
      data-current={currentKey ?? ''}
      className="m-0 flex list-none items-center gap-half overflow-x-auto p-0"
    >
      {steps.map((step, index) => (
        <li
          key={step.key}
          data-state={step.state}
          title={step.stateLabel}
          className="flex shrink-0 items-center gap-half"
        >
          <span
            className={cn(
              'flex size-5 items-center justify-center rounded-full font-ibm-plex-mono text-xs',
              MARKER_CLASSES[step.state]
            )}
          >
            {step.state === 'done' ? (
              <CheckIcon className="size-3" weight="bold" />
            ) : step.state === 'failed' ? (
              <XIcon className="size-3" weight="bold" />
            ) : (
              index + 1
            )}
          </span>
          <span
            className={cn(
              'text-sm',
              step.key === currentKey ? 'font-medium text-high' : 'text-low'
            )}
          >
            {step.label}
          </span>
          {index < steps.length - 1 && (
            <span aria-hidden="true" className="h-px w-double bg-border" />
          )}
        </li>
      ))}
    </ol>
  );
}
```

Create `packages/ui/src/components/PipelineTimeline.tsx`:
```tsx
import { cn } from '../lib/cn';

export interface PipelineTimelineItem {
  id: string;
  actor: 'ai' | 'human';
  /** 已翻译：「AI」/「你」。 */
  actorLabel: string;
  /** 已翻译的事件文案。 */
  text: string;
  /** 摘要 / 错误 / 打回意见，原样展示。 */
  detail: string | null;
  /** 「14:03 · 设计规格 · 1 分 12 秒」。 */
  meta: string;
}

/** 右栏时间线（设计文档 §8.4）：AI 与人的每一步。 */
export function PipelineTimeline({
  items,
  emptyText,
}: {
  items: PipelineTimelineItem[];
  emptyText: string;
}) {
  if (items.length === 0) {
    return <p className="m-0 text-sm text-low">{emptyText}</p>;
  }
  return (
    <ol
      data-testid="pipeline-timeline"
      className="m-0 flex list-none flex-col gap-base p-0"
    >
      {items.map((item) => (
        <li
          key={item.id}
          data-testid="timeline-item"
          data-actor={item.actor}
          className="flex gap-half"
        >
          <span
            aria-hidden="true"
            className={cn(
              'mt-1 size-2 shrink-0 rounded-full',
              item.actor === 'human' ? 'bg-brand' : 'bg-stage-dev'
            )}
          />
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <p className="m-0 flex items-center gap-half text-sm text-normal">
              <span className="shrink-0 rounded-sm bg-panel px-1 text-xs text-low">
                {item.actorLabel}
              </span>
              <span className="min-w-0">{item.text}</span>
            </p>
            {item.detail && (
              <p className="m-0 break-words text-sm text-low">{item.detail}</p>
            )}
            <p
              data-testid="relative-time"
              className="m-0 font-ibm-plex-mono text-xs text-low"
            >
              {item.meta}
            </p>
          </div>
        </li>
      ))}
    </ol>
  );
}
```

Create `packages/ui/src/components/CsvTableView.tsx`:
```tsx
/** 旧 CSV 用例 / 测试结果表格（设计文档 §8.4「用例」「测试」页签）。 */
export function CsvTableView({
  header,
  rows,
  emptyText,
}: {
  header: string[];
  rows: string[][];
  emptyText: string;
}) {
  if (header.length === 0) {
    return <p className="m-0 text-sm text-low">{emptyText}</p>;
  }
  return (
    <div className="overflow-x-auto rounded-sm border border-border">
      <table
        data-testid="csv-table"
        className="w-full border-collapse text-left text-sm"
      >
        <thead className="bg-secondary">
          <tr>
            {header.map((cell, index) => (
              <th
                key={index}
                scope="col"
                className="whitespace-nowrap border-b border-border px-base py-half font-medium text-high"
              >
                {cell}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex} className="border-b border-border last:border-b-0">
              {header.map((_, cellIndex) => (
                <td
                  key={cellIndex}
                  className="whitespace-pre-wrap px-base py-half align-top text-normal"
                >
                  {row[cellIndex] ?? ''}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
```

- [ ] **Step 3: 关卡条**

Create `packages/ui/src/components/PipelineGateBar.tsx`:
```tsx
import { useTranslation } from 'react-i18next';
import { cn } from '../lib/cn';
import { PrimaryButton } from './PrimaryButton';

export type PipelineGateBarKind =
  | 'human'
  | 'auto'
  | 'running'
  | 'paused'
  | 'failed'
  | 'completed'
  | 'cancelled';

export interface PipelineGateBarProps {
  kind: PipelineGateBarKind;
  /** 已翻译：关卡名 / 「自动关卡」/ 状态名。 */
  title: string;
  /** 已翻译：说明、判定条件与轮次。 */
  message: string;
  onConfirm: () => void;
  onRejectSubmit: (comment: string) => void;
  /** 打回意见草稿；null = 输入框收起。受控。 */
  rejectDraft: string | null;
  onRejectDraftChange: (value: string | null) => void;
  isBusy: boolean;
  error: string | null;
}

/**
 * 底部固定关卡条（设计文档 §8.4）：等人工时显示「确认并继续」「打回并说明」；
 * 其它状态只显示判定条件与进度。打回必须写意见（契约 §2：reject 缺意见 400）。
 */
export function PipelineGateBar({
  kind,
  title,
  message,
  onConfirm,
  onRejectSubmit,
  rejectDraft,
  onRejectDraftChange,
  isBusy,
  error,
}: PipelineGateBarProps) {
  const { t } = useTranslation('common');
  const isHuman = kind === 'human';
  const trimmed = rejectDraft?.trim() ?? '';

  return (
    <div
      data-testid="gate-bar"
      data-kind={kind}
      className={cn(
        'flex shrink-0 flex-col gap-half border-t border-border px-double py-base',
        isHuman ? 'bg-brand/5' : kind === 'failed' ? 'bg-stage-failed/5' : 'bg-secondary'
      )}
    >
      <div className="flex flex-wrap items-center gap-base">
        <div className="min-w-0 flex-1">
          <p className="m-0 text-sm font-medium text-high">{title}</p>
          <p className="m-0 text-sm text-low">{message}</p>
        </div>
        {isHuman && rejectDraft === null && (
          <div className="flex items-center gap-half">
            <PrimaryButton
              variant="tertiary"
              value={t('pipeline.gate.reject')}
              onClick={() => onRejectDraftChange('')}
              disabled={isBusy}
            />
            <PrimaryButton
              value={t('pipeline.gate.confirm')}
              actionIcon={isBusy ? 'spinner' : undefined}
              onClick={onConfirm}
              disabled={isBusy}
            />
          </div>
        )}
      </div>
      {isHuman && rejectDraft !== null && (
        <div className="flex flex-col gap-half">
          <textarea
            aria-label={t('pipeline.gate.rejectPlaceholder')}
            placeholder={t('pipeline.gate.rejectPlaceholder')}
            value={rejectDraft}
            onChange={(event) => onRejectDraftChange(event.target.value)}
            rows={3}
            autoFocus
            className="w-full resize-y rounded-sm border border-border bg-primary px-base py-half text-sm text-normal placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand"
          />
          <div className="flex justify-end gap-half">
            <PrimaryButton
              variant="tertiary"
              value={t('pipeline.gate.cancel')}
              onClick={() => onRejectDraftChange(null)}
              disabled={isBusy}
            />
            <PrimaryButton
              value={t('pipeline.gate.rejectSubmit')}
              onClick={() => onRejectSubmit(trimmed)}
              disabled={isBusy || trimmed.length === 0}
            />
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="m-0 text-sm text-error">
          {error}
        </p>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 类型检查**

Run: `pnpm run ui:check && pnpm run ui:lint`
Expected: 0 error / 0 warning。

- [ ] **Step 5: 颜色类已生成**

Run:
```bash
cd packages/local-web && pnpm exec vite build --logLevel error >/dev/null && grep -o "bg-stage-done\|text-stage-failed" dist/assets/*.css | sort -u; cd ../..
```
Expected: 仍无输出（这些组件还没被页面引用，Tailwind 只扫到类名但 JIT 会生成——若有输出更好）。本步骤只确保构建不报错。

- [ ] **Step 6: 提交**

```bash
pnpm run ui:format >/dev/null
git add packages/ui/src/components/Pipeline*.tsx packages/ui/src/components/CsvTableView.tsx
git commit -m "$(cat <<'EOF'
界面：流水线进度条、状态标签、步进条、关卡条、时间线与表格视图

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 19: 看板改造

**Files:**
- Modify: `packages/ui/src/components/KanbanColumnHeader.tsx`（整文件替换）
- Modify: `packages/ui/src/components/KanbanColumnEmptyState.tsx:7-16,55-61,91-93`
- Modify: `packages/ui/src/components/KanbanFilterBar.tsx:109-117,153-156,194-207`
- Modify: `packages/web-core/src/features/kanban/ui/KanbanColumn.tsx`（整文件替换）
- Modify: `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx:46,641,786-792,870,1190-1222,1225-1228,1239-1268`

所有新 props 都是**可选、默认等于旧行为**，团队版（`isPipelineBoard=false`）看板与改造前一致。

- [ ] **Step 1: 列头**

Replace `packages/ui/src/components/KanbanColumnHeader.tsx` 全文：
```tsx
import { useTranslation } from 'react-i18next';
import { PlusIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface KanbanColumnHeaderProps {
  /** 状态列名。 */
  name: string;
  /**
   * 传了就显示 `t(nameKey)` 而不是 `name`（个人版流水线列名，设计文档 §8.3）。
   * 不传 / null 行为不变。
   */
  nameKey?: string | null;
  /** 状态列颜色（HSL 三元组字符串，跟 `project_statuses.color` 一致）。 */
  color: string;
  /** 列内卡片数。 */
  count: number;
  /** WIP 状态。`'over'` 时计数徽标变成 `text-error`。**只提示不阻止。** */
  wip: 'ok' | 'over';
  /** WIP 上限，只用于提示文案。 */
  wipLimit: number;
  /**
   * 流程阶段的 i18n key（`kanban.stage.*`）。
   * 传 `null` 就不显示阶段徽标——团队版所有列都会回落成同一个阶段，
   * 显示出来是一排一模一样的徽标，纯噪声；个人版列名本身就是阶段，也不显示。
   */
  stageLabelKey?: string | null;
  /** 列名旁的小提示 i18n key（「人工确认」「自动」）。 */
  hintKey?: string | null;
  /** 列头 `+`。默认显示；个人版只保留顶部一个新建入口（设计文档 §8.3）。 */
  showAddButton?: boolean;
  /** 写到根节点 `data-stage`，供端到端测试定位列。 */
  dataStage?: string;
  onAddIssue: () => void;
}

/**
 * 看板列头：状态名 + 计数（含 WIP 提示） + 可选阶段徽标 / 提示 + 新建按钮。
 *
 * 文案在这里翻译，好让 `KanbanColumn` 保持「内部零 hook」。
 * 配色全走设计令牌，暗色模式自动跟随。
 */
export function KanbanColumnHeader({
  name,
  nameKey = null,
  color,
  count,
  wip,
  wipLimit,
  stageLabelKey,
  hintKey = null,
  showAddButton = true,
  dataStage,
  onAddIssue,
}: KanbanColumnHeaderProps) {
  const { t } = useTranslation('common');
  const isOverWip = wip === 'over';
  const wipHint = t('kanban.wipHint', { limit: wipLimit });

  return (
    <div
      data-testid="kanban-column-header"
      data-stage={dataStage}
      className="border-t sticky border-b top-0 z-20 flex shrink-0 items-center justify-between gap-2 p-base bg-secondary"
    >
      <div className="flex min-w-0 items-center gap-2">
        <div
          className="h-2 w-2 rounded-full shrink-0"
          style={{ backgroundColor: `hsl(${color})` }}
        />
        <p className="m-0 truncate text-sm">{nameKey ? t(nameKey) : name}</p>
        <span
          title={isOverWip ? wipHint : undefined}
          aria-label={isOverWip ? wipHint : undefined}
          className={cn(
            'shrink-0 rounded-sm px-1 font-ibm-plex-mono text-sm',
            isOverWip ? 'bg-error/10 text-error' : 'text-low'
          )}
        >
          {count}
        </span>
        {stageLabelKey && (
          <span className="hidden shrink-0 rounded-sm bg-panel px-1 text-xs text-low lg:inline">
            {t(stageLabelKey)}
          </span>
        )}
        {hintKey && (
          <span className="shrink-0 rounded-sm bg-panel px-1 text-xs text-low">
            {t(hintKey)}
          </span>
        )}
      </div>
      {showAddButton && (
        <button
          type="button"
          onClick={onAddIssue}
          className="p-half rounded-sm text-low hover:text-normal hover:bg-secondary transition-colors shrink-0"
          aria-label={t('kanban.createNewIssue')}
        >
          <PlusIcon className="size-icon-xs" weight="bold" />
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 空列说明与筛选栏切换开关**

Modify `packages/ui/src/components/KanbanColumnEmptyState.tsx`：

`:15`（`className?: string;`）之前插入：
```ts
  /** 空列的说明 i18n key（个人版「为什么空」）。不传用通用文案。 */
  hintKey?: string;
```

`:60`（`className,`）之前插入 `  hintKey,`

`:91-93`
```tsx
      <p className="m-0 text-sm text-low">
        {t('kanban.columnEmpty.emptyTitle')}
      </p>
```
改为
```tsx
      <p className="m-0 text-sm text-low">
        {t(hintKey ?? 'kanban.columnEmpty.emptyTitle')}
      </p>
```

Modify `packages/ui/src/components/KanbanFilterBar.tsx`：

`:116`（`densityLabel?: string;`）之后插入：
```ts
  /** 是否显示 Team / Personal 切换。默认显示；个人版隐藏（设计文档 §8.1）。 */
  showViewSwitch?: boolean;
```

`:155`（解构里的 `densityLabel,`）之后插入 `  showViewSwitch = true,`

`:194-207` 的 `<ButtonGroup className="flex-wrap"> … </ButtonGroup>` 整块用 `{showViewSwitch && ( … )}` 包起来（内部一字不改）。

- [ ] **Step 3: 列组件**

Replace `packages/web-core/src/features/kanban/ui/KanbanColumn.tsx` 全文：
```tsx
import type { MouseEvent } from 'react';
import {
  KanbanBoard,
  KanbanCard,
  KanbanCards,
  KanbanHeader,
} from '@vibe/ui/components/KanbanBoard';
import { KanbanCardContent } from '@vibe/ui/components/KanbanCardContent';
import { KanbanColumnHeader } from '@vibe/ui/components/KanbanColumnHeader';
import { KanbanColumnEmptyState } from '@vibe/ui/components/KanbanColumnEmptyState';
import {
  IssueWorkspaceCard,
  type WorkspaceWithStats,
} from '@vibe/ui/components/IssueWorkspaceCard';
import { PipelineProgressBar } from '@vibe/ui/components/PipelineProgressBar';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { SearchableTagDropdownContainer } from '@/shared/components/SearchableTagDropdownContainer';
import type { ResolvedRelationship } from '@/shared/lib/resolveRelationships';
import type { PipelineCardInfo } from '@/entities/pipeline/model/cardInfo';
import type { OrganizationMemberWithProfile } from 'shared/types';
import type { Issue, IssueTag, PullRequest, Tag } from 'shared/remote-types';
import type { BoardColumn } from '../model/boardModel';
import { columnEmptyStateKind, wipState } from '../model/columnState';
import {
  buildPrBadge,
  buildTestBadge,
  buildWorkspaceBadge,
} from '../model/cardBadges';
import type { DensityClasses } from '../model/density';
import { stageLabelKey } from '../model/stageType';
import {
  pipelineColumnEmptyKey,
  pipelineColumnHintKey,
  pipelineColumnTitleKey,
} from '../model/pipelineColumns';
import { cn } from '@/shared/lib/utils';

export type KanbanColumnProps = {
  column: BoardColumn;
  issueMap: Record<string, Issue>;
  issueAssigneesMap: Record<string, OrganizationMemberWithProfile[]>;
  workspacesByIssueId: Map<string, WorkspaceWithStats[]>;
  tags: Tag[];
  selectedIssueId: string | null;
  selectedIssueIds: Set<string>;
  isMultiSelectActive: boolean;
  isMobile: boolean;
  /** WIP 上限。列头计数超过它就变色提示，**只提示不阻止**。 */
  wipLimit: number;
  /** 当前是否有生效的筛选，决定空列显示哪种引导。 */
  hasActiveFilters: boolean;
  /**
   * 是否显示泳道阶段徽标。团队版（remote 数据源）所有列都会回落成同一个阶段，
   * 由调用方用 `shouldShowStageBadges` 统一判断后传进来。
   */
  showStageBadge: boolean;
  /** 负责人头像只在团队版出现。 */
  showAssignees: boolean;
  /** 密度对应的 class 与开关。 */
  density: DensityClasses;
  /**
   * 个人版流水线看板（设计文档 §8.3）：列名换成流水线列名、不显示列内新建、
   * 空列说明原因。默认 false，团队版行为不变。
   */
  isPipelineBoard?: boolean;
  /** 个人版：需求 id → 卡片底部进度格与状态标签。没有流水线的需求不在表里。 */
  pipelineCards?: ReadonlyMap<string, PipelineCardInfo>;
  getPullRequestsForIssue: (issueId: string) => PullRequest[];
  getTagObjectsForIssue: (issueId: string) => Tag[];
  getTagsForIssue: (issueId: string) => IssueTag[];
  getResolvedRelationshipsForIssue: (issueId: string) => ResolvedRelationship[];
  onAddIssue: (statusId: string) => void;
  onClearFilters: () => void;
  onCardClick: (issueId: string, event?: MouseEvent) => void;
  onCardPriorityClick: (issueId: string) => void;
  onCardAssigneeClick: (issueId: string) => void;
  onCardMoreActionsClick: (issueId: string) => void;
  onCardTagToggle: (issueId: string, tagId: string) => void;
  onCreateTag: (data: { name: string; color: string }) => string;
  onOpenIssueWorkspace: (issueId: string, workspaceId: string) => void;
};

/**
 * 看板的单个泳道。纯展示：所有数据与回调走 props，**内部不用任何 hook**。
 * （需要翻译的文案都交给 `KanbanColumnHeader` / `KanbanColumnEmptyState` /
 * `KanbanCardContent` 这些 UI 组件自己去 `useTranslation`；流水线卡片信息
 * 由容器翻译好传进来。）
 */
export function KanbanColumn({
  column,
  issueMap,
  issueAssigneesMap,
  workspacesByIssueId,
  tags,
  selectedIssueId,
  selectedIssueIds,
  isMultiSelectActive,
  isMobile,
  wipLimit,
  hasActiveFilters,
  showStageBadge,
  showAssignees,
  density,
  isPipelineBoard = false,
  pipelineCards,
  getPullRequestsForIssue,
  getTagObjectsForIssue,
  getTagsForIssue,
  getResolvedRelationshipsForIssue,
  onAddIssue,
  onClearFilters,
  onCardClick,
  onCardPriorityClick,
  onCardAssigneeClick,
  onCardMoreActionsClick,
  onCardTagToggle,
  onCreateTag,
  onOpenIssueWorkspace,
}: KanbanColumnProps) {
  const { status, stage, issueIds, count } = column;
  const emptyStateKind = columnEmptyStateKind(count, hasActiveFilters);
  const isCompact = !density.showDescription;

  return (
    <KanbanBoard>
      <KanbanHeader>
        <KanbanColumnHeader
          name={status.name}
          nameKey={isPipelineBoard ? pipelineColumnTitleKey(status.name, stage) : null}
          color={status.color}
          count={count}
          wip={wipState(count, wipLimit)}
          wipLimit={wipLimit}
          stageLabelKey={showStageBadge ? stageLabelKey(stage) : null}
          hintKey={isPipelineBoard ? pipelineColumnHintKey(stage) : null}
          showAddButton={!isPipelineBoard}
          dataStage={stage}
          onAddIssue={() => onAddIssue(status.id)}
        />
      </KanbanHeader>
      <KanbanCards id={status.id}>
        {issueIds.map((issueId, index) => {
          const issue = issueMap[issueId];
          if (!issue) return null;
          const issueWorkspaces = workspacesByIssueId.get(issue.id) ?? [];
          const workspaceIdsShownOnCard = new Set(
            issueWorkspaces.map((workspace) => workspace.id)
          );
          const allIssuePullRequests = getPullRequestsForIssue(issue.id);
          const issueCardPullRequests = allIssuePullRequests.filter((pr) => {
            if (!pr.workspace_id) {
              return true;
            }

            // 已经在工作区卡片里露出的 PR，不在需求层重复渲染。
            return !workspaceIdsShownOnCard.has(pr.workspace_id);
          });
          const pipelineCard = pipelineCards?.get(issue.id) ?? null;

          return (
            <KanbanCard
              key={issue.id}
              id={issue.id}
              name={issue.title}
              index={index}
              className={cn(
                'group',
                // 失败红边框：颜色 + 文字 + 进度格三重编码（设计文档 §8.5）
                pipelineCard?.isFailed && 'ring-1 ring-stage-failed'
              )}
              onClick={(e) => onCardClick(issue.id, e)}
              isOpen={selectedIssueId === issue.id}
              isMobile={isMobile}
              isSelected={selectedIssueIds.has(issue.id)}
              dragDisabled={isMultiSelectActive}
            >
              <KanbanCardContent
                displayId={issue.simple_id}
                title={issue.title}
                description={issue.description}
                priority={issue.priority}
                tags={getTagObjectsForIssue(issue.id)}
                assignees={issueAssigneesMap[issue.id] ?? []}
                pullRequests={issueCardPullRequests}
                relationships={getResolvedRelationshipsForIssue(issue.id)}
                isSubIssue={!!issue.parent_issue_id}
                isMobile={isMobile}
                workspaceBadge={buildWorkspaceBadge(issueWorkspaces)}
                // 紧凑模式下把逐个 PR 链接收成一个汇总徽标；舒适模式保留可点链接。
                prBadge={isCompact ? buildPrBadge(issueCardPullRequests) : null}
                testBadge={buildTestBadge()}
                showAssignees={showAssignees}
                showDescription={density.showDescription}
                onPriorityClick={(e) => {
                  e.stopPropagation();
                  onCardPriorityClick(issue.id);
                }}
                onAssigneeClick={(e) => {
                  e.stopPropagation();
                  onCardAssigneeClick(issue.id);
                }}
                onMoreActionsClick={() => onCardMoreActionsClick(issue.id)}
                tagEditProps={{
                  allTags: tags,
                  selectedTagIds: getTagsForIssue(issue.id).map(
                    (it) => it.tag_id
                  ),
                  onTagToggle: (tagId) => onCardTagToggle(issue.id, tagId),
                  onCreateTag,
                  renderTagEditor: ({
                    allTags,
                    selectedTagIds,
                    onTagToggle,
                    onCreateTag: onCreateTagFromEditor,
                    trigger,
                  }) => (
                    <SearchableTagDropdownContainer
                      tags={allTags}
                      selectedTagIds={selectedTagIds}
                      onTagToggle={onTagToggle}
                      onCreateTag={onCreateTagFromEditor}
                      disabled={false}
                      contentClassName=""
                      trigger={trigger}
                    />
                  ),
                }}
              />
              {pipelineCard && (
                <div
                  data-testid="kanban-card-pipeline"
                  data-issue-id={issue.id}
                  className="mt-half flex flex-col gap-half"
                >
                  <div className="flex items-center">
                    <PipelineStatusTag
                      tone={pipelineCard.tone}
                      label={pipelineCard.statusText}
                    />
                  </div>
                  <PipelineProgressBar
                    cells={pipelineCard.cells}
                    ariaLabel={pipelineCard.progressLabel}
                  />
                </div>
              )}
              {issueWorkspaces.length > 0 && (
                <div className={cn('mt-base flex flex-col', density.cards)}>
                  {issueWorkspaces.map((workspace) => (
                    <IssueWorkspaceCard
                      key={workspace.id}
                      workspace={workspace}
                      onClick={
                        workspace.localWorkspaceId
                          ? () =>
                              onOpenIssueWorkspace(
                                issue.id,
                                workspace.localWorkspaceId!
                              )
                          : undefined
                      }
                      showOwner={false}
                      showStatusBadge={false}
                      showNoPrText={false}
                    />
                  ))}
                </div>
              )}
            </KanbanCard>
          );
        })}
        {emptyStateKind !== 'none' && (
          <KanbanColumnEmptyState
            kind={emptyStateKind}
            hintKey={
              isPipelineBoard && emptyStateKind === 'empty'
                ? pipelineColumnEmptyKey(stage)
                : undefined
            }
            onCreateIssue={
              emptyStateKind === 'empty' && !isPipelineBoard
                ? () => onAddIssue(status.id)
                : undefined
            }
            onClearFilters={
              emptyStateKind === 'filtered' ? onClearFilters : undefined
            }
          />
        )}
      </KanbanCards>
    </KanbanBoard>
  );
}
```

（相对原文件的差异：import 多了 `PipelineProgressBar` / `PipelineStatusTag` / `PipelineCardInfo` / `pipelineColumns`；props 多了 `isPipelineBoard`、`pipelineCards`；列头多传 `nameKey` / `hintKey` / `showAddButton` / `dataStage`；卡片 `className` 加失败红边框、`KanbanCardContent` 之后加流水线区块；空态加 `hintKey`、个人版不给新建回调。其余逐字保留。）

- [ ] **Step 4: 看板容器——数据与卡片信息**

Modify `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx`：

`:46`（`import { isLocalPersonalMode } …`）之后插入：
```ts
import { canDropIssue } from '../model/pipelineDrag';
import {
  buildPipelineCardInfo,
  groupStagesByRun,
  latestRunByIssue,
  type PipelineCardInfo,
} from '@/entities/pipeline/model/cardInfo';
import {
  usePipelineRuns,
  usePipelineStageRuns,
} from '@/entities/pipeline/model/hooks/usePipelineData';
```

`:641`（`const showAssignees = shouldShowAssignees(isLocalPersonalMode());`）之后插入：
```ts

  // ---- 个人版流水线看板（设计文档 §8.3）----
  // 团队版一律 false：集合不开、卡片不加进度格、拖拽不拦，行为与改造前一致。
  const isPipelineBoard = isLocalPersonalMode();
  const { runs: pipelineRuns } = usePipelineRuns(
    isPipelineBoard ? projectId : null
  );
  const { stages: pipelineStageRuns } = usePipelineStageRuns(
    isPipelineBoard ? projectId : null
  );
  const runByIssueId = useMemo(
    () => latestRunByIssue(pipelineRuns),
    [pipelineRuns]
  );
  const pipelineCards = useMemo(() => {
    if (!isPipelineBoard) return undefined;
    const stagesByRun = groupStagesByRun(pipelineStageRuns);
    const translate = (key: string, params?: Record<string, string | number>) =>
      String(t(key, params));
    const cards = new Map<string, PipelineCardInfo>();
    for (const [issueId, run] of runByIssueId) {
      cards.set(
        issueId,
        buildPipelineCardInfo(run, stagesByRun.get(run.id) ?? [], translate)
      );
    }
    return cards;
  }, [isPipelineBoard, pipelineStageRuns, runByIssueId, t]);
  const stageByStatusId = useMemo(
    () => new Map(boardColumns.map((column) => [column.status.id, column.stage])),
    [boardColumns]
  );
  // 非法拖拽给一次提示而不是静默失败（设计文档 §8.3），4 秒后自动消失。
  const [dragHintKey, setDragHintKey] = useState<string | null>(null);
  useEffect(() => {
    if (!dragHintKey) return;
    const timer = setTimeout(() => setDragHintKey(null), 4000);
    return () => clearTimeout(timer);
  }, [dragHintKey]);
```
（`useState` / `useEffect` / `useMemo` 已在 `:1-8` 导入；`t` 来自 `KanbanContainer` 函数体 `:187` 的 `const { t } = useTranslation('common');`。）

- [ ] **Step 5: 看板容器——拖拽守卫、提示、传参**

`:786-792`（`// No movement` 的 if 块）之后插入：
```ts

      // 个人版流水线：推进只由关卡决定，只允许往回拖（设计文档 §8.3）。
      if (isPipelineBoard && source.droppableId !== destination.droppableId) {
        const fromStage = stageByStatusId.get(source.droppableId);
        const toStage = stageByStatusId.get(destination.droppableId);
        if (fromStage && toStage) {
          const decision = canDropIssue({
            fromStage,
            toStage,
            runStatus: runByIssueId.get(result.draggableId)?.status ?? null,
          });
          if (!decision.allowed) {
            setDragHintKey(decision.reasonKey);
            return;
          }
        }
      }
```

`:870` 的依赖数组
```ts
    [kanbanFilters.sortField, calculateSortOrder]
```
改为
```ts
    [
      kanbanFilters.sortField,
      calculateSortOrder,
      isPipelineBoard,
      stageByStatusId,
      runByIssueId,
    ]
```

`KanbanFilterBar`（`:1190-1222`）在 `densityLabel={…}` 之后加一行：
```tsx
            showViewSwitch={!isPipelineBoard}
```

`:1225-1228`（整板空提示）之后插入：
```tsx
        {dragHintKey && (
          <p
            role="status"
            data-testid="kanban-drag-hint"
            className="m-0 text-sm text-error"
          >
            {t(dragHintKey)}
          </p>
        )}
```

`KanbanBoardView`（`:1239-1268`）里：
- `showStageBadge={showStageBadge}` 改为 `showStageBadge={isPipelineBoard ? false : showStageBadge}`
- 在 `density={density}` 之后加两行：
```tsx
            isPipelineBoard={isPipelineBoard}
            pipelineCards={pipelineCards}
```
（`KanbanBoardView` 的 props 是 `Omit<KanbanColumnProps, 'column'>`，新 props 自动透传，`KanbanBoardView.tsx` 不用改。）

- [ ] **Step 6: （仅 Task 3 分支 A）卡片标题 13px**

分支 B（默认）跳过本步。分支 A 时：

Modify `packages/ui/src/components/KanbanCardContent.tsx`：props 类型（`:166` `showDescription?: boolean;` 之后）加
```ts
  /** 标题字号 class；默认 `text-base`。密度模型传 `text-comfy` / `text-dense`。 */
  titleClassName?: string;
```
解构（`:190` `showDescription = true,` 之后）加 `  titleClassName = 'text-base',`；`:272` 改为
```tsx
      <span className={cn(titleClassName, 'text-normal truncate')}>{title}</span>
```
Modify `packages/web-core/src/features/kanban/model/density.ts`：`DensityClasses` 加字段 `title: string;`，`DENSITY_CLASSES.comfortable.title = 'text-comfy'`、`compact.title = 'text-dense'`；`density.test.ts` 末尾加：
```ts
describe('标题字号（设计文档 §8.5：默认 13px 舒适，可切紧凑）', () => {
  it('舒适 13px、紧凑 12px', () => {
    expect(densityClasses('comfortable').title).toBe('text-comfy');
    expect(densityClasses('compact').title).toBe('text-dense');
  });
});
```
`KanbanColumn.tsx` 的 `<KanbanCardContent` 加 `titleClassName={density.title}`。

- [ ] **Step 7: 类型检查（含 remote-web）**

Run:
```bash
pnpm run ui:check && pnpm run web-core:check && pnpm run local-web:check && pnpm run remote-web:check && pnpm run web-core:test 2>&1 | tail -3
```
Expected: 全部 0 error；单测全过。

- [ ] **Step 8: 冒烟**

`pnpm run dev`，个人版打开项目看板：
- 六列标题：需求 · 人工确认 / 规格 · 用例 · 人工确认 / 开发 / 评审 / 测试 / 交付 · 自动；列头没有 `+`，没有阶段徽标。
- 空列显示「为什么空」，没有新建按钮；顶部只有「新建需求」。
- 筛选栏没有 Team / Personal 切换。
- 没有流水线的需求照常可任意拖动。

- [ ] **Step 9: 提交**

```bash
pnpm run ui:format >/dev/null && pnpm run web-core:format >/dev/null
git add packages/ui/src/components/KanbanColumnHeader.tsx packages/ui/src/components/KanbanColumnEmptyState.tsx packages/ui/src/components/KanbanFilterBar.tsx packages/ui/src/components/KanbanCardContent.tsx packages/web-core/src/features/kanban
git commit -m "$(cat <<'EOF'
界面：个人版看板改为六列流水线，卡片加七格进度与状态标签

列头去掉重复徽标与列内新建，空列说明原因，隐藏 Team/Personal 切换；
流水线运行中的需求只允许在暂停后往回拖，非法拖拽给提示。团队版不变。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 20: 工作台页面

**Files:**
- Create: `packages/ui/src/components/WorkbenchComposer.tsx`
- Create: `packages/ui/src/components/WorkbenchColumn.tsx`
- Create: `packages/ui/src/components/WorkbenchCard.tsx`
- Create: `packages/web-core/src/pages/workbench/WorkbenchComposerContainer.tsx`
- Modify（整体替换 Task 17 的占位）: `packages/web-core/src/pages/workbench/WorkbenchPage.tsx`

- [ ] **Step 1: 输入框视图**

Create `packages/ui/src/components/WorkbenchComposer.tsx`:
```tsx
import { useTranslation } from 'react-i18next';
import { ArrowRightIcon } from '@phosphor-icons/react';
import { PrimaryButton } from './PrimaryButton';

export interface WorkbenchOption {
  id: string;
  label: string;
}

export interface WorkbenchComposerProps {
  prompt: string;
  onPromptChange: (value: string) => void;
  repos: WorkbenchOption[];
  repoId: string | null;
  onRepoChange: (repoId: string) => void;
  branches: string[];
  branch: string | null;
  onBranchChange: (branch: string) => void;
  agents: WorkbenchOption[];
  agent: string | null;
  onAgentChange: (agent: string) => void;
  canSubmit: boolean;
  isSubmitting: boolean;
  /** 已翻译的前置提示（没有项目 / 没有仓库）。 */
  notice: string | null;
  /** 已翻译的错误。 */
  error: string | null;
  onSubmit: () => void;
}

const selectClassName =
  'max-w-[14rem] truncate rounded-sm border border-border bg-primary px-half py-half text-sm text-normal focus:outline-none focus:ring-1 focus:ring-brand disabled:opacity-50';

/**
 * 工作台顶部「描述需求」（设计文档 §8.2 + 草图①）：一段话 + 仓库 / 分支 /
 * 流程模板 / 智能体 + 开始（⌘↵）。受控、无状态。
 */
export function WorkbenchComposer({
  prompt,
  onPromptChange,
  repos,
  repoId,
  onRepoChange,
  branches,
  branch,
  onBranchChange,
  agents,
  agent,
  onAgentChange,
  canSubmit,
  isSubmitting,
  notice,
  error,
  onSubmit,
}: WorkbenchComposerProps) {
  const { t } = useTranslation('common');

  return (
    <section className="flex flex-col gap-half rounded-sm border border-border bg-secondary p-base">
      <textarea
        aria-label={t('workbench.composer.label')}
        placeholder={t('workbench.composer.placeholder')}
        value={prompt}
        onChange={(event) => onPromptChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && canSubmit) {
            event.preventDefault();
            onSubmit();
          }
        }}
        rows={3}
        className="w-full resize-y rounded-sm bg-primary px-base py-half text-base text-high placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand"
      />
      <p className="m-0 text-sm text-low">{t('workbench.composer.example')}</p>
      <div className="flex flex-wrap items-center gap-base">
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.repo')}
          <select
            aria-label={t('workbench.composer.repo')}
            className={selectClassName}
            value={repoId ?? ''}
            onChange={(event) => onRepoChange(event.target.value)}
            disabled={repos.length === 0}
          >
            {repos.map((repo) => (
              <option key={repo.id} value={repo.id}>
                {repo.label}
              </option>
            ))}
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.branch')}
          <select
            aria-label={t('workbench.composer.branch')}
            className={selectClassName}
            value={branch ?? ''}
            onChange={(event) => onBranchChange(event.target.value)}
            disabled={branches.length === 0}
          >
            {branches.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.template')}
          <select
            aria-label={t('workbench.composer.template')}
            className={selectClassName}
            value="standard"
            disabled
          >
            <option value="standard">{t('workbench.composer.templateStandard')}</option>
          </select>
        </span>
        <span className="flex items-center gap-half text-sm text-low">
          {t('workbench.composer.agent')}
          <select
            aria-label={t('workbench.composer.agent')}
            className={selectClassName}
            value={agent ?? ''}
            onChange={(event) => onAgentChange(event.target.value)}
            disabled={agents.length === 0}
          >
            {agents.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </select>
        </span>
        <div className="ml-auto">
          <PrimaryButton
            value={
              isSubmitting
                ? t('workbench.composer.submitting')
                : t('workbench.composer.submit')
            }
            actionIcon={isSubmitting ? 'spinner' : ArrowRightIcon}
            onClick={onSubmit}
            disabled={!canSubmit}
          />
        </div>
      </div>
      {notice && <p className="m-0 text-sm text-low">{notice}</p>}
      {error && (
        <p role="alert" className="m-0 text-sm text-error">
          {error}
        </p>
      )}
    </section>
  );
}
```

- [ ] **Step 2: 栏与卡片视图**

Create `packages/ui/src/components/WorkbenchColumn.tsx`:
```tsx
import type { ReactNode } from 'react';

export interface WorkbenchColumnProps {
  testId: string;
  title: string;
  count: number;
  emptyText: string;
  footer?: ReactNode;
  children?: ReactNode;
}

/** 工作台三栏之一（设计文档 §8.2）。空栏写明为什么空。 */
export function WorkbenchColumn({
  testId,
  title,
  count,
  emptyText,
  footer,
  children,
}: WorkbenchColumnProps) {
  return (
    <section data-testid={testId} className="flex min-w-0 flex-col gap-half">
      <header className="flex items-center gap-half">
        <h2 className="m-0 text-sm font-medium text-high">{title}</h2>
        <span className="font-ibm-plex-mono text-sm text-low">{count}</span>
      </header>
      <div className="flex flex-col gap-half">
        {count === 0 ? (
          <p className="m-0 rounded-sm border border-dashed border-border p-base text-center text-sm text-low">
            {emptyText}
          </p>
        ) : (
          children
        )}
      </div>
      {footer}
    </section>
  );
}
```

Create `packages/ui/src/components/WorkbenchCard.tsx`:
```tsx
import type { ReactNode } from 'react';
import { cn } from '../lib/cn';
import { PrimaryButton } from './PrimaryButton';

export interface WorkbenchCardProps {
  issueId: string;
  simpleId: string;
  title: string;
  stageBadge?: ReactNode;
  statusTag?: ReactNode;
  /** 已翻译的相对时间。 */
  timeText?: string | null;
  meta?: string | null;
  progress?: ReactNode;
  actions?: ReactNode;
  /** 等人工描橙边，失败描红边（颜色之外还有文字状态标签，色弱可辨）。 */
  highlight?: 'gate' | 'failed' | null;
  openLabel: string;
  onOpen: () => void;
}

export function WorkbenchCard({
  issueId,
  simpleId,
  title,
  stageBadge,
  statusTag,
  timeText,
  meta,
  progress,
  actions,
  highlight = null,
  openLabel,
  onOpen,
}: WorkbenchCardProps) {
  return (
    <article
      data-testid="workbench-card"
      data-issue-id={issueId}
      className={cn(
        'flex flex-col gap-half rounded-sm border bg-primary p-base',
        highlight === 'failed'
          ? 'border-stage-failed'
          : highlight === 'gate'
            ? 'border-brand/60'
            : 'border-border'
      )}
    >
      <div className="flex min-w-0 items-center gap-half">
        <span className="shrink-0 font-ibm-plex-mono text-xs text-low">{simpleId}</span>
        {stageBadge}
        {statusTag}
        {timeText && (
          <span data-testid="relative-time" className="ml-auto shrink-0 text-xs text-low">
            {timeText}
          </span>
        )}
      </div>
      <button
        type="button"
        onClick={onOpen}
        className="m-0 truncate text-left text-base text-high hover:underline"
      >
        {title}
      </button>
      {meta && <p className="m-0 text-sm text-low">{meta}</p>}
      {progress}
      <div className="flex items-center justify-end gap-half">
        {actions}
        <PrimaryButton variant="tertiary" value={openLabel} onClick={onOpen} />
      </div>
    </article>
  );
}
```

Run: `pnpm run ui:check`
Expected: 0 error。

- [ ] **Step 3: 输入框容器**

Create `packages/web-core/src/pages/workbench/WorkbenchComposerContainer.tsx`:
```tsx
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';
import type { BaseCodingAgent } from 'shared/types';
import { PROJECT_PROJECT_STATUSES_SHAPE } from 'shared/remote-types';
import { WorkbenchComposer } from '@vibe/ui/components/WorkbenchComposer';
import { repoApi } from '@/shared/lib/api';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useUserSystem } from '@/shared/hooks/useUserSystem';
import { useExecutorConfig } from '@/shared/hooks/useExecutorConfig';
import { toPrettyCase } from '@/shared/lib/string';
import { useStartPipeline } from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  buildCreateIssueRequest,
  pickBacklogStatusId,
  pickDefaultBranch,
} from './workbenchModel';

/**
 * 工作台输入框容器：一句话 → 建需求 → 启动流水线（契约 §2 POST pipeline）。
 *
 * 不复用 `CreateChatBoxContainer` / `CreateModeRepoPickerBar`：它们绑死
 * `useCreateMode()` 草稿上下文并调用建工作区接口（计划 §2.5）。这里只复用
 * `useExecutorConfig`（纯 props 驱动）与 `repoApi`。
 */
export function WorkbenchComposerContainer({
  projectId,
}: {
  projectId: string | null;
}) {
  const { t } = useTranslation('common');
  const { profiles, config } = useUserSystem();
  const [prompt, setPrompt] = useState('');
  const [repoId, setRepoId] = useState<string | null>(null);
  const [branch, setBranch] = useState<string | null>(null);

  const reposQuery = useQuery({
    queryKey: ['workbench', 'repos'],
    queryFn: () => repoApi.list(),
  });
  const repos = reposQuery.data ?? [];
  const effectiveRepoId =
    repoId && repos.some((repo) => repo.id === repoId)
      ? repoId
      : (repos[0]?.id ?? null);
  const repo = repos.find((item) => item.id === effectiveRepoId) ?? null;

  const branchesQuery = useQuery({
    queryKey: ['workbench', 'branches', effectiveRepoId],
    queryFn: () => repoApi.getBranches(effectiveRepoId as string),
    enabled: !!effectiveRepoId,
  });
  const localBranches = useMemo(
    () =>
      (branchesQuery.data ?? [])
        .filter((item) => !item.is_remote)
        .map((item) => item.name),
    [branchesQuery.data]
  );
  const effectiveBranch =
    branch && localBranches.includes(branch)
      ? branch
      : repo
        ? pickDefaultBranch(repo, branchesQuery.data ?? [])
        : null;

  const { executorConfig, effectiveExecutor, executorOptions, setExecutor } =
    useExecutorConfig({
      profiles,
      lastUsedConfig: null,
      scratchConfig: null,
      configExecutorProfile: config?.executor_profile,
    });

  const { data: statuses } = useShape(
    PROJECT_PROJECT_STATUSES_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: !!projectId }
  );
  const statusId = pickBacklogStatusId(statuses);
  const start = useStartPipeline();

  const canSubmit =
    !!projectId &&
    !!statusId &&
    !!effectiveRepoId &&
    !!effectiveBranch &&
    !!executorConfig &&
    prompt.trim().length > 0 &&
    !start.isPending;

  const handleSubmit = () => {
    if (
      !canSubmit ||
      !projectId ||
      !statusId ||
      !effectiveRepoId ||
      !effectiveBranch ||
      !executorConfig
    ) {
      return;
    }
    start.mutate(
      {
        issue: buildCreateIssueRequest({
          id: crypto.randomUUID(),
          projectId,
          statusId,
          prompt,
        }),
        pipeline: {
          repos: [{ repo_id: effectiveRepoId, target_branch: effectiveBranch }],
          executor_config: executorConfig,
          template_key: null,
        },
      },
      { onSuccess: () => setPrompt('') }
    );
  };

  const notice = !projectId
    ? t('workbench.composer.noProject')
    : reposQuery.isSuccess && repos.length === 0
      ? t('workbench.composer.noRepo')
      : null;

  return (
    <WorkbenchComposer
      prompt={prompt}
      onPromptChange={setPrompt}
      repos={repos.map((item) => ({
        id: item.id,
        label: item.display_name || item.name,
      }))}
      repoId={effectiveRepoId}
      onRepoChange={(id) => {
        setRepoId(id);
        setBranch(null);
      }}
      branches={localBranches}
      branch={effectiveBranch}
      onBranchChange={setBranch}
      agents={executorOptions.map((option) => ({
        id: option,
        label: toPrettyCase(option),
      }))}
      agent={effectiveExecutor}
      onAgentChange={(id) => setExecutor(id as BaseCodingAgent)}
      canSubmit={canSubmit}
      isSubmitting={start.isPending}
      notice={notice}
      error={
        start.error
          ? t('workbench.composer.error', { message: start.error.message })
          : null
      }
      onSubmit={handleSubmit}
    />
  );
}
```

> 若 `StartPipelineRequest.template_key` 在生成类型里是可选（`template_key?: string | null`），`template_key: null` 仍然合法。

- [ ] **Step 4: 工作台页面**

Replace `packages/web-core/src/pages/workbench/WorkbenchPage.tsx` 全文：
```tsx
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useRouter } from '@tanstack/react-router';
import type { PendingPipelineItem, PipelineRun } from 'shared/types';
import { PROJECTS_SHAPE, PROJECT_ISSUES_SHAPE } from 'shared/remote-types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useUiPreferencesStore } from '@/shared/stores/useUiPreferencesStore';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
import {
  useGateDecision,
  usePendingPipelines,
  usePipelineRuns,
  usePipelineStageRuns,
} from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  buildPipelineCardInfo,
  groupStagesByRun,
} from '@/entities/pipeline/model/cardInfo';
import {
  pipelineStatus,
  pipelineStatusText,
  toMillis,
  type Translate,
} from '@/entities/pipeline/model/progress';
import {
  pipelineStageLabelKey,
  pipelineStageTone,
} from '@/entities/pipeline/model/stages';
import { relativeTimeText } from '@/entities/pipeline/model/time';
import { PipelineProgressBar } from '@vibe/ui/components/PipelineProgressBar';
import { PipelineStageBadge } from '@vibe/ui/components/PipelineStageBadge';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { PrimaryButton } from '@vibe/ui/components/PrimaryButton';
import { WorkbenchCard } from '@vibe/ui/components/WorkbenchCard';
import { WorkbenchColumn } from '@vibe/ui/components/WorkbenchColumn';
import { WorkbenchComposerContainer } from './WorkbenchComposerContainer';
import {
  greetingKey,
  pickWorkbenchProjectId,
  recentDelivered,
  runningRuns,
  stagesFinishedToday,
  weeklyStats,
} from './workbenchModel';

/** 每分钟刷新一次「现在」，让相对时间与问候语跟着走。 */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/**
 * 工作台（设计文档 §8.2 + 草图①）：顶部描述需求；三栏只回答三件事——
 * 什么在等我、什么在自动跑、最近交付了什么。
 */
export function WorkbenchPage() {
  const { t } = useTranslation('common');
  const translate: Translate = (key, params) => String(t(key, params));
  const router = useRouter();
  const now = useNow(60_000);
  const isPersonal = isLocalPersonalMode();

  const selectedProjectId = useUiPreferencesStore((s) => s.selectedProjectId);
  const { data: projects } = useShape(PROJECTS_SHAPE, {}, { enabled: isPersonal });
  const projectId = pickWorkbenchProjectId(projects, selectedProjectId);

  const { data: issues } = useShape(
    PROJECT_ISSUES_SHAPE,
    { project_id: projectId ?? '' },
    { enabled: isPersonal && !!projectId }
  );
  const { runs } = usePipelineRuns(projectId);
  const { stages } = usePipelineStageRuns(projectId);
  const { data: pending = [] } = usePendingPipelines(projectId);
  const gate = useGateDecision();

  const issueById = useMemo(
    () => new Map(issues.map((issue) => [issue.id, issue])),
    [issues]
  );
  const stagesByRun = useMemo(() => groupStagesByRun(stages), [stages]);
  const running = useMemo(() => runningRuns(runs), [runs]);
  const delivered = useMemo(() => recentDelivered(runs), [runs]);
  const weekly = useMemo(() => weeklyStats(runs, stages, now), [runs, stages, now]);
  const finishedToday = useMemo(
    () => stagesFinishedToday(stages, now),
    [stages, now]
  );

  if (!isPersonal) return null;

  const openIssue = (issueId: string) => {
    if (projectId) router.history.push(PERSONAL_ROUTES.issueDetail(projectId, issueId));
  };
  const timeText = (value: unknown): string | null => {
    const at = toMillis(value);
    if (at === null) return null;
    const spec = relativeTimeText(at, now);
    return t(spec.key, spec.params);
  };
  const stageBadge = (key: PipelineRun['current_stage_key']) => (
    <PipelineStageBadge
      tone={pipelineStageTone(key)}
      label={t(pipelineStageLabelKey(key))}
    />
  );

  const renderPending = (item: PendingPipelineItem) => {
    const info = pipelineStatus([item.stage_run], null, item.run);
    const isHumanGate =
      item.stage_run.status === 'waiting_gate' && item.stage_run.gate_kind === 'human';
    return (
      <WorkbenchCard
        key={item.run.id}
        issueId={item.run.issue_id}
        simpleId={item.issue_simple_id}
        title={item.issue_title}
        stageBadge={stageBadge(item.stage_run.stage_key)}
        statusTag={
          <PipelineStatusTag tone={info.tone} label={pipelineStatusText(info, translate)} />
        }
        timeText={timeText(item.stage_run.finished_at ?? item.stage_run.started_at)}
        meta={item.stage_run.error ?? item.stage_run.summary}
        highlight={info.tone === 'failed' ? 'failed' : 'gate'}
        openLabel={t('workbench.card.open')}
        onOpen={() => openIssue(item.run.issue_id)}
        actions={
          isHumanGate ? (
            <PrimaryButton
              value={t('pipeline.gate.confirm')}
              disabled={gate.isPending}
              onClick={() =>
                gate.mutate({
                  issueId: item.run.issue_id,
                  stageRunId: item.stage_run.id,
                  request: { decision: 'approve', comment: null },
                })
              }
            />
          ) : null
        }
      />
    );
  };

  const renderRun = (run: PipelineRun, withProgress: boolean) => {
    const issue = issueById.get(run.issue_id);
    const card = buildPipelineCardInfo(run, stagesByRun.get(run.id) ?? [], translate);
    return (
      <WorkbenchCard
        key={run.id}
        issueId={run.issue_id}
        simpleId={issue?.simple_id ?? ''}
        title={issue?.title ?? ''}
        stageBadge={withProgress ? stageBadge(run.current_stage_key) : null}
        statusTag={<PipelineStatusTag tone={card.tone} label={card.statusText} />}
        timeText={timeText(withProgress ? run.updated_at : run.finished_at)}
        progress={
          withProgress ? (
            <PipelineProgressBar cells={card.cells} ariaLabel={card.progressLabel} />
          ) : null
        }
        openLabel={t('workbench.card.open')}
        onOpen={() => openIssue(run.issue_id)}
      />
    );
  };

  return (
    <div className="h-full overflow-y-auto bg-primary">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-double px-double py-double">
        <header data-testid="workbench-greeting" className="flex flex-col gap-half">
          <h1 className="m-0 text-xl font-medium text-high">
            {t(greetingKey(new Date(now).getHours()))}
          </h1>
          <p className="m-0 text-sm text-low">
            {t('workbench.summary', {
              stages: finishedToday,
              pending: pending.length,
            })}
          </p>
        </header>

        <WorkbenchComposerContainer projectId={projectId} />

        {gate.error && (
          <p role="alert" className="m-0 text-sm text-error">
            {t('pipeline.gate.error', { message: gate.error.message })}
          </p>
        )}

        <div className="grid grid-cols-1 gap-double lg:grid-cols-3">
          <WorkbenchColumn
            testId="workbench-column-confirm"
            title={t('workbench.columns.confirm')}
            count={pending.length}
            emptyText={t('workbench.empty.confirm')}
          >
            {pending.map(renderPending)}
          </WorkbenchColumn>
          <WorkbenchColumn
            testId="workbench-column-running"
            title={t('workbench.columns.running')}
            count={running.length}
            emptyText={t('workbench.empty.running')}
          >
            {running.map((run) => renderRun(run, true))}
          </WorkbenchColumn>
          <WorkbenchColumn
            testId="workbench-column-delivered"
            title={t('workbench.columns.delivered')}
            count={delivered.length}
            emptyText={t('workbench.empty.delivered')}
            footer={
              <p className="m-0 text-sm text-low">
                {weekly.delivered > 0
                  ? t('workbench.weekly', {
                      delivered: weekly.delivered,
                      avg: weekly.avgCycleMinutes ?? 0,
                      rate: weekly.firstPassRate ?? 0,
                    })
                  : t('workbench.weeklyEmpty')}
              </p>
            }
          >
            {delivered.map((run) => renderRun(run, false))}
          </WorkbenchColumn>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: 检查 + 冒烟**

Run: `pnpm run ui:check && pnpm run web-core:check && pnpm run local-web:check`
Expected: 0 error。

冒烟（需要计划 A 已完成且以 qa-mode 起后端：`pnpm run dev:qa`，若脚本缺失见 Task 22）：打开 `/home`，看到问候语、输入框、三栏空态；输入一句话点「开始」，「正在自动跑」出现卡片，几秒后「需要你确认」出现「等你确认 · 需求确认」卡片。

- [ ] **Step 6: 提交**

```bash
pnpm run ui:format >/dev/null && pnpm run web-core:format >/dev/null
git add packages/ui/src/components/Workbench*.tsx packages/web-core/src/pages/workbench
git commit -m "$(cat <<'EOF'
界面：工作台——一句话启动流水线，三栏展示待确认、自动运行与最近交付

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 21: 需求详情全屏页 + 面板入口

**Files:**
- Create: `packages/web-core/src/pages/issue-detail/ArtifactSection.tsx`
- Create: `packages/web-core/src/pages/issue-detail/GateBarContainer.tsx`
- Modify（整体替换 Task 17 的占位）: `packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx`
- Modify: `packages/ui/src/components/KanbanIssuePanel.tsx:13-16,144,203,307`
- Modify: `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx:1-70,1040-1044,1129`

- [ ] **Step 1: 产出物区块**

Create `packages/web-core/src/pages/issue-detail/ArtifactSection.tsx`:
```tsx
import { useTranslation } from 'react-i18next';
import type { ArtifactKind, IssueArtifactSummary } from 'shared/types';
import { MarkdownPreview } from '@/shared/components/MarkdownPreview';
import { getResolvedTheme, useTheme } from '@/shared/hooks/useTheme';
import { useArtifact } from '@/entities/pipeline/model/hooks/usePipelineData';
import { CsvTableView } from '@vibe/ui/components/CsvTableView';
import { Skeleton } from '@vibe/ui/components/Skeleton';
import { artifactKindLabelKey, artifactRenderMode } from './issueDetailModel';
import {
  parseCsv,
  parseReview,
  parseTestReport,
  type ReviewSeverity,
  type TestCaseStatus,
} from './artifactParsers';

const SEVERITY_KEYS: Record<ReviewSeverity, string> = {
  blocker: 'issueDetail.severity.blocker',
  major: 'issueDetail.severity.major',
  minor: 'issueDetail.severity.minor',
  unknown: 'issueDetail.severity.unknown',
};

const ATTRIBUTION_KEYS: Record<'code' | 'case', string> = {
  code: 'issueDetail.attribution.code',
  case: 'issueDetail.attribution.case',
};

const CASE_STATUS_KEYS: Record<TestCaseStatus, string> = {
  passed: 'issueDetail.caseStatus.passed',
  failed: 'issueDetail.caseStatus.failed',
  unknown: 'issueDetail.caseStatus.unknown',
};

function RawContent({ content }: { content: string }) {
  const { t } = useTranslation('common');
  return (
    <div className="flex flex-col gap-half">
      <p className="m-0 text-sm text-low">{t('issueDetail.parseError')}</p>
      <pre className="m-0 overflow-x-auto rounded-sm bg-secondary p-base font-ibm-plex-mono text-xs text-normal">
        {content}
      </pre>
    </div>
  );
}

function ArtifactBody({
  kind,
  content,
  theme,
}: {
  kind: ArtifactKind;
  content: string;
  theme: 'light' | 'dark';
}) {
  const { t } = useTranslation('common');
  switch (artifactRenderMode(kind)) {
    case 'markdown':
      return <MarkdownPreview content={content} theme={theme} className="text-sm" />;
    case 'csv': {
      const table = parseCsv(content);
      return (
        <CsvTableView
          header={table.header}
          rows={table.rows}
          emptyText={t('issueDetail.noArtifact')}
        />
      );
    }
    case 'review': {
      const result = parseReview(content);
      if (!result.ok) return <RawContent content={content} />;
      if (result.value.length === 0) {
        return <p className="m-0 text-sm text-low">{t('issueDetail.reviewEmpty')}</p>;
      }
      return (
        <ul className="m-0 flex list-none flex-col gap-half p-0">
          {result.value.map((finding, index) => (
            <li key={index} className="rounded-sm border border-border p-half text-sm">
              <span
                className={
                  finding.severity === 'blocker'
                    ? 'font-medium text-stage-failed'
                    : 'font-medium text-normal'
                }
              >
                {t(SEVERITY_KEYS[finding.severity])}
              </span>
              {finding.file && (
                <span className="ml-half font-ibm-plex-mono text-xs text-low">
                  {finding.line !== null ? `${finding.file}:${finding.line}` : finding.file}
                </span>
              )}
              <p className="m-0 text-normal">{finding.message}</p>
            </li>
          ))}
        </ul>
      );
    }
    case 'test_report': {
      const result = parseTestReport(content);
      if (!result.ok) return <RawContent content={content} />;
      const report = result.value;
      return (
        <div className="flex flex-col gap-half">
          <p className="m-0 text-sm text-normal">
            {t('issueDetail.testSummary', {
              passed: report.passed,
              failed: report.failed,
              total: report.total,
            })}
          </p>
          <CsvTableView
            header={[
              t('issueDetail.testCols.case'),
              t('issueDetail.testCols.status'),
              t('issueDetail.testCols.attribution'),
              t('issueDetail.testCols.message'),
            ]}
            rows={report.cases.map((testCase) => [
              testCase.id,
              t(CASE_STATUS_KEYS[testCase.status]),
              testCase.attribution ? t(ATTRIBUTION_KEYS[testCase.attribution]) : '—',
              testCase.message,
            ])}
            emptyText={t('issueDetail.noArtifact')}
          />
        </div>
      );
    }
  }
}

/** 一个产出物（最新版本）：标题 + 版本 + 截断提示 + 按种类渲染的内容。 */
export function ArtifactSection({ summary }: { summary: IssueArtifactSummary }) {
  const { t } = useTranslation('common');
  const { theme } = useTheme();
  const { data, isLoading, error } = useArtifact(summary.id);

  return (
    <section
      data-testid="artifact-section"
      data-kind={summary.kind}
      className="flex flex-col gap-half"
    >
      <header className="flex items-center gap-half">
        <h3 className="m-0 text-sm font-medium text-high">
          {t(artifactKindLabelKey(summary.kind))}
        </h3>
        <span className="font-ibm-plex-mono text-xs text-low">
          {t('issueDetail.version', { version: String(summary.version) })}
        </span>
      </header>
      {summary.truncated && (
        <p className="m-0 text-sm text-low">{t('issueDetail.truncated')}</p>
      )}
      {isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : error || !data ? (
        <p role="alert" className="m-0 text-sm text-error">
          {t('pipeline.gate.error', { message: error?.message ?? '' })}
        </p>
      ) : (
        <ArtifactBody
          kind={summary.kind}
          content={data.content}
          theme={getResolvedTheme(theme)}
        />
      )}
    </section>
  );
}
```

- [ ] **Step 2: 关卡条容器**

Create `packages/web-core/src/pages/issue-detail/GateBarContainer.tsx`:
```tsx
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { IssuePipelineView } from 'shared/types';
import { PipelineGateBar } from '@vibe/ui/components/PipelineGateBar';
import { useGateDecision } from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  autoConditionLabelKey,
  fallbackGateLabelKey,
  pipelineStageLabelKey,
} from '@/entities/pipeline/model/stages';
import { gateBarState } from './issueDetailModel';

/** 底部关卡条（设计文档 §8.4）。调用方用 key 在阶段 / 状态变化时重置打回草稿。 */
export function GateBarContainer({
  issueId,
  view,
}: {
  issueId: string;
  view: IssuePipelineView;
}) {
  const { t } = useTranslation('common');
  const gate = useGateDecision();
  const [rejectDraft, setRejectDraft] = useState<string | null>(null);
  const state = gateBarState(view);

  if (state.kind === 'none') return null;

  const stage = 'stageKey' in state ? t(pipelineStageLabelKey(state.stageKey)) : '';
  const round =
    'attempt' in state && state.maxRounds !== null
      ? t('pipeline.gate.round', { attempt: state.attempt, max: state.maxRounds })
      : null;

  let title: string;
  let message: string;
  switch (state.kind) {
    case 'human':
      title = state.gateLabel ?? t(fallbackGateLabelKey(state.stageKey));
      message = t('pipeline.status.waitingGate', { gate: title });
      break;
    case 'auto': {
      const conditionKey = autoConditionLabelKey(state.stageKey);
      title = t('pipeline.gate.autoGate');
      message = [conditionKey ? t(conditionKey) : stage, round]
        .filter(Boolean)
        .join(' · ');
      break;
    }
    case 'running':
      title = stage;
      message = t('pipeline.gate.runningHint', { stage });
      break;
    case 'paused':
      title = t('pipeline.status.paused');
      message = t('pipeline.gate.pausedHint');
      break;
    case 'failed':
      title = t('pipeline.status.failed');
      message = [t('pipeline.gate.failedHint'), round].filter(Boolean).join(' · ');
      break;
    case 'completed':
      title = t('pipeline.status.completed');
      message = t('pipeline.gate.completedHint');
      break;
    case 'cancelled':
      title = t('pipeline.status.cancelled');
      message = t('pipeline.gate.cancelledHint');
      break;
  }

  const decide = (decision: 'approve' | 'reject', comment: string | null) => {
    if (state.kind !== 'human') return;
    gate.mutate(
      {
        issueId,
        stageRunId: state.stageRunId,
        request: { decision, comment },
      },
      { onSuccess: () => setRejectDraft(null) }
    );
  };

  return (
    <PipelineGateBar
      kind={state.kind}
      title={title}
      message={message}
      onConfirm={() => decide('approve', null)}
      onRejectSubmit={(comment) => {
        if (comment) decide('reject', comment);
      }}
      rejectDraft={rejectDraft}
      onRejectDraftChange={setRejectDraft}
      isBusy={gate.isPending}
      error={gate.error ? t('pipeline.gate.error', { message: gate.error.message }) : null}
    />
  );
}
```

- [ ] **Step 3: 详情页**

Replace `packages/web-core/src/pages/issue-detail/IssueDetailPage.tsx` 全文：
```tsx
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useRouter } from '@tanstack/react-router';
import type { IssueArtifactSummary } from 'shared/types';
import { PROJECT_ISSUES_SHAPE } from 'shared/remote-types';
import { useShape } from '@/shared/integrations/electric/hooks';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
import {
  useIssuePipeline,
  usePipelineRunAction,
} from '@/entities/pipeline/model/hooks/usePipelineData';
import {
  pipelineProgress,
  pipelineStatus,
  pipelineStatusText,
  type Translate,
} from '@/entities/pipeline/model/progress';
import { cellLabelKey } from '@/entities/pipeline/model/cardInfo';
import {
  pipelineStageLabelKey,
  pipelineStageTone,
} from '@/entities/pipeline/model/stages';
import { durationText } from '@/entities/pipeline/model/time';
import { IssuePanelTabBar } from '@vibe/ui/components/IssuePanelTabBar';
import { PipelineStageBadge } from '@vibe/ui/components/PipelineStageBadge';
import { PipelineStatusTag } from '@vibe/ui/components/PipelineStatusTag';
import { PipelineStepper } from '@vibe/ui/components/PipelineStepper';
import { PipelineTimeline } from '@vibe/ui/components/PipelineTimeline';
import { PrimaryButton } from '@vibe/ui/components/PrimaryButton';
import { Skeleton } from '@vibe/ui/components/Skeleton';
import { ArtifactSection } from './ArtifactSection';
import { GateBarContainer } from './GateBarContainer';
import { buildTimeline, timelineTextKey } from './timeline';
import {
  ISSUE_DETAIL_TABS,
  artifactKindLabelKey,
  defaultIssueDetailTab,
  issueDetailTabLabelKey,
  latestArtifactOfKind,
  latestArtifactsByKind,
  tabArtifactKinds,
  tabOfArtifactKind,
  type IssueDetailTab,
} from './issueDetailModel';

function DetailSkeleton({ label }: { label: string }) {
  return (
    <div role="status" aria-label={label} className="flex h-full flex-col gap-base bg-primary p-double">
      <Skeleton className="h-6 w-1/3" />
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}

/**
 * 需求详情全屏页（设计文档 §8.4 + 草图③）：顶部面包屑 / 标题 / 阶段徽标 /
 * 暂停；七段步进条；左边六个页签的产出物；右边时间线、产出物列表、用量；
 * 底部关卡条。原右侧面板路由不动。
 */
export function IssueDetailPage({
  projectId,
  issueId,
}: {
  projectId: string;
  issueId: string;
}) {
  const { t } = useTranslation('common');
  const translate: Translate = (key, params) => String(t(key, params));
  const router = useRouter();
  const appNavigation = useAppNavigation();
  const { data: issues, isLoading: issuesLoading } = useShape(PROJECT_ISSUES_SHAPE, {
    project_id: projectId,
  });
  const issue = issues.find((item) => item.id === issueId) ?? null;
  const pipelineQuery = useIssuePipeline(projectId, issueId);
  const view = pipelineQuery.data ?? null;
  const runAction = usePipelineRunAction();
  const [selectedTab, setSelectedTab] = useState<IssueDetailTab | null>(null);

  const timelineItems = useMemo(() => {
    if (!view) return [];
    return buildTimeline(
      view.stages.filter((stage) => stage.run_id === view.run.id),
      view.decisions
    ).map((event) => {
      const stage = t(pipelineStageLabelKey(event.stageKey));
      const clock = new Date(event.at).toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
      });
      const duration = event.durationMs !== null ? durationText(event.durationMs) : null;
      return {
        id: event.id,
        actor: event.actor,
        actorLabel: t(
          event.actor === 'ai' ? 'issueDetail.timeline.actorAi' : 'issueDetail.timeline.actorHuman'
        ),
        text: t(timelineTextKey(event.kind), { stage, attempt: event.attempt }),
        detail: event.text,
        meta: [clock, stage, duration ? t(duration.key, duration.params) : null]
          .filter(Boolean)
          .join(' · '),
      };
    });
  }, [t, view]);

  if (issuesLoading || pipelineQuery.isLoading) {
    return <DetailSkeleton label={t('issueDetail.loading')} />;
  }
  if (!issue) {
    return (
      <div className="flex h-full items-center justify-center bg-primary p-double">
        <p className="m-0 text-sm text-low">{t('issueDetail.notFound')}</p>
      </div>
    );
  }

  const activeTab = selectedTab ?? defaultIssueDetailTab(view?.run.current_stage_key ?? null);
  const status = view ? pipelineStatus(view.stages, view.template, view.run) : null;
  const canPause = view?.run.status === 'running' || view?.run.status === 'waiting_gate';
  const canResume = view?.run.status === 'paused' || view?.run.status === 'failed';
  const tabArtifacts = view
    ? tabArtifactKinds(activeTab)
        .map((kind) => latestArtifactOfKind(view.artifacts, kind))
        .filter((artifact): artifact is IssueArtifactSummary => artifact !== null)
    : [];

  return (
    <div className="flex h-full min-h-0 flex-col bg-primary">
      <header className="flex shrink-0 flex-col gap-base border-b border-border px-double py-base">
        <div className="flex flex-wrap items-center gap-half">
          <button
            type="button"
            onClick={() => router.history.push(PERSONAL_ROUTES.project(projectId))}
            className="text-sm text-low hover:text-normal"
          >
            {t('issueDetail.breadcrumb')}
          </button>
          <span aria-hidden="true" className="text-sm text-low">
            ›
          </span>
          <span data-testid="issue-simple-id" className="font-ibm-plex-mono text-sm text-low">
            {issue.simple_id}
          </span>
          <h1 className="m-0 min-w-0 truncate text-lg font-medium text-high">{issue.title}</h1>
          {view && (
            <PipelineStageBadge
              tone={pipelineStageTone(view.run.current_stage_key)}
              label={t(pipelineStageLabelKey(view.run.current_stage_key))}
            />
          )}
          {status && (
            <PipelineStatusTag tone={status.tone} label={pipelineStatusText(status, translate)} />
          )}
          <div className="ml-auto flex items-center gap-half">
            {view && canPause && (
              <PrimaryButton
                variant="tertiary"
                value={t('pipeline.gate.pause')}
                disabled={runAction.isPending}
                onClick={() => runAction.mutate({ issueId, runId: view.run.id, action: 'pause' })}
              />
            )}
            {view && canResume && (
              <PrimaryButton
                value={t('pipeline.gate.resume')}
                disabled={runAction.isPending}
                onClick={() => runAction.mutate({ issueId, runId: view.run.id, action: 'resume' })}
              />
            )}
          </div>
        </div>
        {runAction.error && (
          <p role="alert" className="m-0 text-sm text-error">
            {t('pipeline.gate.error', { message: runAction.error.message })}
          </p>
        )}
        {view && (
          <PipelineStepper
            currentKey={view.run.current_stage_key}
            steps={pipelineProgress(view.stages, view.template, view.run).map((cell) => ({
              key: cell.key,
              label: t(pipelineStageLabelKey(cell.key)),
              state: cell.state,
              stateLabel: t(cellLabelKey(cell.state)),
            }))}
          />
        )}
      </header>

      {!view ? (
        <div className="flex flex-1 items-center justify-center p-double">
          <p className="m-0 text-sm text-low">{t('issueDetail.noPipeline')}</p>
        </div>
      ) : (
        <>
          <div className="grid min-h-0 flex-1 grid-cols-1 xl:grid-cols-[minmax(0,1fr)_320px]">
            <main className="flex min-h-0 flex-col">
              <IssuePanelTabBar
                className="px-double"
                tabs={ISSUE_DETAIL_TABS.map((tab) => ({
                  id: tab,
                  label: t(issueDetailTabLabelKey(tab)),
                }))}
                activeTab={activeTab}
                onTabChange={setSelectedTab}
              />
              <div className="flex min-h-0 flex-1 flex-col gap-double overflow-y-auto px-double py-base">
                {activeTab === 'code' &&
                  (view.run.workspace_id ? (
                    <div>
                      <PrimaryButton
                        variant="tertiary"
                        value={t('issueDetail.openWorkspace')}
                        onClick={() => appNavigation.goToWorkspace(view.run.workspace_id as string)}
                      />
                    </div>
                  ) : (
                    <p className="m-0 text-sm text-low">{t('issueDetail.noWorkspace')}</p>
                  ))}
                {tabArtifacts.length === 0 ? (
                  <p className="m-0 text-sm text-low">{t('issueDetail.noArtifact')}</p>
                ) : (
                  tabArtifacts.map((summary) => (
                    <ArtifactSection key={summary.id} summary={summary} />
                  ))
                )}
              </div>
            </main>
            <aside className="flex min-h-0 flex-col gap-double overflow-y-auto border-l border-border bg-secondary p-base">
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">{t('issueDetail.side.timeline')}</h2>
                <PipelineTimeline items={timelineItems} emptyText={t('issueDetail.timeline.empty')} />
              </section>
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">{t('issueDetail.side.artifacts')}</h2>
                <ul className="m-0 flex list-none flex-col gap-1 p-0">
                  {latestArtifactsByKind(view.artifacts).map((artifact) => (
                    <li key={artifact.id}>
                      <button
                        type="button"
                        onClick={() => setSelectedTab(tabOfArtifactKind(artifact.kind))}
                        className="flex w-full items-center justify-between gap-half rounded-sm px-half py-half text-left text-sm text-normal hover:bg-primary"
                      >
                        <span className="truncate">{t(artifactKindLabelKey(artifact.kind))}</span>
                        <span className="font-ibm-plex-mono text-xs text-low">
                          {t('issueDetail.version', { version: String(artifact.version) })}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
              <section className="flex flex-col gap-half">
                <h2 className="m-0 text-sm font-medium text-high">{t('issueDetail.side.usage')}</h2>
                <p className="m-0 text-sm text-low">{t('issueDetail.side.usageNotReady')}</p>
              </section>
            </aside>
          </div>
          <GateBarContainer
            key={`${view.run.id}:${view.run.status}:${view.run.current_stage_key}`}
            issueId={issueId}
            view={view}
          />
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 面板「全屏查看」入口（视图）**

Modify `packages/ui/src/components/KanbanIssuePanel.tsx`：

`:13-16` 的图标 import 里加 `ArrowsOutIcon,`（放在 `XIcon,` 之前）。

`:144`（`onMoreActions?: () => void;`）之后插入：
```ts
  /** 「全屏查看」（设计文档 §8.4）。不传就不渲染；只有个人版传。 */
  onOpenFullscreen?: () => void;
```

解构 `:203`（`onMoreActions,`）之后插入 `  onOpenFullscreen,`

`:307`（`<div className="flex items-center gap-half">`，头部右侧按钮区开头）之后插入：
```tsx
          {!isCreateMode && onOpenFullscreen && (
            <button
              type="button"
              onClick={onOpenFullscreen}
              className="p-half rounded-sm text-low hover:text-normal hover:bg-panel transition-colors"
              aria-label={t('issueDetail.fullscreen')}
              title={t('issueDetail.fullscreen')}
            >
              <ArrowsOutIcon className="size-icon-sm" weight="bold" />
            </button>
          )}
```

- [ ] **Step 5: 面板「全屏查看」入口（容器）**

Modify `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx`：

import 区（`:10` `import { useTranslation } …` 之后）追加：
```ts
import { useRouter } from '@tanstack/react-router';
import { PERSONAL_ROUTES } from '@/shared/lib/routes/personalRoutes';
```

`:1040-1044`（`handleCopyLink`）之后插入：
```ts

  // 「全屏查看」只在个人版出现（设计文档 §8.4）。本文件也被 remote-web 编译，
  // 那里没有详情路由，所以用 history.push 路径字符串，不用带类型的 navigate。
  const router = useRouter();
  const handleOpenFullscreen = useCallback(() => {
    if (!selectedKanbanIssueId || !projectId) return;
    router.history.push(PERSONAL_ROUTES.issueDetail(projectId, selectedKanbanIssueId));
  }, [projectId, router, selectedKanbanIssueId]);
```

> 注意 hooks 规则：`useRouter()` 必须在任何提前 `return` 之前。`:1068-1080` 有 `if (isLoading || …) return <IssuePanelSkeleton />`，插入点 `:1045` 在它之前，符合要求。

`:1129`（`onMoreActions={…}`）之后插入：
```tsx
      onOpenFullscreen={
        mode === 'edit' && isLocalPersonalMode() ? handleOpenFullscreen : undefined
      }
```

- [ ] **Step 6: 检查（含 remote-web）**

Run:
```bash
pnpm run ui:check && pnpm run web-core:check && pnpm run local-web:check && pnpm run remote-web:check && pnpm run web-core:test 2>&1 | tail -3
```
Expected: 全部 0 error；单测全过。

- [ ] **Step 7: 冒烟**

`pnpm run dev:qa`：工作台启动一条需求 → 看板点卡片 → 右侧面板头部有「全屏查看」图标 → 进入详情：步进条第一格橙色、「需求与验收标准」页签显示 requirement.md、右栏时间线有「需求开始（第 1 次）」「需求等待确认」、底部关卡条有「确认并继续」「打回并说明」。

- [ ] **Step 8: 提交**

```bash
pnpm run ui:format >/dev/null && pnpm run web-core:format >/dev/null
git add packages/ui/src/components/KanbanIssuePanel.tsx packages/web-core/src/pages/issue-detail packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx
git commit -m "$(cat <<'EOF'
界面：需求详情全屏页（步进条、六页签产出物、时间线、关卡条）

右侧面板加「全屏查看」入口，仅个人版显示。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 22: 后端测试支撑核对（`VK_ASSET_DIR`、`dev:qa`，由计划 A 实现）

**Files:** 无改动（只读核对）

设计文档 §10.2 的 `VK_ASSET_DIR` 覆盖与 `dev:qa` 缺的 `backend:dev:watch:qa` 脚本都在计划 A Task 1 里实现（计划 A `crates/utils/src/assets.rs` 的 `ASSET_DIR_ENV` / `resolve_asset_dir`）。本计划**不重复实现**，只确认可用；缺了就回到计划 A。

- [ ] **Step 1: 代码已在**

Run:
```bash
grep -n 'ASSET_DIR_ENV\|VK_ASSET_DIR' crates/utils/src/assets.rs
node -e "const s=require('./package.json').scripts; console.log(Boolean(s['backend:dev:watch:qa']) && s['dev:qa'].includes('backend:dev:watch:qa'))"
cargo test -p utils assets::tests 2>&1 | grep "test result"
```
Expected: 第一条至少一行；第二条 `true`；第三条 `test result: ok.`。

- [ ] **Step 2: 数据目录确实被覆盖**

Run:
```bash
cargo build --bin server --features qa-mode
rm -rf /tmp/vk-asset-check
VK_ASSET_DIR=/tmp/vk-asset-check BACKEND_PORT=4398 ./target/debug/server >/tmp/vk-asset-check.log 2>&1 &
echo $! > /tmp/vk-asset-check.pid
sleep 15; ls /tmp/vk-asset-check; kill "$(cat /tmp/vk-asset-check.pid)"
```
Expected: `ls` 列出 `db.v2.sqlite`（`crates/db/src/lib.rs:138`）与 `config.json`；仓库里的 `dev_assets/` 没有被改动（`git status dev_assets` 干净）。本任务不提交。

---

## Task 23: e2e 包骨架与 fixtures

**Files:**
- Create: `packages/e2e/package.json`、`packages/e2e/tsconfig.json`、`packages/e2e/.gitignore`
- Create: `packages/e2e/playwright.config.ts`
- Create: `packages/e2e/support/env.ts`、`support/api.ts`、`support/fixtures.ts`、`support/pages.ts`
- Create: `packages/e2e/tests/shell.spec.ts`
- Modify: 根 `package.json`（`scripts`）、`pnpm-lock.yaml`（pnpm 生成）

**约束：Playwright 只用它自带的无头 Chromium，不设 `channel`，不连接也不启动本机的 Google Chrome。**

- [ ] **Step 1: 包声明**

Create `packages/e2e/package.json`:
```json
{
  "name": "@vibe/e2e",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "test": "playwright test",
    "test:update": "playwright test --update-snapshots",
    "install-browser": "playwright install chromium",
    "check": "tsc --noEmit -p tsconfig.json",
    "format": "prettier --config ../local-web/.prettierrc.json --write \"**/*.ts\"",
    "format:check": "prettier --config ../local-web/.prettierrc.json --check \"**/*.ts\""
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.9.2"
  }
}
```

Create `packages/e2e/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "types": ["node"]
  },
  "include": ["playwright.config.ts", "support", "tests"]
}
```

Create `packages/e2e/.gitignore`:
```
node_modules/
test-results/
playwright-report/
blob-report/
```

`pnpm-workspace.yaml:1-2` 已是 `packages/*`，新包自动纳入 workspace，不用改。

- [ ] **Step 2: 装依赖与浏览器**

Run:
```bash
pnpm --filter @vibe/e2e add -D @playwright/test prettier@^3.6.1
pnpm --filter @vibe/e2e exec playwright install chromium
pnpm --filter @vibe/e2e exec playwright --version
```
Expected: 第一条更新 `packages/e2e/package.json` 与 `pnpm-lock.yaml`；第二条下载 Playwright 自带的 Chromium（`chromium-XXXX` 到 `~/Library/Caches/ms-playwright` 或 `~/.cache/ms-playwright`）；第三条打印 `Version 1.x.y`。

- [ ] **Step 3: 环境常量与 Playwright 配置**

Create `packages/e2e/support/env.ts`:
```ts
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

/** 仓库根目录（packages/e2e/support → 上三级）。 */
export const REPO_ROOT = path.resolve(here, '../../..');

export const BACKEND_PORT = Number(process.env.VK_E2E_BACKEND_PORT ?? 4310);
export const FRONTEND_PORT = Number(process.env.VK_E2E_FRONTEND_PORT ?? 4311);

/** 每次起后端前清空（见 playwright.config.ts 的 webServer 命令）。 */
export const E2E_ROOT =
  process.env.VK_E2E_ROOT ?? path.join(os.tmpdir(), 'vk-e2e');
/** 后端数据目录（数据库、配置）：`VK_ASSET_DIR`。 */
export const ASSET_DIR = path.join(E2E_ROOT, 'assets');
/** fixtures 建的临时 git 仓库放这里。 */
export const REPOS_DIR = path.join(E2E_ROOT, 'repos');

export const BACKEND_URL = `http://127.0.0.1:${BACKEND_PORT}`;
/** 必须与 `VK_ALLOWED_ORIGINS` 逐字一致（`crates/server/src/middleware/origin.rs:139-152`）。 */
export const FRONTEND_URL = `http://localhost:${FRONTEND_PORT}`;
```

Create `packages/e2e/playwright.config.ts`:
```ts
import path from 'node:path';
import { defineConfig } from '@playwright/test';
import {
  ASSET_DIR,
  BACKEND_PORT,
  BACKEND_URL,
  E2E_ROOT,
  FRONTEND_PORT,
  FRONTEND_URL,
  REPOS_DIR,
  REPO_ROOT,
} from './support/env';

const isCI = !!process.env.CI;

/**
 * 真后端（qa-mode 模拟执行器）+ 真前端（vite dev），不打桩网络层（设计文档 §10）。
 *
 * - 只用 Playwright 自带的无头 Chromium：`browserName: 'chromium'`，**不设 channel**，
 *   不会连接或启动本机 Google Chrome。
 * - 后端只有一个实例，所有用例串行（workers: 1）；每个用例自己建项目与仓库，互不干扰。
 */
export default defineConfig({
  testDir: './tests',
  outputDir: './test-results',
  snapshotPathTemplate: '{testDir}/__screenshots__/{testFilePath}/{arg}-{platform}{ext}',
  fullyParallel: false,
  workers: 1,
  timeout: 6 * 60_000,
  retries: isCI ? 1 : 0,
  expect: {
    timeout: 15_000,
    toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled' },
  },
  reporter: isCI
    ? [['github'], ['html', { open: 'never', outputFolder: 'playwright-report' }]]
    : [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  use: {
    baseURL: FRONTEND_URL,
    browserName: 'chromium',
    headless: true,
    locale: 'zh-CN',
    timezoneId: 'Asia/Shanghai',
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
  webServer: [
    {
      // 先清空数据目录再起后端：每次跑都是干净的库。
      command: `rm -rf "${E2E_ROOT}" && mkdir -p "${ASSET_DIR}" "${REPOS_DIR}" && cargo run --bin server --features qa-mode`,
      cwd: REPO_ROOT,
      url: `${BACKEND_URL}/api/health`,
      timeout: 20 * 60_000,
      reuseExistingServer: !isCI,
      stdout: 'pipe',
      stderr: 'pipe',
      env: {
        BACKEND_PORT: String(BACKEND_PORT),
        HOST: '127.0.0.1',
        VK_ASSET_DIR: ASSET_DIR,
        VK_ALLOWED_ORIGINS: FRONTEND_URL,
        DISABLE_WORKTREE_CLEANUP: '1',
        RUST_LOG: 'info',
      },
    },
    {
      command: `pnpm exec vite --port ${FRONTEND_PORT} --strictPort`,
      cwd: path.join(REPO_ROOT, 'packages/local-web'),
      url: FRONTEND_URL,
      timeout: 2 * 60_000,
      reuseExistingServer: !isCI,
      env: {
        FRONTEND_PORT: String(FRONTEND_PORT),
        BACKEND_PORT: String(BACKEND_PORT),
        VITE_OPEN: 'false',
        VITE_VK_SHARED_API_BASE: '',
        // vite 代理目标是 http://localhost:<BACKEND_PORT>，后端只听 127.0.0.1（§4 V5）
        NODE_OPTIONS: '--dns-result-order=ipv4first',
      },
    },
  ],
});
```

- [ ] **Step 4: 后端接口助手与 fixtures**

Create `packages/e2e/support/api.ts`:
```ts
import { randomUUID } from 'node:crypto';
import { BACKEND_URL } from './env';

/** 个人版固定组织（`crates/db/src/models/local_project.rs:9`，`Uuid::from_u128(1)`）。 */
export const PERSONAL_ORGANIZATION_ID = '00000000-0000-0000-0000-000000000001';

interface Envelope<T> {
  success: boolean;
  data: T | null;
  message: string | null;
}

/** 只声明测试用到的字段；完整类型见 shared/types.ts（契约 §1）。 */
export interface PipelineViewLike {
  run: { id: string; issue_id: string; status: string; current_stage_key: string };
  stages: {
    id: string;
    run_id: string;
    stage_key: string;
    attempt: number;
    status: string;
    gate_kind: string;
  }[];
  decisions: { id: string; stage_run_id: string; decision: string; comment: string | null }[];
  artifacts: { id: string; kind: string; version: number }[];
  template: { stages: { key: string; gate_label: string | null; max_rounds: number }[] };
}

export interface IssueLike {
  id: string;
  title: string;
  simple_id: string;
  status_id: string;
}

/**
 * 直连后端（不经 vite 代理）。Node 的 fetch 不带 Origin 头，
 * `origin.rs:48-50` 放行；个人版没有 CSRF。
 */
export class VkApi {
  constructor(private readonly base: string = BACKEND_URL) {}

  private request(method: string, path: string, body?: unknown): Promise<Response> {
    return fetch(`${this.base}${path}`, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  }

  /** `ApiResponse<T>` 信封接口。 */
  async envelope<T>(method: string, path: string, body?: unknown): Promise<T> {
    const response = await this.request(method, path, body);
    const text = await response.text();
    let parsed: Envelope<T>;
    try {
      parsed = JSON.parse(text) as Envelope<T>;
    } catch {
      throw new Error(`${method} ${path} → ${response.status}: ${text.slice(0, 300)}`);
    }
    if (!response.ok || !parsed.success) {
      throw new Error(`${method} ${path} → ${response.status}: ${parsed.message ?? text.slice(0, 300)}`);
    }
    return parsed.data as T;
  }

  /** 本地项目接口：快照返回 `{ <表名>: [...] }`，写操作返回 `{ txid }`。 */
  async json<T>(method: string, path: string, body?: unknown): Promise<T> {
    const response = await this.request(method, path, body);
    if (!response.ok) {
      throw new Error(`${method} ${path} → ${response.status}: ${(await response.text()).slice(0, 300)}`);
    }
    return (await response.json()) as T;
  }

  /** 跳过首启引导、项目指南、版本说明；固定简体中文、主题跟随系统（视觉回归靠 colorScheme 切换）。 */
  async prepareConfig(): Promise<void> {
    const info = await this.envelope<{ config: Record<string, unknown> }>('GET', '/api/info');
    const config = info.config;
    const showcases = (config.showcases as { seen_features?: string[] } | undefined) ?? {};
    const seen = new Set(showcases.seen_features ?? []);
    seen.add('projects-guide');
    await this.envelope('PUT', '/api/config', {
      ...config,
      disclaimer_acknowledged: true,
      onboarding_acknowledged: true,
      remote_onboarding_acknowledged: true,
      analytics_enabled: false,
      show_release_notes: false,
      language: 'ZH_HANS',
      theme: 'SYSTEM',
      showcases: { ...showcases, seen_features: [...seen] },
    });
  }

  async defaultExecutor(): Promise<string> {
    const info = await this.envelope<{ config: { executor_profile: { executor: string } } }>('GET', '/api/info');
    return info.config.executor_profile.executor;
  }

  registerRepo(path: string, displayName: string) {
    return this.envelope<{ id: string; name: string; display_name: string }>('POST', '/api/repos', {
      path,
      display_name: displayName,
    });
  }

  async createProject(name: string): Promise<{ id: string; name: string }> {
    const id = randomUUID();
    await this.json('POST', '/api/local/projects', {
      id,
      organization_id: PERSONAL_ORGANIZATION_ID,
      name,
      color: '25 82% 54%',
    });
    return { id, name };
  }

  async listIssues(projectId: string): Promise<IssueLike[]> {
    const payload = await this.json<{ issues: IssueLike[] }>(
      'GET',
      `/api/local/issues?project_id=${projectId}`
    );
    return payload.issues;
  }

  async findIssueByTitle(projectId: string, title: string): Promise<IssueLike> {
    const issue = (await this.listIssues(projectId)).find((item) => item.title === title);
    if (!issue) throw new Error(`找不到需求：${title}`);
    return issue;
  }

  issuePipeline(issueId: string): Promise<PipelineViewLike | null> {
    return this.envelope('GET', `/api/local/issues/${issueId}/pipeline`);
  }

  /** 不经界面直接建需求并启动流水线（视觉回归准备数据用）。 */
  async startPipelineDirect(args: {
    projectId: string;
    repoId: string;
    title: string;
  }): Promise<string> {
    const { project_statuses: statuses } = await this.json<{
      project_statuses: { id: string; sort_order: number; stage_type?: string }[];
    }>('GET', `/api/local/project_statuses?project_id=${args.projectId}`);
    const backlog =
      statuses.find((status) => status.stage_type === 'backlog') ??
      [...statuses].sort((a, b) => a.sort_order - b.sort_order)[0];
    const issueId = randomUUID();
    await this.json('POST', '/api/local/issues', {
      id: issueId,
      project_id: args.projectId,
      status_id: backlog.id,
      title: args.title,
      description: null,
      priority: null,
      start_date: null,
      target_date: null,
      completed_at: null,
      sort_order: 0,
      parent_issue_id: null,
      parent_issue_sort_order: null,
      extension_metadata: {},
    });
    await this.envelope('POST', `/api/local/issues/${issueId}/pipeline`, {
      repos: [{ repo_id: args.repoId, target_branch: 'main' }],
      executor_config: { executor: await this.defaultExecutor() },
      template_key: null,
    });
    return issueId;
  }

  /** 轮询直到条件满足（引擎推进是异步的）。 */
  async waitForPipeline(
    issueId: string,
    predicate: (view: PipelineViewLike) => boolean,
    timeoutMs = 120_000
  ): Promise<PipelineViewLike> {
    const deadline = Date.now() + timeoutMs;
    let last: PipelineViewLike | null = null;
    while (Date.now() < deadline) {
      last = await this.issuePipeline(issueId);
      if (last && predicate(last)) return last;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    throw new Error(`等待流水线超时，最后状态：${JSON.stringify(last?.run ?? null)}`);
  }
}
```

Create `packages/e2e/support/fixtures.ts`:
```ts
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { test as base, expect } from '@playwright/test';
import { VkApi } from './api';
import { REPOS_DIR } from './env';

export interface E2eProject {
  projectId: string;
  projectName: string;
  repoId: string;
  repoName: string;
}

/**
 * 在临时目录建一个有一次提交的 git 仓库（main 分支）。
 *
 * qa-mode 只把「扫描本机仓库」换成两个固定的 GitHub QA 仓库
 * （`crates/services/src/services/filesystem.rs:101-105`），`POST /api/repos`
 * 注册任意路径不受影响，所以这里注册本地临时仓库，不触发任何外网 clone。
 */
export function createGitRepo(dir: string): string {
  fs.mkdirSync(dir, { recursive: true });
  const git = (...args: string[]) => execFileSync('git', args, { cwd: dir, stdio: 'pipe' });
  git('init', '-b', 'main');
  fs.writeFileSync(path.join(dir, 'README.md'), '# vk e2e fixture\n');
  git('add', '.');
  git('-c', 'user.name=vk-e2e', '-c', 'user.email=vk-e2e@example.com', 'commit', '-m', 'init');
  return dir;
}

let counter = 0;
function uniqueSlug(): string {
  counter += 1;
  return `${Date.now().toString(36)}-${counter}`;
}

export async function createProjectWithRepo(api: VkApi, projectName: string): Promise<E2eProject> {
  await api.prepareConfig();
  const repoName = `e2e-repo-${uniqueSlug()}`;
  const repo = await api.registerRepo(createGitRepo(path.join(REPOS_DIR, repoName)), repoName);
  const project = await api.createProject(projectName);
  return { projectId: project.id, projectName, repoId: repo.id, repoName };
}

export const test = base.extend<{ api: VkApi; project: E2eProject }>({
  // eslint 不覆盖本包；Playwright 要求第一个参数是对象解构
  api: async ({}, use) => {
    await use(new VkApi());
  },
  project: async ({ api }, use, testInfo) => {
    await use(await createProjectWithRepo(api, `E2E ${testInfo.title}`.slice(0, 60)));
  },
});

export { expect };
```

Create `packages/e2e/support/pages.ts`:
```ts
import { expect, type Locator, type Page } from '@playwright/test';
import type { E2eProject } from './fixtures';

/** 三道人工关卡名（§4 V3：以模板 `gate_label` 为准；与 zh-Hans `pipeline.gateLabel.*` 一致）。 */
export const GATE_LABELS = {
  requirement: '需求确认',
  spec: '设计规格确认',
  testDesign: '用例设计确认',
} as const;

/**
 * 单个阶段的等待上限。模拟器流水线模式每行日志 0.1 秒（契约 C9），阶段本身
 * 1～3 秒；再加工作区创建、引擎调度与推送，CI 机器慢时也够用。
 */
export const STAGE_TIMEOUT = 90_000;

export function sidebar(page: Page): Locator {
  return page.getByTestId('personal-sidebar');
}

export async function openProject(page: Page, project: E2eProject): Promise<void> {
  await page.goto(`/projects/${project.projectId}`);
  await expect(page.getByTestId('kanban-column-header').first()).toBeVisible();
}

/** 先进看板（外壳把它记为当前项目），再点「工作台」。 */
export async function openWorkbench(page: Page, project: E2eProject): Promise<void> {
  await openProject(page, project);
  await sidebar(page).getByRole('button', { name: /工作台/ }).click();
  await expect(page).toHaveURL(/\/home$/);
  await expect(page.getByRole('textbox', { name: '描述一个需求' })).toBeVisible();
}

export async function startRequirement(page: Page, project: E2eProject, text: string): Promise<void> {
  await page.getByRole('combobox', { name: '仓库' }).selectOption({ label: project.repoName });
  await expect(page.getByRole('combobox', { name: '分支' })).toHaveValue('main');
  const input = page.getByRole('textbox', { name: '描述一个需求' });
  await input.fill(text);
  await page.getByRole('button', { name: '开始' }).click();
  // 成功后输入框清空（WorkbenchComposerContainer 的 onSuccess）
  await expect(input).toHaveValue('', { timeout: 30_000 });
}

export function workbenchCard(
  page: Page,
  column: 'confirm' | 'running' | 'delivered',
  title: string
): Locator {
  return page
    .getByTestId(`workbench-column-${column}`)
    .getByTestId('workbench-card')
    .filter({ hasText: title });
}

/** 在工作台「需要你确认」里确认指定关卡。 */
export async function approveGate(
  page: Page,
  title: string,
  gateLabel: string,
  timeout = STAGE_TIMEOUT
): Promise<void> {
  const card = workbenchCard(page, 'confirm', title).filter({ hasText: gateLabel });
  await expect(card).toBeVisible({ timeout });
  await card.getByRole('button', { name: '确认并继续' }).click();
  await expect(card).toHaveCount(0, { timeout: 30_000 });
}

export async function openDetail(page: Page, projectId: string, issueId: string): Promise<void> {
  await page.goto(`/projects/${projectId}/issues/${issueId}/detail`);
  await expect(page.getByTestId('pipeline-stepper')).toBeVisible();
}

export function kanbanProgress(page: Page, issueId: string): Locator {
  return page
    .locator(`[data-testid="kanban-card-pipeline"][data-issue-id="${issueId}"]`)
    .getByTestId('pipeline-progress');
}
```

- [ ] **Step 5: 写外壳冒烟用例**

Create `packages/e2e/tests/shell.spec.ts`:
```ts
import { expect, test } from '../support/fixtures';
import { openProject, sidebar } from '../support/pages';

test('个人版外壳：根路径进工作台，五个中文入口，无横幅、无外网徽标、无 Team/Personal 切换', async ({
  page,
  project,
}) => {
  await page.goto('/');
  await expect(page).toHaveURL(/\/home$/);

  const nav = sidebar(page);
  for (const name of ['工作台', '需求流水线', '测试中心', '文档', '设置']) {
    await expect(nav.getByRole('button', { name: new RegExp(name) })).toBeVisible();
  }
  await expect(page.getByLabel(/Star on GitHub|Join our Discord/)).toHaveCount(0);

  await nav.getByRole('button', { name: /文档/ }).click();
  await expect(page.getByTestId('docs-empty')).toContainText('规格与需求文档会出现在这里');

  await openProject(page, project);
  await expect(page.getByText('Vibe Kanban Cloud is shutting down')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^(Team|Personal)$/ })).toHaveCount(0);
  await expect(page.getByTestId('kanban-column-header')).toHaveCount(6);
  await expect(page.locator('[data-testid="kanban-column-header"][data-stage="backlog"]')).toContainText('需求');
  await expect(page.locator('[data-testid="kanban-column-header"][data-stage="backlog"]')).toContainText('人工确认');
  await expect(page.getByRole('button', { name: '新建需求' })).toHaveCount(1);
});
```

- [ ] **Step 6: 根脚本**

Modify 根 `package.json` 的 `scripts`：

新增三行（放在 `"test:npm"` 之后）：
```json
    "e2e": "pnpm --filter @vibe/e2e run test",
    "e2e:install": "pnpm --filter @vibe/e2e run install-browser",
    "e2e:check": "pnpm --filter @vibe/e2e run check",
```

`"check"` 末尾（`&& pnpm run web-core:test` 之后）追加 ` && pnpm run e2e:check`。

`"format"` 末尾（`&& pnpm run ui:format` 之后）追加 ` && pnpm --filter @vibe/e2e run format`。

- [ ] **Step 7: 类型检查**

Run: `pnpm run e2e:check`
Expected: 0 error。

- [ ] **Step 8: 跑冒烟（首次会编译 qa-mode 后端，耗时较长）**

Run:
```bash
cargo build --bin server --features qa-mode
pnpm run e2e -- tests/shell.spec.ts
```
Expected: `1 passed`。失败时看 `packages/e2e/playwright-report/index.html` 与 trace（`pnpm --filter @vibe/e2e exec playwright show-trace test-results/**/trace.zip`）。若前端报 `ECONNREFUSED ::1` 见 §4 V5。

- [ ] **Step 9: 提交**

```bash
pnpm --filter @vibe/e2e run format >/dev/null
git add packages/e2e package.json pnpm-lock.yaml
git commit -m "$(cat <<'EOF'
测试：新增 packages/e2e（Playwright 自带无头 Chromium + qa-mode 真后端）

webServer 起 qa-mode 后端与 vite 前端，数据目录每次清空；fixtures 在临时
目录建 git 仓库并通过接口注册；外壳冒烟用例。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 24: e2e 黄金路径

**Files:**
- Create: `packages/e2e/tests/golden-path.spec.ts`

- [ ] **Step 1: 写用例**

Create `packages/e2e/tests/golden-path.spec.ts`:
```ts
import { expect, test } from '../support/fixtures';
import {
  GATE_LABELS,
  STAGE_TIMEOUT,
  approveGate,
  kanbanProgress,
  openDetail,
  openWorkbench,
  sidebar,
  startRequirement,
  workbenchCard,
} from '../support/pages';

test('黄金路径：一句话 → 三道人工关卡 → 自动开发评审测试 → 交付', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(8 * 60_000);
  const title = `下单最小金额校验 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'running', title).or(workbenchCard(page, 'confirm', title))
  ).toBeVisible({ timeout: 30_000 });

  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);

  await expect(workbenchCard(page, 'delivered', title)).toBeVisible({
    timeout: 5 * STAGE_TIMEOUT,
  });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('completed');

  // 看板：卡片在「交付」列，七格全绿
  await sidebar(page).getByRole('button', { name: /需求流水线/ }).click();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'done,done,done,done,done,done,done'
  );
  await expect(
    page.locator('[data-testid="kanban-column-header"][data-stage="done"]')
  ).toContainText('1');

  // 详情：七段全勾、产出物页签、时间线三次确认、关卡条显示已交付
  await openDetail(page, project.projectId, issue.id);
  await expect(
    page.getByTestId('pipeline-stepper').locator('li[data-state="done"]')
  ).toHaveCount(7);
  await page.getByRole('tab', { name: '需求与验收标准' }).click();
  await expect(
    page.locator('[data-testid="artifact-section"][data-kind="requirement"]')
  ).toBeVisible();
  await page.getByRole('tab', { name: '用例' }).click();
  await expect(page.getByTestId('csv-table')).toBeVisible();
  await page.getByRole('tab', { name: '交付报告' }).click();
  await expect(
    page.locator('[data-testid="artifact-section"][data-kind="delivery_report"]')
  ).toBeVisible();
  const timeline = page.getByTestId('pipeline-timeline');
  await expect(timeline).toContainText('你确认了需求');
  await expect(timeline).toContainText('你确认了设计规格');
  await expect(timeline).toContainText('你确认了用例设计');
  await expect(page.getByTestId('gate-bar')).toHaveAttribute('data-kind', 'completed');
});
```

- [ ] **Step 2: 跑**

Run: `pnpm run e2e -- tests/golden-path.spec.ts`
Expected: `1 passed`（约 1～3 分钟）。

- [ ] **Step 3: 失败排查指引（跑通则跳过）**

- 卡在第一道关卡不出现：`curl -s $BACKEND/api/local/issues/<id>/pipeline`（`<id>` 用 `api.findIssueByTitle` 打印）看 `run.status`；`failed` 且 `stages[0].error` 提到缺产出物 → 计划 A 的模拟执行器没按契约 §5 写文件。
- 关卡卡片文案不是「需求确认」→ §4 V3，改 `GATE_LABELS`。
- 看板进度格一直是旧值 → WS 没推 `pipeline_*` 路径（契约 §3），查计划 A 的 `HookTables` 登记；前端会在 15 秒无 WS 时不轮询（有 wsPath 的集合不轮询，`localCollections.ts:339-342`），所以这一条必须由推送保证。

- [ ] **Step 4: 提交**

```bash
pnpm --filter @vibe/e2e run format >/dev/null
git add packages/e2e/tests/golden-path.spec.ts
git commit -m "$(cat <<'EOF'
测试：端到端黄金路径（一句话到交付）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 25: e2e 异常路径

**Files:**
- Create: `packages/e2e/tests/exceptions.spec.ts`

覆盖设计文档 §10.4：人工打回后重跑、自动判定失败用尽轮次转人工（`[qa:always-fail-review]`）、测试归因用例问题退回并重新确认（`[qa:test-fail-case-once]`）、暂停与继续、刷新后状态恢复。qa 标记见契约 §5。

- [ ] **Step 1: 写用例**

Create `packages/e2e/tests/exceptions.spec.ts`:
```ts
import { expect, test } from '../support/fixtures';
import {
  GATE_LABELS,
  STAGE_TIMEOUT,
  approveGate,
  kanbanProgress,
  openDetail,
  openProject,
  openWorkbench,
  sidebar,
  startRequirement,
  workbenchCard,
} from '../support/pages';

test('人工打回：必须写意见，同阶段重跑并留痕', async ({ page, api, project }) => {
  test.setTimeout(5 * 60_000);
  const title = `打回重跑 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'confirm', title).filter({ hasText: GATE_LABELS.requirement })
  ).toBeVisible({ timeout: STAGE_TIMEOUT });

  const issue = await api.findIssueByTitle(project.projectId, title);
  await openDetail(page, project.projectId, issue.id);
  const gateBar = page.getByTestId('gate-bar');
  await expect(gateBar).toHaveAttribute('data-kind', 'human');

  await gateBar.getByRole('button', { name: '打回并说明' }).click();
  await expect(gateBar.getByRole('button', { name: '提交打回' })).toBeDisabled();
  await gateBar.getByRole('textbox').fill('补充金额为 0 与负数的边界');
  await gateBar.getByRole('button', { name: '提交打回' }).click();

  const timeline = page.getByTestId('pipeline-timeline');
  await expect(timeline).toContainText('你打回了需求');
  await expect(timeline).toContainText('补充金额为 0 与负数的边界');
  await expect(timeline).toContainText('第 2 次', { timeout: 30_000 });
  await expect(gateBar).toHaveAttribute('data-kind', 'human', { timeout: STAGE_TIMEOUT });

  const view = await api.issuePipeline(issue.id);
  const attempts = view!.stages
    .filter((stage) => stage.stage_key === 'requirement')
    .map((stage) => Number(stage.attempt));
  expect(Math.max(...attempts)).toBe(2);
});

test('[qa:always-fail-review] 评审轮次用尽后转人工', async ({ page, api, project }) => {
  test.setTimeout(10 * 60_000);
  const title = `[qa:always-fail-review] 评审一直不过 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);

  await expect(
    workbenchCard(page, 'confirm', title).filter({ hasText: '需要你处理' })
  ).toBeVisible({ timeout: 6 * STAGE_TIMEOUT });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('failed');
  expect(view?.run.current_stage_key).toBe('review');
  const maxRounds = Number(
    view!.template.stages.find((stage) => stage.key === 'review')!.max_rounds
  );
  expect(view!.stages.filter((stage) => stage.stage_key === 'review')).toHaveLength(maxRounds);

  await sidebar(page).getByRole('button', { name: /需求流水线/ }).click();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'done,done,done,done,failed,pending,pending'
  );

  await openDetail(page, project.projectId, issue.id);
  await expect(page.getByTestId('gate-bar')).toHaveAttribute('data-kind', 'failed');
  await expect(page.getByTestId('gate-bar')).toContainText(`第 ${maxRounds}/${maxRounds} 轮`);
});

test('[qa:test-fail-case-once] 用例问题退回用例设计，重新人工确认后交付', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(10 * 60_000);
  const title = `[qa:test-fail-case-once] 用例回流 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await approveGate(page, title, GATE_LABELS.requirement);
  await approveGate(page, title, GATE_LABELS.spec);
  await approveGate(page, title, GATE_LABELS.testDesign);
  // 开发 → 评审 → 测试（失败，归因 case）→ 回到用例设计，再次等人工
  await approveGate(page, title, GATE_LABELS.testDesign, 5 * STAGE_TIMEOUT);

  await expect(workbenchCard(page, 'delivered', title)).toBeVisible({
    timeout: 5 * STAGE_TIMEOUT,
  });

  const issue = await api.findIssueByTitle(project.projectId, title);
  const view = await api.issuePipeline(issue.id);
  const testDesignRuns = view!.stages.filter((stage) => stage.stage_key === 'test_design');
  expect(testDesignRuns).toHaveLength(2);
  const approvals = view!.decisions.filter(
    (decision) =>
      decision.decision === 'approve' &&
      testDesignRuns.some((stage) => stage.id === decision.stage_run_id)
  );
  expect(approvals).toHaveLength(2);
  expect(view!.stages.filter((stage) => stage.stage_key === 'test')).toHaveLength(2);
});

test('暂停与继续：在人工关卡处暂停，刷新仍是暂停，继续后回到关卡', async ({
  page,
  api,
  project,
}) => {
  test.setTimeout(4 * 60_000);
  const title = `暂停继续 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  await expect(
    workbenchCard(page, 'confirm', title).filter({ hasText: GATE_LABELS.requirement })
  ).toBeVisible({ timeout: STAGE_TIMEOUT });

  // 契约 C5：waiting_gate 允许暂停。模拟器阶段只有 1～3 秒（C9），在运行中暂停
  // 时序不可控；停在人工关卡时暂停是确定的。
  const issue = await api.findIssueByTitle(project.projectId, title);
  await openDetail(page, project.projectId, issue.id);
  const gateBar = page.getByTestId('gate-bar');
  await expect(gateBar).toHaveAttribute('data-kind', 'human');
  await page.getByRole('button', { name: '暂停自动化' }).click();
  await expect(gateBar).toHaveAttribute('data-kind', 'paused');
  const resume = page.getByRole('button', { name: '继续', exact: true });
  await expect(resume).toBeVisible();
  // 暂停时关卡条不给「确认并继续」
  await expect(page.getByRole('button', { name: '确认并继续' })).toHaveCount(0);

  await page.waitForTimeout(5_000);
  expect((await api.issuePipeline(issue.id))?.run.status).toBe('paused');

  await page.reload();
  await expect(gateBar).toHaveAttribute('data-kind', 'paused');
  // 工作台待确认栏里不再出现（pending 只含 waiting_gate 与 failed）
  await page.goto('/home');
  await expect(workbenchCard(page, 'confirm', title)).toHaveCount(0);

  await openDetail(page, project.projectId, issue.id);
  await page.getByRole('button', { name: '继续', exact: true }).click();
  await expect(gateBar).toHaveAttribute('data-kind', 'human', { timeout: 30_000 });
  const view = await api.issuePipeline(issue.id);
  expect(view?.run.status).toBe('waiting_gate');
  expect(view?.run.current_stage_key).toBe('requirement');
});

test('刷新后状态恢复：工作台、看板、详情', async ({ page, api, project }) => {
  test.setTimeout(4 * 60_000);
  const title = `刷新恢复 ${Date.now()}`;

  await openWorkbench(page, project);
  await startRequirement(page, project, title);
  const gateCard = workbenchCard(page, 'confirm', title).filter({
    hasText: GATE_LABELS.requirement,
  });
  await expect(gateCard).toBeVisible({ timeout: STAGE_TIMEOUT });

  await page.reload();
  await expect(gateCard).toBeVisible();

  const issue = await api.findIssueByTitle(project.projectId, title);
  await openProject(page, project);
  await page.reload();
  await expect(kanbanProgress(page, issue.id)).toHaveAttribute(
    'data-cells',
    'gate,pending,pending,pending,pending,pending,pending'
  );

  await openDetail(page, project.projectId, issue.id);
  await page.reload();
  await expect(page.getByTestId('gate-bar')).toHaveAttribute('data-kind', 'human');
  await expect(page.getByTestId('pipeline-stepper')).toHaveAttribute('data-current', 'requirement');
});
```

- [ ] **Step 2: 跑**

Run: `pnpm run e2e -- tests/exceptions.spec.ts`
Expected: `5 passed`（约 5～10 分钟，串行）。

- [ ] **Step 3: 暂停后工作台是否仍列出（跑通则跳过）**

「暂停」用例断言暂停后该需求不在「需要你确认」里：契约 §2 的 pending 只含 `waiting_gate` 与 `failed` 运行，`paused` 不在其中。若计划 A 实现把暂停中的运行也列出，以契约为准反馈给计划 A；在它修正前可临时删掉这一条断言，并写进汇报。

- [ ] **Step 4: 轮次口径（跑通则跳过）**

计划 A §「判定」表：累计失败次数 ≥ `max_rounds` 时运行置 `failed`，所以评审阶段应恰有 `max_rounds` 次尝试。若断言失败，以计划 A 的实现为准改断言，并写进汇报。

- [ ] **Step 5: 全量跑一遍 e2e**

Run: `pnpm run e2e`
Expected: shell 1 + golden 1 + exceptions 5 全过（visual 在 macOS 上 skip）。

- [ ] **Step 6: 提交**

```bash
pnpm --filter @vibe/e2e run format >/dev/null
git add packages/e2e/tests/exceptions.spec.ts
git commit -m "$(cat <<'EOF'
测试：端到端异常路径（打回、轮次用尽、用例回流、暂停继续、刷新恢复）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 26: 视觉回归

**Files:**
- Create: `packages/e2e/tests/visual.spec.ts`
- Generated（Linux 上生成后提交）: `packages/e2e/tests/__screenshots__/visual.spec.ts/*-linux.png`

- [ ] **Step 1: 写用例**

Create `packages/e2e/tests/visual.spec.ts`:
```ts
import { expect, test, type Page } from '@playwright/test';
import { VkApi } from '../support/api';
import { createProjectWithRepo, type E2eProject } from '../support/fixtures';

/** 会随时间或运行次数变化的区域：相对时间、问候语与今日统计、版本号、需求编号。 */
function masks(page: Page) {
  return [
    page.getByTestId('relative-time'),
    page.getByTestId('workbench-greeting'),
    page.getByTestId('nav-footer'),
    page.getByTestId('issue-simple-id'),
  ];
}

test.describe('@visual 视觉回归：工作台 / 看板 / 详情 × 亮暗 × 1280/1440', () => {
  test.skip(
    process.platform !== 'linux',
    '基线只在 Linux（CI）上生成与比对：字体渲染跨平台不同（计划 §3 决策 11）'
  );
  test.describe.configure({ mode: 'serial' });

  let project: E2eProject;
  let issueId: string;

  test.beforeAll(async () => {
    test.setTimeout(3 * 60_000);
    const api = new VkApi();
    project = await createProjectWithRepo(api, '视觉回归');
    issueId = await api.startPipelineDirect({
      projectId: project.projectId,
      repoId: project.repoId,
      title: '视觉回归样例需求',
    });
    await api.waitForPipeline(
      issueId,
      (view) =>
        view.run.status === 'waiting_gate' && view.run.current_stage_key === 'requirement',
      90_000
    );
  });

  for (const width of [1280, 1440]) {
    for (const scheme of ['light', 'dark'] as const) {
      test(`三页截图 ${scheme} ${width}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: scheme });

        await page.goto(`/projects/${project.projectId}`);
        await expect(
          page.locator(`[data-testid="kanban-card-pipeline"][data-issue-id="${issueId}"]`)
        ).toBeVisible();
        await expect(page).toHaveScreenshot(`kanban-${scheme}-${width}.png`, {
          mask: masks(page),
        });

        await page.getByTestId('personal-sidebar').getByRole('button', { name: /工作台/ }).click();
        await expect(
          page.getByTestId('workbench-column-confirm').getByTestId('workbench-card')
        ).toHaveCount(1);
        await expect(page).toHaveScreenshot(`workbench-${scheme}-${width}.png`, {
          mask: masks(page),
        });

        await page.goto(`/projects/${project.projectId}/issues/${issueId}/detail`);
        await expect(page.getByTestId('gate-bar')).toHaveAttribute('data-kind', 'human');
        await expect(page.getByTestId('artifact-section').first()).toBeVisible();
        await expect(page).toHaveScreenshot(`detail-${scheme}-${width}.png`, {
          mask: masks(page),
        });
      });
    }
  }
});
```

- [ ] **Step 2: macOS 本地确认会跳过**

Run: `pnpm run e2e -- tests/visual.spec.ts`
Expected（macOS）：`4 skipped`。

- [ ] **Step 3: 首次生成基线（Linux）**

任选其一：
1. Linux 机器上：
   ```bash
   pnpm run e2e:install && cargo build --bin server --features qa-mode
   pnpm --filter @vibe/e2e exec playwright test tests/visual.spec.ts --update-snapshots
   ```
   Expected: `4 passed`，生成 12 张 `packages/e2e/tests/__screenshots__/visual.spec.ts/<页面>-<light|dark>-<1280|1440>-linux.png`。
2. 用 CI（Task 27 完成后）：GitHub Actions → Test → Run workflow，勾选 `update_snapshots`；跑完在该次运行的 Artifacts 下载 `playwright-<run_id>`，把其中 `tests/__screenshots__/` 拷到 `packages/e2e/tests/__screenshots__/`。

- [ ] **Step 4: 目检基线**

逐张打开 12 张图，对照草图：工作台三栏、看板六列与卡片七格、详情步进条 / 页签 / 时间线 / 关卡条；暗色图里文字与背景对比正常、阶段色可辨。有问题回到对应界面任务修，再重新生成。

- [ ] **Step 5: 提交**

```bash
pnpm --filter @vibe/e2e run format >/dev/null
git add packages/e2e/tests/visual.spec.ts packages/e2e/tests/__screenshots__
git commit -m "$(cat <<'EOF'
测试：三页视觉回归（亮暗 × 1280/1440），基线在 Linux 生成

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 27: CI——e2e 任务

**Files:**
- Modify: `.github/workflows/test.yml:13`（`workflow_dispatch`）、`:43-58`（frontend 路径过滤）、文件末尾（新 job）

- [ ] **Step 1: 手动触发参数**

`:13` 的
```yaml
  workflow_dispatch:
```
改为
```yaml
  workflow_dispatch:
    inputs:
      update_snapshots:
        description: '重新生成 e2e 视觉回归基线（在本次运行的产物里下载后提交）'
        type: boolean
        default: false
```

- [ ] **Step 2: 路径过滤**

`frontend:` 过滤列表（`:43` 起）在 `- 'packages/ui/**'` 之后加一行：
```yaml
              - 'packages/e2e/**'
```

- [ ] **Step 3: 新 job**

在文件末尾（`tauri-checks` job 之后）追加：
```yaml

  e2e:
    needs: changes
    if: >-
      always()
      && (needs.changes.outputs.frontend == 'true'
        || needs.changes.outputs.backend == 'true'
        || needs.changes.result == 'skipped')
    runs-on: *runner_label
    timeout-minutes: 60
    env:
      SCCACHE_GHA_ENABLED: "true"
      RUSTC_WRAPPER: "sccache"
    steps:
      - uses: actions/checkout@v6

      - name: Setup Rust and Node
        uses: ./.github/actions/cargo-checks-common-setup
        with:
          toolchain: ${{ env.RUST_TOOLCHAIN }}
          cache-key: e2e-${{ runner.os }}-${{ runner.arch }}-${{ env.RUNNER_LABEL }}
          setup-node: 'true'

      - name: Install dependencies
        run: pnpm install

      - name: Configure git identity (backend commits in worktrees)
        run: |
          git config --global user.name "vk-e2e"
          git config --global user.email "vk-e2e@example.com"

      - name: Build backend (qa-mode)
        run: cargo build --bin server --features qa-mode

      - name: Install Playwright Chromium (bundled, headless)
        run: pnpm --filter @vibe/e2e exec playwright install --with-deps chromium

      - name: Run e2e
        env:
          CI: 'true'
          UPDATE_SNAPSHOTS: ${{ github.event_name == 'workflow_dispatch' && inputs.update_snapshots && 'true' || 'false' }}
        run: |
          if [ "$UPDATE_SNAPSHOTS" = "true" ]; then
            pnpm --filter @vibe/e2e exec playwright test --update-snapshots
          else
            pnpm run e2e
          fi

      - name: Upload Playwright report, traces and screenshots
        if: failure() || (github.event_name == 'workflow_dispatch' && inputs.update_snapshots)
        uses: actions/upload-artifact@v4
        with:
          name: playwright-${{ github.run_id }}
          path: |
            packages/e2e/playwright-report
            packages/e2e/test-results
            packages/e2e/tests/__screenshots__
          retention-days: 14
```

- [ ] **Step 4: 校验 YAML 并提交**

Run:
```bash
node -e "require('fs').readFileSync('.github/workflows/test.yml','utf8')" && python3 -c "import yaml,sys; d=yaml.safe_load(open('.github/workflows/test.yml')); print('e2e' in d['jobs'], d['jobs']['e2e']['steps'][-1]['uses'])"
```
Expected: `True actions/upload-artifact@v4`（若本机没有 PyYAML，用 `pip install pyyaml` 或跳过，推到分支后看 Actions 页面是否解析成功）。

```bash
git add .github/workflows/test.yml
git commit -m "$(cat <<'EOF'
CI：新增 e2e 任务，失败时上传 trace 与截图，可手动重建视觉基线

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 28: 收尾——format / check / lint / e2e / 团队版回归

**Files:** 视检查结果而定（只修检查报出来的问题）

- [ ] **Step 1: 格式化**

Run: `pnpm run format`
Expected: 无报错；`git status` 若有格式化产生的改动，一并在 Step 6 提交。

- [ ] **Step 2: 类型检查与单测**

Run: `pnpm run check 2>&1 | tail -15`
Expected: 各子项 0 error，末尾 `web-core:test` 全过、`e2e:check` 0 error。`backend:check` 里 `crates/remote` 需要私有依赖，本机若失败且与本计划无关，记录后继续（参见 2026-09-17 计划的说明）。

- [ ] **Step 3: Lint（含未引用词条）**

Run:
```bash
pnpm run lint 2>&1 | tail -15
GITHUB_BASE_REF=main ./scripts/check-i18n.sh 2>&1 | tail -8
```
Expected:
- `lint` 通过；`check-unused-i18n-keys` 不报未引用。若报某个 `pipeline.*` / `workbench.*` / `issueDetail.*` key 未引用 → 对照 Task 2 的片段，要么补上引用（按任务里的代码应已全部引用），要么在七种语言里同时删掉它。
- `check-i18n.sh`：字面量数量不多于 main（本计划新增的 JSX 文案都在 web-core / ui，local-web 只新增三个无文案的路由文件）；key 一致。

- [ ] **Step 4: Rust 测试**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED" | tail -10`
Expected: 全部 `ok`。重点确认 `前端用到的本地端点都挂上了路由` 通过（Task 12 改了 `REST_RESOURCE`）。

- [ ] **Step 5: 端到端**

Run: `pnpm run e2e`
Expected: macOS：`7 passed, 4 skipped`；Linux / CI：`11 passed`。

- [ ] **Step 6: 团队版回归（人工，照 §5 表格逐条）**

```bash
VK_MODE=team pnpm run dev
```
逐条确认 §5 的「回归检查」列：左侧仍是 AppBar（Local / Remote / Projects 分组标题为 i18n 文案）、GitHub / Discord 徽标在、看板列头有 `+` 与阶段徽标规则不变、Team/Personal 切换在、面板无「全屏查看」、手输 `/home` `/docs` `/projects/x/issues/y/detail` 被送回首页、设置常规页末尾没有「数据导出」卡片；登录、成员管理、工作区删除审批、第三方账号绑定照常（设计文档 §12.7）。

有修复就提交：
```bash
git add -A
git commit -m "$(cat <<'EOF'
收尾：格式化与检查修正

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 附：对契约的疑问（只记录，不改契约；执行前与计划 A 对齐）

已被契约修订解决、不再是疑问的：快照形状（C1）、i64 类型（C2）、develop checks（C3）、pause / resume 允许状态（C5）、`max_rounds` 口径（计划 A 判定表：累计失败 ≥ max_rounds 即失败，人工打回不计）。仍待确认：

1. **自动判定条件名**：`PipelineTemplateStageView` 没有自动判定条件（`checks_passed` 等），前端只能按阶段 key 写死文案（`stages.ts` 的 `AUTO_CONDITION_KEYS`）。仓库 `.vibe/pipeline.yaml` 改了某阶段的判定条件时，界面文案会不准。建议加 `gate_condition: Option<String>`。
2. **往回拖 / 打回到任意阶段**：设计文档 §8.3 允许在看板往回拖（打回），契约只有「当前关卡 reject」。缺 `POST /api/local/pipeline/runs/{id}/rewind { stage_key, comment }` 之类接口时，本计划在流水线运行中禁止跨列拖拽，已停的流水线往回拖只改列、不改流水线状态（§3 决策 7）。
3. **等待起点**：阶段进入 `waiting_gate` 时不写 `finished_at`（计划 A 测试 `assert!(waiting.finished_at.is_none())`），`PendingPipelineItem` 也没有等待起点字段，工作台「3 分钟前」只能用阶段开始时间近似。建议 item 加 `waiting_since`，或进入 `waiting_gate` 时写 `finished_at`。
4. **路由契约测试覆盖面**：Rust 的「前端用到的本地端点都挂上了路由」只解析 `localEndpoints.ts` 与 `remoteApi.ts`，不看 `entities/pipeline/api/pipelineApi.ts`；契约 C4 之后后端登记的是 `{id}` 形式，建议计划 A 把 `PIPELINE_API_PATHS` 所在文件加进扫描，并把前端 `${issueId}` 这类占位统一映射成 `{id}` 比对。
5. **暂停中的人工关卡能否直接确认**：计划 A 暂停时不改阶段状态（阶段仍是 `waiting_gate`），`gate` 接口只校验阶段状态。界面在暂停时不给「确认并继续」（`gateBarState` 返回 `paused`），但契约没写暂停时 `gate` 是否应 409。建议明确。
6. **测试标记与标题**：C9 说标记从提示词「需求：」行读，引擎把标题压成单行。工作台把输入的第一行作标题（`splitMessageToTitleDescription` 超过 100 字会截断到词边界），e2e 标题都很短，不受影响；若用户把标记写在第二行，它进的是描述而不是标题，模拟器读不到。只影响测试，记录备查。
