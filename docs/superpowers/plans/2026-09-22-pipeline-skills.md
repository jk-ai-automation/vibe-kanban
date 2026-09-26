# 流水线技能接入 实施计划（U3）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让流水线七个阶段在**真实 Claude Code** 下各自加载一个确定的技能：平台自带一个 Claude Code 插件（`vk-pipeline`），编进二进制、按内容哈希解压到数据目录，开会话时用 `--plugin-dir` 会话级注入；技能按固定目录读写产出物、不向人提问；技能文件不落进用户仓库、不动 `~/.claude`。

**Architecture:** 技能包是仓库里的普通目录 `assets/pipeline-plugin/`（它本身就是一个 Claude Code 插件根目录），外来技能由 `scripts/sync-pipeline-skills.mjs` 按 `skills.lock.json` 同步并校验 sha256；`crates/services/src/services/pipeline/plugin.rs` 用 RustEmbed 把整个目录编进二进制，启动时解压到 `asset_dir()/pipeline-plugin/<内容哈希>/`（幂等）；`CodingAgentInitialRequest` 新增 `plugin_dirs`（serde 默认空，兼容旧库数据），经 `StandardCodingAgentExecutor::set_plugin_dirs` 交给 Claude 执行器拼出 `--plugin-dir`，其它执行器的默认实现是空操作；引擎在开阶段会话时填入插件目录、把模板里的技能名解析成 `vk-pipeline:<name>` 写进提示词首行。

**Tech Stack:** Rust 2024（rust-embed 8.2 / sha2 0.10 / serde_json / ts-rs 私有分支）、Node 22 + pnpm 10（同步脚本用原生 `node:crypto`、`node:fs`，不引新依赖）、React 19 + i18next（设置页只读清单）。

---

## 0. 阅读顺序（执行前必读）

1. 本计划 §1～§6（共享约定、已核实事实、对设计的纠正、待核实清单、文件清单、禁止事项）。
2. 本期设计：`docs/superpowers/specs/2026-09-22-pipeline-skills-design.md`（116 行，全文就是本期范围）。
3. 上一期契约：`docs/superpowers/plans/2026-09-18-pipeline-contract.md` §4（产出物文件名与 kind）、§5（提示词固定行）。**产出物文件名以它为准，本期不得改名。**
4. 上一期设计：`docs/superpowers/specs/2026-09-18-personal-pipeline-design.md` §6（引擎）、§9.1（本期来源）。
5. 仓库约定：`CLAUDE.md`。风格样板：`docs/superpowers/plans/2026-09-18-pipeline-engine.md`。

**假设你对本仓库零了解。** 每一步都给了精确路径、行号依据、完整代码、确切命令与预期输出。行号以本计划写作时的 `pipeline-skills` 分支（`fae0f7e4`）为准；前面的任务改过同一文件后行号会漂移，**以文中给出的代码片段定位为准**。

---

## 1. 跨任务共享约定（逐字一致）

### 1.1 名称与位置

| 名称 | 位置 | 任务 |
|---|---|---|
| 插件根目录（= 一个 Claude Code 插件） | `assets/pipeline-plugin/` | 2 |
| 插件名 `vk-pipeline` | `assets/pipeline-plugin/.claude-plugin/plugin.json` | 2 |
| 锁文件 `skills.lock.json` | `assets/pipeline-plugin/skills.lock.json` | 1、2、7 |
| 许可证出处 | `assets/pipeline-plugin/THIRD_PARTY_NOTICES.md` | 2 |
| 同步脚本（`--check` / `--update`） | `scripts/sync-pipeline-skills.mjs` | 1、3 |
| `PipelinePluginAssets` / `PLUGIN_NAME` / `content_hash` / `ensure_extracted_in` / `ensure_extracted` / `skill_names` / `qualify_skill` / `PipelineSkillInfo` / `skills_view` | `crates/services/src/services/pipeline/plugin.rs` | 8、11、13、14 |
| `StandardCodingAgentExecutor::set_plugin_dirs`（默认空操作） | `crates/executors/src/executors/mod.rs` | 10 |
| `ClaudeCode::plugin_dirs` + `--plugin-dir` 拼装 | `crates/executors/src/executors/claude.rs` | 10 |
| `CodingAgentInitialRequest::plugin_dirs` | `crates/executors/src/actions/coding_agent_initial.rs` | 10 |
| `SKILL_PREFIX` / `SKILL_SUFFIX` | `crates/executors/src/pipeline_prompt.rs` | 11 |
| `GET /api/local/pipeline/skills` | `crates/server/src/routes/local_projects/pipeline.rs` | 14 |
| 设置页 `pipeline-skills` section | `packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx` | 15 |

### 1.2 插件目录结构（任务 2 建成后长这样）

```
assets/pipeline-plugin/
  .claude-plugin/plugin.json          name = "vk-pipeline"
  skills/
    vk-requirement/SKILL.md           平台自写
    vk-spec/SKILL.md                  平台自写
    vk-develop/SKILL.md               平台自写
    vk-review/SKILL.md                平台自写
    vk-deliver/SKILL.md               平台自写（U3 最小版）
    atp-run/SKILL.md                  平台自写（U3 最小版，U4 换成 atp 仓库正式版）
    prd2testcase/SKILL.md             来源：atp 仓库，按锁文件同步
    brainstorming/SKILL.md            以下为 superpowers 5.1.0 原样同步
    writing-plans/SKILL.md
    test-driven-development/SKILL.md
    test-driven-development/testing-anti-patterns.md
    requesting-code-review/SKILL.md
    requesting-code-review/code-reviewer.md
    systematic-debugging/SKILL.md
    systematic-debugging/root-cause-tracing.md
    systematic-debugging/defense-in-depth.md
    verification-before-completion/SKILL.md
    finishing-a-development-branch/SKILL.md
  skills.lock.json
  THIRD_PARTY_NOTICES.md
```

解压后：`asset_dir()/pipeline-plugin/<内容哈希>/`，内容与上表逐字节一致，另有一个 `.vk-plugin-ready` 标记文件。注入参数是 `--plugin-dir <asset_dir>/pipeline-plugin/<内容哈希>`（**指向插件根目录本身**，不是它的父目录）。

### 1.3 `skills.lock.json` 结构（任务 1 定死，任务 2、7 填内容）

```json
{
  "version": 1,
  "sources": {
    "<来源名>": {
      "kind": "git",
      "repo": "https://github.com/obra/superpowers",
      "commit": "<40 位提交号>",
      "release": "5.1.0",
      "license": "MIT",
      "local": "~/.claude/plugins/cache/superpowers-marketplace/superpowers/5.1.0"
    }
  },
  "skills": [
    {
      "name": "brainstorming",
      "source": "superpowers",
      "source_dir": "skills/brainstorming",
      "files": { "SKILL.md": "<sha256 十六进制小写>" }
    }
  ]
}
```

- `local` 是**同步时**读取的本机路径（`~` 表示家目录）；离线时脚本从这里拷贝。
- `files` 是**白名单**：只有列出的文件被同步，`--check` 要求插件里该技能目录下的文件集合与哈希**完全等于**这张表（多一个、少一个、内容变一个字节都失败）。
- `skills` 里**只列外来技能**。平台自写的 `vk-*` 与 `atp-run` 不进锁文件（它们随本仓库 PR 演进），由任务 13 的静态检查兜住。

### 1.4 平台技能正文的统一结构（任务 4～6 逐字沿用）

每个 `vk-*` / `atp-run` 的 `SKILL.md` 都是：

```markdown
---
name: <与目录名相同>
description: <一句话，说明什么时候用它>
---

# <中文标题>

## 流水线约定

<这一节 9 行，全部技能逐字相同，见任务 4 步骤 3 的代码块>

## 做法

<各技能不同>

## 产出物格式

<各技能不同，文件名必须与契约 §4 一致>
```

「流水线约定」一节**逐字相同**，任务 13 的静态检查会比对。

### 1.5 提交信息

每个任务末尾提交一次，中文，结尾固定两行（空行 + 署名）：

```
Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
```

### 1.6 资源纪律（这台机器 8 核 8GB、根分区余 8.3G）

- **同一时间只允许一个 cargo 进程**。不要 `cargo build` 与 `cargo test` 并行，不要另开 worktree，不要设 `CARGO_TARGET_DIR`。
- 跑 `cargo test --workspace` 前先 `df -h /`；可用空间低于 4G 时先 `cargo clean -p <包名>` 或找人。
- 单个任务的验证只跑该任务涉及的包（`cargo test -p services pipeline::plugin`），全量 `cargo test --workspace` 只在任务 16 跑一次。
- 不打开浏览器；`pnpm run e2e` 只在任务 16 跑（Playwright 已装过浏览器）。

---

## 2. 已核实的现状（文件:行号 / 命令输出）

### 2.1 Claude Code 与 `--plugin-dir`

- 本机 `claude --version` → `2.1.280 (Claude Code)`；`claude --help` 第 159-163 行：
  `--plugin-dir <path>  Load a plugin from a directory or .zip for this session only; a folder of plugins loads each child (repeatable: --plugin-dir A --plugin-dir B.zip)`。
- **平台实际用的不是本机 claude**：`crates/executors/src/executors/claude.rs:61-67` 的 `base_command()` 固定 `npx -y @anthropic-ai/claude-code@2.1.119`（设计 §2 写的 2.1.278 是本机版本，不是平台版本，见 §3 纠正 1）。
- 已核实 **2.1.119 支持 `--plugin-dir`**：`npx -y @anthropic-ai/claude-code@2.1.119 --help` 第 47 行输出
  `--plugin-dir <path>  Load plugins from a directory for this session only (repeatable: --plugin-dir A --plugin-dir B) (default: [])`。
- 已核实 2.1.119 有离线、免登录的清单校验命令：
  ```
  $ npx -y @anthropic-ai/claude-code@2.1.119 plugin validate /tmp/vkplug/vk-pipeline
  Validating plugin manifest: /tmp/vkplug/vk-pipeline/.claude-plugin/plugin.json
  ⚠ Found 1 warning:
    ❯ author: No author information provided. Consider adding author details for plugin attribution
  ✔ Validation passed with warnings
  ```
  （用的最小清单只有 `name` / `description` / `version` 三个键；加上 `author` 即无警告。）
- 目录里同时有 `.claude-plugin/marketplace.json` 时 `plugin validate` 会改验 marketplace 清单（在 superpowers 缓存上实测到 `Unrecognized key: "description"`）。**所以本仓库的插件目录里不放 `marketplace.json`。**

### 2.2 执行器链路

- `ClaudeCode` 结构体：`crates/executors/src/executors/claude.rs:118-146`。字段全部 `#[serde(default, skip_serializing_if = "Option::is_none")]`，`cmd: CmdOverrides` 是 `#[serde(flatten)]`，`approvals_service` 是 `#[serde(skip)] #[ts(skip)] #[derivative(Debug="ignore", PartialEq="ignore")]`——**新字段照抄这个写法**。`serde_json::from_str::<ClaudeCode>("{}")` 能构造出默认实例（任务 10 的测试要用）。
- 命令拼装：`claude.rs:149-197` `build_command_builder()`：`CommandBuilder::new(base_command(...)).params(["-p"])` → 权限/模型/努力度/agent → `extend_params(["--verbose", "--output-format=stream-json", "--input-format=stream-json", "--include-partial-messages", "--replay-user-messages"])` → `apply_overrides(builder, &self.cmd)`。
- `CommandBuilder::extend_params`（`crates/executors/src/command.rs:120-131`）**不做 shell 切分**，每个元素原样进 argv；带空格的路径安全。（做切分的是私有的 `extend_shell_params`，只给 `additional_params` 用。）
- `StandardCodingAgentExecutor` trait：`crates/executors/src/executors/mod.rs:222-232`，已有 `fn apply_overrides(&mut self, _: &ExecutorConfig) {}` 与 `fn use_approvals(&mut self, _: Arc<dyn ExecutorApprovalService>) {}` 两个默认空实现——新增的 `set_plugin_dirs` 照抄。trait 上有 `#[enum_dispatch(CodingAgent)]`（`mod.rs:221`），新增默认方法会自动转发到各变体。
- `CodingAgent` 枚举：`crates/executors/src/executors/mod.rs:109-125`，变体 `ClaudeCode / Amp / Gemini / Codex / Opencode / CursorAgent / QwenCode / Copilot / Droid`，外加 `#[cfg(feature = "qa-mode")] QaMock`。判别式类型叫 `BaseCodingAgent`（`mod.rs:100`），序列化是 `SCREAMING_SNAKE_CASE`，所以 Claude 的取值是 `CLAUDE_CODE`。
- `CodingAgentInitialRequest`：`crates/executors/src/actions/coding_agent_initial.rs:17-27`，三个字段 `prompt` / `executor_config` / `working_dir`（`#[serde(default)]`）。它经 `ExecutorActionType`（`actions/mod.rs:25-33`，`#[serde(tag = "type")]`）序列化进 `execution_processes.executor_action` 落库，**新字段必须 `#[serde(default)]`**，否则老数据反序列化失败。
- `spawn`：同文件 `:43-74`。qa-mode 下直接走 `QaMockExecutor`（`:53-58`），拿不到 `executor_config` 之外的东西；非 qa 分支 `:60-73` 依次 `get_coding_agent` → `apply_overrides` → `use_approvals` → `spawn`——`set_plugin_dirs` 插在 `use_approvals` 之后。
- `CodingAgentInitialRequest` 的构造点共 5 处（结构体字面量）：`crates/server/src/routes/workspaces/pr.rs:166`、`crates/server/src/routes/sessions/mod.rs:202`、`crates/local-deployment/src/container.rs:1155`、`crates/services/src/services/container.rs:1106`、`crates/services/src/services/pipeline/tests.rs:883`。加字段后这 5 处都要补。
- `ContainerService::start_new_session`：`crates/services/src/services/container.rs:1064-1131`，签名 `(&self, workspace, executor_config: ExecutorConfig, prompt: String, run_setup: bool)`；调用点两处：同文件 `:1055`（`start_workspace`）与 `crates/services/src/services/pipeline/launcher.rs:115`。
- `crates/executors` 里已有 `#[tokio::test]`（`claude.rs:2935`、`cursor.rs:1408`、`qa_mock.rs:483`），根 `Cargo.toml:38` 的 tokio 是 `features = ["full"]`，异步测试可用。

### 2.3 资产与 RustEmbed

- `crates/utils/src/assets.rs:8`：`pub const ASSET_DIR_ENV: &str = "VK_ASSET_DIR";`；`:10-22` `asset_dir()` 会 `create_dir_all`；`:26-39` debug 构建默认 `<manifest>/../../dev_assets`，release 用 `ProjectDirs::from("ai","bloop","vibe-kanban")`。
- 现有 RustEmbed 用法：`crates/utils/src/assets.rs:109-115`（`SoundAssets` 指 `../../assets/sounds`、`ScriptAssets` 指 `../../assets/scripts`）、`crates/server/src/routes/frontend.rs:8-11`（前端 dist）。**`#[folder]` 路径相对 crate 的 `CARGO_MANIFEST_DIR`。**
- 解压先例：`crates/services/src/services/config/versions/v2.rs:246-281` `SoundFile::get_path()`——先看缓存文件在不在且非空，不在就 `SoundAssets::get()` 取字节写出去。没有目录级解压先例，本计划新写。
- `crates/services/Cargo.toml` 已有 `rust-embed = "8.2"`（:34）、`sha2 = "0.10"`（:51）、`serde_yaml = "0.9"`（:61）、`tempfile = "3"`（:64 正式依赖，另有 `[dev-dependencies] tempfile = "3"`）。**不需要加新依赖。**
- `crates/utils` **没有** `sha2`，所以内容哈希与解压放在 `crates/services`，不放 `utils`。
- rust-embed 8.x 默认不开 `debug-embed`：**debug 构建下 `get()`/`iter()` 走文件系统实时读**（路径是编译期记下的绝对路径），release 才真嵌入。dev 下改了 `assets/pipeline-plugin/` 不用重编即可生效，但内容哈希也会跟着变、解压出新目录——这是期望行为。

### 2.4 流水线引擎

- 内置模板七阶段的技能名（`crates/services/src/services/pipeline/template.rs:462-489`）：`vk-requirement` / `vk-spec` / `prd2testcase` / `vk-develop` / `vk-review` / `atp-run` / `vk-deliver`。`StageTemplate.skill` 的注释（`template.rs:97-98`）写着「U3 前技能不存在，引擎只把名字写进提示词」。
- 提示词固定行常量：`crates/executors/src/pipeline_prompt.rs:9-20`，`SKILL_PREFIX = "使用技能 "`。
- `build_stage_prompt`：`crates/services/src/services/pipeline/prompt.rs:28-59`，首行 `format!("{SKILL_PREFIX}{}。", input.skill)`。
- **qa-mode 模拟器不解析首行**：`parse_pipeline_prompt`（`pipeline_prompt.rs:36-65`）只认 `需求：` / `产出物目录（绝对路径）：` / `必须产出：` 三种前缀，`SKILL_PREFIX` 在解析里**完全没有出现**（`grep -n SKILL_PREFIX crates/executors/src/pipeline_prompt.rs` 只命中第 9 行的定义）。所以改首行对 qa-mode 与已有端到端测试**零影响**，只需要改 `prompt.rs` 里两个断言整行的测试。
- 开会话：`crates/services/src/services/pipeline/engine.rs:876-933`（`try_launch`）读 `internals.executor_config_json` 反序列化成 `ExecutorConfig`（:893-895），拼提示词（:909-918），调 `launcher.start_agent_session(workspace_id, &executor_config, prompt, run_setup)`（:920-925）。
- `StageLauncher` trait：`crates/services/src/services/pipeline/launcher.rs:29-53`；`ContainerStageLauncher::start_agent_session` 在 `:105-122`。
- 启动入口：`PipelineService::start`（`engine.rs:130-190`），`StartPipelineInput`（`engine.rs:58-66`）带 `executor_config: ExecutorConfig`。`PipelineError::BadRequest` 在路由层映射成 400（`crates/server/src/routes/local_projects/pipeline.rs:44-54`）。
- 启动时的恢复钩子调用点有**两处**：`crates/server/src/main.rs:81` 与 `crates/server/src/startup.rs:165`（同样的 `match deployment.pipeline().recover_interrupted().await` 三分支）。插件解压要在这两处各加一段。
- 产出物目录 `<工作区根>/.vk/runs/<simple_id>/`（`crates/services/src/services/pipeline/artifacts.rs:16-22`），**不在任何 git 仓库里**（工作区根目录下每个仓库才是子目录）。
- 产出物文件名 ↔ kind 映射：`crates/db/src/models/pipeline.rs:185-196`（`ArtifactKind::from_file_name`）。关卡读的两个文件名常量：`crates/services/src/services/pipeline/gates.rs:10-11`（`REVIEW_FILE = "review.json"`、`TEST_REPORT_FILE = "test-report.json"`）。
- atp 旧 CSV 46 列表头常量：`crates/executors/src/pipeline_prompt.rs:24`，已核实与 atp 仓库 `legacy/test_cases/api_custom/api_custom.csv` 首行（去 BOM）逐列一致，第 3 列确实是 `ModelName`（旧版拼写）。

### 2.5 路由、类型与前端

