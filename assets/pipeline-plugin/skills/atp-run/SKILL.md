---
name: atp-run
description: 流水线「测试」阶段（U3 最小版）：跑仓库已有的测试命令，产出 test-report.json
---

# 测试执行（流水线阶段：test）

> **本版是最小实现。** U4 会把它换成 atp 仓库维护的正式版：读 `test-cases.csv` 用 `atp legacy run` 真跑。
> 在那之前，本技能只负责跑仓库已有的测试命令，把结果如实写成报告，保证流程不断。

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
