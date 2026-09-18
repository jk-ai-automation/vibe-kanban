# 流水线前后端契约（计划 A 与计划 B 共用）

> 设计文档：`docs/superpowers/specs/2026-09-18-personal-pipeline-design.md`
> 计划 A（后端引擎）：`docs/superpowers/plans/2026-09-18-pipeline-engine.md`
> 计划 B（界面 + 端到端）：`docs/superpowers/plans/2026-09-18-personal-ui.md`
>
> 本文件是两份计划之间唯一的接口约定。任何一方要改名、改字段，先改这里，再改两份计划。
>
> **修订记录**
> - 2026-09-18 计划 A 核实代码后补充（名称与字段不变）：C1 快照接口响应形状；C2 `i64` 字段在 TS 里是 `number`；C3 develop 的 checks 定义；C4 路由表参数名；C5「未结束」与暂停/继续/取消的状态约束；C6 `stages` 排序；C7 `execution_process_id` 含义；C8 `IssueArtifact` 平铺的 ts 属性；C9 模拟器流水线模式。详见计划 A §3。

## 1. 类型（Rust 定义，ts-rs 生成到 `shared/types.ts`）

定义位置：`crates/db/src/models/pipeline.rs`（模型与视图类型同文件，按 `db::models::repo::Repo` 的写法 derive `Serialize, Deserialize, TS`），在 `crates/server/src/bin/generate_types.rs` 的 `generate_types_content()` 里注册 `db::models::pipeline::<类型>::decl()`。所有枚举 `#[serde(rename_all = "snake_case")]`、`#[ts(use_ts_enum)]` 不用，保持字符串联合类型。

**（C2）** 下面所有 `i64` 字段（`template_version`、`attempt`、`version`、`max_rounds`）都加 `#[ts(type = "number")]`，在 `shared/types.ts` 里是 `number`，不是 ts-rs 默认的 `bigint`。时间字段在 TS 里是 ISO 字符串。

```rust
pub enum PipelineStageKey { Requirement, Spec, TestDesign, Develop, Review, Test, Deliver }
pub enum PipelineRunStatus { Running, WaitingGate, Paused, Failed, Completed, Cancelled }
pub enum PipelineStageStatus { Pending, Running, WaitingGate, Passed, Rejected, Failed, Skipped }
pub enum GateKind { Human, Auto, None }
pub enum GateDecisionKind { Approve, Reject }
pub enum ArtifactKind { Requirement, Spec, Plan, TestCases, TraceMatrix, Review, TestReport, DeliveryReport }

pub struct PipelineRun {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub template_key: String,
    pub template_version: i64,
    pub status: PipelineRunStatus,
    pub current_stage_key: PipelineStageKey,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

pub struct PipelineStageRun {
    pub id: Uuid,
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub stage_key: PipelineStageKey,
    pub attempt: i64,
    pub status: PipelineStageStatus,
    pub gate_kind: GateKind,
    pub session_id: Option<Uuid>,
    pub execution_process_id: Option<Uuid>,  // （C7）启动时是会话首个进程（可能是 setup），智能体结束后改为智能体进程，检查脚本运行期间指向检查进程
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub error: Option<String>,               // 失败原因或打回意见
}

pub struct PipelineGateDecision {
    pub id: Uuid,
    pub stage_run_id: Uuid,
    pub decision: GateDecisionKind,
    pub comment: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: DateTime<Utc>,
}

pub struct IssueArtifactSummary {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub stage_run_id: Uuid,
    pub kind: ArtifactKind,
    pub rel_path: String,
    pub version: i64,
    pub truncated: bool,
    pub created_at: DateTime<Utc>,
}

pub struct IssueArtifact {
    #[serde(flatten)] #[sqlx(flatten)]   // （C8）ts-rs 分支默认 serde-compat，读 serde 的 flatten 即平铺，不必再写 #[ts(flatten)]
    pub summary: IssueArtifactSummary,
    pub content: String,
}

pub struct PipelineTemplateStageView {
    pub key: PipelineStageKey,
    pub skill: String,
    pub gate_kind: GateKind,
    pub gate_label: Option<String>,   // 人工关卡中文名，如「需求确认」
    pub max_rounds: i64,
}

pub struct PipelineTemplateView {
    pub key: String,
    pub version: i64,
    pub stages: Vec<PipelineTemplateStageView>,
}

pub struct IssuePipelineView {
    pub run: PipelineRun,
    pub stages: Vec<PipelineStageRun>,          // 全部尝试，按创建顺序（C6：已启动者即 started_at 升序，pending 排最后）
    pub decisions: Vec<PipelineGateDecision>,   // 全部关卡决策，按 decided_at 升序
    pub artifacts: Vec<IssueArtifactSummary>,   // 每个 kind 的全部版本
    pub template: PipelineTemplateView,
}

pub struct StartPipelineRepo { pub repo_id: Uuid, pub target_branch: String }

pub struct StartPipelineRequest {
    pub repos: Vec<StartPipelineRepo>,
    pub executor_config: ExecutorConfig,        // 与 create_and_start_workspace 相同的类型
    pub template_key: Option<String>,           // 缺省 "standard"
}

pub struct GateDecisionRequest {
    pub decision: GateDecisionKind,
    pub comment: Option<String>,               // reject 时必填，后端校验非空
}

pub struct PendingPipelineItem {
    pub run: PipelineRun,
    pub stage_run: PipelineStageRun,           // 当前等待或失败的那一条
    pub issue_simple_id: String,
    pub issue_title: String,
}
```