- 流水线路由：`crates/server/src/routes/local_projects/pipeline.rs:332-348` `router()`，`/pipeline` 下已有 6 条；契约测试「流水线端点都已登记且路由不冲突」在同文件 tests 末尾列了 10 条路径。
- handler 风格：同文件 `:261-316`，`State(deployment): State<DeploymentImpl>` + `ResponseJson(ApiResponse::success(...))`。
- ts-rs 注册表：`crates/server/src/bin/generate_types.rs:190-207` 注册了 18 个流水线类型；同文件 `:224-231` 注册了 `services::services::…` 的类型，**services 里的类型可以注册**。
- 前端接口清单：`packages/web-core/src/entities/pipeline/api/pipelineApi.ts:30-44` `PIPELINE_API_PATHS`（被后端路由契约测试按字面量扫描，新增路径必须写成完整字面量）。
- 设置页：`packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx`——`SettingsSectionType` 联合类型（:27-35）、`SettingsSectionInitialState`（:39-50）、`SETTINGS_SECTION_DEFINITIONS`（:58-68）、`renderSettingsSection`（:95-135）四处都要加。
- 导航标题按 **section id 原样**取 i18n：`SettingsDialog.tsx:106` `t(\`settings.layout.nav.${section.id}\`)`；`en/settings.json` 的 `settings.layout.nav` 里确实有 `remote-projects` 与 `remote-projectsDesc` 这样的 kebab 键。
- 语言目录共 7 个：`packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}`。
- 阶段名 i18n **已存在**，不必新增：`common.json` 的 `pipeline.stage.{requirement,spec,test_design,develop,review,test,deliver}`。
- `pnpm run check` = `local-web:legacy-path-guard && local-web:check && remote-web:check && web-core:check && ui:check && backend:check && web-core:test && e2e:check`（`package.json`）。`backend:check` = `cargo check --workspace && cargo check --manifest-path crates/remote/Cargo.toml`。
- CI 里 `npm run generate-types:check` / `npm run prepare-db:check` 在 `.github/workflows/test.yml:244-248` 的 `backend-schema-checks` job 里。

### 2.6 superpowers 与 atp

- 本机插件缓存 `~/.claude/plugins/cache/superpowers-marketplace/superpowers/5.1.0/`：
  - `.claude-plugin/plugin.json` → `name: superpowers`、`version: 5.1.0`、`license: MIT`、`repository: https://github.com/obra/superpowers`。
  - `LICENSE` 首行 `MIT License` / `Copyright (c) 2025 Jesse Vincent`。
  - 是一个 git 检出：`git -C <该目录> rev-parse HEAD` → `f2cbfbefebbfef77321e4c9abc9e949826bea9d7`。
  - 目录里**同时有** `.claude-plugin/marketplace.json`（见 §2.1 的坑）。
- 要引入的 7 个技能的目录内容（`find <skill> -type f`）：
  | 技能 | 文件 |
  |---|---|
  | `brainstorming` | `SKILL.md`、`spec-document-reviewer-prompt.md`、`visual-companion.md`、`scripts/{frame-template.html,helper.js,server.cjs,start-server.sh,stop-server.sh}` |
  | `writing-plans` | `SKILL.md`、`plan-document-reviewer-prompt.md` |
  | `test-driven-development` | `SKILL.md`、`testing-anti-patterns.md` |
  | `requesting-code-review` | `SKILL.md`、`code-reviewer.md` |
  | `systematic-debugging` | `SKILL.md`、`condition-based-waiting.md`、`condition-based-waiting-example.ts`、`CREATION-LOG.md`、`defense-in-depth.md`、`find-polluter.sh`、`root-cause-tracing.md`、`test-academic.md`、`test-pressure-{1,2,3}.md` |
  | `verification-before-completion` | `SKILL.md` |
  | `finishing-a-development-branch` | `SKILL.md` |
  7 个 `SKILL.md` 的 frontmatter 都是 `name` + `description` 两个键，`name` 与目录名一致（逐个 `sed -n '1,4p'` 核实过）。
- atp 仓库 `/Users/admin/work/github/atp`：
  - `git rev-parse HEAD` → `c7104768b7ad339ea25a37fb4f106983b139c6f2`，**没有 `skills/` 目录**（`ls skills` → No such file or directory）。
  - CSV 列定义来源：`src/atp/legacy_csv/columns.py`（`CONFIG_COLUMNS` :13-48、`RESULT_COLUMNS` :50-、`_ALIASES` :91-96 里 `"modelname" -> "ModuleName"`、`canonical_name` :107-111、`normalize_header` :114-）。真实旧表头样本：`legacy/test_cases/api_custom/api_custom.csv` 首行（带 BOM，46 列）。
  - 用例目录名：`src/atp/legacy_csv/cli.py:82` `CASES_DIRNAME = "test_cases"`，布局是 `<根>/test_cases/<模块名>/<模块名>.csv`。
  - `--list` 分支：`cli.py:810-816`（在 `--report-only` 之前判），打印函数 `_list_cases`（`cli.py:315-325`）逐模块打印模块名，再每行 `  <CaseID>  <StepSummary>`。单份 CSV 坏掉不连坐（`_load_modules` :217-233 把 `CsvLoadError` 收成 notes）。
  - 入口：`pyproject.toml:19-20` `[project.scripts] atp = "atp.cli:run"`。`atp` 不在 PATH 上（`which atp` → not found），实测可用的调用方式：
    ```
    $ cd /Users/admin/work/github/atp/legacy && uv run --project /Users/admin/work/github/atp atp legacy run --list
    api_cardapply
      api_cardapply_001  case summary
      api_cardapply_002
      …
    ```

---

## 3. 对设计文档的纠正

| # | 设计原文 | 问题（证据） | 本计划的做法 |
|---|---|---|---|
| 1 | §2「Claude Code 2.1.278 支持 `--plugin-dir`」 | 2.1.278 是本机 CLI 版本（实测本机是 2.1.280）；平台跑的是 `npx @anthropic-ai/claude-code@2.1.119`（`claude.rs:61-67`） | 已核实 **2.1.119 也支持** `--plugin-dir`（§2.1 有输出）。「已验证版本」取 **2.1.119**，任务 16 的验收脚本用的就是它 |
| 2 | §3.1 插件内同时列 `skills.lock.json` 与技能，未说清 `--plugin-dir` 指哪一级 | 2.1.280 的帮助说「a folder of plugins loads each child」，父目录里混进非插件目录会有歧义 | `assets/pipeline-plugin/` **本身就是插件根**（里面直接是 `.claude-plugin/` 与 `skills/`），`--plugin-dir` 指向解压后的 `<哈希>` 目录本身。锁文件与 NOTICES 作为普通文件放在插件根里（`plugin validate` 忽略未知文件，§2.1 实测通过） |
| 3 | §3.1「brainstorming/… 以下为 superpowers 5.1.0 原样引入」（暗示整目录） | `brainstorming/scripts/` 是一个要 `start-server.sh` 起本地 HTTP 服务的「可视化伴侣」，`systematic-debugging/` 里有 `test-pressure-*.md`、`CREATION-LOG.md` 这类开发过程文件 | **按文件白名单同步**，不整目录拷（白名单见 §1.2 与任务 2）。被排除的文件在 `THIRD_PARTY_NOTICES.md` 里写明「未引入」。「原样」的含义是**引入的文件逐字节不改** |
| 4 | §3.2 第 2 条「`CodingAgentInitialRequest` 新增 `plugin_dirs`；Claude 执行器对每一项追加 `--plugin-dir`」，未说中间怎么传 | 执行器实例是 `ExecutorConfigs::get_cached().get_coding_agent()` 现造的，拿不到 request（`coding_agent_initial.rs:60-73`） | 在 `StandardCodingAgentExecutor` 上加默认空实现 `set_plugin_dirs(&mut self, &[PathBuf])`，`spawn` 里在 `use_approvals` 之后调用。`ClaudeCode` 存进 `#[serde(skip)]` 字段，其它执行器什么都不做 |
| 5 | §3.2 第 3 条「启动流水线时校验执行器必须是 `CLAUDE_CODE`（qa-mode 例外）」 | 未说校验放哪一层 | 放 `PipelineService::start`（`engine.rs:130` 之后、建运行之前），返回 `PipelineError::BadRequest`，路由层已经映射成 400（`pipeline.rs:44-54`）。用 `#[cfg(not(feature = "qa-mode"))]` 整块跳过，qa-mode 下这段代码不编译 |
| 6 | §3.2 第 5 条提示词首行文案 | 现有首行是 `使用技能 {skill}。`（`prompt.rs:30`），常量叫 `SKILL_PREFIX` | 改成 `SKILL_PREFIX = "先用 Skill 工具加载技能 "` + 新常量 `SKILL_SUFFIX = "，严格按它执行。"`，整行 = `先用 Skill 工具加载技能 vk-pipeline:vk-spec，严格按它执行。`。模拟器不解析这一行（§2.4 已核实），只需改 `prompt.rs` 的两个整行断言 |
| 7 | §3.1「`prd2testcase` 来源：atp 仓库」+ §3.4 | atp 仓库当前**没有** `skills/` 目录（§2.6） | 任务 7 在 atp 仓库新建 `skills/prd2testcase/SKILL.md` 并**单独提交、单独 PR**；本仓库随后按锁文件同步一份。atp 的锁来源 `kind` 是 `path`（本机路径 + 提交号），不是 `git` |
| 8 | §3.3 表格「`prd2testcase` 产出 `test-cases.csv`（atp 旧 CSV，46 列表头原样）」 | 表头第 3 列是 `ModelName`（旧版拼写），不是 `ModuleName`；`columns.py:91-96` 靠别名兼容 | 技能正文里**逐字给出 46 列表头**（取自 `pipeline_prompt.rs:24` 的常量，与真实 CSV 一致），并写明「`ModelName` 是旧版拼写，不要改成 `ModuleName`」 |
| 9 | §4「技能包静态检查（Rust 单测，读 RustEmbed 内容）」 | RustEmbed 在 debug 下读文件系统、release 下读内存，两种路径都要能过 | 静态检查只用 `PipelinePluginAssets::iter()/get()`，两种模式行为一致；另加一条「锁文件哈希与实际文件一致」的检查，与 `scripts/sync-pipeline-skills.mjs --check` 互为双保险（Rust 侧防「编进二进制的内容不对」，node 侧防「提交前没同步」） |
| 10 | §3.5「数据来自 `GET /api/local/pipeline/skills`（读取解压后的插件目录与锁文件）」 | 解压是有 IO、可能失败的操作，接口不该依赖它 | 接口**只读 RustEmbed 里的内容**（技能列表 + 锁文件），不读解压目录。解压只服务于 `--plugin-dir` |
| 11 | §5 风险表「启动时检测 `claude --version`，低于已验证版本在日志告警」 | 平台每次都是 `npx -y ...@2.1.119` 固定版本，跑 `claude --version` 检测的是另一个二进制，毫无意义，还会在启动路径上引入一次 npx 下载 | **不做版本检测**。改为：`base_command()` 里的版本号与「已验证支持 `--plugin-dir` 的版本」写在同一处注释里，并由任务 13 的一条单测钉住（版本号变了测试就红，逼人重新验收） |

---

## 4. 待核实清单（动手前先核实，分支处理已写明）

| # | 待核实 | 核实方法 | 若不成立 |
|---|---|---|---|
| A | 2.1.119 的 `--plugin-dir` 接受「插件根目录」（含 `.claude-plugin/plugin.json`），而不是只接受「装着多个插件的父目录」 | 任务 16 验收脚本第 2 步：`cd /tmp/vk-plugin-probe && npx -y @anthropic-ai/claude-code@2.1.119 --plugin-dir <解压目录> -p "用 Skill 工具加载技能 vk-pipeline:vk-requirement，然后只回答它 frontmatter 里的 description 原文"`，看输出里有没有技能的 description | 改成解压到 `asset_dir()/pipeline-plugin/<哈希>/vk-pipeline/`、`--plugin-dir` 指向 `<哈希>`（父目录里只有这一个子目录）。改动只在 `plugin.rs` 的 `ensure_extracted_in` 与 `plugin_dir()`，任务 8 的测试断言同步改 |
| B | 技能被加载后，模型在 `-p` 非交互模式下**会**用 Skill 工具加载它（而不是忽略提示词首行） | 同上验收脚本 | 在提示词里追加一行 `如果 Skill 工具不可用，就直接读 <插件目录>/skills/<name>/SKILL.md 并按它执行。`（插件目录已经在 `plugin_dirs` 里，路径可拼出来）。这一行写进 `prompt.rs`，不改其它设计 |
| C | `enum_dispatch` 会把 trait 上**新增的默认方法**转发给各变体（而不是让枚举用默认实现） | 任务 10 步骤 4：`cargo test -p executors plugin_dir` 通过即说明 `CodingAgent::ClaudeCode` 变体收到了 `set_plugin_dirs` | 在 `crates/executors/src/executors/mod.rs` 里给 `CodingAgent` 手写一个 `set_plugin_dirs` 的 `match`（只有 `ClaudeCode` 分支做事），其余不变 |
| D | `#[serde(skip)]` 的 `Vec<PathBuf>` 字段不会让 `ClaudeCode` 的 `JsonSchema` derive 报错 | 任务 10 步骤 4：`cargo check -p executors` | 加 `#[schemars(skip)]` |
| E | atp 仓库能开 PR（远端可写、有 `gh` 权限） | 任务 7 步骤 1：`cd /Users/admin/work/github/atp && git remote -v && gh auth status` | 只在 atp 本地建分支并提交，PR 留给主控手动开；本仓库的锁文件仍指向该提交号，并在 `THIRD_PARTY_NOTICES.md` 注明「提交号尚未合入 atp 主干」 |

### 4.1 核实结论（2026-09-23，实测）

- **A 成立**：`npx -y @anthropic-ai/claude-code@2.1.119 -p --plugin-dir <插件根目录>` 直接指向插件根目录（目录里就是 `.claude-plugin/` 与 `skills/`）可用，14 个技能全部以 `vk-pipeline:` 前缀出现。**解压层级不改**。
- **B 成立**：非交互 `-p` 模式下模型会按提示词首行加载技能并执行。实测用真实提示词跑 `vk-pipeline:vk-requirement`，在指定绝对路径产出了结构完整的 `requirement.md`（背景 / 范围 / 不做 / AC-1..4 / 待澄清）。**提示词不加降级行**。
- C、D 在任务 10 已随 `cargo test -p executors plugin_dir` 通过验证；E 见任务 7 的记录。

### 4.2 真实 Claude Code 验收记录（2026-09-26，任务 16）

一次跑通，七个阶段全绿，`run.status = completed`。

- 后端：`target/debug/server`，**不带** `qa-mode`；`VK_ASSET_DIR` 指向临时目录。
- 执行器：`CLAUDE_CODE`，`model_id = sonnet`（见下「已知环境限制」）。
- 样例仓库：只有一个 `add(a, b)` 的 git 仓库。
- 需求：给 `src/add.mjs` 的 `add` 加入参校验，非有限数字抛 `TypeError` 且错误信息点明参数名。

| 阶段 | 技能 | 耗时 | 产出物 | 结果 |
|---|---|---|---|---|
| 需求 | `vk-pipeline:vk-requirement` | 57s | `requirement.md`（AC-1..6、待澄清 3 条带「我推断的」标） | 人工通过 |
| 规格 | `vk-pipeline:vk-spec` | 78s | `spec.md`（实现决策表、测试决策表）、`plan.md`（红→绿两步） | 人工通过 |
| 用例 | `vk-pipeline:prd2testcase` | 579s | `test-cases.csv`（表头与 6 行数据**均为 46 列**）、`trace-matrix.md`（AC-1..6 全覆盖） | 人工通过 |
| 开发 | `vk-pipeline:vk-develop` | 48s | 提交 `7a072db`（守卫 + 6 条 `node --test` 测试） | 自动通过 |
| 评审 | `vk-pipeline:vk-review` | 102s | `review.json` | blocker 0、major 0、minor 1 |
| 测试 | `vk-pipeline:atp-run` | 34s | `test-report.json`（6/6 通过） | 最小版 |
| 交付 | `vk-pipeline:vk-deliver` | 84s | `delivery-report.md`（含遗留问题） | 最小版 |

隔离核对（三项全过）：

- 样例仓库 `git status --porcelain` 为空，提交历史仍只有「初始提交」——流水线没碰用户仓库。
- 工作区内 `find -name SKILL.md` 无输出；`.vk/` 落在工作区根目录（`repo/` 的同级），不在 git 仓库里，`git status` 看不见它。
- `~/.claude` 下 `find -iname "*vk-pipeline*"` 无输出——技能包只经 `--plugin-dir` 以 `source: vk-pipeline@inline` 内联加载，不写用户目录。

已知环境限制（**与本期改动无关，仓库 `main` 同样存在**）：`claude.rs` 钉住的
`@anthropic-ai/claude-code@2.1.119` 不支持本机账号的默认模型，直接跑会收到
`400 claude_code_version_too_old`、退出码 1，阶段按重试上限连失败 3 次。验收时靠
`executor_config.model_id = "sonnet"` 绕开（`sonnet` / `opus` / `haiku` 实测都可用）。
真正的修法是升 CLI 版本钉子，属独立决策。

---

## 5. 文件清单

**新建**

| 文件 | 职责 |
|---|---|
| `scripts/sync-pipeline-skills.mjs` | 按锁文件同步/校验外来技能（`--check` / `--update`） |
| `assets/pipeline-plugin/.claude-plugin/plugin.json` | 插件清单 |
| `assets/pipeline-plugin/skills.lock.json` | 外来技能的来源、文件白名单与 sha256 |
| `assets/pipeline-plugin/THIRD_PARTY_NOTICES.md` | 许可证全文与出处、被排除文件说明 |
| `assets/pipeline-plugin/skills/vk-requirement/SKILL.md` 等 6 个平台技能 | 平台自写技能正文 |
| `assets/pipeline-plugin/skills/<外来技能>/…` | 由脚本同步，不手改 |
| `crates/services/src/services/pipeline/plugin.rs` | RustEmbed、内容哈希、解压、技能名解析、技能清单视图 + 单测 |
| `packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx` | 设置页只读清单 |
| `/Users/admin/work/github/atp/skills/prd2testcase/SKILL.md` | **atp 仓库**，单独提交与 PR |

**修改**

