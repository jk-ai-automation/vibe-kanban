# vibe-kanban 研发测试一体化优化方案

> 状态：已确认方向，待分阶段实施｜更新日期：2026-09-15
> 关联：AI 测试框架 atp（https://github.com/jk-ai-automation/atp），设计见该仓库 `docs/superpowers/specs/2026-09-15-atp-architecture.md`

## 1. 背景与目标

上游 vibe-kanban 已宣布停止云服务（提交 `97123d52` 把看板改为“仅导出”页面，`9f101503` 在 README 加入停服横幅），本仓库作为自维护分叉继续演进。

目标：把 vibe-kanban 改造成约 5 人团队使用的**研发测试一体化平台**，打通“需求 → 开发（AI 编码）→ 评审 → 测试（AI 测试）→ 发布”全流程。

## 2. 已确认的决策

| 事项 | 决策 |
|---|---|
| 使用模式 | 登录时选择「团队版」（连接团队服务器，成员互相可见项目）或「个人版」（本机单用户，无需服务器），可随时切换 |
| 登录认证 | 团队服务器数据库账号密码 + 飞书 + Lark + Google；不使用 GitHub 登录；暂不集成 LDAP/SAML 企业单点登录 |
| 代码仓库 | 支持 GitHub 与自建 GitLab 等多个不同服务器；登录一次后拉取到本地目录，并具备推送权限 |
| Git 凭据 | 只保存在每个人本机（系统钥匙串），不托管到团队服务器 |
| 大模型 | 统一使用 Claude Code |
| 测试 | 整合自研 AI 测试框架 atp：PRD + spec → 生成用例 → 生成自动化代码 → 联调 → 执行 → 报告；覆盖 API、Web、App 与跨端组合场景 |
| 通知 | 第一阶段只接 Lark 机器人 |

## 3. 现状分析要点

### 3.1 架构
- 本地端：`crates/server` + SQLite（`crates/db`），负责仓库、工作区（git worktree）、会话、执行进程、PR 跟踪，支持 9 种编码智能体、AI 评审、工具调用审批、MCP。本地端自身没有用户体系。
- 远端：`crates/remote` + Postgres + ElectricSQL，负责用户、组织、项目、需求、评论、通知；认证为 GitHub/Google OAuth，外加一个环境变量配置的单账号。

### 3.2 关键发现
1. 看板相关后端接口与前端组件（`KanbanContainer` 1153 行、`KanbanIssuePanelContainer` 1113 行）仍在代码中，只是前端入口被替换，**可以快速恢复**。
2. `crates/remote/src/auth/local.rs:51-56` 用明文比较环境变量里的单个账号，`users` 表没有密码列；但 JWT、刷新令牌、`auth_sessions` 体系完善，可直接复用，且刷新流程对 `provider="local"` 已跳过第三方校验。
3. 开发侧能力强，测试侧空白：没有测试脚本类型、测试结果、CI 状态与合并门禁；需求只有自由状态列，没有需求/缺陷类型。前端只有 1 个单元测试。
4. 前端所有项目与需求数据都经过 `createShapeCollection`（`packages/web-core/src/shared/lib/electric/collections.ts:763`），是个人版与团队版切换数据源的天然接缝。
5. 本地端只能注册已存在的本地目录（`crates/server/src/routes/repo.rs:370`），没有克隆能力；`crates/git-host` 只支持 GitHub（依赖 `gh` 命令行登录）与 Azure，靠 URL 子串识别，不支持 GitLab。
6. 界面支持简体中文，但停服相关新界面是硬编码英文。

## 4. 总体架构

```
┌──────────────────────── 登录页：选择使用模式 ────────────────────────┐
│  ○ 个人版（本机，无需账号）   ○ 团队版（服务器地址 → 账号密码 / 飞书 / Lark / Google） │
└──────────────┬───────────────────────────────────────┬─────────────┘
        个人版  │                                       │ 团队版
┌──────────────▼───────────────┐      ┌────────────────▼──────────────────┐
│ 本地端 + SQLite               │      │ 团队服务器（remote + Postgres）     │
│ 项目/需求/缺陷/用例/文档/报告  │      │ 用户/组织/项目（团队可见）          │
│ 实现与远端相同的接口子集       │      │ 需求/缺陷/用例/文档/报告、Git 服务器配置 │
└──────────────┬───────────────┘      └────────────────┬──────────────────┘
               └─────────── 两种模式共用的本地执行层 ────────┘
      工作区 · 编码智能体 · atp 测试流水线 · 多 Git 服务器凭据（系统钥匙串）
```

