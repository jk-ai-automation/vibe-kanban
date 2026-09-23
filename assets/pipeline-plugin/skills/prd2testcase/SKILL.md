---
name: prd2testcase
description: 由需求与规格生成 atp 旧版 CSV 测试用例与 AC 追踪矩阵
---

# 需求转测试用例（流水线阶段：test_design）

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

## 做法

分三步，**每一步都要写下来再进下一步**；写完再做第四步的格式自检。

### 第一步：需求分析

读「产出物目录」里的 `requirement.md` 与 `spec.md`。把每条验收标准（`AC-1`、`AC-2`…）拆成测试点：

- 正例：AC 描述的主路径。
- 反例：前置条件不满足、输入非法、越权。
- 边界：空、最大、最小、重复、并发。

每个测试点记下它覆盖的 AC 编号。`spec.md` 的「测试决策」表已经给了每条 AC 怎么验证、在哪一层，
与它冲突时以 `spec.md` 为准，并在 `trace-matrix.md` 里写明差异。

### 第二步：测试方案

给每个测试点定三件事：

- **端**：接口（HTTP）还是界面（Web）。本技能只产出**接口**用例；界面用例写进 `trace-matrix.md` 的「本次不覆盖的端」。
- **优先级**：`P0`（主流程，坏了就不可用）、`P1`（重要分支）、`P2`（边界与体验）。
- **场景归属**：需要多步串起来的（登录 → 下单 → 查询）归到同一个 `ScenarioName`，用 `Steps` 表示步内顺序；
  单步用例 `ScenarioName` 留空、`Steps` 填 1。步与步之间靠 `StoreOutput` 存变量、下一步用 `${变量名}` 引用。

### 第三步：写用例

按下面「产出物格式」写 `test-cases.csv`，再写 `trace-matrix.md`。

### 第四步：自检

`atp legacy run --list` 只加载解析、不发任何请求，是最省事的格式检查。它要求的目录布局是
`<项目根>/test_cases/<模块名>/<模块名>.csv`，所以在**产出物目录之外**建一个临时目录跑：

```bash
TMP=$(mktemp -d)
mkdir -p "$TMP/test_cases/vk_gen"
cp <产出物目录>/test-cases.csv "$TMP/test_cases/vk_gen/vk_gen.csv"
cd "$TMP" && atp legacy run --list
```

预期输出第一行是模块名 `vk_gen`，后面每行 `  <CaseID>  <StepSummary>`，行数等于用例条数
（同一 CaseID 的多个步骤只列一次）。报错就按报错改 CSV 再跑一次，跑通为止。跑完删掉 `$TMP`。

**降级顺序**（前一种不可用才试下一种，用哪一种都要记进 `trace-matrix.md` 的「自检」一节）：

1. `atp` 在 PATH 上 → 直接 `atp legacy run --list`。
2. `atp` 不在 PATH，但本机有 atp 仓库检出且装了 `uv` → `cd "$TMP" && uv run --project <atp 仓库路径> atp legacy run --list`
   （例：`uv run --project /Users/admin/work/github/atp atp legacy run --list`）。
3. 两者都没有 → **跳过 atp 自检**，改用只依赖 `python3` 的列数检查，每一行都必须是 46：

   ```bash
   python3 -c 'import csv,sys
   for i, row in enumerate(csv.reader(open(sys.argv[1], newline="", encoding="utf-8")), 1):
       if len(row) != 46: print(f"第 {i} 行 {len(row)} 列，应为 46")' <产出物目录>/test-cases.csv
   ```

   没有任何输出即通过。
4. 连 `python3` 都没有 → 逐行人工核对列数。

跳过第 1、2 种时，在 `trace-matrix.md` 末尾写明「未做 atp 格式自检：<原因>」。

## 产出物格式

### `test-cases.csv`

第一行必须是下面这 46 列，**逐字照抄、顺序不变**（这是旧版表头。第 3 列 `ModelName` 是旧版的拼写，
`atp` 靠别名兼容，**不要**改成 `ModuleName`；`stepkw` 全小写、`multi_procs#` 带井号，也都不要改）：

```
CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario
```

文件用 UTF-8，不写 BOM，每行字段数**必须等于 46**（用不到的列留空，但逗号一个都不能少）。