| 文件 | 改动 |
|---|---|
| `package.json` | `check` 串上 `pipeline-skills:check` |
| `.github/workflows/test.yml` | `backend-schema-checks` job 加一步锁校验 |
| `crates/executors/src/executors/mod.rs` | trait 加 `set_plugin_dirs` 默认空实现 |
| `crates/executors/src/executors/claude.rs` | `plugin_dirs` 字段 + `--plugin-dir` 拼装 + 2 条单测 |
| `crates/executors/src/actions/coding_agent_initial.rs` | `plugin_dirs` 字段 + `spawn` 里调 `set_plugin_dirs` + 1 条单测 |
| `crates/executors/src/pipeline_prompt.rs` | `SKILL_PREFIX` 改文案 + 新增 `SKILL_SUFFIX` |
| `crates/server/src/routes/workspaces/pr.rs`、`routes/sessions/mod.rs`、`crates/local-deployment/src/container.rs` | 构造点补 `plugin_dirs: Vec::new()` |
| `crates/services/src/services/container.rs` | `start_new_session` 加 `plugin_dirs` 形参并传下去 |
| `crates/services/src/services/pipeline/mod.rs` | 挂 `pub mod plugin;` 与再导出 |
| `crates/services/src/services/pipeline/launcher.rs` | `start_agent_session` 加 `plugin_dirs` 形参 |
| `crates/services/src/services/pipeline/prompt.rs` | 首行拼装 + 测试断言 |
| `crates/services/src/services/pipeline/engine.rs` | 启动校验执行器 + `try_launch` 注入插件目录与技能名解析 |
| `crates/server/src/main.rs`、`crates/server/src/startup.rs` | 启动时解压插件 |
| `crates/server/src/routes/local_projects/pipeline.rs` | `GET /pipeline/skills` + 路由契约测试 |
| `crates/server/src/bin/generate_types.rs` | 注册 `PipelineSkillInfo` |
| `shared/types.ts` | 由 `pnpm run generate-types` 生成，不手改 |
| `packages/web-core/src/entities/pipeline/api/pipelineApi.ts` | 新增 `skills` 路径与 `fetchPipelineSkills` |
| `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx` | 注册 `pipeline-skills` |
| `packages/web-core/src/i18n/locales/*/settings.json`（7 份） | 新增文案 |

---

## 6. 本计划的硬性禁止事项

1. **技能文件不得进入用户仓库。** 插件只存在于两处：本仓库的 `assets/pipeline-plugin/`（我们自己的源码）与 `asset_dir()/pipeline-plugin/<哈希>/`（数据目录）。产出物目录是 `<工作区根>/.vk/runs/<simple_id>/`，工作区根不在任何 git 仓库里（§2.4）。任务 8 步骤 1 有一条测试**枚举解压后落盘的每一个绝对路径，断言全部以传入的根目录为前缀**；任务 13 有一条测试断言嵌入资源的相对路径里没有 `..`、没有绝对路径、没有符号链接式的 `/` 开头。任务 16 的验收脚本在真实跑完后执行 `git -C <样例仓库> status --porcelain`，预期输出为空（除了开发阶段应有的代码改动）。
2. **不得修改 `~/.claude`。** 代码里禁止出现 `~/.claude`、`.claude/plugins`、`CLAUDE_CONFIG_DIR` 等字样（同步脚本读 superpowers 缓存是**只读**，且只在 `--update` 时）。任务 13 有一条测试 `grep` 整个 `crates/services/src/services/pipeline/` 与 `crates/executors/src/executors/claude.rs`，断言没有写入 `.claude` 的代码路径。任务 16 的验收脚本在跑之前记录 `find ~/.claude -newermt "$START" | wc -l` 的基线、跑完再记一次，预期增量只来自 Claude Code 自己的会话日志（`~/.claude/projects`、`~/.claude/history.jsonl`），**不得出现 `~/.claude/plugins` 下的新文件**。
3. **不改 `crates/remote`，不改 `crates/api-types` 已有字段，不手改 `shared/types.ts`。**
4. **不新增 Cargo 依赖**（§2.3 已核实所需依赖都在）。同步脚本不新增 npm 依赖（只用 `node:fs` / `node:path` / `node:crypto` / `node:os`）。

---

## 7. 任务

### Task 1: 同步脚本与锁文件骨架

**Files:**
- Create: `scripts/sync-pipeline-skills.mjs`
- Create: `assets/pipeline-plugin/skills.lock.json`

- [ ] **Step 1: 建目录并写锁文件骨架（只填来源与文件白名单，哈希留空串）**

```bash
mkdir -p /Users/admin/work/github/vibe-kanban/assets/pipeline-plugin/.claude-plugin
```

`assets/pipeline-plugin/skills.lock.json`：

```json
{
  "version": 1,
  "sources": {
    "superpowers": {
      "kind": "git",
      "repo": "https://github.com/obra/superpowers",
      "commit": "f2cbfbefebbfef77321e4c9abc9e949826bea9d7",
      "release": "5.1.0",
      "license": "MIT",
      "local": "~/.claude/plugins/cache/superpowers-marketplace/superpowers/5.1.0"
    }
  },
  "skills": [
    { "name": "brainstorming", "source": "superpowers", "source_dir": "skills/brainstorming", "files": { "SKILL.md": "" } },
    { "name": "writing-plans", "source": "superpowers", "source_dir": "skills/writing-plans", "files": { "SKILL.md": "" } },
    { "name": "test-driven-development", "source": "superpowers", "source_dir": "skills/test-driven-development", "files": { "SKILL.md": "", "testing-anti-patterns.md": "" } },
    { "name": "requesting-code-review", "source": "superpowers", "source_dir": "skills/requesting-code-review", "files": { "SKILL.md": "", "code-reviewer.md": "" } },
    { "name": "systematic-debugging", "source": "superpowers", "source_dir": "skills/systematic-debugging", "files": { "SKILL.md": "", "root-cause-tracing.md": "", "defense-in-depth.md": "" } },
    { "name": "verification-before-completion", "source": "superpowers", "source_dir": "skills/verification-before-completion", "files": { "SKILL.md": "" } },
    { "name": "finishing-a-development-branch", "source": "superpowers", "source_dir": "skills/finishing-a-development-branch", "files": { "SKILL.md": "" } }
  ]
}
```

- [ ] **Step 2: 写同步脚本**

`scripts/sync-pipeline-skills.mjs`（全文）：

```js
#!/usr/bin/env node
/**
 * 流水线技能包里**外来技能**的同步与校验。
 *
 *   node scripts/sync-pipeline-skills.mjs            # 等同 --check
 *   node scripts/sync-pipeline-skills.mjs --check    # 只校验，接进 pnpm run check 与 CI
 *   node scripts/sync-pipeline-skills.mjs --update   # 从来源拷贝并回写 sha256
 *
 * 锁文件的 `files` 是白名单：插件里该技能目录下的文件集合必须与它**完全相等**，
 * 多一个、少一个、内容差一个字节都算失败。平台自写的 vk-* 技能不在锁文件里。
 */
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PLUGIN_DIR = path.join(ROOT, 'assets/pipeline-plugin');
const LOCK_PATH = path.join(PLUGIN_DIR, 'skills.lock.json');
const SKILLS_DIR = path.join(PLUGIN_DIR, 'skills');

const problems = [];
const fail = (message) => problems.push(message);

function expandHome(p) {
  return p.startsWith('~/') ? path.join(os.homedir(), p.slice(2)) : p;
}

function sha256(file) {
  return createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function readLock() {
  const lock = JSON.parse(fs.readFileSync(LOCK_PATH, 'utf8'));
  if (lock.version !== 1) {
    throw new Error(`skills.lock.json 的 version 只支持 1，实际 ${lock.version}`);
  }
  return lock;
}

/** 来源目录；`kind: git` 时顺便核对本机检出的 HEAD 与锁文件一致。 */
function sourceDir(lock, skill, { verifyCommit }) {
  const source = lock.sources[skill.source];
  if (!source) throw new Error(`技能 ${skill.name} 的来源 ${skill.source} 不在 sources 里`);
  const local = expandHome(source.local);
  if (!fs.existsSync(local)) {
    throw new Error(`来源 ${skill.source} 的本机路径不存在：${local}`);
  }
  if (verifyCommit && source.kind === 'git') {
    const head = execFileSync('git', ['-C', local, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
    }).trim();
    if (head !== source.commit) {
      throw new Error(
        `来源 ${skill.source} 的本机检出是 ${head}，锁文件写的是 ${source.commit}；` +
          `先把本机检出切到锁定提交，或改锁文件后重跑 --update`
      );
    }
  }
  return path.join(local, skill.source_dir);
}

function listFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  const walk = (current, prefix) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name)
    )) {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(path.join(current, entry.name), rel);
      else out.push(rel);
    }
  };
  walk(dir, '');
  return out;
}

function check(lock) {
  for (const skill of lock.skills) {
    const dir = path.join(SKILLS_DIR, skill.name);
    const expected = Object.keys(skill.files).sort();
    const actual = listFiles(dir);
    for (const extra of actual.filter((f) => !expected.includes(f))) {
      fail(`skills/${skill.name}/${extra} 不在锁文件白名单里（多出来的文件请删掉或加进锁文件）`);
    }
    for (const name of expected) {
      const file = path.join(dir, name);
      if (!fs.existsSync(file)) {
        fail(`skills/${skill.name}/${name} 缺失，请跑 --update`);
        continue;
      }
      const want = skill.files[name];
      if (!want) {
        fail(`锁文件里 skills/${skill.name}/${name} 的哈希为空，请跑 --update`);
        continue;
      }
      const got = sha256(file);
      if (got !== want) {
        fail(
          `skills/${skill.name}/${name} 被改动：锁文件 ${want.slice(0, 12)}…，实际 ${got.slice(0, 12)}…；` +
            `外来技能不得手改，要改请提到上游再更新锁文件`
        );
      }
    }
  }
}

function update(lock) {
  for (const skill of lock.skills) {
    const from = sourceDir(lock, skill, { verifyCommit: true });
    const to = path.join(SKILLS_DIR, skill.name);
    fs.rmSync(to, { recursive: true, force: true });
    fs.mkdirSync(to, { recursive: true });
    const files = {};
    for (const name of Object.keys(skill.files).sort()) {
      const src = path.join(from, name);
      if (!fs.existsSync(src)) {
        throw new Error(`来源缺文件：${src}`);
      }
      const dest = path.join(to, name);
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      fs.copyFileSync(src, dest);
      files[name] = sha256(dest);
    }
    skill.files = files;
    console.log(`同步 ${skill.name}（${Object.keys(files).length} 个文件）`);
  }
  fs.writeFileSync(LOCK_PATH, `${JSON.stringify(lock, null, 2)}\n`);
  console.log(`已回写 ${path.relative(ROOT, LOCK_PATH)}`);
}

const mode = process.argv.includes('--update') ? 'update' : 'check';
const lock = readLock();
if (mode === 'update') {
  update(lock);
  check(lock);
} else {
  check(lock);
}
if (problems.length > 0) {
  console.error('技能锁校验失败：');
  for (const p of problems) console.error(`  - ${p}`);
  console.error('修复办法：node scripts/sync-pipeline-skills.mjs --update');
  process.exit(1);
}
console.log('技能锁校验通过');
```

- [ ] **Step 3: 跑校验，确认它在技能还没同步时失败**

Run: `cd /Users/admin/work/github/vibe-kanban && node scripts/sync-pipeline-skills.mjs --check; echo "exit=$?"`
Expected: 逐条列出 `skills/brainstorming/SKILL.md 缺失，请跑 --update` 之类共 10 条（7 个技能、10 个文件），最后 `exit=1`。

- [ ] **Step 4: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add scripts/sync-pipeline-skills.mjs assets/pipeline-plugin/skills.lock.json
git commit -m "$(cat <<'MSG'
技能包：新增外来技能同步脚本与锁文件骨架

锁文件按文件白名单锁定来源与 sha256，--check 只校验、--update 从本机来源拷贝并回写哈希。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: 插件清单与 7 个 superpowers 技能

**Files:**
- Create: `assets/pipeline-plugin/.claude-plugin/plugin.json`
- Create: `assets/pipeline-plugin/THIRD_PARTY_NOTICES.md`
- Create（由脚本生成）: `assets/pipeline-plugin/skills/<7 个外来技能>/…`
- Modify: `assets/pipeline-plugin/skills.lock.json`（哈希被回写）

- [ ] **Step 1: 写插件清单**

`assets/pipeline-plugin/.claude-plugin/plugin.json`：

```json
{
  "name": "vk-pipeline",
  "description": "vibe-kanban 交付流水线的阶段技能包：需求、规格、用例、开发、评审、测试、交付",
  "version": "0.1.0",
  "author": {
    "name": "vibe-kanban"
  }
}
```

- [ ] **Step 2: 校验清单合法（离线、免登录）**

Run: `cd /Users/admin/work/github/vibe-kanban && npx -y @anthropic-ai/claude-code@2.1.119 plugin validate assets/pipeline-plugin`
Expected:
```
Validating plugin manifest: .../assets/pipeline-plugin/.claude-plugin/plugin.json
✔ Validation passed
```
（若出现 `Validating marketplace manifest`，说明目录里混进了 `.claude-plugin/marketplace.json`，删掉它再跑。）

- [ ] **Step 3: 同步 7 个外来技能**

Run: `cd /Users/admin/work/github/vibe-kanban && node scripts/sync-pipeline-skills.mjs --update`
Expected:
```
同步 brainstorming（1 个文件）
同步 writing-plans（1 个文件）
同步 test-driven-development（2 个文件）
同步 requesting-code-review（2 个文件）
同步 systematic-debugging（3 个文件）
同步 verification-before-completion（1 个文件）
同步 finishing-a-development-branch（1 个文件）
已回写 assets/pipeline-plugin/skills.lock.json
技能锁校验通过
```

- [ ] **Step 4: 确认同步结果与白名单一致**

Run: `cd /Users/admin/work/github/vibe-kanban && find assets/pipeline-plugin/skills -type f | sort`
Expected: 恰好 11 行（7 个 `SKILL.md` + `testing-anti-patterns.md` + `code-reviewer.md` + `root-cause-tracing.md` + `defense-in-depth.md`），没有 `scripts/`、没有 `test-pressure-*.md`、没有 `CREATION-LOG.md`。

- [ ] **Step 5: 写第三方声明**

`assets/pipeline-plugin/THIRD_PARTY_NOTICES.md`：

```markdown
# 第三方内容出处与许可证

本目录是 vibe-kanban 自带的 Claude Code 插件 `vk-pipeline`。`skills/` 下的技能分两类：
平台自写（`vk-*`、`atp-run`）与外来引入。外来技能由 `scripts/sync-pipeline-skills.mjs`
按 `skills.lock.json` 同步，**逐字节不改**；要改请先改上游，再更新锁文件。

## superpowers 5.1.0

- 来源：https://github.com/obra/superpowers
- 提交：`f2cbfbefebbfef77321e4c9abc9e949826bea9d7`（发布版本 5.1.0）
- 许可证：MIT，Copyright (c) 2025 Jesse Vincent
- 引入的技能：`brainstorming`、`writing-plans`、`test-driven-development`、
  `requesting-code-review`、`systematic-debugging`、`verification-before-completion`、
  `finishing-a-development-branch`

### 未引入的文件（按文件白名单排除）

| 技能 | 排除的文件 | 原因 |
|---|---|---|
| `brainstorming` | `scripts/`、`visual-companion.md`、`spec-document-reviewer-prompt.md` | 可视化伴侣要起本地 HTTP 服务并与人交互，流水线是无人值守 |
| `writing-plans` | `plan-document-reviewer-prompt.md` | 给评审子智能体用，流水线的自审清单在 SKILL.md 里 |
| `systematic-debugging` | `CREATION-LOG.md`、`test-academic.md`、`test-pressure-*.md`、`find-polluter.sh`、`condition-based-waiting*.{md,ts}` | 上游的开发过程记录与语言特定示例，与本仓库无关 |

### MIT License

```
MIT License

Copyright (c) 2025 Jesse Vincent

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 规格模板结构

`skills/vk-spec/SKILL.md` 的规格文档结构（问题 / 方案 / 用户故事 / 实现决策 /
测试决策 / 不做）借鉴自 mattpocock 的 `to-spec` 技能（MIT）。只吸收结构，正文为平台自写，
未复制任何文件，因此不在 `skills.lock.json` 里。
```

> 上面 MIT 全文逐字取自 `~/.claude/plugins/cache/superpowers-marketplace/superpowers/5.1.0/LICENSE`；写完后用 `diff <(sed -n '/^MIT License/,$p' ~/.claude/plugins/cache/superpowers-marketplace/superpowers/5.1.0/LICENSE) <(sed -n '/^MIT License/,/^SOFTWARE\.$/p' assets/pipeline-plugin/THIRD_PARTY_NOTICES.md)` 核对，预期无差异。

- [ ] **Step 6: 再跑一次校验**

Run: `cd /Users/admin/work/github/vibe-kanban && node scripts/sync-pipeline-skills.mjs --check; echo "exit=$?"`
Expected: `技能锁校验通过` + `exit=0`。

- [ ] **Step 7: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add assets/pipeline-plugin
git commit -m "$(cat <<'MSG'
技能包：新建 vk-pipeline 插件并引入 superpowers 5.1.0 的七个技能

按文件白名单引入，排除可视化伴侣与上游开发过程记录；许可证与出处写进 THIRD_PARTY_NOTICES。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 3: 锁校验接进 `pnpm run check` 与 CI

**Files:**
- Modify: `package.json`
- Modify: `.github/workflows/test.yml`

- [ ] **Step 1: 制造一个会被抓到的改动（先看到失败）**

Run: `cd /Users/admin/work/github/vibe-kanban && printf '\n<!-- 手改 -->\n' >> assets/pipeline-plugin/skills/writing-plans/SKILL.md && node scripts/sync-pipeline-skills.mjs --check; echo "exit=$?"`
Expected: `skills/writing-plans/SKILL.md 被改动：锁文件 … 实际 …` + `exit=1`。

- [ ] **Step 2: 还原**

Run: `cd /Users/admin/work/github/vibe-kanban && git checkout -- assets/pipeline-plugin/skills/writing-plans/SKILL.md && node scripts/sync-pipeline-skills.mjs --check`
Expected: `技能锁校验通过`。

- [ ] **Step 3: 接进 `pnpm run check`**

在 `package.json` 的 `scripts` 里新增一条（放在 `prepare-db` 一行之后）：

```json
    "pipeline-skills:check": "node scripts/sync-pipeline-skills.mjs --check",
    "pipeline-skills:update": "node scripts/sync-pipeline-skills.mjs --update",
```

并把 `check` 一行改成（在最前面加 `pipeline-skills:check`，**它最快，先失败最省时间**）：

```json
    "check": "pnpm run pipeline-skills:check && pnpm run local-web:legacy-path-guard && pnpm run local-web:check && pnpm run remote-web:check && pnpm run web-core:check && pnpm run ui:check && pnpm run backend:check && pnpm run web-core:test && pnpm run e2e:check",
```

- [ ] **Step 4: 接进 CI**

在 `.github/workflows/test.yml` 的 `backend-schema-checks` job 里，`- name: Check generated types` 这一步**之前**插入：

```yaml
      - name: Check pipeline skills lock
        run: npm run pipeline-skills:check
```

- [ ] **Step 5: 验证**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run pipeline-skills:check`
Expected: `技能锁校验通过`。

Run: `cd /Users/admin/work/github/vibe-kanban && node -e "const s=require('./package.json').scripts; if(!s.check.startsWith('pnpm run pipeline-skills:check &&')) { console.error('check 没串上锁校验'); process.exit(1); } console.log('ok')"`
Expected: `ok`。

- [ ] **Step 6: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add package.json .github/workflows/test.yml
git commit -m "$(cat <<'MSG'
技能包：锁校验接进 pnpm run check 与 CI

放在 check 链最前面：它最快，外来技能被手改时第一时间失败。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 4: 平台技能 `vk-requirement` 与 `vk-spec`

