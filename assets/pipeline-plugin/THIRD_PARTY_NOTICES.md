# 第三方内容出处与许可证

本目录是 vibe-kanban 自带的 Claude Code 插件 `vk-pipeline`。`skills/` 下的技能分三类：
平台自写（`vk-*`、`atp-run`，随本仓库演进）、自有仓库同步（`prd2testcase`，来自 atp 仓库）、
第三方引入（superpowers）。后两类都由 `scripts/sync-pipeline-skills.mjs` 按 `skills.lock.json`
同步，**逐字节不改**；要改请先改上游，再更新锁文件。本文件的「第三方」只指 superpowers 一节，
atp 一节记的是自有仓库的出处，不涉及第三方授权。

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

## atp（自有仓库，非第三方）

- 来源：https://github.com/jk-ai-automation/atp.git
- 提交：`10feaa1ea6e99719e88435d71244d851047779ed`（分支 `skills-prd2testcase`，PR
  https://github.com/jk-ai-automation/atp/pull/5，**已合入 atp 主干（PR #5）**；合入后把锁文件里的
  `commit` 改成主干提交号并重跑 `node scripts/sync-pipeline-skills.mjs --update`）
- 许可证：内部仓库，与本仓库同一所有方，不涉及第三方授权
- 引入的技能：`prd2testcase`（文件白名单只有 `SKILL.md`）
- 为什么放在 atp 仓库：旧版 CSV 的 46 列定义由 atp 维护（`src/atp/legacy_csv/columns.py`），
  技能正文里的列说明与格式自检命令必须跟着列定义走，所以技能与格式同仓维护，这里只锁提交号与哈希。

## 规格模板结构

`skills/vk-spec/SKILL.md` 的规格文档结构（问题 / 方案 / 用户故事 / 实现决策 /
测试决策 / 不做）借鉴自 mattpocock 的 `to-spec` 技能（MIT）。只吸收结构，正文为平台自写，
未复制任何文件，因此不在 `skills.lock.json` 里。
