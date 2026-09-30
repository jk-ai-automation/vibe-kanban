# 流水线测试阶段接入真实执行设计（U4）

> 状态：已确认方向，待实施｜日期：2026-09-26
> 上级：`docs/superpowers/specs/2026-09-18-personal-pipeline-design.md` §9.2
> 前置：U1–U3 已合并 main（PR #7、#8、#9）；技能以插件形式注入已验证

## 1. 目标

把测试阶段从「最小版：跑一下仓库测试命令」变成真实可用的两路执行：

- **接口路**：用 `test-cases.csv` 调 atp 真跑 HTTP 用例，被测服务由平台自动拉起。
- **仓库路**：跑仓库自身的测试命令（单元/集成）。

两路结果合并成同一份 `test-report.json`，失败按归因回流到开发或用例设计。

## 2. 为什么要两路（验收暴露的问题）

2026-09-26 真实验收里，需求是「给 `add(a,b)` 加入参校验」这种纯函数级改动。`prd2testcase` 生成的 6 条用例 `NOAUTO` 全为 `Y`——因为 atp 旧 CSV 是面向 HTTP 接口的，没有接口可打时只能退化成人工用例，真正的执行还得靠 `node --test`。

所以测试阶段不能只有 atp 一条路。两路并存，各自产出，合并成一份报告。

## 3. 已核实的现状

| 事实 | 依据 | 影响 |
|---|---|---|
| **atp 没有 Web/App 能力** | 依赖只有 httpx/pytest/jsonpath；`--web-engine` 解析后不生效；`engine/` 全是 HTTP | 本期只做 API 与仓库测试，Web/App 明确不做 |
| `atp legacy run` 的 base_url 只能命令行传，无回退 | `legacy_csv/cli.py:290-313` | 平台必须把被测地址算出来传进去 |
| `--env` 接受但不生效，只写进结果 | `cli.py:107-113,926-928` | 环境差异靠 base_url 与环境变量表达 |
| `results.json` 用例级只有 6 个字段：case_id、title、module、status、duration_s、message | `src/atp/results.py:19-36` | **没有 attribution，也没有附件索引**，归因要另算 |
| 归因线索在结果 CSV 的 `RootCause` 列；步骤统计塞在 message 文本里 | `legacy_csv/columns.py:52-64`、`cli.py:499-534` | 技能读这两处来判归因 |
| 退出码：0 全过 / 1 有失败或未执行 / 2 参数错 / 3 框架错 / 130 中断 | `src/atp/exit_codes.py` | 区分「测试失败」与「没跑起来」 |
| legacy 支持 `--list`（只解析）、`--case`（按 ID 选）、`-j`（模块级并发） | `cli.py:240-288,812-819` | 自检与筛选可用 |
| 密钥从进程环境变量读，优先于 `.env` | `src/atp/config.py:34-68` | 平台注入环境变量即可 |
| 平台字段是 `dev_server_script`（不是 dev_script），启动入口在 `routes/workspaces/execution.rs:76-126` | 同左 | 预览环境复用这套 |
| **被测地址目前靠前端正则扫日志识别**，后端不知道 | `packages/web-core/src/shared/hooks/usePreviewUrl.ts:253-310` | 后端要能拿到地址，需要新机制 |
| 测试阶段现状：技能 `atp-run` 产出 `test-report.json`，关卡 `all_cases_passed`，失败按 attribution 回流 | `template.rs:255-260`、`gates.rs:192-261` | 结构不变，只换执行内容 |
| 模板 `checks` 已能以脚本方式在同会话跑外部命令并收产物 | `engine.rs:329-362`、`launcher.rs:58-65,133-160` | 新增「跑命令收产物」几乎零成本 |
| 「测试」页签只展示用例/状态/归因/说明四列，无耗时、无报告链接 | `artifactParsers.ts:140-186`、`ArtifactSection.tsx:109-140` | 本期扩展报告结构需同步契约与前后端 |

## 4. 方案

### 4.1 被测地址：约定端口 + 日志兜底

平台为测试阶段准备被测服务，地址按以下顺序确定：

1. **约定端口（主路径）**：平台分配一个空闲端口，启动 `dev_server_script` 时注入 `PORT` 与 `VK_BASE_URL` 两个环境变量；仓库脚本按 `PORT` 监听。地址即 `http://127.0.0.1:<PORT>`。
2. **日志兜底**：脚本忽略 `PORT` 时，把前端 `usePreviewUrl` 的地址识别正则搬到后端（抽成共用纯函数并单测），从 dev server 日志前若干行提取地址。
3. **就绪判定**：对地址做 HTTP 探测，最多等 60 秒（可在模板里配）；超时则本次接口路判定为「未就绪」，报告里如实写明，不伪装成通过。