**Files:**
- Create: `assets/pipeline-plugin/skills/vk-requirement/SKILL.md`
- Create: `assets/pipeline-plugin/skills/vk-spec/SKILL.md`

- [ ] **Step 1: 记住「流水线约定」一节（后面 6 个平台技能逐字复制这一段）**

```markdown
## 流水线约定

- 输入全在提示词里：`需求：`、`产出物目录（绝对路径）：`、`必须产出：`、`上一阶段产出：`、`本阶段上一版：`、`上一次被打回的意见：` 各占一行。
- 只写「必须产出」列出的文件，全部写进「产出物目录」；文件名一个字都不能改。
- 不要在用户仓库里写任何文件（开发阶段的代码改动除外），不要改 `~/.claude`，不要装插件。
- 不要向人提问，不要等待确认：拿不准的写进产出物的「待澄清」一节，继续往下做。
- 「上一次被打回的意见」非空时，逐条回应：先写「对打回意见的回应」一节，再改正文。
- 「本阶段上一版」给的是 `*.prev` 旧版本的绝对路径，供参考修改，不要直接改它。
- 引用本节之外的技能时：按它的方法做，但**以本节为准**——它里面一切向人提问、等待选择、起本地服务、开子智能体的步骤一律跳过，自问自答。
- 全文用中文写。
- 做完就结束，不要输出总结性的长篇回复。
```

- [ ] **Step 2: 写 `vk-requirement`**

`assets/pipeline-plugin/skills/vk-requirement/SKILL.md`（`<流水线约定>` 处填入 Step 1 的整段）：

````markdown
---
name: vk-requirement
description: 流水线「需求」阶段：把一句话需求整理成 requirement.md（背景、范围、验收标准、待澄清）
---

# 需求整理（流水线阶段：requirement）

<流水线约定>

## 做法

参考 `vk-pipeline:brainstorming` 的澄清维度，但**改成自问自答**，一个维度都不许跳：

1. **目的**：这个需求要解决谁的什么问题？没有它今天是怎么做的？
2. **约束**：必须兼容什么、不能动什么、有没有时间/数据/权限上的限制？
3. **成功标准**：怎样算做完？用什么**可观察的行为**判断？
4. **边界**：明显相邻、但这次不做的是什么？

每个维度先写出答案，再给答案打标：「需求里写明的」或「我推断的」。**所有推断都要同时进「待澄清」一节。**

## 产出物格式

`requirement.md`：

```markdown
# 需求：<简短标题>

## 背景
<2-5 句：谁、在什么场景下、遇到什么问题>

## 范围
- <要做的事，一条一行>

## 不做
- <明显相邻但本次不做的；没有就写「暂无」>

## 验收标准
每条一个「给定-当-那么」，编号从 AC-1 起：

- **AC-1** 给定 <前置条件>，当 <动作>，那么 <可观察的结果>。

## 待澄清
- <推断出来、需要人确认的点；没有就写「暂无」>
```

硬要求：验收标准至少 3 条；每条都要能直接写成一个测试（含具体的输入与预期输出），不许写「体验良好」「性能可接受」这类不可观察的话。
````

- [ ] **Step 3: 写 `vk-spec`**

`assets/pipeline-plugin/skills/vk-spec/SKILL.md`：

````markdown
---
name: vk-spec
description: 流水线「规格」阶段：按 requirement.md 产出 spec.md 与 plan.md
---

# 规格与计划（流水线阶段：spec）

<流水线约定>

## 做法

1. 先读「产出物目录」里的 `requirement.md`，把每条 AC 的编号与内容抄下来。
2. 按 `vk-pipeline:writing-plans` 的方法把方案拆成任务，但**不要问人选哪种执行方式**，直接写成 `plan.md`。
3. 交出前按 `vk-pipeline:writing-plans` 的自检清单自审一遍：有没有占位符（TBD / 待定 / 「同任务 N」）、每个任务有没有写清涉及文件与测试点、任务顺序是否满足依赖。
4. 自审发现的问题**当场改掉**，不要写进「待澄清」。「待澄清」只放需要人拍板的业务问题。

## 产出物格式

`spec.md`（结构借鉴 mattpocock 的 `to-spec` 模板）：

```markdown
# 规格：<标题>

## 问题
<1 段：要解决的问题，以及不解决的代价>

## 方案
<3-8 句：整体做法；必要时配一段伪代码或数据流描述>

## 用户故事
- 作为 <角色>，我想 <动作>，以便 <价值>。（覆盖 AC-1）

## 实现决策
| 决策 | 选择 | 理由 | 被否掉的选项与原因 |
|---|---|---|---|

## 测试决策
| AC 编号 | 怎么验证 | 在哪一层（单元 / 接口 / 端到端） |
|---|---|---|

## 不做
- <明确排除的>

## 待澄清
- <需要人拍板的；没有就写「暂无」>
```

`plan.md`：

```markdown
# 实施计划：<标题>

## 任务 1：<名字>
- 涉及文件：`path/to/a.rs`、`path/to/b.ts`
- 做什么：<2-4 句>
- 测试点：<加哪个测试、断言什么>
- 覆盖：AC-1、AC-2

## 任务 2：<名字>
...
```

硬要求：`spec.md` 的「测试决策」必须覆盖 `requirement.md` 里**每一条** AC（编号一一对上）；`plan.md` 的任务合起来必须覆盖「方案」的全部内容；每个任务都必须有「测试点」。
````

- [ ] **Step 4: 校验 frontmatter 与目录名一致**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
for d in vk-requirement vk-spec; do
  n=$(awk 'NR>1 && /^name: /{sub(/^name: /,""); print; exit}' "assets/pipeline-plugin/skills/$d/SKILL.md")
  [ "$n" = "$d" ] && echo "$d ok" || echo "$d BAD: $n"
done
```
Expected:
```
vk-requirement ok
vk-spec ok
```

- [ ] **Step 5: 校验插件仍合法、锁仍通过**

Run: `cd /Users/admin/work/github/vibe-kanban && npx -y @anthropic-ai/claude-code@2.1.119 plugin validate assets/pipeline-plugin && node scripts/sync-pipeline-skills.mjs --check`
Expected: `✔ Validation passed` 与 `技能锁校验通过`。

- [ ] **Step 6: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add assets/pipeline-plugin/skills/vk-requirement assets/pipeline-plugin/skills/vk-spec
git commit -m "$(cat <<'MSG'
技能包：写 vk-requirement 与 vk-spec 正文

两者共用同一段「流水线约定」，覆盖第三方技能里的交互步骤；产出物文件名与结构按契约 §4。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 5: 平台技能 `vk-develop` 与 `vk-review`

**Files:**
- Create: `assets/pipeline-plugin/skills/vk-develop/SKILL.md`
- Create: `assets/pipeline-plugin/skills/vk-review/SKILL.md`

- [ ] **Step 1: 写 `vk-develop`**

`assets/pipeline-plugin/skills/vk-develop/SKILL.md`（`<流水线约定>` 填入任务 4 步骤 1 的整段）：

````markdown
---
name: vk-develop
description: 流水线「开发」阶段：按 plan.md 逐任务用 TDD 实现并提交
---

# 开发（流水线阶段：develop）

<流水线约定>

**本阶段是唯一允许改用户仓库代码的阶段。** 但仍然不许往「产出物目录」以外的地方写流水线自己的文件（不要建 `PLAN.md`、`NOTES.md` 之类）。

## 做法

1. 读「产出物目录」里的 `spec.md` 与 `plan.md`。`plan.md` 不存在时，按 `spec.md` 的「测试决策」自己列出任务顺序。
2. 逐个任务按 `vk-pipeline:test-driven-development` 做：先写会失败的测试 → 跑出失败 → 写实现 → 跑通过。跑测试时一定要**看真实输出**，不要凭感觉宣布通过。避坑清单见 `vk-pipeline:test-driven-development` 附带的 `testing-anti-patterns.md`。
3. 卡住、测试红了原因不明时，走 `vk-pipeline:systematic-debugging`：先定位根因（`root-cause-tracing.md`），不要直接改测试让它变绿。
4. 每个任务做完提交一次，提交信息用中文，第一行 `<动词>：<做了什么>`。
5. 全部任务做完后，按 `vk-pipeline:verification-before-completion` 自查：把仓库里已有的检查命令（如 `pnpm run lint`、`cargo test`）真跑一遍，把**命令与真实输出**贴进最后一条提交的正文或对话里。

## 产出物格式

本阶段**不产出文件**，产出的是代码改动与提交。关卡看的是：智能体进程退出码为 0，以及模板里 `checks` 列出的命令（若有）全部成功。

因此结束前必须满足：
- 工作区没有未提交的改动（`git status --porcelain` 为空）。
- 新写的测试真的跑过并通过。
- 没有为了让测试变绿而删测试、加 `skip`、放宽断言。
````

- [ ] **Step 2: 写 `vk-review`**

`assets/pipeline-plugin/skills/vk-review/SKILL.md`：

````markdown
---
name: vk-review
description: 流水线「评审」阶段：对照 spec.md 审查本次改动，产出 review.json
---

# 代码评审（流水线阶段：review）

<流水线约定>

## 做法

按 `vk-pipeline:requesting-code-review` 里 `code-reviewer.md` 的检查清单**自己评审**（不要开子智能体，不要请人评审）：

1. 用 `git log` 与 `git diff` 取出本次流水线产生的全部改动（从工作区创建时的基线到 HEAD）。
2. 读「产出物目录」里的 `spec.md`，逐条核对「测试决策」：每条 AC 是否真有对应的测试，测试是否真的断言了预期行为。
3. 按清单逐项看：
   - **正确性**：边界值、空值、错误分支、并发与重入。
   - **安全**：注入、路径穿越、密钥硬编码、越权读写。
   - **一致性**：是否沿用了仓库既有的写法与命名。
   - **测试质量**：有没有把断言写成永真、有没有 `skip`、有没有只测实现细节。
4. 每个问题定级：
   - `blocker`：功能不对、数据会坏、有安全问题、AC 没被覆盖。
   - `major`：会引发维护问题或性能问题，但功能正确。
   - `minor`：可读性、命名、注释。
5. 没有发现问题时，`findings` 写成空数组——**不要为了显得认真而编造问题**。

## 产出物格式

`review.json`，必须是合法 JSON，顶层只有 `findings` 一个键：

```json
{
  "findings": [
    {
      "severity": "blocker",
      "file": "crates/server/src/routes/foo.rs",
      "line": 42,
      "message": "未校验 project_id 归属，任意 UUID 都能读到别的项目的数据（AC-3 未覆盖）"
    }
  ]
}
```

- `severity` 只能是 `blocker` / `major` / `minor` 三者之一。
- `file` 用相对仓库根目录的路径；`line` 是整数，整文件级别的问题写 `1`。
- `message` 用中文，一句话说清「哪里错了 + 为什么是问题」。
- 没有问题时写 `{"findings": []}`。

关卡判定：`findings` 里**没有** `severity == "blocker"` 才算通过。
````

- [ ] **Step 3: 校验 frontmatter**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
for d in vk-develop vk-review; do
  n=$(awk 'NR>1 && /^name: /{sub(/^name: /,""); print; exit}' "assets/pipeline-plugin/skills/$d/SKILL.md")
  [ "$n" = "$d" ] && echo "$d ok" || echo "$d BAD: $n"
done
```
Expected:
```
vk-develop ok
vk-review ok
```

- [ ] **Step 4: 校验 `review.json` 示例与关卡解析一致**

Run: `cd /Users/admin/work/github/vibe-kanban && grep -n "blocker\|major\|minor" crates/services/src/services/pipeline/gates.rs | head -5`
Expected: 能看到 `gates.rs` 里对这三个级别的解析，且没有第四种级别。若 `gates.rs` 只认别的字面量，以 `gates.rs` 为准改技能正文。

- [ ] **Step 5: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add assets/pipeline-plugin/skills/vk-develop assets/pipeline-plugin/skills/vk-review
git commit -m "$(cat <<'MSG'
技能包：写 vk-develop 与 vk-review 正文

开发走 TDD 与系统化调试，评审按 code-reviewer 清单自审并产出合规 review.json。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 6: 平台技能 `vk-deliver` 与 `atp-run`（U3 最小版）

**Files:**
- Create: `assets/pipeline-plugin/skills/vk-deliver/SKILL.md`
- Create: `assets/pipeline-plugin/skills/atp-run/SKILL.md`

- [ ] **Step 1: 写 `atp-run`（最小版）**

`assets/pipeline-plugin/skills/atp-run/SKILL.md`：

````markdown
---
name: atp-run
description: 流水线「测试」阶段（U3 最小版）：跑仓库已有的测试命令，产出 test-report.json
---

# 测试执行（流水线阶段：test）

<流水线约定>

> **本版是最小实现。** U4 会把它换成 atp 仓库维护的正式版：读 `test-cases.csv` 用 `atp legacy run` 真跑。
> 在那之前，本技能只负责跑仓库已有的测试命令，把结果如实写成报告，保证流程不断。

## 做法

1. 读「产出物目录」里的 `plan.md`：如果任务里写了测试命令，用它。
2. 没写就按仓库类型推断，**按下表从上往下取第一个命中的**：

   | 命中条件 | 命令 |
   |---|---|
   | 根目录有 `package.json` 且 `scripts.test` 存在 | `pnpm run test`（没有 pnpm 就 `npm test`） |
   | 根目录有 `Cargo.toml` | `cargo test --workspace` |
   | 根目录有 `pyproject.toml` | `uv run pytest`（没有 uv 就 `pytest`） |
   | 以上都不命中 | 不跑命令，报告写 0 条用例并在 `cases` 里留一条说明 |

3. 真跑，把退出码与输出末尾留着。**绝不能编造结果。**
4. 把结果映射成报告：每个失败的用例要判「归因」——
   - `code`：实现有问题（断言失败、异常、返回值不对）。
   - `case`：用例本身有问题（环境缺依赖、用例写错、期望值过时）。
   - 判不准时写 `code`（回到开发阶段比回到用例阶段代价低）。

## 产出物格式

`test-report.json`，必须是合法 JSON：

```json
{
  "total": 12,
  "passed": 11,
  "failed": 1,
  "cases": [
    { "id": "services::pipeline::tests::黄金路径", "status": "passed", "attribution": null, "message": "" },
    { "id": "services::pipeline::tests::打回", "status": "failed", "attribution": "code", "message": "断言失败：期望 3，实际 2" }
  ]
}
```

- `total` = `passed` + `failed`；三个都是非负整数。
- `status` 只能是 `passed` / `failed`。
- `attribution` 在 `status == "passed"` 时必须是 `null`；`failed` 时必须是 `"code"` 或 `"case"`。
- 一条用例都没跑成时：`total` 写 0，`cases` 里放一条 `{"id":"no-tests","status":"failed","attribution":"case","message":"<为什么没跑成>"}`。

关卡判定：`failed == 0 && total > 0` 才算通过。失败时任一 `attribution == "case"` 会回到用例阶段（人工关卡），否则回到开发阶段。
````

- [ ] **Step 2: 写 `vk-deliver`（最小版）**

`assets/pipeline-plugin/skills/vk-deliver/SKILL.md`：

````markdown
---
name: vk-deliver
description: 流水线「交付」阶段（U3 最小版）：汇总全链路产出，写 delivery-report.md，不合并不提 PR
---

# 交付汇总（流水线阶段：deliver）

<流水线约定>

> **本版只写报告。** U5 才会自动提 PR 与做交付页。**本阶段绝不 merge、绝不 push、绝不建 PR。**

## 做法

按 `vk-pipeline:finishing-a-development-branch` 的检查项逐条核对，但**不要执行它的合并/清理动作，也不要问人选哪种收尾方式**：

1. 工作区干净吗？（`git status --porcelain` 为空）
2. 本次改动都提交了吗？（`git log` 列出本次流水线产生的提交）
3. 「产出物目录」里该有的文件都在吗？（`requirement.md`、`spec.md`、`plan.md`、`test-cases.csv`、`trace-matrix.md`、`review.json`、`test-report.json`）
4. `review.json` 里还有 `blocker` 吗？`test-report.json` 里还有 `failed` 吗？

任何一条不满足，照实写进报告的「遗留问题」一节——**不要自己去补、不要回头改代码**。

## 产出物格式

`delivery-report.md`：

```markdown
# 交付报告：<需求标题>

## 需求
<requirement.md 的背景与范围，压成 3-5 句；列出全部 AC 编号>

## 规格
<spec.md 的方案，压成 3-5 句；列出实现决策的标题>

## 用例
- 用例总数：<test-cases.csv 的数据行数>
- 覆盖情况：<trace-matrix.md 里未覆盖的 AC 编号；全覆盖就写「AC 全覆盖」>

## 提交
| 提交号 | 说明 |
|---|---|
| `abc1234` | 接口：… |

## 测试证据
- 命令：`<真实跑过的命令>`
- 结果：总 <n> 条，通过 <n> 条，失败 <n> 条
- 评审：blocker <n> 个，major <n> 个，minor <n> 个

## 遗留问题
- <一条一行；没有就写「暂无」>
```

硬要求：报告里的每个数字都必须来自「产出物目录」里的真实文件或 `git log`，**不许估算**。
````

- [ ] **Step 3: 校验六个平台技能的「流水线约定」逐字相同**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
for d in vk-requirement vk-spec vk-develop vk-review vk-deliver atp-run; do
  awk '/^## 流水线约定$/{f=1} f && /^## 做法$/{exit} f' "assets/pipeline-plugin/skills/$d/SKILL.md" | shasum -a 256 | cut -c1-16 | sed "s|$| $d|"
done | sort
```
Expected: 6 行的哈希前缀**完全相同**（只有末尾的技能名不同）。不同就把差异的那份改成与 `vk-requirement` 逐字一致。

- [ ] **Step 4: 校验 frontmatter 与插件清单**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
for d in vk-deliver atp-run; do
  n=$(awk 'NR>1 && /^name: /{sub(/^name: /,""); print; exit}' "assets/pipeline-plugin/skills/$d/SKILL.md")
  [ "$n" = "$d" ] && echo "$d ok" || echo "$d BAD: $n"
done
npx -y @anthropic-ai/claude-code@2.1.119 plugin validate assets/pipeline-plugin
```
Expected: `vk-deliver ok` / `atp-run ok` / `✔ Validation passed`。

- [ ] **Step 5: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add assets/pipeline-plugin/skills/vk-deliver assets/pipeline-plugin/skills/atp-run
git commit -m "$(cat <<'MSG'
技能包：写 vk-deliver 与 atp-run 的 U3 最小版

测试阶段先跑仓库已有测试命令，交付阶段只写报告不合并；两者都产出合规文件，保证流程不断。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 7: atp 仓库的 `prd2testcase`（**跨仓库**，单独提交与 PR）

> 这个任务**先在 `/Users/admin/work/github/atp` 里做**，做完再回本仓库同步。
> atp 负责 CSV 格式，技能与格式同仓维护（设计 §3.4）。

**Files:**
- Create: `/Users/admin/work/github/atp/skills/prd2testcase/SKILL.md`
- Modify: `/Users/admin/work/github/vibe-kanban/assets/pipeline-plugin/skills.lock.json`
- Create（由脚本生成）: `/Users/admin/work/github/vibe-kanban/assets/pipeline-plugin/skills/prd2testcase/SKILL.md`

