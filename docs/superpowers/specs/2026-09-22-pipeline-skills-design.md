# 流水线技能接入设计（U3）

> 状态：已确认方向，待实施｜日期：2026-09-22
> 上级：`docs/superpowers/specs/2026-09-18-personal-pipeline-design.md` §9.1
> 前置：U1 + U2（PR #7）已完成，流水线引擎在 qa-mode 模拟执行器下跑通七个阶段

## 1. 目标

让流水线在**真实 Claude Code** 下跑起来：每个阶段调用一个确定的技能，技能按固定目录读写产出物、不向人提问。U3 完成后，一条真实需求能从一句话自动走过三道人工关卡，并完成开发与评审。

不在本期：`atp-run` 与测试阶段真跑（U4）、交付阶段提 PR 与交付报告页（U5）。U3 期间测试与交付阶段的技能先给出能产出合规文件的最小版本，保证流程不断。

## 2. 已核实的事实

| 事实 | 依据 | 影响 |
|---|---|---|
| Claude Code 2.1.278 支持 `--plugin-dir <path>`，只对当次会话加载插件，可重复 | `claude --help` | 技能以插件形式按会话注入，不写进用户仓库，也不动 `~/.claude` |
| 插件内技能以 `插件名:技能名` 引用 | superpowers 插件的实际用法 | 提示词里要写带命名空间的名字 |
| 平台 Claude 执行器的命令由 `build_command_builder` 拼装，已有 `extend_params` | `crates/executors/src/executors/claude.rs:149-190` | 在这里按会话追加 `--plugin-dir` |
| 默认 Claude Code 配置 `dangerously_skip_permissions: true` | `crates/executors/default_profiles.json:3-8` | 流水线无人值守可行，不需要额外审批通道 |
| superpowers 5.1.0 为 MIT，技能无外部运行时依赖 | 本机插件缓存 `plugin.json`、各 `SKILL.md` | 可原样引入 |
| gstack 的 `plan-eng-review`、`document-release` 依赖 `~/.gstack`、遥测与专用命令 | `gstack/plan-eng-review/SKILL.md:26-60` | **不引入**，自审改用 superpowers 的方法 |
| mattpocock `to-spec` 定位是发布到 issue 跟踪系统，依赖 setup 技能 | 该仓库 `skills/engineering/to-spec/SKILL.md` | 不原样引入，只吸收其规格模板结构（MIT，注明出处） |
| atp `legacy run --list` 会加载解析 CSV 且只列不跑 | atp `src/atp/legacy_csv/cli.py:217-315` | `prd2testcase` 用它做格式自检 |

## 3. 方案

### 3.1 技能包：平台自带一个 Claude Code 插件

仓库新增 `assets/pipeline-plugin/`，结构：

```
assets/pipeline-plugin/
  .claude-plugin/plugin.json        name = "vk-pipeline"
  skills/
    vk-requirement/SKILL.md         平台技能（自写）
    vk-spec/SKILL.md
    vk-develop/SKILL.md
    vk-review/SKILL.md
    vk-deliver/SKILL.md             U3 为最小版本，U5 完善
    prd2testcase/SKILL.md           来源：atp 仓库，按锁文件同步
    atp-run/SKILL.md                U3 为占位最小版本，U4 替换为 atp 仓库的正式版
    brainstorming/…                 以下为 superpowers 5.1.0 原样引入
    writing-plans/…
    test-driven-development/…
    requesting-code-review/…        （含 code-reviewer.md）
    systematic-debugging/…
    verification-before-completion/…
    finishing-a-development-branch/…
  skills.lock.json                  每个外来技能：来源仓库、提交号、路径、文件 sha256、许可证
  THIRD_PARTY_NOTICES.md            许可证全文与出处（superpowers MIT、mattpocock 模板 MIT）
```

`scripts/sync-pipeline-skills.mjs`：按锁文件从来源拉取（本地路径或 git）并校验哈希；`--check` 模式只校验，接入 `pnpm run check` 与 CI，防止有人手改外来技能。

### 3.2 注入方式

1. 插件目录用 RustEmbed 编进二进制；服务启动时解压到 `asset_dir()/pipeline-plugin/<内容哈希>/`，同哈希已存在则跳过。
2. `CodingAgentInitialRequest` 新增 `plugin_dirs: Vec<PathBuf>`（serde 默认空，旧数据兼容）；Claude 执行器对每一项追加 `--plugin-dir`。其它执行器忽略该字段。
3. 引擎开阶段会话时填入插件目录。启动流水线时校验执行器必须是 `CLAUDE_CODE`（qa-mode 例外），否则 400「流水线目前只支持 Claude Code」。
4. 技能名解析：模板里的技能名若含 `:` 原样使用；若是插件内技能，加前缀 `vk-pipeline:`；否则视为仓库自带技能（仓库 `.claude/skills/`）原样使用。
5. 提示词首行改为「先用 Skill 工具加载技能 `vk-pipeline:<name>`，严格按它执行。」