#### 必填的两列

| 列 | 怎么填 |
|---|---|
| `CaseID` | `<模块名>_<三位序号>`，例 `vk_gen_001`。同一个 CaseID 的多行＝同一条用例的多个步骤，按出现顺序执行 |
| `stepkw` | 步骤关键字，**不能为空**——为空这一步不发请求、直接记 SKIP。旧用例写成 `<鉴权用例>_<方法>`（如 `api_auth_002_POST`），新写用例填一个简短标识即可，如 `api_GET` / `api_POST` |

#### 描述与分组

| 列 | 怎么填 |
|---|---|
| `ScenarioName` | 多步串联的用例填同一个场景名；单步用例留空。同场景的用例共享变量，按 `Steps` 顺序执行 |
| `ModelName` | 模块标识，只用于展示与结果分组。填模块名（与 CSV 文件名一致）或被测接口路径都可以 |
| `Priority` | `P0` / `P1` / `P2` |
| `Steps` | 场景内的步序号，从 1 起；单步用例填 1 |
| `StepSummary` | 一句话中文，说明这一步干什么。`--list` 打印的就是它 |

#### 请求怎么发

| 列 | 怎么填 |
|---|---|
| `Precondition` | **HTTP 方法**（旧版把方法放在这一列）：`GET` / `POST` / `PUT` / `DELETE`；留空按 `GET` |
| `StepInput` | 请求 URL。优先写成 `$base_url/<路径>`，`$base_url` 由运行时的 `--base-url` 替换；也可写完整 URL。可用 `${变量名}` 引用前面 `StoreOutput` 存下的值 |
| `Data` | 请求体。`{…}` 或 `[…]` 成对包裹时按 JSON 发送；`k=v&k2=v2` 按原样字符串发送；GET 留空 |
| `Header` / `Cookie` | 需要时填素材文件名（放在用例目录或 `<项目根>/cookies/` 下），否则留空。**不要写绝对路径** |
| `FollowRedirects` | 默认跟随重定向；只有明确写否定词（如 `N`）才不跟随。一般留空 |

#### 断言

| 列 | 怎么填 |
|---|---|
| `ExpectedResult` | 期望的 HTTP 状态码，如 `200`、`201`、`403`。留空表示不校验状态码 |
| `ExpectedOutput` | 响应体里必须出现的内容。默认是**包含**匹配；用半角空格分隔多个词表示**全部都要出现**（AND）；`regex:` 前缀走整串正则；`mixed:` 前缀写多条规则且全部满足。期望非空而实际响应为空直接判失败 |
| `ExcludedOutput` | 响应体里**不能**出现的内容，命中即失败。空格分隔的多个词是**任一命中即失败**（OR，与 `ExpectedOutput` 相反）；同样支持 `regex:` 与 `mixed:`。没有就留空 |
| `OutputMatchStrategy` | 匹配策略，`exact`（整串相等）或 `regex`（整串正则）。留空即默认的包含匹配 |

#### 变量与参数化

| 列 | 怎么填 |
|---|---|
| `StoreOutput` | 从响应里取值存成变量，`;` 分隔多条 `名=提取式`。提取式前缀：`$.` / `$[` 走 JSON 路径，`header:名` 取响应头，`cookie:名` 取 cookie，`value:字面量` 直接赋值，其余按正则。例 `orderId=$.data.orderId` |
| `ParamSets` | 参数化，`名=v1,v2;名2=v3,v4`，按键的首次出现顺序做笛卡尔积，每个组合跑一次。一般留空 |
| `IterationDataSrc` | 数据驱动的数据源文件，**不能与 `ParamSets` 同时使用**。一般留空 |

#### 执行控制

