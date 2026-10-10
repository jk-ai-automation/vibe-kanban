---
name: atp-run
description: 流水线「测试」阶段：接口用例用 atp 真跑、仓库测试命令真跑，两路结果合并成 test-report.json
---

# 测试执行（流水线阶段：test）

两路都要真跑，**绝不能编造结果**；任何一路没跑起来都要如实记成失败用例。

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

## 平台给的三行

除了上面的固定行，本阶段的提示词还有三行：

- `被测地址：` 三种取值——
  - 一个地址（如 `http://127.0.0.1:43127`）：被测服务已由平台拉起并确认可访问，**不要自己再起服务**。
  - `无（原因）`：正当跳过接口路（没有可自动化用例，或仓库没配开发服务器脚本）。只跑仓库路，不记失败。
  - `未就绪（原因）`：该跑但服务没起来。接口路记**一条**失败用例（见下），然后照常跑仓库路。
- `atp 项目根：` 平台搭好的目录（里面是 `test_cases/vk_gen/vk_gen.csv`，上一轮的 `test_results/` 已清空）；接口路被跳过时是 `无`。
- `atp 命令：` 调用 atp 的命令，可能带空格（如 `uv run --project /opt/atp atp`），原样拼在前面用，**不要加引号包成一个词**。

## 接口路（被测地址是一个地址时才做）

1. 进到「atp 项目根」，执行：

   ```bash
   cd "<atp 项目根>" && <atp 命令> legacy run --all --base-url "<被测地址>" --out test_results --html-report
   ```

   - 必须在项目根下执行：atp 以当前目录为项目根，从 `./test_cases/` 读用例，`--out` 也必须落在项目根内。
   - `--html-report` 在新版 atp 里接受但不生效，HTML 报告默认就会生成；**不要加 `--no-report`**。
   - 记下退出码与输出末尾。输出里会有 `HTML 报告：<路径>` 与 `结果文件：<路径>` 两行。
   - **不要改 `test_cases/` 里的 CSV**，那是人工确认过的用例。

2. 按退出码处理：

   | 退出码 | 含义 | 做法 |
   |---|---|---|
   | 0 | 全部通过 | 读结果 |
   | 1 | 有用例失败/出错，或一条都没执行 | 读结果 |
   | 2 | 用法或配置错误（参数错、用例里有 `$base_url` 却没给地址等） | 接口路记一条失败用例（见下） |
   | 3 | 框架错误（用例目录缺失、CSV 无效、模块级异常） | 接口路记一条失败用例；`results.json` 若已生成，照样读出来一并计入 |
   | 127 等其它 | 命令不存在或被中断 | 接口路记一条失败用例 |

   「接口路记一条失败用例」：`id` 写 `service-not-ready`，`attribution` 写 `code`，`message` 写「atp 没跑起来：退出码 N」加输出末尾几行，`duration_s`、`module` 写 `null`，`source` 写 `"atp"`。

3. 读 `test_results/results.json`。结构是 `{"run_id", "env", "started_at", "finished_at", "cases": [...], "summary": {...}}`，`cases` 里每条有 `case_id`、`title`、`module`、`status`、`duration_s`、`message`。`status` 只有四种取值：`passed` / `failed` / `error` / `skipped`。映射成报告用例：

   | 报告字段 | 取值 |
   |---|---|
   | `id` | `case_id` |
   | `status` | `passed` → `passed`；`failed`、`error` → `failed`；`skipped` 见下 |
   | `message` | 通过时写 `""`；失败时写 atp 的 `message`（末尾那行「步骤：总 …」可以留着） |
   | `duration_s` | 原样（数字） |
   | `module` | 原样（这里是 `vk_gen`） |
   | `source` | `"atp"` |

   `skipped` 的用例要到结果 CSV（`test_results/vk_gen/vk_gen_result.csv`，每个步骤一行）里看该 `CaseID` 的行：
   - 所有行的 `NOAUTO` 都非空（`RootCause` 为 `Manual test step skipped, NOAUTO='…'`）：人工用例，atp 本就不执行，**不计入报告**。
   - 其它情况（如 `RootCause` 为「同场景的前序用例失败，场景中止」，或 `stepkw` 为空被跳过）：计为 `failed`，`message` 写 `RootCause` 原文。

   `error` 是请求本身没成（连不上、超时等），`failed` 是收到了响应但断言不符；两者在报告里都记 `failed`。