## 2. 接口

全部挂在 `crates/server/src/routes/local_projects/` 的 `/local` 下（因此完整前缀是 `/api/local`），并登记进该模块的 `RouteRegistry` 契约表。响应统一用现有的 `ApiResponse<T>` 包装。

| 方法 | 路径 | 请求 | 响应 `data` | 错误 |
|---|---|---|---|---|
| POST | `/api/local/issues/{issue_id}/pipeline` | `StartPipelineRequest` | `IssuePipelineView` | 404 需求不存在；409 已有未结束的运行 |
| GET | `/api/local/issues/{issue_id}/pipeline` | — | `IssuePipelineView \| null`（没有运行时为 null） | 404 需求不存在 |
| GET | `/api/local/pipeline/artifacts/{artifact_id}` | — | `IssueArtifact` | 404 |
| POST | `/api/local/pipeline/stage-runs/{stage_run_id}/gate` | `GateDecisionRequest` | `IssuePipelineView` | 409 该阶段不在 `waiting_gate`；400 打回缺意见 |
| POST | `/api/local/pipeline/runs/{run_id}/pause` | — | `IssuePipelineView` | 409 状态不允许 |
| POST | `/api/local/pipeline/runs/{run_id}/resume` | — | `IssuePipelineView` | 409 状态不允许 |
| POST | `/api/local/pipeline/runs/{run_id}/cancel` | — | `IssuePipelineView` | 409 已结束 |
| GET | `/api/local/pipeline/pending?project_id={uuid}` | — | `PendingPipelineItem[]`（`waiting_gate` 与 `failed`，按等待时长降序） | — |

`project_id` 参数可省略，省略时返回全部项目。

**（C4）** 路径参数在后端路由表里一律登记为 `{id}`（例：`/api/local/issues/{id}/pipeline`），因为 axum 要求同一位置参数同名，而 `/api/local/issues/{id}` 已存在。前端拼出的 URL 不变。

**（C5）** 状态约束：
- 「未结束的运行」= `status` 不是 `completed` / `cancelled`。`failed` 可以继续，算未结束；要对同一需求重新启动，先取消。
- `pause`：只允许 `running`、`waiting_gate`；正在跑的进程不杀，阶段结束后不再调度（下一阶段建成 `pending`）。
- `resume`：只允许 `paused`、`failed`。`failed` 继续时在失败的阶段开一次新尝试，上次失败原因进提示词。
- `cancel`：允许所有未结束状态；最后一条尝试若是 `pending`/`running`/`waiting_gate` 则置 `skipped`。
- `gate`：`reject` 的 `comment` 去掉首尾空白后不能为空。

## 3. 实时推送