| 列 | 怎么填 |
|---|---|
| `IfFailedContinue` | 本步失败后是否继续同一用例的后续步骤：`Y` / `是` / `true` 继续，其余一律中止并把后续步骤记 SKIPPED。一般留空 |
| `Loop` | 本组重复执行的次数（非负整数）；留空按 1 次，填 `0` 表示一次都不跑 |
| `waittime` | 两轮 `Loop` 之间等待的秒数；留空不等待 |
| `multi_procs#` | 并发进程数。流水线里**一律留空**（串行更好定位失败） |
| `NOAUTO` | 非空表示这一步是人工用例、不自动执行。自动化用例留空 |
| `PreAction` / `TakeAction` / `TeardownAction` / `PostAction` | 主步骤前后要执行的关键字 id（来自 `keymap/`）。`PreAction` / `TakeAction` 失败会跳过主步骤并判失败，`TeardownAction` / `PostAction` 失败只记录。没有就留空 |
| `PreActionStore` / `PostActionStore` | 从上述动作的输出里取值存成变量，写法同 `StoreOutput`。没有就留空 |
| `SoftwareVersion` / `MobileAppVersion` | 被测版本号，只用于报告展示。没有就留空 |

#### 一律留空的结果列（由执行器回填）

`ActualResult`、`ActualOutput`、`TestResult`、`RootCause`、`StoreOutputResult`、`ExecStartTime`、
`ResultUpdateTime`、`ActualReturn4Loop`、`ActualOutput4Loop`、`MultiProcResults`、`MultiProcSummary`、
`ActualReturn4LoopScenario`、`ActualOutput4LoopScenario`——**写了也会被覆盖，一律留空**。

#### 转义

字段里有逗号、双引号或换行时，按 CSV 规范用双引号把整个字段包起来，字段内部的双引号写两个。
例：期望响应里出现 `"status":"ok"`，这一格要写成 `"""status"":""ok"""`。

#### 完整示例（表头 + 3 行，每行都是 46 个字段）

```csv
CaseID,ScenarioName,ModelName,Priority,Steps,stepkw,StepSummary,Precondition,StepInput,ExpectedResult,ExpectedOutput,ExcludedOutput,ActualResult,ActualOutput,TestResult,RootCause,ParamSets,StoreOutput,StoreOutputResult,Header,Cookie,Data,IfFailedContinue,TakeAction,TeardownAction,PreAction,PreActionStore,PostAction,PostActionStore,Loop,waittime,multi_procs#,OutputMatchStrategy,FollowRedirects,SoftwareVersion,MobileAppVersion,IterationDataSrc,ExecStartTime,ResultUpdateTime,ActualReturn4Loop,ActualOutput4Loop,NOAUTO,MultiProcResults,MultiProcSummary,ActualReturn4LoopScenario,ActualOutput4LoopScenario
vk_gen_001,,vk_gen,P0,1,api_GET,健康检查返回 200,GET,$base_url/health,200,"""status"":""ok""",,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
vk_gen_002,下单,vk_gen,P0,1,api_POST,创建订单返回订单号,POST,$base_url/api/orders,201,orderId,,,,,,,orderId=$.data.orderId,,,,"{""sku"":""A1"",""qty"":2}",,,,,,,,,,,,,,,,,,,,,,,,
vk_gen_003,下单,vk_gen,P1,2,api_GET,按订单号查询订单,GET,$base_url/api/orders/${orderId},200,A1,error,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
```

第 1 行是单步用例（`ScenarioName` 留空）；第 2、3 行是同一个场景 `下单` 的两步，
第 2 步用 `StoreOutput` 存下 `orderId`，第 3 步在 URL 里用 `${orderId}` 引用它。

### `trace-matrix.md`

```markdown
# 追踪矩阵

| AC 编号 | 测试点 | CaseID | 优先级 |
|---|---|---|---|
| AC-1 | 正常下单返回 201 | vk_gen_002 | P0 |

## 未覆盖
> **AC-3**：<为什么这次覆盖不了>

## 本次不覆盖的端
- <界面用例等；没有就写「暂无」>

## 自检
- 命令：`atp legacy run --list`
- 结果：<列出的用例条数>，或「未做 atp 格式自检：<原因>」

## 待澄清
- <需要人拍板的；没有就写「暂无」>
```

硬要求：

- `requirement.md` 里**每一条** AC 都要在矩阵里出现一行；覆盖不了的进「未覆盖」，不许悄悄漏掉。
- 「未覆盖」一节里每条都用 `> **AC-x**：<原因>` 的引用块写（醒目等同于标红）。全覆盖时写 `> 暂无`。
- 矩阵里的每个 `CaseID` 都必须在 `test-cases.csv` 里真实存在，拼写一字不差。