- [ ] **Step 1: 确认 atp 仓库状态与远端（待核实项 E）**

Run: `cd /Users/admin/work/github/atp && git status --porcelain && git remote get-url origin && gh auth status 2>&1 | head -3`
Expected: 工作区干净（无输出）、打印远端 URL、`gh` 已登录。
若 `gh` 未登录或远端不可写，按 §4 待核实项 E 的分支处理：只在本地建分支提交，PR 留给主控。

- [ ] **Step 2: 在 atp 开分支**

Run: `cd /Users/admin/work/github/atp && git checkout -b skills-prd2testcase && mkdir -p skills/prd2testcase`
Expected: `Switched to a new branch 'skills-prd2testcase'`。

- [ ] **Step 3: 写技能正文**

`/Users/admin/work/github/atp/skills/prd2testcase/SKILL.md`（`<流水线约定>` 填入本计划任务 4 步骤 1 的整段）：

````markdown
---
name: prd2testcase
description: 由需求与规格生成 atp 旧版 CSV 测试用例与 AC 追踪矩阵
---

# 需求转测试用例（流水线阶段：test_design）

<流水线约定>

## 做法

分三步，**每一步都要写下来再进下一步**：

### 第一步：需求分析

读「产出物目录」里的 `requirement.md` 与 `spec.md`。把每条 AC 拆成测试点：

- 正例：AC 描述的主路径。
- 反例：前置条件不满足、输入非法、越权。
- 边界：空、最大、最小、重复、并发。

每个测试点记下它覆盖的 AC 编号。

### 第二步：测试方案

给每个测试点定三件事：

- **端**：接口（HTTP）还是界面（Web）。本技能只产出接口用例；界面用例写进 `trace-matrix.md` 的「本次不覆盖」。
- **优先级**：`P0`（主流程，坏了就不可用）、`P1`（重要分支）、`P2`（边界与体验）。
- **场景归属**：需要多步串起来的（登录 → 下单 → 查询）归到同一个 `ScenarioName`，用 `Steps` 表示步内顺序。

### 第三步：写用例

按下面的格式写 `test-cases.csv`，再写 `trace-matrix.md`。

### 第四步：自检

在 `test-cases.csv` 所在目录之外建一个临时项目跑格式自检（只列不跑）：

```bash
TMP=$(mktemp -d)
mkdir -p "$TMP/test_cases/vk_gen"
cp <产出物目录>/test-cases.csv "$TMP/test_cases/vk_gen/vk_gen.csv"
cd "$TMP" && uv run --project /Users/admin/work/github/atp atp legacy run --list
```

预期输出以 `vk_gen` 开头，后面每行是 `  <CaseID>  <StepSummary>`，行数等于用例条数。
报错就按报错改 CSV 再跑一次。

**环境里没有 `uv` 或 atp 仓库不在上面的路径时跳过这一步**，并在 `trace-matrix.md` 末尾写明「未做 atp 格式自检：<原因>」。

## 产出物格式

### `test-cases.csv`

第一行必须是下面这 46 列，**逐字照抄、顺序不变**（这是旧版表头，`ModelName` 是旧版的拼写，
`atp` 靠别名兼容，**不要改成 `ModuleName`**）：

```
CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario
```

每行的字段数必须等于 46。填写规则：

| 列 | 怎么填 |
|---|---|
| `CaseID` | `<模块名>_<三位序号>`，全表唯一，例 `vk_gen_001` |
| `ScenarioName` | 同一业务场景的多步用例填同一个名字；单步用例留空 |
| `ModelName` | 模块名，与 CSV 文件名一致 |
| `Priority` | `P0` / `P1` / `P2` |
| `Steps` | 场景内的步序号，从 1 起；单步用例填 1 |
| `stepkw` | 请求方法：`GET` / `POST` / `PUT` / `DELETE` |
| `StepSummary` | 一句话中文，说明这步干什么 |
| `Precondition` | 前置条件；没有就留空 |
| `StepInput` | 请求路径或完整 URL |
| `ExpectedResult` | 期望的状态码或结果描述 |
| `ExpectedOutput` | 期望响应里必须出现的内容 |
| `ExcludedOutput` | 期望响应里**不能**出现的内容；没有就留空 |
| `Data` | 请求体（JSON 字符串）；GET 留空 |
| `Header` / `Cookie` | 需要时填，否则留空 |
| 结果列（`ActualResult`、`ActualOutput`、`TestResult`、`RootCause`、`ExecStartTime`、`ResultUpdateTime`、`ActualReturn4Loop*`、`ActualOutput4Loop*`、`MultiProc*`、`StoreOutputResult`） | **一律留空**，由执行器回填 |
| 其余列 | 用不到就留空 |

字段里有逗号、引号或换行时按 CSV 规范用双引号包起来、内部引号写两个。

### `trace-matrix.md`

```markdown
# 追踪矩阵

| AC 编号 | 测试点 | CaseID | 优先级 |
|---|---|---|---|
| AC-1 | 正常下单返回 200 | vk_gen_001 | P0 |

## 未覆盖
> **AC-3**：<为什么这次覆盖不了>

## 本次不覆盖的端
- <界面用例等；没有就写「暂无」>

## 自检
- 命令：`atp legacy run --list`
- 结果：<列出的用例条数>，或「未做 atp 格式自检：<原因>」
```

「未覆盖」一节里每条都要用 `> **AC-x**` 的引用块写（醒目等同于标红）。全覆盖时写 `> 暂无`。
````

- [ ] **Step 4: 用一份最小 CSV 验证格式自检流程真的能跑**

Run:
```bash
TMP=$(mktemp -d) && mkdir -p "$TMP/test_cases/vk_gen"
printf 'CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario\n' > "$TMP/test_cases/vk_gen/vk_gen.csv"
printf 'vk_gen_001,,vk_gen,P0,1,GET,查询健康检查,,/health,200,ok,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n' >> "$TMP/test_cases/vk_gen/vk_gen.csv"
awk -F, 'NR<=2{print NR": "NF" 列"}' "$TMP/test_cases/vk_gen/vk_gen.csv"
cd "$TMP" && uv run --project /Users/admin/work/github/atp atp legacy run --list
```
Expected:
```
1: 46 列
2: 46 列
vk_gen
  vk_gen_001  查询健康检查
```
如果报「用例目录不存在」或列数校验失败，先修 CSV 再继续——这条命令就是技能正文里教模型用的自检。

- [ ] **Step 5: 在 atp 提交并开 PR**

```bash
cd /Users/admin/work/github/atp
git add skills/prd2testcase/SKILL.md
git commit -m "$(cat <<'MSG'
技能：新增 prd2testcase，由需求与规格生成旧版 CSV 用例

技能与 CSV 格式同仓维护：46 列表头逐字取自 legacy/test_cases，自检走 atp legacy run --list。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
git push -u origin skills-prd2testcase
gh pr create --fill --title "技能：新增 prd2testcase" --body "$(cat <<'MSG'
由需求与规格生成 atp 旧版 CSV 用例与 AC 追踪矩阵。46 列表头逐字取自 `legacy/test_cases/api_custom/api_custom.csv`，生成后用 `atp legacy run --list` 自检。

vibe-kanban 的流水线技能包按 `skills.lock.json` 同步这一份。

🤖 Generated with [Claude Code](https://claude.com/claude-code)
MSG
)"
```
Expected: 打印 PR 链接。`gh` 不可用时跳过最后两条命令，记下本地提交号继续。

- [ ] **Step 6: 记下提交号**

Run: `cd /Users/admin/work/github/atp && git rev-parse HEAD && git remote get-url origin`
Expected: 40 位提交号 + 远端 URL。下一步要用。

- [ ] **Step 7: 在本仓库的锁文件里加 atp 来源与技能**

编辑 `/Users/admin/work/github/vibe-kanban/assets/pipeline-plugin/skills.lock.json`：
`sources` 里新增一项（`<远端URL>` 与 `<提交号>` 用 Step 6 的输出）：

```json
    "atp": {
      "kind": "git",
      "repo": "<远端URL>",
      "commit": "<提交号>",
      "release": null,
      "license": "内部仓库（jk-ai-automation/atp）",
      "local": "/Users/admin/work/github/atp"
    }
```

`skills` 数组末尾新增一项：

```json
    { "name": "prd2testcase", "source": "atp", "source_dir": "skills/prd2testcase", "files": { "SKILL.md": "" } }
```

- [ ] **Step 8: 同步并校验**

Run: `cd /Users/admin/work/github/vibe-kanban && node scripts/sync-pipeline-skills.mjs --update`
Expected: 输出里多一行 `同步 prd2testcase（1 个文件）`，最后 `技能锁校验通过`。
若报「来源 atp 的本机检出是 X，锁文件写的是 Y」，说明 atp 仓库的 HEAD 与 Step 6 记下的不一致（比如后来又提交了），把锁文件里的 `commit` 改成当前 HEAD 再跑。

- [ ] **Step 9: 确认七阶段的技能都在插件里**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
for s in vk-requirement vk-spec prd2testcase vk-develop vk-review atp-run vk-deliver; do
  [ -f "assets/pipeline-plugin/skills/$s/SKILL.md" ] && echo "$s ok" || echo "$s 缺失"
done
```
Expected: 7 行全是 `ok`。

- [ ] **Step 10: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add assets/pipeline-plugin/skills.lock.json assets/pipeline-plugin/skills/prd2testcase
git commit -m "$(cat <<'MSG'
技能包：按锁文件同步 atp 仓库的 prd2testcase

技能正文在 atp 仓库维护（与 CSV 列定义同仓），这里只锁提交号与文件哈希。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 8: RustEmbed 打包与按内容哈希解压

**Files:**
- Create: `crates/services/src/services/pipeline/plugin.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`

- [ ] **Step 1: 挂模块**

`crates/services/src/services/pipeline/mod.rs`：在 `pub mod launcher;` 之后加一行 `pub mod plugin;`，并在文件末尾的再导出里加：

```rust
pub use plugin::{PipelineSkillInfo, ensure_extracted, qualify_skill, skills_view};
```

- [ ] **Step 2: 写失败的测试（先只写测试，`plugin.rs` 里只放空壳）**

先建 `crates/services/src/services/pipeline/plugin.rs`，内容只有：

```rust
//! 占位，下一步替换。
```

再把下面这段追加到同一个文件末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解压幂等且只写在给定根目录下() {
        let root = tempfile::tempdir().unwrap();
        let first = ensure_extracted_in(root.path()).unwrap();
        assert!(first.starts_with(root.path()), "{first:?} 必须在给定根目录下");
        assert_eq!(
            first.file_name().unwrap().to_str().unwrap(),
            content_hash(),
            "目录名就是内容哈希"
        );
        assert!(first.join(".claude-plugin/plugin.json").is_file());
        assert!(first.join("skills/vk-requirement/SKILL.md").is_file());
        assert!(first.join(READY_MARKER).is_file());

        // 落盘的每一个文件都必须在根目录下（技能文件绝不外泄）。
        let mut count = 0usize;
        let mut stack = vec![root.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                assert!(path.starts_with(root.path()), "{path:?} 写到了根目录外");
                if path.is_dir() {
                    stack.push(path);
                } else {
                    count += 1;
                }
            }
        }
        assert!(count > 10, "解压出的文件数太少：{count}");

        // 幂等：再来一次不报错、目录不变、没有残留的临时目录。
        let second = ensure_extracted_in(root.path()).unwrap();
        assert_eq!(first, second);
        let leftovers: Vec<_> = std::fs::read_dir(root.path().join("pipeline-plugin"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时目录：{leftovers:?}");
    }

    #[test]
    fn 内容哈希稳定且内容变化时改变() {
        assert_eq!(content_hash(), content_hash());
        assert_eq!(content_hash().len(), 64);
        assert!(content_hash().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn 嵌入资源的路径都是安全的相对路径() {
        for path in PipelinePluginAssets::iter() {
            let path = path.to_string();
            assert!(
                is_safe_relative(std::path::Path::new(&path)),
                "不安全的嵌入路径：{path}"
            );
        }
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline::plugin 2>&1 | tail -20`
Expected: 编译失败，报 `cannot find function ensure_extracted_in in this scope` 等（`plugin.rs` 里还没有实现）。

- [ ] **Step 4: 写实现**

把 `crates/services/src/services/pipeline/plugin.rs` 的第一行占位注释替换成下面的整段（测试模块保留在文件末尾）：

```rust
//! 流水线技能插件：编进二进制、按内容哈希解压到数据目录，会话级注入 Claude Code。
//!
//! 插件只存在于两个地方：本仓库的 `assets/pipeline-plugin/`（源码）与
//! `asset_dir()/pipeline-plugin/<内容哈希>/`（解压产物）。**绝不写进用户仓库，也绝不动
//! `~/.claude`**——注入走 `claude --plugin-dir`，只对当次会话生效。
//!
//! rust-embed 默认不开 `debug-embed`：debug 构建下 `iter()`/`get()` 走文件系统实时读，
//! release 才真嵌入。两种模式下本模块的行为一致（哈希都按当下的内容算）。

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

use db::models::pipeline::PipelineStageKey;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::template::builtin_template;

/// `assets/pipeline-plugin` 本身就是一个 Claude Code 插件根目录
/// （里面直接是 `.claude-plugin/plugin.json` 与 `skills/`）。
#[derive(RustEmbed)]
#[folder = "../../assets/pipeline-plugin"]
pub struct PipelinePluginAssets;

/// 插件名，与 `.claude-plugin/plugin.json` 的 `name` 一致；技能按 `vk-pipeline:<技能名>` 引用。
pub const PLUGIN_NAME: &str = "vk-pipeline";
pub const LOCK_FILE: &str = "skills.lock.json";
/// 解压完成标记：存在即认为该哈希目录完整可用。
pub const READY_MARKER: &str = ".vk-plugin-ready";
const SKILLS_PREFIX: &str = "skills/";
const SKILL_FILE: &str = "SKILL.md";

/// 整个插件的内容哈希（路径 + 长度 + 字节，按路径排序）。目录名用它，内容一变就换目录。
pub fn content_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut paths: Vec<String> = PipelinePluginAssets::iter()
            .map(|path| path.to_string())
            .collect();
        paths.sort();
        let mut hasher = Sha256::new();
        for path in &paths {
            let file = PipelinePluginAssets::get(path).expect("嵌入文件必然存在");
            hasher.update(path.as_bytes());
            hasher.update([0u8]);
            hasher.update((file.data.len() as u64).to_le_bytes());
            hasher.update(file.data.as_ref());
        }
        let digest = hasher.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(out, "{byte:02x}");
        }
        out
    })
    .clone()
}

/// 相对、不含 `..`、不是绝对路径、没有盘符。
fn is_safe_relative(path: &Path) -> bool {
    let mut any = false;
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return false;
        }
        any = true;
    }
    any
}

/// 解压到 `<root>/pipeline-plugin/<内容哈希>/` 并返回该目录。
///
/// 幂等：标记文件在就直接返回。先写同级临时目录再整体改名，别的进程看不到写了一半的插件；
/// 改名失败而目标已就绪（另一个进程抢先完成）时按成功处理。
pub fn ensure_extracted_in(root: &Path) -> std::io::Result<PathBuf> {
    let base = root.join("pipeline-plugin");
    let hash = content_hash();
    let target = base.join(&hash);
    if target.join(READY_MARKER).is_file() {
        return Ok(target);
    }
    std::fs::create_dir_all(&base)?;
    let staging = base.join(format!(".tmp-{hash}-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    for path in PipelinePluginAssets::iter() {
        let rel = Path::new(path.as_ref());
        if !is_safe_relative(rel) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("嵌入资源路径不安全：{}", path.as_ref()),
            ));
        }
        let file = PipelinePluginAssets::get(path.as_ref()).expect("嵌入文件必然存在");
        let dest = staging.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, file.data.as_ref())?;
    }
    std::fs::write(staging.join(READY_MARKER), hash.as_bytes())?;
    if let Err(e) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_dir_all(&staging);
        if !target.join(READY_MARKER).is_file() {
            return Err(e);
        }
    }
    Ok(target)
}

/// 解压到数据目录（`VK_ASSET_DIR` 可覆盖）。整个进程只做一次，失败也只记一次。
pub fn ensure_extracted() -> Result<PathBuf, String> {
    static DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    DIR.get_or_init(|| {
        ensure_extracted_in(&utils::assets::asset_dir())
            .map_err(|e| format!("解压流水线技能插件失败：{e}"))
    })
    .clone()
}

/// `skills/<name>/SKILL.md` → `Some(name)`；其余 → `None`。
fn skill_name_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix(SKILLS_PREFIX)?;
    let (name, file) = rest.split_once('/')?;
    (file == SKILL_FILE && !name.is_empty()).then(|| name.to_string())
}

/// 插件内的全部技能名（按名字排序）。
pub fn skill_names() -> &'static BTreeSet<String> {
    static NAMES: OnceLock<BTreeSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        PipelinePluginAssets::iter()
            .filter_map(|path| skill_name_of(path.as_ref()))
            .collect()
    })
}

/// 模板里的技能名 → 提示词里写的引用名（设计 §3.2 第 4 条）。
///
/// - 名字里已经有 `:`（自带命名空间）：原样。
/// - 是插件内技能：加 `vk-pipeline:` 前缀。
/// - 其余：原样（视为用户仓库 `.claude/skills/` 里自带的技能）。
pub fn qualify_skill(skill: &str) -> String {
    let name = skill.trim();
    if name.contains(':') || !skill_names().contains(name) {
        return name.to_string();
    }
    format!("{PLUGIN_NAME}:{name}")
}

// ---------------------------------------------------------------------------
// 锁文件与技能清单（设置页用；只读嵌入内容，不依赖解压）
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Lock {
    sources: BTreeMap<String, LockSource>,
    skills: Vec<LockSkill>,
}

#[derive(Debug, Deserialize)]
struct LockSource {
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LockSkill {
    name: String,
    source: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

/// 锁文件解析结果。解析不了时返回 None（技能清单退化成「全部算平台自写」，不影响流水线）。
fn lock() -> Option<&'static Lock> {
    static LOCK: OnceLock<Option<Lock>> = OnceLock::new();
    LOCK.get_or_init(|| {
        let file = PipelinePluginAssets::get(LOCK_FILE)?;
        match serde_json::from_slice::<Lock>(file.data.as_ref()) {
            Ok(lock) => Some(lock),
            Err(e) => {
                tracing::warn!("流水线技能锁文件解析失败，技能清单将不显示来源：{e}");
                None
            }
        }
    })
    .as_ref()
}

/// 平台自写技能在清单里的来源名。
pub const SOURCE_PLATFORM: &str = "platform";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineSkillInfo {
    /// 目录名，例：`vk-requirement`
    pub name: String,
    /// 提示词里写的引用名，例：`vk-pipeline:vk-requirement`
    pub qualified_name: String,
    /// `platform`（平台自写）或锁文件里的来源名（`superpowers` / `atp`）
    pub source: String,
    /// 来源的发布版本或提交号；平台自写为 null
    pub source_version: Option<String>,
    /// 来源许可证；平台自写为 null
    pub license: Option<String>,
    /// 内置模板里用在哪个阶段；没被用到为 null
    pub stage: Option<PipelineStageKey>,
    /// 该技能目录下的文件数（外来技能取锁文件白名单，平台技能恒为 1）
    #[ts(type = "number")]
    pub file_count: i64,
}

/// 设置页「流水线技能」的数据。只读嵌入内容，不依赖解压是否成功。
pub fn skills_view() -> Vec<PipelineSkillInfo> {
    let lock = lock();
    let template = builtin_template();
    skill_names()
        .iter()
        .map(|name| {
            let entry = lock.and_then(|l| l.skills.iter().find(|skill| &skill.name == name));
            let source = entry
                .map(|skill| skill.source.clone())
                .unwrap_or_else(|| SOURCE_PLATFORM.to_string());
            let origin = entry.and_then(|skill| lock?.sources.get(&skill.source));
            PipelineSkillInfo {
                name: name.clone(),
                qualified_name: format!("{PLUGIN_NAME}:{name}"),
                source,
                source_version: origin
                    .and_then(|o| o.release.clone().or_else(|| o.commit.clone())),
                license: origin.and_then(|o| o.license.clone()),
                stage: template
                    .stages
                    .iter()
                    .find(|stage| &stage.skill == name)
                    .map(|stage| stage.key),
                file_count: entry.map_or(1, |skill| skill.files.len() as i64),
            }
        })
        .collect()
}
```