设计原则：两种模式共用同一套接口契约与数据类型（`crates/api-types`），前端只切换基础地址与同步方式，业务界面只写一套。

## 5. 认证与模式选择

### 5.1 团队服务器（`crates/remote`）
1. 迁移：`users` 增加 `password_hash`、`password_updated_at`、`must_change_password`、`disabled_at`、`failed_login_count`、`locked_until`；`username` 唯一。
2. 改写 `auth/local.rs`：按邮箱或用户名查询，argon2id 校验；连续失败 5 次锁定 15 分钟；复用现有会话与 JWT 签发。
3. 新增接口：修改密码；管理员用户管理（创建、停用、重置密码、分配角色）；可选注册（默认关闭）。
4. 首个管理员：用户表为空时由 `VK_BOOTSTRAP_ADMIN_EMAIL/PASSWORD` 创建并强制改密；提供命令行重置管理员密码。
5. 第三方登录：保留 Google（`auth/provider.rs:500` 已实现）；新增飞书/Lark 登录提供者，配置区分 `open.feishu.cn` 与 `open.larksuite.com`，两者可同时启用；按邮箱绑定已有用户，管理员可配置是否自动开户、允许的邮箱域名与企业（tenant）。
6. 移除 GitHub OAuth；保留 OAuth 握手（PKCE）流程供本地客户端使用飞书/Lark/Google 登录。
7. 角色：`admin` / `developer` / `tester`；扩展 `organization_members.rs:583-694` 的权限检查（例如只有测试可以标记测试通过）。
8. 项目可见性：`projects.visibility`（`team` / `private`），团队项目成员可见进度、PR 状态与测试报告。
9. 安全基线：HTTPS、`VK_ALLOWED_ORIGINS`、登录限流、登录审计表。

### 5.2 本地端与前端
- 登录页：模式选择、服务器地址、四种登录方式；顶部显示当前模式并可切换。
- 本地端 `crates/server/src/routes/oauth.rs` 保留 `local_login` 代理，新增修改密码代理；`VK_SHARED_API_BASE` 默认值不再指向 `api.vibekanban.com`。
- 个人版发布到团队版：复用 `crates/remote/src/routes/export.rs` 导出格式，新增导入接口。

## 6. 个人版：本地项目存储

1. `crates/db` 新增与远端同构的项目表：`projects`、`project_statuses`、`issues`、`tags`、`issue_tags`、`issue_assignees`、`issue_relationships`、`issue_comments`，以及测试、文档相关表；删除遗留的 `project.rs`/`task.rs`。
2. 本地端新增 `routes/local_projects/*`，路径与 JSON 与远端 `/v1/*` 一致。
3. 前端：`createShapeCollection` 按模式选择 Electric 集合（团队版）或查询集合 + 本地事件流失效（个人版）；`remoteApi.ts` 基础地址按模式切换。
4. 接口契约测试：同一套测试分别跑两个后端，防止实现漂移。
5. 分步交付：先支持项目、需求、缺陷、状态，再补评论、关联关系。

## 7. 多 Git 服务器

### 7.1 数据模型
| 表 | 存放位置 | 内容 |
|---|---|---|
| `git_hosts` | 团队服务器（管理员配置）；个人版存本地 | 类型（github / github_enterprise / gitlab / gitea）、`base_url`、`api_url`、名称 |
| `git_credentials` | 本地 SQLite 存元数据，令牌存系统钥匙串 | host_id、用户名、认证方式（令牌 / SSH 密钥）、是否可推送、校验时间 |
| `project_repos` | 团队服务器 | 项目与仓库地址、默认分支、脚本模板 |
| `repo_bindings` | 本地 | 团队仓库与本机目录的对应关系 |

### 7.2 流程
1. 每个 Git 服务器绑定一次凭据，当场校验推送权限（GitHub 查 `permissions.push`，GitLab 查 `access_level >= 30`）。
2. 项目仓库列表一键「拉取到本地」，默认目录 `~/vibe-repos/<服务器>/<分组>/<仓库>`，完成后自动调用 `register_repo`；显示未拉取 / 已拉取 / 落后提交数 / 可推送 / 只读。
3. Git 操作注入凭据：命令行用 `GIT_ASKPASS` 或单次 `-c http.<url>.extraHeader`，git2 用 `RemoteCallbacks::credentials`；令牌不写入远程地址与 `.git/config`。
4. 只读仓库禁用推送与创建合并请求，仍可本地开发与测试。