4. 给每条失败用例判归因。依据是结果 CSV 里该用例的 `TestResult`、`RootCause`、`ActualResult`、`ActualOutput` 列，以及 `test_results/vk_gen/<CaseID>/` 下抓下来的请求/响应文件：
   - `code`：服务返回的内容与期望不符（状态码不对、字段缺失、值不对）、服务报 5xx、连接被拒或超时。
   - `case`：用例自身的问题（请求写错了路径或参数名、期望值与需求/规格矛盾、引用了不存在的变量、`stepkw` 为空）。
   - 因前序用例失败而中止的，跟随前序用例的归因。
   - 判不准写 `code`。

5. 在 `test_results/` 下找 `test_report_*.html`（文件名形如 `test_report_20261002_101500.html`，有多个取最新的一个；也可直接用输出里 `HTML 报告：` 那一行的文件名）。找到就把顶层 `report_path` 写成 `atp/test_results/<文件名>`；没有就不写这个字段。

**被测地址是 `未就绪（原因）` 时**：不执行上面的命令，接口路只记一条：

```json
{ "id": "service-not-ready", "status": "failed", "attribution": "code", "message": "被测服务未就绪（原因原样抄过来）", "duration_s": null, "module": null, "source": "atp" }
```

## 仓库路（每次都做）

1. 读「产出物目录」里的 `plan.md`：任务里写了测试命令就用它。
2. 没写就按仓库类型推断，**按下表从上往下取第一个命中的**：

   | 命中条件 | 命令 |
   |---|---|
   | 根目录有 `package.json` 且 `scripts.test` 存在 | `pnpm run test`（没有 pnpm 就 `npm test`） |
   | 根目录有 `Cargo.toml` | `cargo test --workspace` |
   | 根目录有 `pyproject.toml` | `uv run pytest`（没有 uv 就 `pytest`） |
   | 以上都不命中 | 不跑命令，仓库路不产出用例 |

3. 真跑，留着退出码与输出末尾。从输出里解析出每条用例与结果，`source` 写 `"repo"`，`module` 写 `null`，`duration_s` 有就写、没有写 `null`。
4. 命令起不来（依赖缺失、命令不存在）时仓库路记一条失败：`{"id":"repo-tests-not-run","status":"failed","attribution":"case","message":"<原因>","duration_s":null,"module":null,"source":"repo"}`。
5. 失败用例的归因：
   - `code`：实现有问题（断言失败、异常、返回值不对）。
   - `case`：测试本身有问题（环境缺依赖、测试写错、期望值过时）。
   - 判不准写 `code`（回到开发阶段比回到用例阶段代价低）。

## 产出物格式

`test-report.json`，必须是合法 JSON，`cases` 是两路用例的合集：

```json
{
  "total": 3,
  "passed": 2,
  "failed": 1,
  "report_path": "atp/test_results/test_report_20261002_101500.html",
  "cases": [
    { "id": "vk_gen_001", "status": "passed", "attribution": null, "message": "", "duration_s": 0.42, "module": "vk_gen", "source": "atp" },
    { "id": "vk_gen_002", "status": "failed", "attribution": "code", "message": "期望 400，实际 200", "duration_s": 0.31, "module": "vk_gen", "source": "atp" },
    { "id": "add 对非数字入参抛错", "status": "passed", "attribution": null, "message": "", "duration_s": null, "module": null, "source": "repo" }
  ]
}
```

- `total` = `passed` + `failed` = `cases` 的条数；三个都是非负整数。
- `status` 只能是 `passed` / `failed`。
- `attribution` 在 `passed` 时必须是 `null`；`failed` 时必须是 `"code"` 或 `"case"`。
- `source` 只能是 `"atp"` 或 `"repo"`。
- 两路一条用例都没有时：`total` 写 0，`cases` 里放一条 `{"id":"no-tests","status":"failed","attribution":"case","message":"<为什么没跑成>","duration_s":null,"module":null,"source":"repo"}`。

关卡判定：`failed == 0 && total > 0` 才算通过。失败时任一 `attribution == "case"` 会回到用例阶段（人工关卡），否则回到开发阶段。