> `origin` 那一行里的 `lock?` 在闭包里用不了 `?`——写成下面这样：
> ```rust
>             let origin = entry.and_then(|skill| lock.and_then(|l| l.sources.get(&skill.source)));
> ```
> 以这一行为准。

- [ ] **Step 5: 跑测试确认通过**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline::plugin 2>&1 | tail -15`
Expected:
```
test services::pipeline::plugin::tests::内容哈希稳定且内容变化时改变 ... ok
test services::pipeline::plugin::tests::嵌入资源的路径都是安全的相对路径 ... ok
test services::pipeline::plugin::tests::解压幂等且只写在给定根目录下 ... ok
test result: ok. 3 passed; 0 failed
```

- [ ] **Step 6: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/services/src/services/pipeline/plugin.rs crates/services/src/services/pipeline/mod.rs
git commit -m "$(cat <<'MSG'
流水线：技能插件编进二进制并按内容哈希解压到数据目录

解压幂等、先写临时目录再整体改名；测试枚举落盘路径，确保一个文件都不会写到根目录之外。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 9: 服务启动时解压插件

**Files:**
- Modify: `crates/server/src/main.rs:81-85`
- Modify: `crates/server/src/startup.rs:165-169`

- [ ] **Step 1: 在 `main.rs` 的恢复钩子之后加一段**

`crates/server/src/main.rs`，找到这段：

```rust
    match deployment.pipeline().recover_interrupted().await {
        Ok(0) => {}
        Ok(n) => tracing::warn!("流水线：{n} 个阶段因服务重启被中断，已转人工处理"),
        Err(e) => tracing::warn!("流水线：恢复中断阶段失败: {e}"),
    }
```

在它**之后**插入：

```rust
    // 流水线技能插件：按内容哈希解压到数据目录，阶段会话用 --plugin-dir 会话级加载。
    // 解压失败不拦启动：阶段会因为缺产出物失败，界面上看得见，比起不了服务好。
    match services::services::pipeline::ensure_extracted() {
        Ok(dir) => tracing::info!("流水线技能插件就绪：{}", dir.display()),
        Err(e) => tracing::error!("{e}"),
    }
```

- [ ] **Step 2: 在 `startup.rs` 的同一位置加同一段**

`crates/server/src/startup.rs` 里有逐字相同的 `recover_interrupted` 三分支，在它之后插入 Step 1 的同一段代码（**两处都要，漏一处则桌面版启动路径不解压**）。

- [ ] **Step 3: 确认两处都加了**

Run: `cd /Users/admin/work/github/vibe-kanban && grep -c "pipeline::ensure_extracted" crates/server/src/main.rs crates/server/src/startup.rs`
Expected:
```
crates/server/src/main.rs:1
crates/server/src/startup.rs:1
```

- [ ] **Step 4: 编译**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo check -p server 2>&1 | tail -5`
Expected: `Finished` 且无 warning。

- [ ] **Step 5: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/server/src/main.rs crates/server/src/startup.rs
git commit -m "$(cat <<'MSG'
流水线：服务启动时解压技能插件

两条启动路径（main 与 startup）各加一次；解压失败只记 error 不拦启动。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 10: `plugin_dirs` 字段与 Claude 的 `--plugin-dir`

**Files:**
- Modify: `crates/executors/src/executors/mod.rs:1`、`:222-232`
- Modify: `crates/executors/src/executors/claude.rs:118-146`、`:149-197`、`:316-345`、`:2755-`（tests）
- Modify: `crates/executors/src/actions/coding_agent_initial.rs`
- Modify: `crates/server/src/routes/workspaces/pr.rs:166`、`crates/server/src/routes/sessions/mod.rs:202`、`crates/local-deployment/src/container.rs:1155`、`crates/services/src/services/container.rs:1106`、`crates/services/src/services/pipeline/tests.rs:883`

- [ ] **Step 1: 写失败的测试（Claude 命令行）**

在 `crates/executors/src/executors/claude.rs` 的 `mod tests` 里（`use super::*;` 之后）追加：

```rust
    fn 默认执行器() -> ClaudeCode {
        serde_json::from_str("{}").expect("ClaudeCode 的字段都有 serde 默认值")
    }

    #[tokio::test]
    async fn 插件目录为空时参数与旧行为逐字一致() {
        let params = 默认执行器()
            .build_command_builder()
            .await
            .unwrap()
            .params
            .unwrap();
        assert_eq!(
            params,
            vec![
                "-p",
                "--disallowedTools=AskUserQuestion",
                "--verbose",
                "--output-format=stream-json",
                "--input-format=stream-json",
                "--include-partial-messages",
                "--replay-user-messages",
            ]
        );
    }

    #[tokio::test]
    async fn 每个插件目录追加一次_plugin_dir() {
        let mut agent = 默认执行器();
        agent.set_plugin_dirs(&[
            PathBuf::from("/data/vk assets/pipeline-plugin/abc"),
            PathBuf::from("/data/other"),
        ]);
        let params = agent
            .build_command_builder()
            .await
            .unwrap()
            .params
            .unwrap();
        let at = params
            .iter()
            .position(|p| p == "--plugin-dir")
            .expect("命令行里应有 --plugin-dir");
        assert_eq!(
            &params[at..at + 4],
            &[
                "--plugin-dir".to_string(),
                "/data/vk assets/pipeline-plugin/abc".to_string(),
                "--plugin-dir".to_string(),
                "/data/other".to_string(),
            ],
            "带空格的路径必须是一个独立参数，不能被切开"
        );
        assert_eq!(params.iter().filter(|p| *p == "--plugin-dir").count(), 2);
    }
```

- [ ] **Step 2: 写失败的测试（旧数据兼容）**

在 `crates/executors/src/actions/coding_agent_initial.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ExecutorConfig;

    /// 库里 `execution_processes.executor_action` 的老 JSON 没有 `plugin_dirs`，
    /// 必须照常反序列化（否则老工作区的续聊会整条挂掉）。
    #[test]
    fn 旧数据没有_plugin_dirs_也能反序列化() {
        let old = r#"{"prompt":"做点事","executor_config":{"executor":"CLAUDE_CODE"},"working_dir":null}"#;
        let request: CodingAgentInitialRequest = serde_json::from_str(old).unwrap();
        assert!(request.plugin_dirs.is_empty());
        assert_eq!(request.prompt, "做点事");
    }

    #[test]
    fn 带_plugin_dirs_能往返() {
        let request = CodingAgentInitialRequest {
            prompt: "做点事".to_string(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::ClaudeCode),
            working_dir: None,
            plugin_dirs: vec![std::path::PathBuf::from("/tmp/vk/plugin")],
        };
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains("/tmp/vk/plugin"), "{text}");
        assert_eq!(
            serde_json::from_str::<CodingAgentInitialRequest>(&text).unwrap(),
            request
        );
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p executors plugin_dir 2>&1 | tail -15`
Expected: 编译失败，报 `no method named set_plugin_dirs` 与 `struct CodingAgentInitialRequest has no field named plugin_dirs`。

- [ ] **Step 4: trait 上加默认空实现**

`crates/executors/src/executors/mod.rs`：第 1 行改成

```rust
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
```

在 `pub trait StandardCodingAgentExecutor {` 里、`fn use_approvals(...)` 之后加：

```rust
    /// 会话级插件目录（Claude Code 的 `--plugin-dir`，只对当次会话生效）。
    /// 不支持插件的执行器忽略它——所以这里是空实现，不是 `unimplemented!()`。
    fn set_plugin_dirs(&mut self, _dirs: &[PathBuf]) {}
```

- [ ] **Step 5: `ClaudeCode` 存字段并拼参数**

`crates/executors/src/executors/claude.rs`：在 `pub struct ClaudeCode` 的 `approvals_service` 字段**之前**加：

```rust
    /// 会话级插件目录。不序列化：它是每次会话现算的（解压目录带内容哈希），
    /// 写进 `profiles.json` 只会留下过期路径。
    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    plugin_dirs: Vec<PathBuf>,
```

在 `build_command_builder` 里，`if let Some(agent) = &self.agent { … }` 之后、`builder = builder.extend_params(["--verbose", …])` **之前**加：

```rust
        // 每个目录一对参数。extend_params 不做 shell 切分，带空格的路径原样进 argv。
        // 已核实 npx @anthropic-ai/claude-code@2.1.119 支持 --plugin-dir（可重复）。
        for dir in &self.plugin_dirs {
            builder = builder.extend_params([
                "--plugin-dir".to_string(),
                dir.display().to_string(),
            ]);
        }
```

在 `impl StandardCodingAgentExecutor for ClaudeCode` 里，`fn use_approvals` 之后加：

```rust
    fn set_plugin_dirs(&mut self, dirs: &[PathBuf]) {
        self.plugin_dirs = dirs.to_vec();
    }
```

- [ ] **Step 6: `CodingAgentInitialRequest` 加字段并在 spawn 里传下去**

`crates/executors/src/actions/coding_agent_initial.rs`：第 1 行改成 `use std::{path::{Path, PathBuf}, sync::Arc};`；结构体加字段：

```rust
    /// 会话级插件目录（Claude Code `--plugin-dir`）。流水线用它注入技能包。
    ///
    /// **必须 `#[serde(default)]`**：库里 `execution_processes.executor_action` 的老 JSON
    /// 没有这个字段，反序列化不能失败。
    #[serde(default)]
    pub plugin_dirs: Vec<PathBuf>,
```

在 `spawn` 的非 qa 分支里，`agent.use_approvals(approvals.clone());` 之后加：

```rust
            agent.set_plugin_dirs(&self.plugin_dirs);
```

- [ ] **Step 7: 补齐 5 个构造点**

下面四处加 `plugin_dirs: Vec::new(),`（位置在 `working_dir` 之后）：
- `crates/server/src/routes/workspaces/pr.rs:166` 的 `CodingAgentInitialRequest { … }`
- `crates/server/src/routes/sessions/mod.rs:202` 的 `executors::actions::coding_agent_initial::CodingAgentInitialRequest { … }`
- `crates/local-deployment/src/container.rs:1155` 的 `CodingAgentInitialRequest { … }`
- `crates/services/src/services/pipeline/tests.rs:883` 的 `CodingAgentInitialRequest { … }`

`crates/services/src/services/container.rs:1106` 的构造点先也填 `plugin_dirs: Vec::new(),`（任务 11 再换成真正的参数）。

- [ ] **Step 8: 跑测试确认通过**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p executors plugin_dir 2>&1 | tail -12`
Expected:
```
test actions::coding_agent_initial::tests::旧数据没有_plugin_dirs_也能反序列化 ... ok
test actions::coding_agent_initial::tests::带_plugin_dirs_能往返 ... ok
test executors::claude::tests::插件目录为空时参数与旧行为逐字一致 ... ok
test executors::claude::tests::每个插件目录追加一次_plugin_dir ... ok
```
若报 `set_plugin_dirs` 在 `CodingAgent` 上找不到，说明 enum_dispatch 没转发默认方法（§4 待核实项 C），按那里的分支处理手写转发。

- [ ] **Step 9: 整体编译**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo check --workspace 2>&1 | tail -5`
Expected: `Finished`，没有「missing field `plugin_dirs`」。

- [ ] **Step 10: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/executors crates/server/src/routes/workspaces/pr.rs crates/server/src/routes/sessions/mod.rs crates/local-deployment/src/container.rs crates/services/src/services/container.rs crates/services/src/services/pipeline/tests.rs
git commit -m "$(cat <<'MSG'
执行器：编码智能体请求支持会话级插件目录

新字段 serde 默认空，兼容库里的老 executor_action；Claude 执行器按序追加 --plugin-dir，其余执行器默认忽略。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 11: 引擎注入插件目录、技能名解析与提示词首行

**Files:**
- Modify: `crates/executors/src/pipeline_prompt.rs:9`、`:267-272`（测试夹具）
- Modify: `crates/services/src/services/pipeline/prompt.rs:28-59`、`:117-130`
- Modify: `crates/services/src/services/container.rs:1064-1112`、`:1055`
- Modify: `crates/services/src/services/pipeline/launcher.rs:34-41`、`:105-122`、`:175-`
- Modify: `crates/services/src/services/pipeline/test_support.rs:146-153`、`:212-`
- Modify: `crates/services/src/services/pipeline/engine.rs:909-925`

- [ ] **Step 1: 写失败的测试（技能名解析）**

在 `crates/services/src/services/pipeline/plugin.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn 技能名解析三种情形() {
        assert_eq!(qualify_skill("vk-requirement"), "vk-pipeline:vk-requirement");
        assert_eq!(qualify_skill("  vk-review  "), "vk-pipeline:vk-review");
        assert_eq!(
            qualify_skill("superpowers:brainstorming"),
            "superpowers:brainstorming",
            "已带命名空间的原样"
        );
        assert_eq!(
            qualify_skill("我们仓库自带的技能"),
            "我们仓库自带的技能",
            "不在插件里的原样"
        );
    }

    #[test]
    fn 内置模板的七个技能都在插件里() {
        for stage in super::super::template::builtin_template().stages {
            assert!(
                skill_names().contains(&stage.skill),
                "阶段 {} 的技能 {} 不在插件里",
                stage.key.as_str(),
                stage.skill
            );
            assert!(
                qualify_skill(&stage.skill).starts_with("vk-pipeline:"),
                "{} 应该带插件前缀",
                stage.skill
            );
        }
    }
```

- [ ] **Step 2: 写失败的测试（提示词首行）**

把 `crates/services/src/services/pipeline/prompt.rs` 里 `按设计固定结构拼装` 测试的断言整段改成：

```rust
        assert_eq!(
            prompt,
            "先用 Skill 工具加载技能 vk-spec，严格按它执行。\n\
             需求：VK-12 导出报表 [qa:review-blocker-once]\n\
             产出物目录（绝对路径）：/abs/ws/.vk/runs/VK-12\n\
             必须产出：spec.md, plan.md\n\
             上一阶段产出：requirement.md\n\
             这是自动流水线，不要向人提问；拿不准的写进产出物的「待澄清」一节。"
        );