### 7.3 `crates/git-host` 改造
- 新增 `gitlab/` 提供者（REST API v4）：创建合并请求、状态、按分支查询、讨论、流水线状态。
- GitHub 改为使用本机令牌调用 REST API，不再要求 `gh auth login`，保留命令行作为后备。
- `detection.rs` 优先匹配已配置 `git_hosts` 的 `base_url`，再回退到现有规则。
- 接口增加 `get_checks`（CI 状态）与 `check_push_permission`。

## 8. 研发测试流程模型

### 8.1 生命周期
```
待规划 → 待开发 → 开发中 → 自测中 → 待评审 → 待测试 → 测试中 → 待发布 → 已完成
                    │         │          │                    │
                 智能体开发  测试脚本    AI+人工评审        atp 测试流水线
                                                         失败 → 自动建缺陷 → 回流原工作区修复
```

### 8.2 数据模型变更
| 位置 | 变更 | 用途 |
|---|---|---|
| 远端 `project_statuses` | 新增 `stage_type`（backlog/todo/dev/self_test/review/qa/release/done） | 列名自由，自动化按语义识别 |
| 远端 `issues` | 新增 `issue_type`（需求/任务/缺陷）、严重程度、发现环境、来源工作区 | 区分需求与缺陷，缺陷可追溯 |
| 新增 | `documents`（PRD/spec，在线编辑或引用仓库路径+提交，带版本） | 测试输入 |
| 新增 | `test_cases`、`test_pipeline_runs`、`test_runs`、`test_results` | 用例库、流水线、执行结果（结构与 atp 的结果模型对齐） |
| 新增 | `quality_gates`（测试通过、AI 评审、人工批准数、CI 通过） | 阶段门禁 |
| 本地 `repos` | 新增 `test_script`、`lint_script` | 自测 |
| 执行器 | 新增 `ScriptContext::TestScript` 与 `ExecutionProcessRunReason::TestScript`，新增测试流水线步骤动作 | 测试成为一等执行过程 |
| 本地 | `pull_request_checks`（由 `pr_monitor.rs` 同周期轮询） | CI 状态上卡片 |

### 8.3 执行链
```
准备脚本 → 编码智能体 → lint → 测试 ─失败─► 自动把失败日志发回智能体修复（最多 N 轮）
                                 └通过─► AI 评审 → 人工评审 → 清理脚本
```

## 9. 与 atp 测试框架的整合

平台负责编排、存储与展示，atp 负责测试执行与 AI 测试能力（基于 Claude Code）。

| 流水线阶段 | 平台动作 | atp 命令 | 平台展示 |
|---|---|---|---|
| 文档解析 | 测试流水线步骤 | `atp parse` | 文档中心：需求点 |
| 用例生成 | 测试流水线步骤 | `atp gen-cases` | 用例库（AI 标记，待审核） |
| 用例审核 | 人工门禁 | — | 测试中心批量审核 |
| 生成自动化代码 | 编码智能体会话（测试仓库工作区） | 智能体调用 `atp lint`、`atp run` | 代码变更、评审 |
| 联调 | 智能体与流水线循环 | `atp run --case`、`atp attribute` | 联调轮次、归因、自动建缺陷 |
| 执行 | 测试流水线步骤 | `atp run` | 测试运行记录、失败详情 |
| 报告 | 测试流水线步骤 | `atp report junit`、`atp notify lark` | 报告页、覆盖矩阵、门禁、Lark 通知 |

对接约定：
- 结果文件：`results.json`（运行编号、环境、用例列表、状态、耗时、失败信息、汇总）与 `junit.xml`。
- 退出码：`0` 全部通过，`1` 有用例失败或出错，`2` 命令参数错误，`3` 框架错误（配置缺失、密钥缺失等）。
- 进度事件：标准输出逐行 JSON（A4 阶段实现），日志写标准错误。
- 测试环境密钥保存在本机钥匙串，平台启动 atp 时以环境变量注入；环境配置中只写 `${变量名}`。
- 被测服务：复用平台开发服务器脚本启动，地址以 `BASE_URL` 传入。
- 定时回归：由 GitHub Actions / GitLab CI 定时运行 atp 镜像，结果上报团队服务器。

## 10. 界面方案

### 10.1 导航（左侧）
| 板块 | 内容 | 复用 |
|---|---|---|
| 我的工作台 | 按角色聚合待办 | 新建 |
| 迭代看板 | 研发测试流程模板列、WIP 限制、泳道 | 恢复 `KanbanContainer`、`KanbanBoard` |
| 需求 / 缺陷 | 列表与筛选 | `IssueListView` |
| 开发 | 现有工作区页面 | `WorkspacesLayout` |
| 评审中心 | 待评审 PR 与 AI 评审结果 | `ChangesPanelContainer`、`ReviewProvider`、`PrCommentsDialog` |
| 测试中心 | 用例库、流水线运行、测试报告、预览环境 | `PreviewBrowserContainer` |
| 文档中心 | PRD / spec 编辑与版本，一键生成用例 | 新建 |
| 度量 | 交付周期、一次通过率、缺陷趋势、智能体成功率、用例采纳率 | 新建 |
| 设置 | 用户与角色、Git 服务器、流程模板、质量门禁、仓库脚本、智能体 | `SettingsDialog` |