### 3.3 平台技能的统一约定

每个 `vk-*` 技能开头有一节「流水线约定」，内容一致：

- 输入：提示词里的「需求」「产出物目录」「必须产出」「上一阶段产出」「本阶段上一版」「打回意见」各行（格式见 `crates/executors/src/pipeline_prompt.rs`）。
- 输出：只写「必须产出」列出的文件，写到「产出物目录」；不得写到仓库里（开发阶段的代码改动除外）。
- 非交互：禁止向人提问；拿不准的写进产出物的「待澄清」一节；被打回时逐条回应打回意见。
- 引用第三方技能时，明确说明「按它的方法做，但以本约定为准」，覆盖掉第三方技能里的交互步骤（例如 brainstorming 的逐条提问、writing-plans 的执行方式选择）。

各技能要点：

| 技能 | 产出 | 方法 |
|---|---|---|
| `vk-requirement` | `requirement.md`：背景、范围与不做、验收标准（给定-当-那么，编号 AC-1…）、待澄清 | brainstorming 的澄清维度（目的、约束、成功标准），改为自问自答 |
| `vk-spec` | `spec.md`（问题、方案、用户故事、实现决策、测试决策、不做——结构取自 to-spec 模板）、`plan.md`（任务列表，每个任务带文件与测试点） | writing-plans；交出前按它的自检清单自审 |
| `prd2testcase` | `test-cases.csv`（atp 旧 CSV，46 列表头原样）、`trace-matrix.md`（AC ↔ CaseID，未覆盖标红） | 三步：需求分析（每条 AC 拆测试点）→ 测试方案（端、优先级、正反例）→ 用例；最后用 `atp legacy run --list` 自检（环境没有 atp 时跳过并在追踪矩阵注明） |
| `vk-develop` | 代码改动与提交 | test-driven-development；按 `plan.md` 逐任务；完成前 verification-before-completion |
| `vk-review` | `review.json`（findings：severity blocker/major/minor、file、line、message） | requesting-code-review 的清单，对照 `spec.md` 查正确性与安全 |
| `atp-run`（U3 最小版） | `test-report.json` | 运行仓库已有测试命令（从 `plan.md` 或常见约定推断），按结果写报告；U4 替换 |
| `vk-deliver`（U3 最小版） | `delivery-report.md`：需求 → 规格 → 用例 → 提交 → 测试证据 | finishing-a-development-branch 的检查项，只写报告不合并 |

### 3.4 `prd2testcase` 放在 atp 仓库

atp 负责 CSV 格式，技能与格式同仓维护：atp 仓库新增 `skills/prd2testcase/SKILL.md`（含 46 列说明与示例行），vibe-kanban 通过锁文件同步。U4 的 `atp-run` 同样放 atp 仓库。

### 3.5 界面

设置里新增「流水线技能」只读页：列出插件内每个技能的名字、来源（平台 / superpowers / atp）、版本或提交号、用在哪个阶段。数据来自 `GET /api/local/pipeline/skills`（读取解压后的插件目录与锁文件）。

## 4. 测试

- **技能包静态检查**（Rust 单测，读 RustEmbed 内容）：每个 `SKILL.md` frontmatter 合法、`name` 与目录名一致、有 `description`；内置模板里每个阶段的技能都存在于插件中；`vk-*` 技能正文提到的产出文件名与契约 §4 一致；锁文件里的哈希与实际文件一致。
- **注入**：Claude 执行器在 `plugin_dirs` 非空时命令行包含对应 `--plugin-dir`；为空时与旧行为逐字一致。
- **解压**：同哈希幂等；内容变化生成新目录。
- **提示词**：技能名解析三种情形；首行格式。
- **真实跑一次（人工验收，记录在 PR）**：用真实 Claude Code、一个小样例仓库，走过三道人工关卡并完成开发与评审阶段；记录每阶段耗时与产出物。

## 5. 风险

| 风险 | 对策 |
|---|---|
| 第三方技能里的交互步骤让智能体卡住等人 | 平台技能明确覆盖；提示词再强调一次；U2 已有轮次与超时兜底 |
| Claude Code 版本变化导致 `--plugin-dir` 行为变化 | 启动时检测 `claude --version`，低于已验证版本在日志告警；注入失败时阶段按缺产出物失败，界面可见 |
| 技能内容被随意修改导致流程漂移 | 外来技能哈希锁定并在 CI 校验；平台技能改动走 PR |
| 真实运行消耗额度 | 验收只跑一次小样例；日常测试仍用 qa-mode |

## 6. 验收标准

1. 真实 Claude Code 下，一句话需求自动产出 `requirement.md`，三道人工关卡依次出现，确认后进入开发并产生提交，评审产出合法 `review.json`。
2. 用户仓库里不出现任何技能文件或 `.vk/` 产出物；`~/.claude` 不被修改。
3. 设置页能看到技能清单与来源。
4. `pnpm run check`（含技能锁校验）、`pnpm run lint`、`cargo test --workspace`、`pnpm run e2e` 全绿。