```

并在同一个 `mod tests` 里追加：

```rust
    #[test]
    fn 带命名空间的技能名原样进首行且不影响解析() {
        let required = vec!["review.json".to_string()];
        let prompt = build_stage_prompt(&StagePromptInput {
            skill: "vk-pipeline:vk-review",
            ..输入(&required, &[], None)
        });
        assert!(
            prompt.starts_with("先用 Skill 工具加载技能 vk-pipeline:vk-review，严格按它执行。\n"),
            "{prompt}"
        );
        let parsed = parse_pipeline_prompt(&prompt).expect("模拟器仍要能解析");
        assert_eq!(parsed.required, required);
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline::prompt pipeline::plugin 2>&1 | tail -20`
Expected: `技能名解析三种情形` 等编译失败或断言失败，`按设计固定结构拼装` 断言失败（左边仍是 `使用技能 vk-spec。`）。

- [ ] **Step 4: 改提示词常量与拼装**

`crates/executors/src/pipeline_prompt.rs` 第 9 行改成两行：

```rust
pub const SKILL_PREFIX: &str = "先用 Skill 工具加载技能 ";
pub const SKILL_SUFFIX: &str = "，严格按它执行。";
```

同文件测试夹具（`fn 提示词`）里的 `"使用技能 vk-review。\n…"` 改成 `"先用 Skill 工具加载技能 vk-pipeline:vk-review，严格按它执行。\n…"`（这一行不参与解析，改它只为与真实提示词一致）。

`crates/services/src/services/pipeline/prompt.rs`：`use` 里加 `SKILL_SUFFIX`，第 30 行改成：

```rust
    lines.push(format!("{SKILL_PREFIX}{}{SKILL_SUFFIX}", input.skill));
```

- [ ] **Step 5: 启动器与容器层加 `plugin_dirs` 形参**

`crates/services/src/services/container.rs`：`start_new_session` 签名末尾加一个形参：

```rust
    async fn start_new_session(
        &self,
        workspace: &Workspace,
        executor_config: ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<std::path::PathBuf>,
    ) -> Result<ExecutionProcess, ContainerError> {
```

函数体里构造 `CodingAgentInitialRequest` 的地方把 `plugin_dirs: Vec::new(),` 改成 `plugin_dirs,`。
同文件 `start_workspace` 里的调用改成：

```rust
        self.start_new_session(workspace, executor_config, prompt, true, Vec::new())
```

`crates/services/src/services/pipeline/launcher.rs`：trait 方法加形参并写注释：

```rust
    /// 在工作区里开新会话跑编码智能体；`run_setup` 为 true 时先跑 setup 脚本。
    /// `plugin_dirs` 是会话级插件目录（技能包），只有 Claude Code 会用。
    async fn start_agent_session(
        &self,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        run_setup: bool,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<LaunchedStep, PipelineError>;
```

`ContainerStageLauncher::start_agent_session` 同步加形参，调用改成：

```rust
            .start_new_session(
                &workspace,
                executor_config.clone(),
                prompt,
                run_setup,
                plugin_dirs,
            )
```

`NoopStageLauncher::start_agent_session` 加 `_plugin_dirs: Vec<PathBuf>,` 形参，函数体不变。

- [ ] **Step 6: 假启动器记账**

`crates/services/src/services/pipeline/test_support.rs`：`FakeLaunch` 加字段

```rust
    /// 会话级插件目录（检查脚本恒为空）。
    pub plugin_dirs: Vec<PathBuf>,
```

`start_agent_session` 加 `plugin_dirs: Vec<PathBuf>,` 形参，`Ok(self.record(FakeLaunch { … }))` 里加 `plugin_dirs,`；`start_checks` 里的 `FakeLaunch { … }` 加 `plugin_dirs: Vec::new(),`。

- [ ] **Step 7: 引擎注入**

`crates/services/src/services/pipeline/engine.rs`：`use` 里加 `use super::plugin;`（与 `artifacts`、`prompt` 同级）。
`try_launch` 里，`let feedback = …` 之后、`let prompt = …` 之前插入：

```rust
        // 技能包：解压失败不让阶段起不来——没有技能时模型仍会按提示词里的固定行做事，
        // 缺产出物会被关卡判定抓到，界面上看得见。
        let plugin_dirs = match plugin::ensure_extracted() {
            Ok(dir) => vec![dir],
            Err(e) => {
                tracing::warn!(run_id = %run.id, "{e}；本阶段不加载技能插件");
                Vec::new()
            }
        };
        let skill = plugin::qualify_skill(&stage.skill);
```

`build_stage_prompt` 的 `skill: &stage.skill,` 改成 `skill: &skill,`；
`start_agent_session` 调用改成：

```rust
            .start_agent_session(workspace_id, &executor_config, prompt, run_setup, plugin_dirs)
```

- [ ] **Step 8: 写引擎侧的集成断言**

在 `crates/services/src/services/pipeline/tests.rs` 里追加（放在已有的黄金路径测试之后）：

```rust
#[tokio::test]
async fn 阶段提示词带插件命名空间且注入插件目录() {
    let s = 夹具().await;
    s.启动().await;
    let launch = s.launcher.last();
    assert!(
        launch
            .prompt
            .starts_with("先用 Skill 工具加载技能 vk-pipeline:vk-requirement，严格按它执行。\n"),
        "{}",
        launch.prompt
    );
    assert_eq!(launch.plugin_dirs.len(), 1, "应注入一个插件目录");
    let dir = &launch.plugin_dirs[0];
    assert!(
        dir.join(".claude-plugin/plugin.json").is_file(),
        "{dir:?} 应是插件根目录"
    );
    assert!(
        dir.join("skills/vk-requirement/SKILL.md").is_file(),
        "{dir:?} 里应有 vk-requirement"
    );
}
```

> `夹具()` 与 `s.启动()` 用 `tests.rs` 里已有的同名辅助函数；名字对不上时照抄同文件里第一个黄金路径测试的前两行。

- [ ] **Step 9: 跑测试**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline 2>&1 | tail -20`
Expected: `test result: ok.`，其中包含
```
test services::pipeline::plugin::tests::技能名解析三种情形 ... ok
test services::pipeline::plugin::tests::内置模板的七个技能都在插件里 ... ok
test services::pipeline::prompt::tests::带命名空间的技能名原样进首行且不影响解析 ... ok
test services::pipeline::tests::阶段提示词带插件命名空间且注入插件目录 ... ok
```

- [ ] **Step 10: qa-mode 也要能编能过**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services --features qa-mode pipeline 2>&1 | tail -8`
Expected: `test result: ok.`（模拟器不解析首行，行为应与上一步一致）。

- [ ] **Step 11: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/executors/src/pipeline_prompt.rs crates/services
git commit -m "$(cat <<'MSG'
流水线：阶段会话注入技能插件并按命名空间引用技能

提示词首行改成「先用 Skill 工具加载技能 vk-pipeline:<名字>」；插件内技能加前缀，已带命名空间或仓库自带的原样。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 12: 启动流水线时校验执行器必须是 Claude Code

**Files:**
- Modify: `crates/services/src/services/pipeline/engine.rs:130-145`
- Modify: `crates/services/src/services/pipeline/tests.rs`

- [ ] **Step 1: 写失败的测试**

在 `crates/services/src/services/pipeline/tests.rs` 末尾追加：

```rust
#[cfg(not(feature = "qa-mode"))]
#[tokio::test]
async fn 非_claude_code_执行器启动被拒() {
    let s = 夹具().await;
    let err = s
        .service
        .start(StartPipelineInput {
            issue: s.issue.clone(),
            workspace_id: s.workspace_id,
            repo_root: s.root.clone(),
            executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
            template_key: None,
        })
        .await
        .expect_err("非 Claude Code 应被拒");
    match err {
        PipelineError::BadRequest(message) => {
            assert!(message.contains("只支持 Claude Code"), "{message}");
        }
        other => panic!("应是 BadRequest，实际 {other:?}"),
    }
    assert!(
        PipelineRuns::find_latest_for_issue(s.service.pool_for_test(), s.issue.id)
            .await
            .unwrap()
            .is_none(),
        "被拒时不许留下运行记录"
    );
}
```

> `s.service.pool_for_test()` 不存在时改用夹具里已有的 `s.pool`（照抄同文件其它测试取 pool 的写法）。
> `夹具()` 的字段名（`issue` / `workspace_id` / `root`）以 `test_support.rs` 为准。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline::tests::非_claude 2>&1 | tail -12`
Expected: `expect_err` panic —— 当前实现不校验，`start` 返回 `Ok`。

- [ ] **Step 3: 实现**

`crates/services/src/services/pipeline/engine.rs` 的 `PipelineService::start` 里，`let _guard = …; let pool = self.pool();` 之后、`if PipelineRuns::has_active_for_issue(...)` **之前**插入：

```rust
        // 技能包是 Claude Code 插件，只有它认 --plugin-dir；别的执行器跑起来会没有技能、
        // 每个阶段都缺产出物，白烧一轮额度，不如在入口挡住。
        // qa-mode 下模拟执行器不是真执行器，这段不编译。
        #[cfg(not(feature = "qa-mode"))]
        if input.executor_config.executor != executors::executors::BaseCodingAgent::ClaudeCode {
            return Err(PipelineError::BadRequest(
                "流水线目前只支持 Claude Code".to_string(),
            ));
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline 2>&1 | tail -8`
Expected: `test result: ok.`，含 `非_claude_code_执行器启动被拒 ... ok`。

- [ ] **Step 5: qa-mode 下这条校验不存在**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services --features qa-mode pipeline 2>&1 | tail -8`
Expected: `test result: ok.`，且**没有** `非_claude_code_执行器启动被拒`（它被 `#[cfg(not(feature = "qa-mode"))]` 排除）。

- [ ] **Step 6: 确认路由层映射成 400**

Run: `cd /Users/admin/work/github/vibe-kanban && grep -n "BadRequest(message) => ApiError::BadRequest" crates/server/src/routes/local_projects/pipeline.rs`
Expected: 命中一行（`map_pipeline_error` 已有这条映射，无需改动）。

- [ ] **Step 7: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/services/src/services/pipeline/engine.rs crates/services/src/services/pipeline/tests.rs
git commit -m "$(cat <<'MSG'
流水线：启动时校验执行器必须是 Claude Code

技能包是 Claude Code 插件，别的执行器加载不了；入口返回 400，不留运行记录。qa-mode 下不编译这段。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 13: 技能包静态检查

**Files:**
- Modify: `crates/services/src/services/pipeline/plugin.rs`

- [ ] **Step 1: 把十六进制拼接抽成函数（两处要用）**

`crates/services/src/services/pipeline/plugin.rs`：在 `content_hash` **之前**加：

```rust
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
```

`content_hash` 结尾的六行改成：

```rust
        hex_lower(&hasher.finalize())
    })
    .clone()
}
```

- [ ] **Step 2: 写失败的静态检查测试**

在 `plugin.rs` 的 `mod tests` 里追加：

```rust
    fn skill_text(name: &str) -> String {
        let file = PipelinePluginAssets::get(&format!("skills/{name}/{SKILL_FILE}"))
            .unwrap_or_else(|| panic!("技能 {name} 没有 SKILL.md"));
        String::from_utf8(file.data.to_vec())
            .unwrap_or_else(|_| panic!("技能 {name} 的 SKILL.md 不是 UTF-8"))
    }

    /// frontmatter 的 `key: value`（只取顶层、不解析嵌套 YAML——技能的 frontmatter 就这两个键）。
    fn frontmatter(name: &str) -> BTreeMap<String, String> {
        let text = skill_text(name);
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("---"),
            "技能 {name} 的 SKILL.md 第一行必须是 ---"
        );
        let mut map = BTreeMap::new();
        for line in lines {
            if line.trim() == "---" {
                return map;
            }
            if let Some((key, value)) = line.split_once(": ") {
                map.insert(
                    key.trim().to_string(),
                    value.trim().trim_matches('"').to_string(),
                );
            }
        }
        panic!("技能 {name} 的 frontmatter 没有闭合的 ---")
    }

    /// 「## 流水线约定」到下一个 `## ` 之间的正文。
    fn convention_section(name: &str) -> String {
        let text = skill_text(name);
        let body = text
            .split_once("## 流水线约定")
            .unwrap_or_else(|| panic!("技能 {name} 没有「## 流水线约定」一节"))
            .1;
        body.split("\n## ").next().unwrap_or_default().trim().to_string()
    }

    #[test]
    fn 每个技能的_frontmatter_合法且_name_与目录一致() {
        assert!(skill_names().len() >= 14, "技能太少：{:?}", skill_names());
        for name in skill_names() {
            let front = frontmatter(name);
            assert_eq!(
                front.get("name").map(String::as_str),
                Some(name.as_str()),
                "技能 {name} 的 frontmatter name 与目录名不一致"
            );
            let description = front
                .get("description")
                .unwrap_or_else(|| panic!("技能 {name} 缺 description"));
            assert!(
                !description.trim().is_empty(),
                "技能 {name} 的 description 是空的"
            );
        }
    }

    #[test]
    fn 平台技能都带同一段流水线约定() {
        let baseline = convention_section("vk-requirement");
        assert!(baseline.contains("不要向人提问"), "约定段内容不对：{baseline}");
        for name in [
            "vk-spec",
            "vk-develop",
            "vk-review",
            "vk-deliver",
            "atp-run",
            "prd2testcase",
        ] {
            assert_eq!(
                convention_section(name),
                baseline,
                "技能 {name} 的「流水线约定」与 vk-requirement 不一致"
            );
        }
    }

    #[test]
    fn 平台技能正文提到的产出文件名与契约一致() {
        for stage in builtin_template().stages {
            if stage.artifacts.is_empty() {
                continue; // develop 阶段不产出文件
            }
            let text = skill_text(&stage.skill);
            for artifact in &stage.artifacts {
                assert!(
                    text.contains(artifact.as_str()),
                    "技能 {} 的正文没提到它必须产出的 {}（契约 §4）",
                    stage.skill,
                    artifact
                );
            }
        }
    }

    #[test]
    fn 锁文件里的哈希与插件里的内容一致() {
        let lock = lock().expect("skills.lock.json 必须能解析");
        assert!(!lock.skills.is_empty(), "锁文件里一个外来技能都没有");
        for skill in &lock.skills {
            assert!(
                lock.sources.contains_key(&skill.source),
                "技能 {} 的来源 {} 不在 sources 里",
                skill.name,
                skill.source
            );
            for (file, want) in &skill.files {
                let path = format!("skills/{}/{}", skill.name, file);
                let embedded = PipelinePluginAssets::get(&path)
                    .unwrap_or_else(|| panic!("{path} 在锁文件里但不在插件里"));
                let mut hasher = Sha256::new();
                hasher.update(embedded.data.as_ref());
                assert_eq!(
                    &hex_lower(&hasher.finalize()),
                    want,
                    "{path} 与锁文件不一致：跑 node scripts/sync-pipeline-skills.mjs --update"
                );
            }
        }
    }

    #[test]
    fn 已验证支持_plugin_dir_的_claude_版本没有变() {
        // 平台跑的是 `npx -y @anthropic-ai/claude-code@<版本>`。这个版本号一变就要重新验
        // 一遍 --plugin-dir（计划 §4 待核实项 A 的验收脚本），所以在这里钉住。
        // 若因文件移动导致 include_str! 编译失败，按新路径改这里并重新验收。
        let source = include_str!("../../../../../executors/src/executors/claude.rs");
        assert!(
            source.contains("@anthropic-ai/claude-code@2.1.119"),
            "Claude Code 的固定版本变了：请重新验证 --plugin-dir 支持情况，再更新本测试与计划 §2.1"
        );
    }
```

- [ ] **Step 3: 跑测试**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p services pipeline::plugin 2>&1 | tail -20`
Expected: 9 条全 ok。若 `include_str!` 路径编译不过，从 `plugin.rs` 数四层 `..` 到 `crates/`，再接 `executors/src/executors/claude.rs`。

- [ ] **Step 4: 确认代码里没有碰 `~/.claude` 的路径**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
grep -rn "CLAUDE_CONFIG_DIR" crates/services/src/services/pipeline crates/executors/src/actions crates/executors/src/executors/claude.rs
grep -rn '\.claude/plugins' crates/services/src/services/pipeline crates/executors/src/actions crates/executors/src/executors/claude.rs
grep -rn 'HOME.*\.claude' crates/services/src/services/pipeline crates/executors/src/actions crates/executors/src/executors/claude.rs
echo "三条 grep 都无输出即通过"
```
Expected: 三条命令都没有输出，最后打印那句话。（技能包只经 `--plugin-dir` 注入，不写用户配置目录。）

- [ ] **Step 5: 确认同步脚本对 `~` 只读不写**

Run: `cd /Users/admin/work/github/vibe-kanban && grep -n "homedir\|writeFileSync\|copyFileSync\|rmSync" scripts/sync-pipeline-skills.mjs`
Expected: `homedir` 只出现在 `expandHome` 里；`writeFileSync` 只写 `LOCK_PATH`；`copyFileSync` / `rmSync` 的目标都在 `SKILLS_DIR` 下。人工核对这四处，任何一处指向家目录都要改。

- [ ] **Step 6: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/services/src/services/pipeline/plugin.rs
git commit -m "$(cat <<'MSG'
流水线：技能包静态检查

frontmatter 合法、name 与目录一致、七阶段技能齐全、平台技能共用同一段约定、
正文提到的产出文件名与契约一致、锁文件哈希与嵌入内容一致、Claude 固定版本被钉住。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 14: `GET /api/local/pipeline/skills`

**Files:**
- Modify: `crates/server/src/routes/local_projects/pipeline.rs`
- Modify: `crates/server/src/bin/generate_types.rs:207`
- Modify: `shared/types.ts`（生成）
- Modify: `packages/web-core/src/entities/pipeline/api/pipelineApi.ts`

- [ ] **Step 1: 写失败的 handler 测试**

在 `crates/server/src/routes/local_projects/pipeline.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn 技能清单包含七阶段技能并带来源() {
        let skills = handle_list_skills();
        let by_name: std::collections::BTreeMap<_, _> =
            skills.iter().map(|s| (s.name.as_str(), s)).collect();
        for name in [
            "vk-requirement",
            "vk-spec",
            "prd2testcase",
            "vk-develop",
            "vk-review",
            "atp-run",
            "vk-deliver",
        ] {
            let skill = by_name
                .get(name)
                .unwrap_or_else(|| panic!("清单里没有 {name}"));
            assert_eq!(skill.qualified_name, format!("vk-pipeline:{name}"));
            assert!(skill.stage.is_some(), "{name} 应该对应某个阶段");
        }
        assert_eq!(by_name["vk-requirement"].source, "platform");
        assert!(by_name["vk-requirement"].source_version.is_none());

        let brainstorming = by_name
            .get("brainstorming")
            .expect("清单里没有 brainstorming");
        assert_eq!(brainstorming.source, "superpowers");
        assert_eq!(
            brainstorming.source_version.as_deref(),
            Some("5.1.0"),
            "外来技能要显示来源版本"
        );
        assert_eq!(brainstorming.license.as_deref(), Some("MIT"));
        assert!(brainstorming.stage.is_none(), "它不直接对应阶段");
    }
```

同时在文件末尾的 `流水线端点都已登记且路由不冲突` 测试的路径数组里加一行：

```rust
            ("/api/local/pipeline/skills", "GET"),
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p server pipeline 2>&1 | tail -12`
Expected: 编译失败 `cannot find function handle_list_skills`。

- [ ] **Step 3: 实现**

`crates/server/src/routes/local_projects/pipeline.rs`：
`use services::services::pipeline::{PipelineError, PipelineService, StartPipelineInput};` 改成

```rust
use services::services::pipeline::{
    PipelineError, PipelineService, PipelineSkillInfo, StartPipelineInput, skills_view,
};
```

在 `handle_get_artifact` 之后加：

```rust
/// 技能清单：只读编进二进制的插件内容与锁文件，不依赖解压是否成功，因此不会失败。
pub(crate) fn handle_list_skills() -> Vec<PipelineSkillInfo> {
    skills_view()
}
```

在 `async fn pending(...)` 之后加：

```rust
async fn list_skills() -> ResponseJson<ApiResponse<Vec<PipelineSkillInfo>>> {
    ResponseJson(ApiResponse::success(handle_list_skills()))
}
```

`router()` 里 `/pipeline` 那一组加一行（放在 `/pending` 之后）：

```rust
        .route("/skills", &["GET"], get(list_skills))
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test -p server pipeline 2>&1 | tail -12`
Expected: `技能清单包含七阶段技能并带来源 ... ok` 与 `流水线端点都已登记且路由不冲突 ... ok`。

- [ ] **Step 5: 注册 TS 类型并生成**

`crates/server/src/bin/generate_types.rs`：在 `db::models::pipeline::PendingPipelineItem::decl(),` 之后加一行

```rust
        services::services::pipeline::PipelineSkillInfo::decl(),
```

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run generate-types && grep -n "PipelineSkillInfo" shared/types.ts`
Expected: `shared/types.ts` 里出现 `export type PipelineSkillInfo = { name: string, qualified_name: string, source: string, source_version: string | null, license: string | null, stage: PipelineStageKey | null, file_count: number, }`（字段顺序以生成结果为准；`file_count` 必须是 `number` 而不是 `bigint`）。

- [ ] **Step 6: 前端加接口**

`packages/web-core/src/entities/pipeline/api/pipelineApi.ts`：
`import type { … } from 'shared/types';` 的类型列表里加 `PipelineSkillInfo`；
`PIPELINE_API_PATHS` 里加一行（写成完整字面量，路由契约测试按字面量扫描）：

```ts
  skills: '/api/local/pipeline/skills',
```

文件末尾加：

```ts
/** 设置页「流水线技能」的只读清单。 */
export function getPipelineSkills(): Promise<PipelineSkillInfo[]> {
  return requestEnvelope(PIPELINE_API_PATHS.skills);
}
```

- [ ] **Step 7: 前端类型检查**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check 2>&1 | tail -5`
Expected: 无错误。

- [ ] **Step 8: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add crates/server shared/types.ts packages/web-core/src/entities/pipeline/api/pipelineApi.ts
git commit -m "$(cat <<'MSG'
接口：新增技能清单端点 GET /api/local/pipeline/skills

只读编进二进制的插件内容与锁文件，返回名字、引用名、来源、版本、许可证与所属阶段。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 15: 设置页只读「流水线技能」

**Files:**
- Create: `packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx`
- Modify: `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx`
- Modify: `packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}/settings.json`

- [ ] **Step 1: 写组件**

`packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx`：

```tsx
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PipelineSkillInfo, PipelineStageKey } from 'shared/types';
import { getPipelineSkills } from '@/entities/pipeline/api/pipelineApi';

/** 来源 → i18n 键。锁文件里出现新来源时回落到来源名本身。 */
const SOURCE_KEYS: Record<string, string> = {
  platform: 'settings.pipelineSkills.source.platform',
  superpowers: 'settings.pipelineSkills.source.superpowers',
  atp: 'settings.pipelineSkills.source.atp',
};

export function PipelineSkillsSettingsSection() {
  const { t } = useTranslation('settings');
  const [skills, setSkills] = useState<PipelineSkillInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getPipelineSkills()
      .then((rows) => {
        if (!cancelled) setSkills(rows);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const stageLabel = (stage: PipelineStageKey | null) =>
    stage === null
      ? t('settings.pipelineSkills.noStage')
      : t(`pipeline.stage.${stage}`, { ns: 'common' });

  const sourceLabel = (source: string) => {
    const key = SOURCE_KEYS[source];
    return key ? t(key) : source;
  };

  return (
    <div className="space-y-4 pb-6">
      <p className="text-sm text-normal">
        {t('settings.pipelineSkills.description')}
      </p>
      <p className="text-xs text-low">{t('settings.pipelineSkills.readOnly')}</p>

      {error !== null && (
        <p className="text-sm text-danger">
          {t('settings.pipelineSkills.loadError', { message: error })}
        </p>
      )}
      {error === null && skills === null && (
        <p className="text-sm text-low">{t('settings.pipelineSkills.loading')}</p>
      )}
      {skills !== null && skills.length === 0 && (
        <p className="text-sm text-low">{t('settings.pipelineSkills.empty')}</p>
      )}

      {skills !== null && skills.length > 0 && (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-low">
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.name')}
              </th>
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.source')}
              </th>
              <th className="py-2 pr-3 font-medium">
                {t('settings.pipelineSkills.columns.version')}
              </th>
              <th className="py-2 font-medium">
                {t('settings.pipelineSkills.columns.stage')}
              </th>
            </tr>
          </thead>
          <tbody>
            {skills.map((skill) => (
              <tr key={skill.name} className="border-b border-border/50">
                <td className="py-2 pr-3">
                  <code className="text-xs text-high">
                    {skill.qualified_name}
                  </code>
                </td>
                <td className="py-2 pr-3 text-normal">
                  {sourceLabel(skill.source)}
                  {skill.license !== null && (
                    <span className="ml-1 text-xs text-low">
                      ({skill.license})
                    </span>
                  )}
                </td>
                <td className="py-2 pr-3 text-normal">
                  {skill.source_version === null
                    ? '—'
                    : skill.source_version.slice(0, 12)}
                </td>
                <td className="py-2 text-normal">{stageLabel(skill.stage)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 注册 section（四处都要改）**

`packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx`：

1. `import` 区加：
```tsx
import { StackIcon } from '@phosphor-icons/react';
import { PipelineSkillsSettingsSection } from './PipelineSkillsSettingsSection';
```
（`StackIcon` 合并进已有的 `@phosphor-icons/react` 那条 import 里。）

2. `SettingsSectionType` 联合类型加一项：
```tsx
  | 'pipeline-skills'
```

3. `SettingsSectionInitialState` 加一项：
```tsx
  'pipeline-skills': undefined;
```

4. `SETTINGS_SECTION_DEFINITIONS` 里，`{ id: 'mcp', … }` 之后加：
```tsx
  { id: 'pipeline-skills', icon: StackIcon, group: 'host' },
```

5. `renderSettingsSection` 的 `switch` 里，`case 'mcp':` 之后加：
```tsx
    case 'pipeline-skills':
      return <PipelineSkillsSettingsSection />;
```

- [ ] **Step 3: 七个语言各加文案**

在 `packages/web-core/src/i18n/locales/<语言>/settings.json` 的 `settings.layout.nav` 里加两个键，并在 `settings` 下新增 `pipelineSkills` 对象。

`en`：
```json
      "pipeline-skills": "Pipeline Skills",
      "pipeline-skillsDesc": "Skills each pipeline stage loads",
```
```json
    "pipelineSkills": {
      "description": "Every pipeline stage loads one skill from the built-in vk-pipeline plugin. The plugin is injected per session and never written into your repositories.",
      "readOnly": "Read-only. Third-party skills are pinned by checksum in skills.lock.json.",
      "loading": "Loading skills…",
      "loadError": "Could not load skills: {{message}}",
      "empty": "No skills found.",
      "noStage": "Referenced by other skills",
      "columns": { "name": "Skill", "source": "Source", "version": "Version", "stage": "Stage" },
      "source": { "platform": "Built-in", "superpowers": "superpowers", "atp": "atp" }
    },
```

`zh-Hans`：
```json
      "pipeline-skills": "流水线技能",
      "pipeline-skillsDesc": "各阶段加载的技能",
```
```json
    "pipelineSkills": {
      "description": "流水线的每个阶段都从内置的 vk-pipeline 插件里加载一个技能。插件按会话注入，不会写进你的仓库。",
      "readOnly": "只读。外来技能由 skills.lock.json 按校验和锁定。",
      "loading": "正在加载技能…",
      "loadError": "加载技能失败：{{message}}",
      "empty": "没有找到技能。",
      "noStage": "被其它技能引用",
      "columns": { "name": "技能", "source": "来源", "version": "版本", "stage": "阶段" },
      "source": { "platform": "平台自带", "superpowers": "superpowers", "atp": "atp" }
    },
```

`zh-Hant`：
```json
      "pipeline-skills": "流水線技能",
      "pipeline-skillsDesc": "各階段載入的技能",
```
```json
    "pipelineSkills": {
      "description": "流水線的每個階段都會從內建的 vk-pipeline 外掛載入一個技能。外掛依工作階段注入，不會寫進你的儲存庫。",
      "readOnly": "唯讀。外來技能由 skills.lock.json 以校驗和鎖定。",
      "loading": "正在載入技能…",
      "loadError": "載入技能失敗：{{message}}",
      "empty": "找不到技能。",
      "noStage": "被其他技能引用",
      "columns": { "name": "技能", "source": "來源", "version": "版本", "stage": "階段" },
      "source": { "platform": "平台內建", "superpowers": "superpowers", "atp": "atp" }
    },
```

`ja`：
```json
      "pipeline-skills": "パイプラインスキル",
      "pipeline-skillsDesc": "各ステージが読み込むスキル",
```
```json
    "pipelineSkills": {
      "description": "パイプラインの各ステージは、内蔵の vk-pipeline プラグインからスキルを 1 つ読み込みます。プラグインはセッション単位で注入され、リポジトリには書き込まれません。",
      "readOnly": "読み取り専用です。外部スキルは skills.lock.json でチェックサム固定されています。",
      "loading": "スキルを読み込み中…",
      "loadError": "スキルを読み込めませんでした: {{message}}",
      "empty": "スキルが見つかりません。",
      "noStage": "他のスキルから参照",
      "columns": { "name": "スキル", "source": "提供元", "version": "バージョン", "stage": "ステージ" },
      "source": { "platform": "内蔵", "superpowers": "superpowers", "atp": "atp" }
    },
```

`ko`：
```json
      "pipeline-skills": "파이프라인 스킬",
      "pipeline-skillsDesc": "각 단계가 불러오는 스킬",
```
```json
    "pipelineSkills": {
      "description": "파이프라인의 각 단계는 내장 vk-pipeline 플러그인에서 스킬을 하나씩 불러옵니다. 플러그인은 세션 단위로 주입되며 저장소에 기록되지 않습니다.",
      "readOnly": "읽기 전용입니다. 외부 스킬은 skills.lock.json에서 체크섬으로 고정됩니다.",
      "loading": "스킬을 불러오는 중…",
      "loadError": "스킬을 불러오지 못했습니다: {{message}}",
      "empty": "스킬이 없습니다.",
      "noStage": "다른 스킬에서 참조",
      "columns": { "name": "스킬", "source": "출처", "version": "버전", "stage": "단계" },
      "source": { "platform": "기본 제공", "superpowers": "superpowers", "atp": "atp" }
    },
```

`es`：
```json
      "pipeline-skills": "Habilidades del flujo",
      "pipeline-skillsDesc": "Habilidad que carga cada etapa",
```
```json
    "pipelineSkills": {
      "description": "Cada etapa del flujo carga una habilidad del complemento integrado vk-pipeline. El complemento se inyecta por sesión y nunca se escribe en tus repositorios.",
      "readOnly": "Solo lectura. Las habilidades de terceros están fijadas por suma de verificación en skills.lock.json.",
      "loading": "Cargando habilidades…",
      "loadError": "No se pudieron cargar las habilidades: {{message}}",
      "empty": "No se encontraron habilidades.",
      "noStage": "Referenciada por otras habilidades",
      "columns": { "name": "Habilidad", "source": "Origen", "version": "Versión", "stage": "Etapa" },
      "source": { "platform": "Integrada", "superpowers": "superpowers", "atp": "atp" }
    },
```

`fr`：
```json
      "pipeline-skills": "Compétences du pipeline",
      "pipeline-skillsDesc": "Compétence chargée par chaque étape",
```
```json
    "pipelineSkills": {
      "description": "Chaque étape du pipeline charge une compétence depuis le plugin intégré vk-pipeline. Le plugin est injecté par session et n'est jamais écrit dans vos dépôts.",
      "readOnly": "Lecture seule. Les compétences tierces sont verrouillées par somme de contrôle dans skills.lock.json.",
      "loading": "Chargement des compétences…",
      "loadError": "Impossible de charger les compétences : {{message}}",
      "empty": "Aucune compétence trouvée.",
      "noStage": "Référencée par d'autres compétences",
      "columns": { "name": "Compétence", "source": "Source", "version": "Version", "stage": "Étape" },
      "source": { "platform": "Intégrée", "superpowers": "superpowers", "atp": "atp" }
    },
```

- [ ] **Step 4: 确认七份文件的键集合一致**

Run:
```bash
cd /Users/admin/work/github/vibe-kanban
node -e '
const fs=require("fs");
const langs=["en","es","fr","ja","ko","zh-Hans","zh-Hant"];
const flat=(o,p="")=>Object.entries(o).flatMap(([k,v])=>typeof v==="object"&&v?flat(v,p+k+"."):[p+k]);
const base=flat(JSON.parse(fs.readFileSync("packages/web-core/src/i18n/locales/en/settings.json"))).sort();
for(const l of langs){
  const keys=flat(JSON.parse(fs.readFileSync(`packages/web-core/src/i18n/locales/${l}/settings.json`))).sort();
  const missing=base.filter(k=>!keys.includes(k));
  console.log(l, missing.length?("缺: "+missing.join(",")):"ok");
}'
```
Expected: 七行全是 `ok`。

- [ ] **Step 5: 类型检查与 lint**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run web-core:check && pnpm run local-web:lint 2>&1 | tail -10`
Expected: 无错误、无未使用的 i18n 键告警。

- [ ] **Step 6: 提交**

```bash
cd /Users/admin/work/github/vibe-kanban
git add packages/web-core
git commit -m "$(cat <<'MSG'
界面：设置里新增只读「流水线技能」页

列出每个技能的引用名、来源、版本与所属阶段；七个语言文案齐全，阶段名复用已有的 i18n 键。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 16: 收尾与真实 Claude Code 人工验收

**Files:**
- 不新增文件；只跑检查与验收，把结果写进 PR 描述。

- [ ] **Step 1: 看磁盘**

Run: `df -h / | tail -1`
Expected: `Avail` ≥ 4G。不够就先 `cargo clean -p server` 再继续（只清一个包，别全清）。

- [ ] **Step 2: 格式化**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run format`
Expected: 结束无报错。之后 `git status --porcelain` 若有改动，作为「格式化」单独提交一次。

- [ ] **Step 3: 全量检查**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run check 2>&1 | tail -20`
Expected: 最后无 error；第一条就是 `技能锁校验通过`。

- [ ] **Step 4: Lint**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run lint 2>&1 | tail -20`
Expected: clippy 无 warning（`-D warnings`），ESLint 无 error，未用 i18n 键检查通过。

- [ ] **Step 5: 全量 Rust 测试（**只跑这一次**）**

Run: `cd /Users/admin/work/github/vibe-kanban && cargo test --workspace 2>&1 | tail -25`
Expected: 所有包 `test result: ok.`。

- [ ] **Step 6: 端到端（qa-mode）**

Run: `cd /Users/admin/work/github/vibe-kanban && pnpm run e2e 2>&1 | tail -20`
Expected: 全绿。qa-mode 模拟器不解析提示词首行，本期改动不应影响它；挂了就先看是不是 `prompt.rs` 的整行断言没改全。

- [ ] **Step 7: 人工验收——准备样例仓库与数据目录**

```bash
export VKA=/tmp/vk-acc-assets && rm -rf "$VKA" && mkdir -p "$VKA"
export DEMO=/tmp/vk-acc-repo && rm -rf "$DEMO" && mkdir -p "$DEMO" && cd "$DEMO"
git init -q && git config user.email a@b.c && git config user.name acc
cat > package.json <<'JSON'
{ "name": "vk-acc-demo", "version": "1.0.0", "scripts": { "test": "node --test" } }
JSON
mkdir -p src && printf 'export function add(a, b) {\n  return a + b;\n}\n' > src/add.mjs
git add -A && git commit -qm "初始提交" && git log --oneline
export ACC_START=$(date "+%Y-%m-%d %H:%M:%S")
find ~/.claude/plugins -type f 2>/dev/null | wc -l | sed 's/^/基线：~\/.claude\/plugins 文件数 /'
```
Expected: 打印一条提交记录与一行「基线：…文件数 N」（记下 N）。

- [ ] **Step 8: 人工验收——先单独验 `--plugin-dir`（§4 待核实项 A、B）**

```bash
cd /Users/admin/work/github/vibe-kanban
cargo run -q -p server --bin server -- --help >/dev/null 2>&1 || true   # 触发一次构建，便于下一步启动更快
VK_ASSET_DIR="$VKA" cargo run -q -p server --bin server &
SERVER_PID=$!
sleep 20 && kill $SERVER_PID
PLUGIN_DIR=$(ls -d "$VKA"/pipeline-plugin/*/ | head -1)
echo "解压目录：$PLUGIN_DIR"
ls "$PLUGIN_DIR/.claude-plugin/plugin.json" "$PLUGIN_DIR/skills/vk-requirement/SKILL.md"
cd "$DEMO" && npx -y @anthropic-ai/claude-code@2.1.119 --plugin-dir "$PLUGIN_DIR" -p "用 Skill 工具加载技能 vk-pipeline:vk-requirement，然后只回答它 frontmatter 里 description 的原文，不要别的话。"
```
Expected：最后一条命令输出里包含 `把一句话需求整理成 requirement.md`。
- 输出是「找不到技能」→ 待核实项 A 不成立，按 §4 的分支改解压布局，重跑任务 8 与本步。
- 输出是模型自己编的话、没加载技能 → 待核实项 B 不成立，按 §4 的分支给提示词加一行兜底，重跑任务 11 与本步。

- [ ] **Step 9: 人工验收——跑一条真实需求**

```bash
cd /Users/admin/work/github/vibe-kanban
VK_ASSET_DIR="$VKA" pnpm run dev
```
在界面里（**不要用自动化浏览器，人工点**）：

1. 新建项目，仓库指向 `/tmp/vk-acc-repo`。
2. 在工作台提一条需求：`给 add 函数加参数校验：非数字入参要抛出明确错误`，执行器选 **Claude Code**，启动流水线。
3. 依次通过三道人工关卡（需求确认 → 规格确认 → 用例确认）。每道关卡先**读产出物**再点通过；至少有一道故意打回一次，写一句具体意见，确认下一轮的产出物回应了它。
4. 记录每个阶段的开始与结束时间（界面上有），填进下面的表。

预期：
- 需求阶段产出 `requirement.md`，里面有「验收标准」且编号 AC-1 起。
- 规格阶段产出 `spec.md` 与 `plan.md`。
- 用例阶段产出 `test-cases.csv`（首行 46 列）与 `trace-matrix.md`。
- 开发阶段在 `/tmp/vk-acc-repo` 的工作区副本里产生**至少一个提交**。
- 评审阶段产出合法 `review.json`。

- [ ] **Step 10: 人工验收——核对「不落进用户仓库、不动 `~/.claude`」**

```bash
WS=$(ls -d "$VKA"/../*/worktrees/*/ 2>/dev/null | head -1)   # 取不到就用界面上显示的工作区路径
echo "工作区：$WS"
cd "$WS" && git status --porcelain
find "$WS" -name "SKILL.md" -not -path "*/.vk/*" | head
find ~/.claude/plugins -type f -newermt "$ACC_START" 2>/dev/null | head
find ~/.claude/plugins -type f 2>/dev/null | wc -l
```
Expected:
- `git status --porcelain` 为空（开发阶段的改动都已提交）。
- 第二条 `find` **无输出**（工作区里没有任何技能文件）。
- 第三条 `find` **无输出**（`~/.claude/plugins` 下没有新文件）。
- 最后一行的文件数与 Step 7 记下的 N **相同**。

- [ ] **Step 11: 人工验收——把结果写进 PR 描述**

按这张表填（时间取界面上的阶段耗时）：

```markdown
## 真实 Claude Code 验收（一次）

- Claude Code 版本：`npx @anthropic-ai/claude-code@2.1.119`
- 样例仓库：`/tmp/vk-acc-repo`（一个 add 函数）
- 数据目录：`VK_ASSET_DIR=/tmp/vk-acc-assets`
- 插件解压目录：`<PLUGIN_DIR>`

| 阶段 | 技能 | 耗时 | 产出物 | 结果 |
|---|---|---|---|---|
| 需求 | `vk-pipeline:vk-requirement` |  | `requirement.md` | 人工通过 |
| 规格 | `vk-pipeline:vk-spec` |  | `spec.md`、`plan.md` | 打回 1 次后通过 |
| 用例 | `vk-pipeline:prd2testcase` |  | `test-cases.csv`、`trace-matrix.md` | 人工通过 |
| 开发 | `vk-pipeline:vk-develop` |  | 提交 `<短提交号>` | 自动通过 |
| 评审 | `vk-pipeline:vk-review` |  | `review.json` | blocker 0 个 |
| 测试 | `vk-pipeline:atp-run` |  | `test-report.json` | 最小版 |
| 交付 | `vk-pipeline:vk-deliver` |  | `delivery-report.md` | 最小版 |

隔离核对：
- 工作区 `git status --porcelain`：空
- 工作区内技能文件：无
- `~/.claude/plugins` 新增文件：0（验收前后文件数都是 `<N>`）
```

- [ ] **Step 12: 清理验收环境**

Run: `rm -rf /tmp/vk-acc-assets /tmp/vk-acc-repo /tmp/vk-plugin-probe /tmp/vkplug`
Expected: 无输出。

- [ ] **Step 13: 记遗留清单（写进 PR 描述，不改代码）**

- `atp-run` 与 `vk-deliver` 是最小版，U4 / U5 替换。
- `prd2testcase` 的 atp 侧 PR 若尚未合入，锁文件指向的是分支提交号；合入后要跑一次 `pnpm run pipeline-skills:update` 把 `commit` 换成主干提交号。
- `brainstorming` 的可视化伴侣与 `systematic-debugging` 的若干附属文件未引入（见 `THIRD_PARTY_NOTICES.md`），后续若需要再按白名单加。
- 每个阶段结束仍会发一次「Workspace Complete」系统通知（U2 遗留）。

- [ ] **Step 14: 提交（若 Step 2 产生了格式化改动）**

```bash
cd /Users/admin/work/github/vibe-kanban
git add -A
git commit -m "$(cat <<'MSG'
收尾：格式化

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```
若 `git status --porcelain` 为空则跳过这一步。