### 10.2 关键页面
- 看板卡片：类型、优先级、负责人、智能体运行状态、测试通过率、PR/CI 状态、评审状态；拖拽触发门禁校验。
- 需求详情页签：概述 / 开发记录 / 代码变更 / 测试 / 关联缺陷 / 动态。
- 工作区顶部阶段步进条（开发 → 自测 → 评审 → 提测 → 合并），右侧新增「质量」页签。
- 测试中心：用例按需求分组、AI 标记、批量审核；流水线五阶段步进与实时日志；报告页含失败截图与 trace、覆盖矩阵、趋势，失败项一键提缺陷；「打回开发」把缺陷描述发给原智能体会话。
- 项目页「仓库」页签：Git 服务器、拉取状态、推送权限、一键拉取/更新。

### 10.3 视觉规范
- 阶段语义色：灰=规划、蓝=开发、紫=评审、琥珀=测试、绿=完成、红=阻塞/失败，定义在 `packages/web-core/src/app/styles/new/index.css`；品牌橙只用于主要操作。
- 保持 12px 紧凑密度；提交哈希、测试计数、耗时使用等宽字体。
- 默认简体中文，新增与停服相关的硬编码文案全部走国际化，并由 `lint:i18n` 约束。

## 11. 自动化与闭环

1. 状态自动流转：从需求创建工作区 → 开发中；测试通过且创建 PR → 待评审；评审通过 → 待测试；测试流水线通过 → 待发布；PR 合并 → 已完成。
2. 失败回流：测试失败自动发回智能体；评审意见与 PR 评论一键发回智能体；缺陷一键「用智能体修复」，基于原分支并附带上下文。
3. 工作区启动时自动注入需求描述、验收标准与关联用例。
4. MCP 扩展（`crates/mcp`）：查询用例、上报测试结果、创建缺陷。
5. Lark 机器人通知：阶段变更、门禁失败、被提及。
6. 后续：简单规则引擎（触发 → 条件 → 动作）。

## 12. 清理与去云依赖

- 回滚 `97123d52` 恢复看板；删除 `ProjectSunsetPage`、`CloudShutdownExportBanner` 与 README 停服横幅；`RootRedirectPage` 默认进入「我的工作台」。
- 去除 `api.vibekanban.com` 依赖：`crates/review` 上传地址、计费（`billing`、`organization_billing`）、发布说明拉取；全仓搜索 `vibekanban.com` 硬编码。
- 删除死代码：`/workspaces/electric-test` 调试页、遗留本地 `projects`/`tasks` 表与类型。
- 部署精简为 postgres + electric + remote-server；附件存储由 azurite 改为本地磁盘或 MinIO；中继服务按需保留。

## 13. 实施路线

| 阶段 | vibe-kanban 主线 | atp 测试框架 | 预估 |
|---|---|---|---|
| 第 1～3 周 | P0 分叉基线（去云依赖、自托管、恢复看板）；P1 认证（账号密码、飞书/Lark/Google、用户与角色、模式选择登录页）；P3 多 Git 服务器 | A0 安全止血（已完成）；A1 工程化底座 | 两人并行 |
| 第 4～6 周 | P2 个人版（本地项目存储、集合切换、契约测试、发布到团队） | A2 引擎重构（用例模型、迁移工具、交易所画像、鉴权） | |
| 第 7～10 周 | P4 流程与界面（阶段、门禁、CI 状态、工作台、看板增强） | A3 AI 能力（Claude Code 技能：用例生成、代码生成、联调归因） | |
| 第 11～12 周 | P5 测试流水线整合 | A4 平台对接（事件协议、缺陷回流、报告） | 联调 |
| 第 13～14 周 | P6 自动化与度量 | 持续优化 | |

每个阶段结束执行 `pnpm run format`、`pnpm run check`、`pnpm run lint`、`cargo test --workspace`。

## 14. 依赖测试环境改造的事项（需向后端研发提出）

交易所复杂业务自动化覆盖依赖测试环境提供：标记价格注入（或行情回放）、可控测试时钟、测试账号加款接口、验证码/二次验证白名单。缺少这些时，强平、资金费率结算、交割到期、登录验证等场景无法稳定自动化。