- `pipeline_runs`、`pipeline_stage_runs` 两表加入 `HookTables`；JSON Patch 路径分别是 `/pipeline_runs/{id}`、`/pipeline_stage_runs/{id}`，值为上面的 `PipelineRun` / `PipelineStageRun`。
- 两者都走现有 `GET /api/issues/streams/ws?project_id=`，首帧对这两个路径各发一次 replace（全量，按 project_id 过滤），之后增量。
- 前端在 `shared/lib/local/localEndpoints.ts` 里新增两个集合名 `pipeline_runs`、`pipeline_stage_runs`，REST 快照分别取：
  - `GET /api/local/pipeline_runs?project_id=` → `{ "pipeline_runs": PipelineRun[] }`
  - `GET /api/local/pipeline_stage_runs?project_id=` → `{ "pipeline_stage_runs": PipelineStageRun[] }`
  （这两条是给集合机制用的快照接口，同样挂在 `/local` 下。**（C1）** 与其它本地快照接口一致：**不包 `ApiResponse`**，返回 `{ "<表名>": 行数组 }`，因为 `extractFallbackRows`（`packages/web-core/src/shared/lib/electric/rows.ts`）读的是 `payload[table]`。）
- 产出物、关卡决策不推送。前端在看到对应 stage run 变化后重新 `GET /api/local/issues/{id}/pipeline`。

## 4. 产出物文件名与 kind 对应

| 阶段 | 文件名（位于 `<工作区目录>/.vk/runs/<simple_id>/`） | kind |
|---|---|---|
| requirement | `requirement.md` | requirement |
| spec | `spec.md`、`plan.md` | spec、plan |
| test_design | `test-cases.csv`、`trace-matrix.md` | test_cases、trace_matrix |
| review | `review.json` | review |
| test | `test-report.json` | test_report |
| deliver | `delivery-report.md` | delivery_report |

develop 阶段不产出文件，关卡看 `checks`。**（C3）** `repos` 表没有 lint/test 脚本字段，`checks` 定义为模板阶段里的 shell 命令列表（`.vibe/pipeline.yaml` 中 `checks: ["pnpm run lint", "cargo test"]`），内置模板为空。`checks_passed` = 智能体进程退出码 0，且（有 checks 时）检查脚本退出码 0。检查脚本在同一会话里以 `run_reason = "pipelinestep"` 的执行进程运行；阶段智能体进程本身仍是 `codingagent`。

`review.json` 最小结构：`{"findings":[{"severity":"blocker|major|minor","file":"...","line":1,"message":"..."}]}`；`no_blocking_findings` = 没有 `severity == "blocker"`。

`test-report.json` 最小结构：`{"total":n,"passed":n,"failed":n,"cases":[{"id":"...","status":"passed|failed","attribution":"code|case|null","message":"..."}]}`；`all_cases_passed` = `failed == 0 && total > 0`。失败时：任一 `attribution == "case"` → 回到 test_design（人工关卡）；否则 → 回到 develop。

## 5. qa-mode 模拟执行器约定

提示词中固定出现两行（引擎拼装，模拟器解析）：

```
产出物目录（绝对路径）：/abs/path/.vk/runs/ISS-12
必须产出：requirement.md, spec.md
```

模拟器看到这两行时，在目录里写出每个文件的占位内容：`.md` 写一段中文标题与说明；`.csv` 写 atp 旧 CSV 表头加一行；`review.json` 写 `{"findings":[]}`；`test-report.json` 写 `{"total":3,"passed":3,"failed":0,"cases":[...]}`。

需求标题里包含以下标记时，模拟器改变行为（仅供端到端测试）：

| 标记 | 行为 |
|---|---|
| `[qa:review-blocker-once]` | 第一次评审产出一个 blocker，第二次起为空 |
| `[qa:test-fail-code-once]` | 第一次测试失败且归因 code，之后全过 |
| `[qa:test-fail-case-once]` | 第一次测试失败且归因 case，之后全过 |
| `[qa:always-fail-review]` | 评审每次都有 blocker（用来测轮次用尽） |

「第一次」按产出物目录里是否已存在同名文件的旧版本来判断（模拟器把旧文件改名为 `*.prev` 再写新文件）。

**（C9）** 模拟器识别到上面两行（目录必须是绝对路径）即进入流水线模式：只写产出物，**不做**原来的随机删改文件；日志每行间隔 0.1 秒（原 1 秒）。标记从提示词的「需求：」行读取（引擎把 `{simple_id} {标题}` 写在这一行，标题压成单行）。提示词常量与解析函数在 `crates/executors/src/pipeline_prompt.rs`，引擎与模拟器共用。