仓库没有 `dev_server_script` 时跳过接口路，只走仓库路。

### 4.2 接口路：在工作区里搭一个 atp 项目根

atp 旧 CSV 依赖「项目根 + `test_cases/<模块>/<模块>.csv` + `test_results/`」的目录约定，而产出物目录里只有一个扁平的 `test-cases.csv`。所以测试阶段在工作区里现搭一份：

```
<工作区根>/.vk/runs/<simple_id>/atp/
  test_cases/<模块>/<模块>.csv     ← 由 test-cases.csv 按 ModelName 分组生成
  keymap/                          ← 若产出物里有 keymap 则一并放入
  test_results/                    ← atp 输出，results.json 与结果 CSV 在此
```

模块名取 CSV 的 `ModelName` 列；该列为空时统一放进 `default` 模块。

执行：`atp legacy run --all --base-url <地址> --out test_results`，工作目录为上面的 atp 根。

**atp 命令怎么来**：新增本地配置项 `atp_command`（默认 `atp`），设置页可填，例如 `uv run --project /path/to/atp atp`。平台把它通过环境变量 `VK_ATP_CMD` 传给技能。命令不可用时跳过接口路，并在报告里注明原因——不静默当成通过。

### 4.3 仓库路：跑仓库自身测试命令

沿用 U3 最小版的做法：优先用 `plan.md` 里写明的测试命令，否则按 `package.json` → `Cargo.toml` → `pyproject.toml` 推断。真跑，不许编造。

### 4.4 合并与归因

`atp-run` 技能负责合并两路结果，产出一份 `test-report.json`：

- 接口路：从 `test_results/results.json` 取每条用例（case_id、title、module、status、duration_s、message），归因参考同目录结果 CSV 的 `RootCause` 列。
- 仓库路：从测试命令输出解析用例与结果。
- `total` / `passed` / `failed` 为两路之和；任一路「没跑起来」（命令缺失、环境未就绪、退出码 2/3）都要作为一条 failed 用例如实记入，并在 `message` 写明原因。
- 归因仍是 `code` / `case` 二选一，判不准写 `code`（保持现有回流语义：`case` 会退回用例设计并重新过人工关卡）。

### 4.5 报告结构扩展（需同步契约）

`test-report.json` 的用例项新增三个可选字段，均向后兼容（旧报告不带也能解析）：

| 字段 | 含义 |
|---|---|
| `duration_s` | 用例耗时（秒） |
| `module` | 模块名（atp 的 `ModelName`，仓库路可为空） |
| `source` | `atp` 或 `repo`，标明来自哪一路 |

顶层新增可选 `report_path`：atp 的 HTML 报告相对路径，界面上给一个「查看完整报告」的链接。

三处同改并保持一致：契约文档 §4 → Rust `gates.rs` 的解析结构 → 前端 `artifactParsers.ts`。界面「测试」页签相应增加耗时列、来源标记与报告链接。

## 5. 不做

- **Web / App 端自动化**：atp 当前没有这个能力，属于 atp 自身的演进（架构文档里的 A2.4），不在本期。
- 跨端组合场景、定时回归、测试环境密钥托管。
- 不改 atp 的结果结构去迁就平台——平台做适配，保持 atp 旧 CSV 与结果目录的兼容承诺。

## 6. 测试

- **纯函数**：地址识别正则、CSV 按模块分组、results.json → test-report.json 的映射、两路合并与计数、归因判定，全部抽成纯函数单测（Rust 侧与技能侧各自覆盖）。
- **引擎**：测试阶段能拉起 dev server、注入端口、就绪探测超时的行为，用假启动器覆盖。
- **端到端**：qa-mode 下模拟器继续产出合规报告，现有 e2e 不受影响；新增一条「接口路未就绪时报告如实标注失败」的用例。
- **真实验收（人工）**：用一个带 HTTP 接口的小样例仓库，走完整流水线，确认 atp 真跑、报告合并、界面展示耗时与报告链接；再用纯函数需求确认仓库路照常工作。

## 7. 验收标准

1. 有接口的需求：测试阶段自动拉起被测服务，atp 真实执行 CSV 用例，报告含每条用例的状态与耗时，界面可点开完整 HTML 报告。
2. 纯函数需求：跳过接口路，跑仓库测试命令，报告同样合规，流水线不中断。
3. 任一路没跑起来时，报告如实标为失败并写明原因，不会被误判为通过。
4. 失败归因为用例问题时退回用例设计并重新过人工关卡；否则退回开发。
5. `cargo test --workspace`、前端单测、e2e、clippy、格式检查全绿。
