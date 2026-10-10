# 流水线测试阶段接入真实执行（U4）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 测试阶段由平台自动拉起被测服务、用 atp 真跑 CSV 用例，并与仓库自身测试命令的结果合并成一份 `test-report.json`；任一路没跑起来都如实记为失败。

**Architecture:** 引擎进入测试阶段时先做确定性的准备（判断有没有可自动化用例 → 搭 atp 项目根 → 分配端口并起开发服务器），然后在**锁外**的后台任务里等服务就绪，就绪或超时后再回到锁内启动智能体；结果通过提示词里的三行固定文本（被测地址 / atp 项目根 / atp 命令）交给 `atp-run` 技能。技能负责真跑两路并合并报告，报告结构向后兼容地多三个用例字段和一个顶层 `report_path`，界面相应加列并提供 HTML 报告链接。

**Tech Stack:** Rust（tokio、sqlx、reqwest、regex、csv）、React + TypeScript（Vitest）、Playwright e2e、Claude Code 技能（Markdown）。

**设计文档:** `docs/superpowers/specs/2026-09-26-pipeline-test-stage-design.md`

---

## 0. 写计划时核实出的偏差（相对设计文档，实施以本节为准）

| 设计文档写法 | 实际做法 | 原因 |
|---|---|---|
| §4.2 按 `ModelName` 列分组成多个模块 | 整份 CSV 放进**一个**模块 `vk_gen`：`atp/test_cases/vk_gen/vk_gen.csv` | `prd2testcase` 技能明确 `ModelName`「只用于展示与结果分组，填模块名或被测接口路径都可以」，可能是 `/api/add` 这种路径，不能当目录名；`vk_gen` 与该技能自检时用的模块名一致 |
| §4.2 用环境变量 `VK_ATP_CMD` 把 atp 命令传给技能 | 写进提示词的固定行 `atp 命令：…` | 现有执行层没有「给智能体会话注入环境变量」的通道，提示词固定行是引擎与技能/模拟器之间已有的契约形式（`executors::pipeline_prompt`） |
| §4.2 keymap 一并放入 | 不做 | `prd2testcase` 不产出 keymap，产出物清单里没有它 |
| §4.1 就绪探测 | 在后台任务里做，不占引擎锁 | 引擎所有状态变更共用一把全局锁，在锁里等 60 秒会卡住所有需求的关卡、暂停、取消 |
| （新增）| CSV 里没有可自动化用例（所有行 `NOAUTO` 非空，与 atp 一致：去首尾空白、转小写后不是空 / `nan` / `none` 即算人工）时不起开发服务器，直接跳过接口路 | 纯函数需求不该白等一个服务 |
| （新增）| 多个仓库都配了开发服务器脚本时，只起**第一个** | 一个端口只能给一个服务；多服务编排不在本期 |
| 地址识别返回全部候选、localhost 同时探 IPv4/IPv6 | 审查发现：日志里先出现数据库地址会误判；Node 17+ 上 localhost 可能只绑 ::1 |
| U3 技能文件里写「U4 换成 atp 仓库维护的正式版」 | `atp-run` 仍由本仓库维护 | 它依赖平台提示词的固定行，放在 atp 仓库会形成跨仓库契约 |

**环境约束（必须遵守）:** 这台机器 8 核 8GB，**同一时间只允许一个 cargo 进程**；不要另建 worktree 或独立 `target`；长命令后台运行并定时看输出。

## 1. 文件结构

**新建**
- `crates/services/src/services/pipeline/test_env.rs` — 测试阶段的纯函数与数据类型：服务状态、地址识别、可自动化用例计数、atp 项目根搭建、空闲端口、HTML 报告路径校验。
- `docs/superpowers/plans/2026-10-02-pipeline-test-stage.md` — 本文件。

**修改**
- `crates/executors/src/pipeline_prompt.rs` — 三个固定行常量；解析「被测地址」；模拟器在「未就绪」时写失败报告。
- `crates/services/Cargo.toml` — 加 `regex`、`csv`。
- `crates/services/src/services/pipeline/mod.rs` — 挂 `test_env`。
- `crates/services/src/services/pipeline/prompt.rs` — 提示词多三行。
- `crates/services/src/services/pipeline/template.rs` — 阶段多 `ready_timeout_s`。
- `crates/services/src/services/pipeline/launcher.rs` — 启动器接口多四个方法及其容器实现。
- `crates/services/src/services/pipeline/test_support.rs` — 假启动器跟进。
- `crates/services/src/services/pipeline/engine.rs` — 测试阶段的准备、后台等待、收尾停服务、HTML 报告定位。
- `crates/services/src/services/pipeline/tests.rs` — 引擎用例。
- `crates/services/src/services/config/versions/v8.rs` — `pipeline_atp_command`。
- `crates/local-deployment/src/lib.rs` — 构造启动器时传入配置。
- `crates/server/src/routes/local_projects/pipeline.rs` — HTML 报告接口。
- `packages/web-core/src/pages/issue-detail/artifactParsers.ts`（及同目录测试）、`ArtifactSection.tsx` — 报告新字段与界面。
- `packages/web-core/src/entities/pipeline/api/pipelineApi.ts` — 报告链接地址。
- `packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx` — atp 命令输入框。
- `packages/web-core/src/i18n/locales/*/common.json`、`*/settings.json` — 文案。
- `assets/pipeline-plugin/skills/atp-run/SKILL.md` — 技能重写。
- `docs/superpowers/plans/2026-09-18-pipeline-contract.md` — 契约 §4、§5。
- `packages/e2e/support/api.ts`、`packages/e2e/tests/exceptions.spec.ts` — 新增端到端用例。
- `shared/types.ts` — 由 `pnpm run generate-types` 生成，不手改。

---

### Task 1: 提示词固定行与模拟器

**Files:**
- Modify: `crates/executors/src/pipeline_prompt.rs`

- [ ] **Step 1: 写失败的测试**

在文件末尾 `mod tests` 里追加：

```rust
    #[test]
    fn 解析被测地址行且只认第一次出现() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = format!(
            "{ARTIFACTS_DIR_LABEL}{}\n{REQUIRED_ARTIFACTS_LABEL}test-report.json\n{SERVICE_URL_LABEL}未就绪（60 秒内没有响应）\n{FEEDBACK_LABEL}上次\n{SERVICE_URL_LABEL}http://127.0.0.1:1",
            dir.path().display()
        );
        let parsed = parse_pipeline_prompt(&prompt).unwrap();
        assert_eq!(parsed.service.as_deref(), Some("未就绪（60 秒内没有响应）"));
    }

    #[test]
    fn 没有被测地址行时为空() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = 提示词(dir.path(), "spec.md", "VK-1 标题");
        assert_eq!(parse_pipeline_prompt(&prompt).unwrap().service, None);
    }

    #[test]
    fn 被测服务未就绪时模拟器写一条失败用例() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = PipelinePrompt {
            requirement: "VK-1 标题".to_string(),
            artifacts_dir: dir.path().to_path_buf(),
            required: vec!["test-report.json".to_string()],
            service: Some("未就绪（60 秒内 http://127.0.0.1:4000 没有响应）".to_string()),
        };
        write_mock_artifacts(&prompt).unwrap();
        let report: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("test-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["failed"], 1);
        assert_eq!(report["cases"][0]["id"], SERVICE_NOT_READY_CASE_ID);
        assert_eq!(report["cases"][0]["attribution"], "code");
        assert_eq!(report["cases"][0]["source"], "atp");
        assert!(
            report["cases"][0]["message"]
                .as_str()
                .unwrap()
                .contains("60 秒内")
        );
    }
```

现有测试里所有直接构造 `PipelinePrompt { … }` 的地方补上 `service: None`（编译器会逐个指出）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p executors pipeline_prompt 2>&1 | tail -20`
Expected: 编译失败，`SERVICE_URL_LABEL`、`service` 字段、`SERVICE_NOT_READY_CASE_ID` 不存在。

- [ ] **Step 3: 实现**

常量区（`PREVIOUS_VERSION_HINT` 之后）加：

```rust
/// 测试阶段专用的三行（U4）。值的形态见 `services::pipeline::test_env::ServiceState`。
pub const SERVICE_URL_LABEL: &str = "被测地址：";
pub const ATP_ROOT_LABEL: &str = "atp 项目根：";
pub const ATP_COMMAND_LABEL: &str = "atp 命令：";
/// 「被测地址」的值以它开头时表示服务没起来，接口路必须记一条失败用例。
pub const SERVICE_NOT_READY_PREFIX: &str = "未就绪";
/// 接口路没跑起来时那条失败用例的编号（技能与模拟器共用）。
pub const SERVICE_NOT_READY_CASE_ID: &str = "service-not-ready";
```

`PipelinePrompt` 加字段：

```rust
    /// 「被测地址：」行冒号后的内容；只有测试阶段的提示词里有。
    pub service: Option<String>,
```

`parse_pipeline_prompt` 里加一个局部变量 `let mut service: Option<String> = None;`，在 `required` 分支后接一个分支，并在返回值里带上 `service`：

```rust
        } else if service.is_none()
            && let Some(rest) = line.strip_prefix(SERVICE_URL_LABEL)
        {
            service = Some(rest.trim().to_string());
        }
```

`write_mock_artifacts` 的循环里，把 `std::fs::write(&path, mock_artifact_content(…))` 换成：

```rust
        let content = match prompt.service.as_deref() {
            Some(service)
                if name == "test-report.json" && service.starts_with(SERVICE_NOT_READY_PREFIX) =>
            {
                service_not_ready_report(service)
            }
            _ => mock_artifact_content(name, first, &markers, &prompt.requirement),
        };
        std::fs::write(&path, content)?;
```

新增函数：

```rust
/// 被测服务没起来时的报告：接口路记一条失败，归因 code（回到开发阶段）。
fn service_not_ready_report(service: &str) -> String {
    serde_json::json!({
        "total": 1,
        "passed": 0,
        "failed": 1,
        "cases": [{
            "id": SERVICE_NOT_READY_CASE_ID,
            "status": "failed",
            "attribution": "code",
            "message": format!("被测服务{service}"),
            "source": "atp"
        }]
    })
    .to_string()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p executors pipeline_prompt 2>&1 | tail -20`
Expected: 全部通过。

`PipelinePrompt` 在别的 crate 里也被构造或解构时（`git grep -n "PipelinePrompt {" -- crates`），同样补 `service`，然后 `cargo check -p services -p executors` 无报错。

- [ ] **Step 5: 提交**

```bash
git add crates/executors/src/pipeline_prompt.rs
git commit -m "提示词：新增测试阶段的被测地址等三行固定文本，模拟器识别未就绪"
```

---

### Task 2: 测试环境的纯函数（`test_env.rs`）

**Files:**
- Create: `crates/services/src/services/pipeline/test_env.rs`
- Modify: `crates/services/src/services/pipeline/mod.rs`
- Modify: `crates/services/Cargo.toml`

- [ ] **Step 1: 加依赖并挂模块**

`crates/services/Cargo.toml` 的 `[dependencies]` 加两行（版本与仓库里其它 crate 一致）：

```toml
regex = "1.11.1"
csv = "1.3"
```

`mod.rs` 按字母序加 `pub mod test_env;`。

- [ ] **Step 2: 写失败的测试**

新建 `test_env.rs`，先只放测试（放在文件末尾的 `#[cfg(test)] mod tests`）：

```rust
#[cfg(test)]
mod tests {
    use executors::pipeline_prompt::MOCK_TEST_CASES_HEADER;

    use super::*;

    fn csv_with(rows: &[(&str, &str)]) -> String {
        let columns: Vec<&str> = MOCK_TEST_CASES_HEADER.split(',').collect();
        let mut text = format!("{MOCK_TEST_CASES_HEADER}\n");
        for (case_id, noauto) in rows {
            let row: Vec<&str> = columns
                .iter()
                .map(|column| match *column {
                    "CaseID" => *case_id,
                    "NOAUTO" => *noauto,
                    _ => "",
                })
                .collect();
            text.push_str(&row.join(","));
            text.push('\n');
        }
        text
    }

    #[test]
    fn 服务状态的提示词写法() {
        assert_eq!(
            ServiceState::Ready("http://127.0.0.1:4000".into()).prompt_value(),
            "http://127.0.0.1:4000"
        );
        assert_eq!(
            ServiceState::Skipped("仓库没有配置开发服务器脚本".into()).prompt_value(),
            "无（仓库没有配置开发服务器脚本）"
        );
        assert_eq!(
            ServiceState::NotReady("60 秒内没有响应".into()).prompt_value(),
            "未就绪（60 秒内没有响应）"
        );
    }

    #[test]
    fn 可自动化用例按编号去重且跳过人工用例() {
        let text = csv_with(&[("a_001", ""), ("a_001", ""), ("a_002", "Y"), ("a_003", " y ")]);
        assert_eq!(count_automatable_cases(&text), Ok(1));
    }

    #[test]
    fn 全是人工用例时为零() {
        assert_eq!(count_automatable_cases(&csv_with(&[("a_001", "Y")])), Ok(0));
    }

    #[test]
    fn 带引号换行的单元格不会拆行() {
        let mut text = csv_with(&[]);
        let columns: Vec<String> = MOCK_TEST_CASES_HEADER
            .split(',')
            .map(|column| match column {
                "CaseID" => "a_001".to_string(),
                "Steps" => "\"第一步\n第二步, 带逗号\"".to_string(),
                _ => String::new(),
            })
            .collect();
        text.push_str(&columns.join(","));
        text.push('\n');
        assert_eq!(count_automatable_cases(&text), Ok(1));
    }

    #[test]
    fn 缺列时报错并指名() {
        let error = count_automatable_cases("CaseID,Steps\na_001,x\n").unwrap_err();
        assert!(error.contains("NOAUTO"), "{error}");
    }

    fn candidates(log: &str) -> Vec<String> {
        service_url_candidates(log)
    }

    #[test]
    fn localhost_同时给出_ipv4_与_ipv6_候选() {
        assert_eq!(
            candidates("  ➜  Local:   http://localhost:5173/"),
            ["http://127.0.0.1:5173", "http://[::1]:5173"]
        );
    }

    #[test]
    fn 从日志里识别各种本机地址() {
        for (log, expected) in [
            ("Listening on 0.0.0.0:8080", vec!["http://127.0.0.1:8080"]),
            (
                "server at https://127.0.0.1:8443/app",
                vec!["https://127.0.0.1:8443"],
            ),
            ("[::1]:3000", vec!["http://[::1]:3000"]),
            (
                "\u{1b}[32mready\u{1b}[0m http://[::1]:3000",
                vec!["http://[::1]:3000"],
            ),
            ("proxy 4000.localhost:3009 up", vec![]),
            ("see https://example.com:443/docs", vec![]),
            ("localhost:00", vec![]),
            ("nothing here", vec![]),
            (
                "http://localhost:3000端口",
                vec!["http://127.0.0.1:3000", "http://[::1]:3000"],
            ),
        ] {
            assert_eq!(candidates(log), expected, "{log}");
        }
    }

    #[test]
    fn 带协议的地址排在裸地址之前() {
        let log = "ECONNREFUSED 127.0.0.1:5432\n  Local: http://localhost:5173/\n";
        let found = candidates(log);
        assert_eq!(found[0], "http://127.0.0.1:5173");
        assert!(found.contains(&"http://127.0.0.1:5432".to_string()));
    }

    #[test]
    fn 端口非法的地址跳过且不影响后面的() {
        let found = candidates("localhost:70000 ... http://localhost:3000");
        assert_eq!(found, ["http://127.0.0.1:3000", "http://[::1]:3000"]);
    }

    #[test]
    fn 重复地址只保留一次() {
        let log = "Local: http://localhost:3000\nagain http://127.0.0.1:3000\n0.0.0.0:3000";
        assert_eq!(
            candidates(log),
            ["http://127.0.0.1:3000", "http://[::1]:3000"]
        );
    }

    #[test]
    fn 约定端口先_ipv4_再_ipv6() {
        assert_eq!(
            port_candidates(4000),
            ["http://127.0.0.1:4000", "http://[::1]:4000"]
        );
    }

    #[test]
    fn 带字节序标记的_csv_也能数用例() {
        assert_eq!(
            count_automatable_cases("\u{feff}CaseID,NOAUTO\na_001,\n"),
            Ok(1)
        );
    }

    #[cfg(unix)]
    #[test]
    fn 搭项目根不跟随结果目录的符号链接() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.txt"), "x").unwrap();
        let atp = dir.path().join("atp");
        std::fs::create_dir_all(&atp).unwrap();
        std::os::unix::fs::symlink(outside.path(), atp.join("test_results")).unwrap();

        let root = prepare_atp_root(dir.path(), "CaseID\na_001\n").unwrap();

        assert!(outside.path().join("keep.txt").exists(), "外部文件不能被删");
        let meta = std::fs::symlink_metadata(root.join("test_results")).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn 搭项目根遇到指向外部的_atp_链接或悬空链接() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("atp")).unwrap();
        let root = prepare_atp_root(dir.path(), "CaseID\na_001\n").unwrap();
        assert!(outside.path().join("keep.txt").exists());
        assert!(
            !outside.path().join("test_cases").exists(),
            "不能写进外部目录"
        );
        assert!(root.join("test_cases/vk_gen/vk_gen.csv").is_file());

        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("atp")).unwrap();
        let root = prepare_atp_root(dir.path(), "CaseID\na_001\n").unwrap();
        assert!(root.join("test_cases/vk_gen/vk_gen.csv").is_file());
    }

    #[test]
    fn 搭项目根并清掉上一轮结果() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("atp/test_results/results.json");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, "{}").unwrap();

        let root = prepare_atp_root(dir.path(), "CaseID\na_001\n").unwrap();

        assert_eq!(root, dir.path().join("atp"));
        assert_eq!(
            std::fs::read_to_string(root.join("test_cases/vk_gen/vk_gen.csv")).unwrap(),
            "CaseID\na_001\n"
        );
        assert!(!stale.exists(), "上一轮的结果应被清掉");
    }

    #[test]
    fn 空闲端口可以立刻绑定() {
        let port = free_port().unwrap();
        assert!(port > 0);
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn 报告路径只允许结果目录里的_html() {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("atp/test_results");
        std::fs::create_dir_all(&results).unwrap();
        std::fs::write(results.join("test_report_1.html"), "<html></html>").unwrap();
        std::fs::write(results.join("results.json"), "{}").unwrap();
        std::fs::write(dir.path().join("secret.html"), "x").unwrap();

        assert!(resolve_html_report(dir.path(), "atp/test_results/test_report_1.html").is_some());
        for bad in [
            "atp/test_results/results.json",
            "atp/test_results/../../secret.html",
            "secret.html",
            "/etc/passwd",
            "atp/test_results/missing.html",
            "",
        ] {
            assert!(resolve_html_report(dir.path(), bad).is_none(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn 报告路径不跟随指向目录外的符号链接() {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("atp/test_results");
        std::fs::create_dir_all(&results).unwrap();
        std::fs::write(dir.path().join("outside.html"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside.html"), results.join("link.html"))
            .unwrap();
        assert!(resolve_html_report(dir.path(), "atp/test_results/link.html").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn 结果目录本身是链接时读不到外部报告() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("r.html"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("atp")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("atp/test_results")).unwrap();
        assert!(resolve_html_report(dir.path(), "atp/test_results/r.html").is_none());
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p services pipeline::test_env 2>&1 | tail -20`
Expected: 编译失败，上述函数与类型都不存在。

- [ ] **Step 4: 实现**

在测试模块之前写入：

```rust
//! 测试阶段的环境准备（U4 设计 §4.1、§4.2）：全是纯函数或只碰产出物目录的小函数，
//! 引擎负责把它们与启动器串起来。

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

use executors::pipeline_prompt::{NONE_VALUE, SERVICE_NOT_READY_PREFIX};
use regex::Regex;

pub const TEST_CASES_FILE: &str = "test-cases.csv";
/// 相对产出物目录。
pub const ATP_DIR: &str = "atp";
/// 整份 CSV 放进这一个模块（与 prd2testcase 自检用的模块名一致）。
pub const ATP_MODULE: &str = "vk_gen";
pub const ATP_CASES_DIR: &str = "test_cases";
pub const ATP_RESULTS_DIR: &str = "test_results";
pub const DEFAULT_READY_TIMEOUT_S: u64 = 60;
pub const MAX_READY_TIMEOUT_S: u64 = 600;

/// 被测服务的状态，决定接口路怎么走。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    /// 服务已就绪，值是地址。
    Ready(String),
    /// 正当地跳过接口路（没有可自动化用例、仓库没配开发服务器脚本）。
    Skipped(String),
    /// 应该跑但没跑起来：技能必须记一条失败用例。
    NotReady(String),
}

impl ServiceState {
    /// 「被测地址：」行冒号后的内容。
    pub fn prompt_value(&self) -> String {
        match self {
            ServiceState::Ready(url) => url.clone(),
            ServiceState::Skipped(reason) => format!("{NONE_VALUE}（{reason}）"),
            ServiceState::NotReady(reason) => format!("{SERVICE_NOT_READY_PREFIX}（{reason}）"),
        }
    }
}

/// 交给技能的测试环境，对应提示词里的三行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestEnvironment {
    pub service: ServiceState,
    /// 搭好的 atp 项目根；接口路被跳过时为 None。
    pub atp_root: Option<PathBuf>,
    pub atp_command: String,
}

/// 可自动化用例数：`NOAUTO` 为空（与 atp 一致：空 / nan / none）的行按 `CaseID` 去重（同一编号多行是多个步骤）。
pub fn count_automatable_cases(csv_text: &str) -> Result<usize, String> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| format!("{TEST_CASES_FILE} 表头读不出来：{e}"))?
        .clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|header| header.trim() == name)
            .ok_or_else(|| format!("{TEST_CASES_FILE} 缺少 {name} 列"))
    };
    let case_id = column("CaseID")?;
    let noauto = column("NOAUTO")?;

    let mut cases = BTreeSet::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("{TEST_CASES_FILE} 解析失败：{e}"))?;
        let id = record.get(case_id).unwrap_or_default().trim();
        let manual = record
            .get(noauto)
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("y");
        if !id.is_empty() && !manual {
            cases.insert(id.to_string());
        }
    }
    Ok(cases.len())
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap());
/// 本机地址：可带协议，主机只认回环与 0.0.0.0；前一个字符不能是主机名字符
/// （挡住 `4000.localhost:3009` 这种把 `localhost:3009` 当后缀的情况）。
static LOCAL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[^A-Za-z0-9_.\-])(?:(https?)://)?(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):(\d{1,6})(?-u:\b)",
    )
    .unwrap()
});

/// 从开发服务器日志里找出全部本机地址候选，按出现顺序去重；带 `http(s)://` 的排在裸 `host:port` 之前。
/// `localhost` 同时给出 `127.0.0.1` 与 `[::1]` 两个候选（Node 17+ 上 localhost 可能只绑 IPv6）；
/// 显式写 `[::1]` 的保留 IPv6；`0.0.0.0` / `127.0.0.1` 给 `127.0.0.1`。端口 0 或超过 65535 的跳过。
pub fn service_url_candidates(log: &str) -> Vec<String> {
    let clean = ANSI.replace_all(log, "");
    let mut with_scheme = Vec::new();
    let mut bare = Vec::new();
    for found in LOCAL_URL.captures_iter(&clean) {
        let Some(port) = found
            .get(3)
            .and_then(|port| port.as_str().parse::<u16>().ok())
            .filter(|port| *port > 0)
        else {
            continue;
        };
        let scheme = found
            .get(1)
            .map(|scheme| scheme.as_str().to_ascii_lowercase());
        let host = found[2].to_ascii_lowercase();
        let hosts: &[&str] = match host.as_str() {
            "localhost" => &["127.0.0.1", "[::1]"],
            "[::1]" => &["[::1]"],
            _ => &["127.0.0.1"],
        };
        let target = if scheme.is_some() {
            &mut with_scheme
        } else {
            &mut bare
        };
        for host in hosts {
            target.push(format!(
                "{}://{host}:{port}",
                scheme.as_deref().unwrap_or("http")
            ));
        }
    }
    let mut seen = BTreeSet::new();
    with_scheme
        .into_iter()
        .chain(bare)
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

/// 平台注入的 `PORT` 对应的探测地址：先 IPv4 再 IPv6。
pub fn port_candidates(port: u16) -> Vec<String> {
    vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://[::1]:{port}"),
    ]
}

/// 路径本身是符号链接（含悬空链接）时只删链接，不碰目标。
fn remove_symlink(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_file(path)?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// 在产出物目录下搭 atp 项目根，返回根目录。上一轮的结果目录整个删掉：
/// 本轮判定只认本轮跑出来的结果。
pub fn prepare_atp_root(artifacts_dir: &Path, csv_text: &str) -> std::io::Result<PathBuf> {
    let root = artifacts_dir.join(ATP_DIR);
    let results = root.join(ATP_RESULTS_DIR);
    // 链接只删链接本身，不跟随到目标目录里删东西。
    remove_symlink(&root)?;
    if !remove_symlink(&results)? && results.exists() {
        std::fs::remove_dir_all(&results)?;
    }
    std::fs::create_dir_all(&results)?;
    let module_dir = root.join(ATP_CASES_DIR).join(ATP_MODULE);
    std::fs::create_dir_all(&module_dir)?;
    std::fs::write(module_dir.join(format!("{ATP_MODULE}.csv")), csv_text)?;
    Ok(root)
}

/// 向系统要一个当前空闲的回环端口。拿到后立刻释放，存在被别人抢走的极小窗口，
/// 抢走的后果是开发服务器起不来 → 就绪探测超时 → 报告如实记失败。
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

/// 报告里 `report_path` 指向的 HTML 文件。只放行 `atp/test_results/` 下真实存在的
/// `.html`，并用规范化路径再比一次，挡住 `..` 与指向目录外的符号链接。
pub fn resolve_html_report(artifacts_dir: &Path, report_path: &str) -> Option<PathBuf> {
    let relative = Path::new(report_path);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    if !relative.starts_with(Path::new(ATP_DIR).join(ATP_RESULTS_DIR)) {
        return None;
    }
    let is_html = relative
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html"));
    if !is_html {
        return None;
    }
    // 基准只对产出物目录本身做规范化，atp 与 atp/test_results 两级必须是真实目录。
    let atp = artifacts_dir.canonicalize().ok()?.join(ATP_DIR);
    let allowed = atp.join(ATP_RESULTS_DIR);
    for level in [&atp, &allowed] {
        if std::fs::symlink_metadata(level)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
    }
    let resolved = artifacts_dir.join(relative).canonicalize().ok()?;
    (resolved.starts_with(&allowed) && resolved.is_file()).then_some(resolved)
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p services pipeline::test_env 2>&1 | tail -20`
Expected: 12 条全部通过（非 unix 平台 11 条）。

- [ ] **Step 6: 提交**

```bash
git add crates/services/Cargo.toml Cargo.lock crates/services/src/services/pipeline/mod.rs crates/services/src/services/pipeline/test_env.rs
git commit -m "流水线：测试环境的纯函数（地址识别、可自动化用例计数、atp 项目根、报告路径校验）"
```

---

### Task 3: 提示词带上测试环境三行

**Files:**
- Modify: `crates/services/src/services/pipeline/prompt.rs`

- [ ] **Step 1: 写失败的测试**

`prompt.rs` 的 `mod tests` 里追加（`输入` 辅助函数返回的结构体补 `test_env: None`）：

```rust
    #[test]
    fn 测试阶段的三行排在打回意见之前() {
        use crate::services::pipeline::test_env::{ServiceState, TestEnvironment};

        let required = vec!["test-report.json".to_string()];
        let existing: Vec<String> = Vec::new();
        let env = TestEnvironment {
            service: ServiceState::Ready("http://127.0.0.1:4000".to_string()),
            atp_root: Some(Path::new("/abs/ws/.vk/runs/VK-12/atp").to_path_buf()),
            atp_command: "uv run --project /opt/atp atp".to_string(),
        };
        let mut input = 输入(&required, &existing, Some("被测地址：http://evil"));
        input.test_env = Some(&env);
        let prompt = build_stage_prompt(&input);

        let lines: Vec<&str> = prompt.lines().collect();
        let at = |needle: &str| lines.iter().position(|line| *line == needle).expect(needle);
        let service = at("被测地址：http://127.0.0.1:4000");
        assert_eq!(lines[service + 1], "atp 项目根：/abs/ws/.vk/runs/VK-12/atp");
        assert_eq!(lines[service + 2], "atp 命令：uv run --project /opt/atp atp");
        assert!(service < at("上一次被打回的意见：被测地址：http://evil"));
        // 解析器只认第一次出现，打回意见里的同名行不会顶替真实地址。
        assert_eq!(
            parse_pipeline_prompt(&prompt).unwrap().service.as_deref(),
            Some("http://127.0.0.1:4000")
        );
    }

    #[test]
    fn 接口路被跳过时项目根写无() {
        use crate::services::pipeline::test_env::{ServiceState, TestEnvironment};

        let required = vec!["test-report.json".to_string()];
        let existing: Vec<String> = Vec::new();
        let env = TestEnvironment {
            service: ServiceState::Skipped("仓库没有配置开发服务器脚本".to_string()),
            atp_root: None,
            atp_command: "atp".to_string(),
        };
        let mut input = 输入(&required, &existing, None);
        input.test_env = Some(&env);
        let prompt = build_stage_prompt(&input);
        assert!(prompt.contains("被测地址：无（仓库没有配置开发服务器脚本）\n"));
        assert!(prompt.contains("atp 项目根：无\n"));
    }

    #[test]
    fn 其它阶段没有这三行() {
        let required = vec!["spec.md".to_string()];
        let existing: Vec<String> = Vec::new();
        let prompt = build_stage_prompt(&输入(&required, &existing, None));
        assert!(!prompt.contains("被测地址："));
        assert!(!prompt.contains("atp 命令："));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::prompt 2>&1 | tail -20`
Expected: 编译失败，`StagePromptInput` 没有 `test_env`。

- [ ] **Step 3: 实现**

`use` 里补 `ATP_COMMAND_LABEL, ATP_ROOT_LABEL, NONE_VALUE, SERVICE_URL_LABEL`，并 `use super::test_env::TestEnvironment;`。

`StagePromptInput` 加字段：

```rust
    /// 只有测试阶段有：被测地址、atp 项目根、atp 命令。
    pub test_env: Option<&'a TestEnvironment>,
```

`build_stage_prompt` 里，在「本阶段上一版」的循环之后、`feedback` 之前插入：

```rust
    if let Some(env) = input.test_env {
        lines.push(format!("{SERVICE_URL_LABEL}{}", env.service.prompt_value()));
        lines.push(format!(
            "{ATP_ROOT_LABEL}{}",
            env.atp_root
                .as_ref()
                .map(|root| root.display().to_string())
                .unwrap_or_else(|| NONE_VALUE.to_string())
        ));
        lines.push(format!(
            "{ATP_COMMAND_LABEL}{}",
            single_line(&env.atp_command)
        ));
    }
```

`engine.rs` 的 `try_launch` 里构造 `StagePromptInput` 的地方临时补 `test_env: None,`（Task 6 会换掉）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p services pipeline::prompt 2>&1 | tail -20`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/prompt.rs crates/services/src/services/pipeline/engine.rs
git commit -m "流水线：测试阶段的提示词带上被测地址、atp 项目根与 atp 命令"
```

---

### Task 4: 模板支持 `ready_timeout_s`

**Files:**
- Modify: `crates/services/src/services/pipeline/template.rs`

- [ ] **Step 1: 写失败的测试**

`template.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn 就绪超时默认六十秒且可在模板里改() {
        assert_eq!(
            builtin_template()
                .stage(PipelineStageKey::Test)
                .unwrap()
                .ready_timeout_s,
            60
        );
        let template = parse_template_yaml(
            "version: 1\nstages:\n  - key: test\n    skill: atp-run\n    artifacts: [test-report.json]\n    ready_timeout_s: 5\n",
        )
        .unwrap();
        assert_eq!(template.stages[0].ready_timeout_s, 5);
    }

    #[test]
    fn 就绪超时越界报错() {
        for value in ["0", "601", "-1"] {
            let yaml = format!(
                "version: 1\nstages:\n  - key: test\n    skill: atp-run\n    ready_timeout_s: {value}\n"
            );
            assert!(
                matches!(
                    parse_template_yaml(&yaml),
                    Err(TemplateError::BadReadyTimeout { .. })
                ),
                "{value}"
            );
        }
    }

    #[test]
    fn 旧快照没有就绪超时字段也能读() {
        let mut value = serde_json::to_value(builtin_template()).unwrap();
        for stage in value["stages"].as_array_mut().unwrap() {
            stage.as_object_mut().unwrap().remove("ready_timeout_s");
        }
        let template: PipelineTemplate = serde_json::from_value(value).unwrap();
        assert_eq!(template.stages[0].ready_timeout_s, 60);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::template 2>&1 | tail -20`
Expected: 编译失败，`ready_timeout_s`、`BadReadyTimeout` 不存在。

- [ ] **Step 3: 实现**

顶部 `use super::test_env::{DEFAULT_READY_TIMEOUT_S, MAX_READY_TIMEOUT_S};`

`StageTemplate` 加字段（运行中的旧快照 `template_json` 没有这个键，所以要 `serde(default)`）：

```rust
    /// 测试阶段等被测服务就绪的秒数上限；其它阶段不用。
    #[serde(default = "default_ready_timeout_s")]
    pub ready_timeout_s: u64,
```

```rust
fn default_ready_timeout_s() -> u64 {
    DEFAULT_READY_TIMEOUT_S
}
```

`TemplateError` 加：

```rust
    #[error("阶段 {stage} 的 ready_timeout_s 必须在 1 到 {max} 之间，实际 {value}")]
    BadReadyTimeout { stage: String, value: i64, max: u64 },
```

`RawStage` 加：

```rust
    #[serde(default)]
    ready_timeout_s: Option<i64>,
```

`builtin_template` 的 `stage` 辅助函数里构造 `StageTemplate` 时加 `ready_timeout_s: DEFAULT_READY_TIMEOUT_S,`。

`parse_template_yaml` 里，在 `max_rounds` 校验之后、`stages.push` 之前加：

```rust
        let ready_timeout_s = match raw_stage.ready_timeout_s {
            None => DEFAULT_READY_TIMEOUT_S,
            Some(value) => u64::try_from(value)
                .ok()
                .filter(|seconds| (1..=MAX_READY_TIMEOUT_S).contains(seconds))
                .ok_or(TemplateError::BadReadyTimeout {
                    stage: name.clone(),
                    value,
                    max: MAX_READY_TIMEOUT_S,
                })?,
        };
```

注意上面的 `max_rounds` 校验里用的是 `stage: name`（把 `name` 移走了），需要改成 `stage: name.clone()`，否则这里借不到。`stages.push(StageTemplate { … })` 里加 `ready_timeout_s,`。

其它地方直接构造 `StageTemplate { … }` 的（`git grep -n "StageTemplate {" -- crates`）补上 `ready_timeout_s: 60`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p services pipeline::template 2>&1 | tail -20`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/template.rs
git commit -m "流水线模板：测试阶段可配就绪等待秒数 ready_timeout_s"
```

---

### Task 5: 启动器接口——开发服务器、探测、atp 命令

**Files:**
- Modify: `crates/services/src/services/pipeline/launcher.rs`
- Modify: `crates/services/src/services/pipeline/test_support.rs`
- Modify: `crates/services/src/services/config/versions/v8.rs`
- Modify: `crates/local-deployment/src/lib.rs:306-309`
- Regenerate: `shared/types.ts`

- [ ] **Step 1: 写失败的测试**

`launcher.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn 开发服务器脚本前注入环境变量() {
        let script = dev_server_script(
            "pnpm run dev",
            &[
                ("PORT".to_string(), "4000".to_string()),
                ("VK_BASE_URL".to_string(), "http://127.0.0.1:4000".to_string()),
            ],
        );
        assert_eq!(
            script,
            "export PORT='4000'\nexport VK_BASE_URL='http://127.0.0.1:4000'\npnpm run dev"
        );
    }

    #[test]
    fn 环境变量值里的单引号被转义() {
        let script = dev_server_script("true", &[("A".to_string(), "it's".to_string())]);
        assert_eq!(script, "export A='it'\\''s'\ntrue");
    }

    #[test]
    fn atp_命令为空时回落到默认() {
        assert_eq!(effective_atp_command(None), "atp");
        assert_eq!(effective_atp_command(Some("   ")), "atp");
        assert_eq!(
            effective_atp_command(Some(" uv run --project /opt/atp atp ")),
            "uv run --project /opt/atp atp"
        );
    }
```

并在「空启动器所有启动都报错」那条测试末尾加：

```rust
        assert!(launcher.start_dev_server(id, &[]).await.is_err());
        assert!(!launcher.probe_http("http://127.0.0.1:1").await);
        assert_eq!(launcher.atp_command().await, "atp");
        launcher.stop_dev_server(id).await;
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::launcher 2>&1 | tail -20`
Expected: 编译失败。

- [ ] **Step 3: 配置项**

`crates/services/src/services/config/versions/v8.rs` 的 `Config` 末尾加：

```rust
    /// 流水线测试阶段调用 atp 的命令，例如 `uv run --project /path/to/atp atp`；
    /// 不填用 `atp`。
    #[serde(default)]
    pub pipeline_atp_command: Option<String>,
```

同文件里所有构造 `Config { … }` 的地方（`from_v7_config`、`Default`）补 `pipeline_atp_command: None,`。字段带 `serde(default)`，旧配置文件无需升版本。

- [ ] **Step 4: 启动器接口与实现**

`launcher.rs` 的 `use` 补：

```rust
use std::{sync::Arc, time::Duration};

use db::models::{
    execution_process::{ExecutionProcess, ExecutionProcessStatus},
    session::CreateSession,
    workspace_repo::WorkspaceRepo,
};
use tokio::sync::RwLock;

use crate::services::config::Config;
```

（已有的 `use` 行合并进去，不要重复导入 `ExecutionProcessRunReason`、`Session`、`Workspace`。）

`StageLauncher` 加四个方法：

```rust
    /// 测试阶段：先停掉工作区里已有的开发服务器（新一轮起服务前清场），再起**第一个**
    /// 配置了开发服务器脚本的仓库，脚本前注入 `env`。没有仓库配脚本时返回 `Ok(None)`。
    async fn start_dev_server(
        &self,
        workspace_id: Uuid,
        env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError>;

    /// 只停这一个开发服务器进程（`stop_workspace` 不停开发服务器）；已不在运行就什么也不做。
    async fn stop_dev_server(&self, execution_process_id: Uuid);

    /// 探测一次：能在超时内拿到任何 HTTP 响应（不看状态码）就算就绪。
    async fn probe_http(&self, url: &str) -> bool;

    /// 进程从第 `offset` 条日志消息起的输出文本（只取标准输出与标准错误），以及目前的消息
    /// 总数（下次从这里读）。运行中读内存，已结束读库。
    async fn process_output_since(
        &self,
        execution_process_id: Uuid,
        offset: usize,
    ) -> (String, usize);

    /// 调用 atp 的命令（设置页可改）。
    async fn atp_command(&self) -> String;
```

（五个方法——比小节标题多一个 `process_output_since`：运行中的进程日志还没落库，只有执行层的内存里有。）

> 审查后修订：停服务原先是按工作区停的 `stop_dev_servers(workspace_id)`，会连带停掉用户在界面上对
> 同一工作区手动起的开发服务器，改为按进程 id 停的 `stop_dev_server`；读日志原先是每轮全量复制的
> `process_output`，改为增量的 `process_output_since`（`MsgStore` 配套新增 `history_since`，只克隆新消息）。

模块级函数：

```rust
pub const DEFAULT_ATP_COMMAND: &str = "atp";
/// 单次就绪探测的超时。引擎还会用就绪截止前的剩余时间进一步截断。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 在开发服务器脚本前导出环境变量。值用单引号包住，内部单引号写成 `'\''`。
pub fn dev_server_script(script: &str, env: &[(String, String)]) -> String {
    let mut text = String::new();
    for (key, value) in env {
        text.push_str(&format!("export {key}='{}'\n", value.replace('\'', r"'\''")));
    }
    text.push_str(script);
    text
}

pub fn effective_atp_command(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .unwrap_or(DEFAULT_ATP_COMMAND)
        .to_string()
}

fn output_text(messages: &[utils::log_msg::LogMsg]) -> String {
    messages
        .iter()
        .filter_map(|message| match message {
            utils::log_msg::LogMsg::Stdout(text) | utils::log_msg::LogMsg::Stderr(text) => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}
```

`ContainerStageLauncher` 带上配置，并在构造时建一次探测用的 HTTP 客户端复用：

```rust
pub struct ContainerStageLauncher<C> {
    container: C,
    config: Arc<RwLock<Config>>,
    /// 就绪探测用的客户端，构造时建一次复用。建不出来时探测一律不就绪。
    probe_client: Option<reqwest::Client>,
}

impl<C> ContainerStageLauncher<C> {
    pub fn new(container: C, config: Arc<RwLock<Config>>) -> Self {
        // 不走系统代理：配了 HTTP_PROXY 时代理会替回环地址回 502，服务没起也被判成就绪。
        let probe_client = reqwest::Client::builder()
            .no_proxy()
            .timeout(PROBE_TIMEOUT)
            .danger_accept_invalid_certs(true)
            .build()
            .inspect_err(|e| tracing::warn!("构造就绪探测用的 HTTP 客户端失败: {e}"))
            .ok();
        Self { container, config, probe_client }
    }
}
```

`impl StageLauncher for ContainerStageLauncher<C>` 里加：

```rust
    async fn start_dev_server(
        &self,
        workspace_id: Uuid,
        env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError> {
        let pool = &self.container.db().pool;
        let workspace = self.load_workspace(workspace_id).await?;
        let repos = WorkspaceRepo::find_repos_for_workspace(pool, workspace.id).await?;
        let Some(repo) = repos.iter().find(|repo| {
            repo.dev_server_script
                .as_ref()
                .is_some_and(|script| !script.trim().is_empty())
        }) else {
            return Ok(None);
        };
        // 清场：停掉工作区里已有的开发服务器（stop_process 停一个进程，失败只记日志）。
        match ExecutionProcess::find_running_dev_servers_by_workspace(pool, workspace_id).await {
            Ok(running) => {
                for process in running {
                    self.stop_process(&process).await;
                }
            }
            Err(e) => tracing::warn!(%workspace_id, "查询运行中的开发服务器失败: {e}"),
        }

        // 独立会话：开发服务器的退出事件不会被当成某个阶段的终点进程。
        let session = Session::create(
            pool,
            &CreateSession {
                executor: Some("dev-server".to_string()),
                name: None,
            },
            Uuid::new_v4(),
            workspace.id,
        )
        .await?;
        let action = ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script: dev_server_script(repo.dev_server_script.as_deref().unwrap_or(""), env),
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::DevServer,
                working_dir: Some(repo.name.clone()),
            }),
            None,
        );
        let process = self
            .container
            .start_execution(
                &workspace,
                &session,
                &action,
                &ExecutionProcessRunReason::DevServer,
            )
            .await
            .map_err(launch_error)?;
        Ok(Some(process.id))
    }

    async fn stop_dev_server(&self, execution_process_id: Uuid) {
        match ExecutionProcess::find_by_id(&self.container.db().pool, execution_process_id).await {
            Ok(Some(process)) if process.status == ExecutionProcessStatus::Running => {
                self.stop_process(&process).await;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(%execution_process_id, "读取开发服务器进程失败: {e}"),
        }
    }

    async fn probe_http(&self, url: &str) -> bool {
        match &self.probe_client {
            Some(client) => client.get(url).send().await.is_ok(),
            None => false,
        }
    }

    async fn process_output_since(&self, execution_process_id: Uuid, offset: usize) -> (String, usize) {
        if let Some(store) = self
            .container
            .get_msg_store_by_id(&execution_process_id)
            .await
        {
            let (messages, total) = store.history_since(offset);
            return (output_text(&messages), total);
        }
        match crate::services::execution_process::load_raw_log_messages(
            &self.container.db().pool,
            execution_process_id,
        )
        .await
        {
            Some(messages) => (
                output_text(messages.get(offset..).unwrap_or_default()),
                messages.len().max(offset),
            ),
            None => (String::new(), offset),
        }
    }

    async fn atp_command(&self) -> String {
        effective_atp_command(self.config.read().await.pipeline_atp_command.as_deref())
    }
```

实施时核对三处签名并按实际调整（写计划时只看了名字）：`ExecutionProcess::find_running_dev_servers_by_workspace` 的参数与返回类型（`crates/db/src/models/execution_process.rs:316`）、`Session::create` 的返回类型、`danger_accept_invalid_certs` 在 workspace 的 reqwest feature 下是否可用（不可用就去掉这一行——本机 dev server 基本都是 http）。

`NoopStageLauncher` 加：

```rust
    async fn start_dev_server(
        &self,
        _workspace_id: Uuid,
        _env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError> {
        Err(PipelineError::Launch("未接入执行层".to_string()))
    }

    async fn stop_dev_server(&self, _execution_process_id: Uuid) {}

    async fn probe_http(&self, _url: &str) -> bool {
        false
    }

    async fn process_output_since(&self, _execution_process_id: Uuid, offset: usize) -> (String, usize) {
        (String::new(), offset)
    }

    async fn atp_command(&self) -> String {
        DEFAULT_ATP_COMMAND.to_string()
    }
```

- [ ] **Step 5: 假启动器**

`test_support.rs` 的 `FakeLauncher` 加字段并在 `new` 里初始化（`dev_server: AtomicBool::new(false)`，其余 `Mutex::new(Vec::new())` / `Mutex::new(String::new())`）：

```rust
    /// 打开后 `start_dev_server` 会「起」一个开发服务器（落一条 running 的进程）。
    pub(crate) dev_server: AtomicBool,
    /// 每次起开发服务器时注入的环境变量。
    pub(crate) dev_server_envs: Mutex<Vec<Vec<(String, String)>>>,
    /// 起出来的开发服务器进程 id。
    pub(crate) dev_server_processes: Mutex<Vec<Uuid>>,
    /// 这些地址探测成功；`"*"` 表示任何地址都成功。
    pub(crate) ready_urls: Mutex<Vec<String>>,
    /// 每次探测先睡这么多毫秒再返回（模拟连不上时卡到超时的探测）。
    pub(crate) probe_delay_ms: AtomicU64,
    /// 开发服务器的输出。按行拆成消息：每行（含换行）是一条。
    pub(crate) dev_server_output: Mutex<String>,
    /// 每次读开发服务器输出时引擎给的起始消息序号。
    pub(crate) output_offsets: Mutex<Vec<usize>>,
    /// 被要求停的开发服务器进程 id（按要求的先后）。
    pub(crate) dev_server_stopped_ids: Mutex<Vec<Uuid>>,
```

`impl StageLauncher for FakeLauncher` 加：

```rust
    async fn start_dev_server(
        &self,
        workspace_id: Uuid,
        env: &[(String, String)],
    ) -> Result<Option<Uuid>, PipelineError> {
        if !self.dev_server.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let session_id = Uuid::new_v4();
        Session::create(
            &self.pool,
            &CreateSession {
                executor: Some("dev-server".to_string()),
                name: None,
            },
            session_id,
            workspace_id,
        )
        .await
        .map_err(|e| PipelineError::Launch(e.to_string()))?;
        let id = 落进程(&self.pool, session_id, ExecutionProcessRunReason::DevServer).await?;
        self.dev_server_envs.lock().unwrap().push(env.to_vec());
        self.dev_server_processes.lock().unwrap().push(id);
        Ok(Some(id))
    }

    async fn stop_dev_server(&self, execution_process_id: Uuid) {
        self.dev_server_stopped_ids.lock().unwrap().push(execution_process_id);
        // 把该进程标成 killed（与容器停进程一致）。
        sqlx::query("UPDATE execution_processes SET status = 'killed' WHERE id = ? AND status = 'running'")
            .bind(execution_process_id)
            .execute(&self.pool)
            .await
            .expect("标记开发服务器已停");
    }

    async fn probe_http(&self, url: &str) -> bool {
        let delay = self.probe_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
        self.ready_urls
            .lock()
            .unwrap()
            .iter()
            .any(|ready| ready == "*" || ready == url)
    }

    async fn process_output_since(&self, _execution_process_id: Uuid, offset: usize) -> (String, usize) {
        self.output_offsets.lock().unwrap().push(offset);
        let output = self.dev_server_output.lock().unwrap().clone();
        let messages: Vec<&str> = output.split_inclusive('\n').collect();
        (messages.get(offset..).unwrap_or_default().concat(), messages.len().max(offset))
    }

    async fn atp_command(&self) -> String {
        "atp".to_string()
    }
```

- [ ] **Step 6: 部署接线与类型生成**

`crates/local-deployment/src/lib.rs:306-309` 改成传入配置（该函数里已有的配置变量名以实际为准，类型是 `Arc<RwLock<Config>>`）：

```rust
        let pipeline = PipelineService::new(
            db.clone(),
            Arc::new(ContainerStageLauncher::new(container.clone(), config.clone())),
        );
```

Run: `pnpm run generate-types`
Expected: `shared/types.ts` 的 `Config` 多出 `pipeline_atp_command: string | null`。

- [ ] **Step 7: 跑测试确认通过**

Run: `cargo test -p services pipeline::launcher 2>&1 | tail -20 && cargo check -p local-deployment -p server 2>&1 | tail -5`
Expected: 测试通过，检查无报错。

- [ ] **Step 8: 提交**

```bash
git add crates/services/src/services/pipeline/launcher.rs crates/services/src/services/pipeline/test_support.rs crates/services/src/services/config/versions/v8.rs crates/local-deployment/src/lib.rs shared/types.ts
git commit -m "流水线启动器：起停开发服务器、就绪探测、读进程输出、atp 命令配置"
```

---

### Task 6: 引擎——测试阶段先备环境再起智能体

**Files:**
- Modify: `crates/services/src/services/pipeline/engine.rs`（`try_launch` 一带，约 887-965 行）
- Modify: `crates/services/src/services/pipeline/tests.rs`

**行为（本任务要实现的全部）：**

| 情况 | 被测地址行 | atp 项目根行 | 起开发服务器 | 智能体何时启动 |
|---|---|---|---|---|
| 产出物目录里没有 `test-cases.csv` | `无（没有 test-cases.csv）` | `无` | 否 | 立即 |
| CSV 解析失败 | `未就绪（<解析错误>）` | `无` | 否 | 立即 |
| 没有可自动化用例 | `无（用例全部是人工用例，跳过接口路）` | `无` | 否 | 立即 |
| 仓库没配开发服务器脚本 | `无（仓库没有配置开发服务器脚本，跳过接口路）` | `无` | 否 | 立即 |
| 服务就绪（约定端口） | `http://127.0.0.1:<端口>` | 绝对路径 | 是 | 就绪后 |
| 服务就绪（从日志识别到别的端口） | 识别到的地址 | 绝对路径 | 是 | 就绪后 |
| 超时没就绪 | `未就绪（N 秒内 <地址> 没有响应）` | 绝对路径 | 是，随后停掉 | 超时后 |
| 开发服务器中途退出 | `未就绪（开发服务器已退出）` | 绝对路径 | 是 | 发现退出后 |
| 等待期间运行被取消/阶段不再 running | — | — | 停掉 | 不启动 |

- [ ] **Step 1: 写失败的测试**

`tests.rs` 里加辅助与用例。先看文件里现有的「走到某阶段」辅助怎么写（`grep -n "async fn 走到\|async fn 通过" crates/services/src/services/pipeline/tests.rs`），下面的 `走到测试阶段前` 指「开发、评审都已通过，下一步进入测试」的那个现成辅助；没有就照黄金路径用例的写法（三道人工关卡 `decide_gate` 通过 + 对 develop、review 的 `结束(&launch, true)`）抽一个。

```rust
impl 场景 {
    /// 等后台任务把智能体启动起来（测试阶段有开发服务器时启动在锁外的任务里）。
    async fn 等到智能体启动数(&self, expected: usize) {
        for _ in 0..100 {
            let agents = self
                .launcher
                .launches()
                .iter()
                .filter(|launch| launch.kind == FakeLaunchKind::Agent)
                .count();
            if agents >= expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("等了 5 秒智能体启动数仍不到 {expected}");
    }

    fn 智能体启动数(&self) -> usize {
        self.launcher
            .launches()
            .iter()
            .filter(|launch| launch.kind == FakeLaunchKind::Agent)
            .count()
    }

    fn 提示词行(&self, label: &str) -> String {
        self.launcher
            .last()
            .prompt
            .lines()
            .find_map(|line| line.strip_prefix(label).map(str::to_string))
            .unwrap_or_else(|| panic!("提示词里没有 {label} 行"))
    }
}

#[tokio::test]
async fn 测试阶段_仓库没有开发服务器脚本时跳过接口路() {
    let 场 = 场景::新建("加法接口").await;
    场.走到测试阶段前().await;

    assert_eq!(场.当前().await.0, PipelineStageKey::Test);
    assert_eq!(
        场.提示词行("被测地址："),
        "无（仓库没有配置开发服务器脚本，跳过接口路）"
    );
    assert_eq!(场.提示词行("atp 项目根："), "无");
    assert_eq!(场.提示词行("atp 命令："), "atp");
}

#[tokio::test]
async fn 测试阶段_服务就绪后带着地址与项目根启动智能体() {
    let 场 = 场景::新建("加法接口").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    场.launcher.ready_urls.lock().unwrap().push("*".to_string());
    let before = 场.走到测试阶段前().await;
    场.等到智能体启动数(before + 1).await;

    let envs = 场.launcher.dev_server_envs.lock().unwrap().clone();
    let port = envs[0].iter().find(|(key, _)| key == "PORT").unwrap().1.clone();
    let url = format!("http://127.0.0.1:{port}");
    assert!(envs[0].contains(&("VK_BASE_URL".to_string(), url.clone())));
    assert_eq!(场.提示词行("被测地址："), url);

    let root = PathBuf::from(场.提示词行("atp 项目根："));
    assert!(root.join("test_cases/vk_gen/vk_gen.csv").is_file());
}

#[tokio::test]
async fn 测试阶段_脚本忽略端口时从日志识别地址() {
    let 场 = 场景::新建("加法接口").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    场.launcher
        .ready_urls
        .lock()
        .unwrap()
        .push("http://127.0.0.1:5173".to_string());
    *场.launcher.dev_server_output.lock().unwrap() =
        "  ➜  Local:   http://localhost:5173/\n".to_string();
    let before = 场.走到测试阶段前().await;
    场.等到智能体启动数(before + 1).await;

    assert_eq!(场.提示词行("被测地址："), "http://127.0.0.1:5173");
}

#[tokio::test]
async fn 测试阶段_日志里先出现数据库地址也能找到真实地址() {
    let 场 = 场景::新建("加法接口").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    场.launcher
        .ready_urls
        .lock()
        .unwrap()
        .push("http://127.0.0.1:5173".to_string());
    *场.launcher.dev_server_output.lock().unwrap() =
        "ECONNREFUSED 127.0.0.1:5432\n  ➜  Local:   http://localhost:5173/\n".to_string();
    let before = 场.走到测试阶段前().await;
    场.等到智能体启动数(before + 1).await;

    assert_eq!(场.提示词行("被测地址："), "http://127.0.0.1:5173");
}

#[tokio::test]
async fn 测试阶段_超时没就绪时如实告诉技能并停掉服务() {
    let 场 = 场景::新建("加法接口").await;
    场.写仓库模板(
        "version: 1\nstages:\n  - key: requirement\n    skill: vk-requirement\n    artifacts: [requirement.md]\n  - key: test_design\n    skill: prd2testcase\n    artifacts: [test-cases.csv, trace-matrix.md]\n  - key: test\n    skill: atp-run\n    artifacts: [test-report.json]\n    ready_timeout_s: 1\n    gate:\n      auto: all_cases_passed\n",
    );
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    let before = 场.走到测试阶段前().await;
    场.等到智能体启动数(before + 1).await;

    let service = 场.提示词行("被测地址：");
    assert!(service.starts_with("未就绪（1 秒内 http://127.0.0.1:"), "{service}");
    assert!(service.ends_with("没有响应）"), "{service}");
    // 停服务在锁外的任务里做，轮询等它；停的是本轮起的那个进程。
    场.等到停掉开发服务器(process).await;
    assert_eq!(场.停掉的开发服务器(), vec![process]);

    // 假启动器按提示词写出的是「未就绪」报告 → 关卡判失败，归因 code。
    场.结束(&场.launcher.last(), true).await;
    let view = 场.视图().await;
    let test = view
        .stages
        .iter()
        .rev()
        .find(|stage| stage.stage_key == PipelineStageKey::Test)
        .unwrap();
    assert_eq!(test.status, PipelineStageStatus::Failed);
}

#[tokio::test]
async fn 测试阶段_开发服务器中途退出时不等到超时() {
    let 场 = 场景::新建("加法接口").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    let before = 场.走到测试阶段前().await;
    // 等开发服务器「起」出来，再把它标成已失败。
    for _ in 0..100 {
        if !场.launcher.dev_server_processes.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let process = 场.launcher.dev_server_processes.lock().unwrap()[0];
    sqlx::query("UPDATE execution_processes SET status = 'failed', exit_code = 1 WHERE id = ?")
        .bind(process)
        .execute(场.test_db.pool())
        .await
        .unwrap();
    场.等到智能体启动数(before + 1).await;

    assert_eq!(场.提示词行("被测地址："), "未就绪（开发服务器已退出）");
}

#[tokio::test]
async fn 测试阶段_等待期间取消则不再启动智能体() {
    let 场 = 场景::新建("加法接口").await;
    场.写仓库模板(
        "version: 1\nstages:\n  - key: test_design\n    skill: prd2testcase\n    artifacts: [test-cases.csv, trace-matrix.md]\n  - key: test\n    skill: atp-run\n    artifacts: [test-report.json]\n    ready_timeout_s: 1\n",
    );
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    let before = 场.走到测试阶段前().await;
    let process = 场.等到开发服务器(0).await;
    let run_id = 场.视图().await.run.id;
    场.service.cancel(run_id).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
    assert_eq!(场.智能体启动数(), before, "取消后不应再启动智能体");
    assert_eq!(场.停掉的开发服务器(), vec![process]);
}

#[tokio::test]
async fn 测试阶段_全是人工用例时不起开发服务器() {
    let 场 = 场景::新建("加法纯函数").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    // 进入测试阶段前把 CSV 改成只有人工用例。
    场.走到测试阶段前_并在进入前(|artifacts_dir| {
        let path = artifacts_dir.join("test-cases.csv");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let noauto = lines[0].split(',').position(|c| c == "NOAUTO").unwrap();
        let mut cells: Vec<String> = lines[1].split(',').map(str::to_string).collect();
        cells[noauto] = "Y".to_string();
        lines[1] = cells.join(",");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    })
    .await;

    assert_eq!(
        场.提示词行("被测地址："),
        "无（用例全部是人工用例，跳过接口路）"
    );
    assert!(场.launcher.dev_server_envs.lock().unwrap().is_empty());
}
```

（审查后修订：超时用例里的 `process` 取自 `等到开发服务器(0)`；`停掉的开发服务器` / `等到停掉开发服务器` 读假启动器的 `dev_server_stopped_ids`。另补了「探测很慢时也按就绪超时收尾」「后到的日志也能识别地址且只读新输出」「后台任务出错时尝试记失败」等用例。）

`走到测试阶段前` 返回进入测试阶段**之前**的智能体启动数；`走到测试阶段前_并在进入前(f)` 是同一个辅助多一个钩子：在评审阶段 `结束` 之前调用 `f(&产出物目录)`（产出物目录 = `artifacts::artifacts_dir(&self.dir.path().join("workspace"), &self.issue.simple_id)`）。两者共用一个私有实现，`走到测试阶段前` 传空闭包。用例里模板写法（省掉哪些阶段）以能过 `parse_template_yaml` 为准——必须保持 requirement → … → deliver 的相对顺序，可以省略阶段。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::tests::测试阶段 2>&1 | tail -30`
Expected: 新用例失败（提示词里没有「被测地址：」行）。

- [ ] **Step 3: 实现——拆出「启动智能体」这一步**

`engine.rs` 的 `use super::{…}` 里加 `test_env::{self, ServiceState, TestEnvironment}`，文件顶部加 `use std::time::{Duration, Instant};`。

新增一个拥有所有权的提示词底稿（后台任务要带着它跨 `await`）：

```rust
/// 提示词里与测试环境无关的部分。测试阶段要等服务就绪后才能补全，所以先存成自有数据。
struct PromptBase {
    skill: String,
    simple_id: String,
    title: String,
    dir: PathBuf,
    required: Vec<String>,
    existing: Vec<String>,
    previous_versions: Vec<String>,
    feedback: Option<String>,
}

impl PromptBase {
    fn render(&self, test_env: Option<&TestEnvironment>) -> String {
        prompt::build_stage_prompt(&StagePromptInput {
            skill: &self.skill,
            simple_id: &self.simple_id,
            title: &self.title,
            artifacts_dir: &self.dir,
            required: &self.required,
            existing: &self.existing,
            previous_versions: &self.previous_versions,
            feedback: self.feedback.as_deref(),
            test_env,
        })
    }
}

/// 测试阶段准备的结果。
enum TestPrep {
    /// 环境已定，立即启动智能体。
    Ready(TestEnvironment),
    /// 开发服务器已起，要等它就绪。
    Waiting(ServiceWait),
}

struct ServiceWait {
    process_id: Uuid,
    /// 平台注入的约定端口（`PORT`）；就绪探测据此先试 IPv4 再试 IPv6。
    port: u16,
    /// 约定地址 `http://127.0.0.1:{port}`，超时文案用。
    url: String,
    atp_root: PathBuf,
    atp_command: String,
    timeout: Duration,
}

/// 等服务就绪期间需要带到后台任务里的全部东西。
struct PendingTestLaunch {
    run_id: Uuid,
    stage_run_id: Uuid,
    workspace_id: Uuid,
    executor_config: ExecutorConfig,
    base: PromptBase,
    plugin_dirs: Vec<PathBuf>,
    wait: ServiceWait,
}

const SERVICE_POLL_INTERVAL: Duration = Duration::from_millis(300);
```

把 `try_launch` 的后半段（从 `let prompt = prompt::build_stage_prompt(…)` 到函数末尾）换成：

```rust
        let base = PromptBase {
            skill,
            simple_id: issue.simple_id.clone(),
            title: issue.title.clone(),
            dir: dir.clone(),
            required: stage.artifacts.clone(),
            existing,
            previous_versions,
            feedback,
        };
        if stage_run.stage_key != PipelineStageKey::Test {
            return self
                .start_agent(
                    run.id,
                    stage_run.id,
                    workspace_id,
                    &executor_config,
                    base.render(None),
                    plugin_dirs,
                )
                .await;
        }
        match self
            .prepare_test_env(workspace_id, stage_run.id, &dir, stage.ready_timeout_s)
            .await?
        {
            TestPrep::Ready(env) => {
                self.start_agent(
                    run.id,
                    stage_run.id,
                    workspace_id,
                    &executor_config,
                    base.render(Some(&env)),
                    plugin_dirs,
                )
                .await
            }
            TestPrep::Waiting(wait) => {
                self.spawn_test_launch(PendingTestLaunch {
                    run_id: run.id,
                    stage_run_id: stage_run.id,
                    workspace_id,
                    executor_config,
                    base,
                    plugin_dirs,
                    wait,
                });
                Ok(())
            }
        }
    }

    /// 启动智能体会话并把会话、进程记到尝试上。
    async fn start_agent(
        &self,
        run_id: Uuid,
        stage_run_id: Uuid,
        workspace_id: Uuid,
        executor_config: &ExecutorConfig,
        prompt: String,
        plugin_dirs: Vec<PathBuf>,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let run_setup = !PipelineStageRuns::any_launched(pool, run_id).await?;
        let step = self
            .inner
            .launcher
            .start_agent_session(workspace_id, executor_config, prompt, run_setup, plugin_dirs)
            .await?;
        PipelineStageRuns::mark_started(
            pool,
            stage_run_id,
            step.session_id,
            step.execution_process_id,
        )
        .await?;
        Ok(())
    }
```

（`skill` 原来是 `let skill = plugin::qualify_skill(&stage.skill);`，保持不变，它是 `String`，直接移进 `PromptBase`。）

- [ ] **Step 4: 实现——准备测试环境**

```rust
    /// 测试阶段的确定性准备（U4 设计 §4.1、§4.2）。只在需要等服务时返回 `Waiting`。
    async fn prepare_test_env(
        &self,
        workspace_id: Uuid,
        stage_run_id: Uuid,
        dir: &std::path::Path,
        ready_timeout_s: u64,
    ) -> Result<TestPrep, PipelineError> {
        let launcher = &self.inner.launcher;
        let atp_command = launcher.atp_command().await;
        let skipped = |service: ServiceState, atp_command: String| {
            TestPrep::Ready(TestEnvironment {
                service,
                atp_root: None,
                atp_command,
            })
        };

        let Ok(csv_text) = tokio::fs::read_to_string(dir.join(test_env::TEST_CASES_FILE)).await
        else {
            return Ok(skipped(
                ServiceState::Skipped(format!("没有 {}", test_env::TEST_CASES_FILE)),
                atp_command,
            ));
        };
        match test_env::count_automatable_cases(&csv_text) {
            Err(reason) => return Ok(skipped(ServiceState::NotReady(reason), atp_command)),
            Ok(0) => {
                return Ok(skipped(
                    ServiceState::Skipped("用例全部是人工用例，跳过接口路".to_string()),
                    atp_command,
                ));
            }
            Ok(_) => {}
        }

        let port = test_env::free_port()?;
        let url = format!("http://127.0.0.1:{port}");
        let env = [
            ("PORT".to_string(), port.to_string()),
            ("VK_BASE_URL".to_string(), url.clone()),
        ];
        let Some(process_id) = launcher.start_dev_server(workspace_id, &env).await? else {
            return Ok(skipped(
                ServiceState::Skipped("仓库没有配置开发服务器脚本，跳过接口路".to_string()),
                atp_command,
            ));
        };
        // 登记「测试尝试 → 开发服务器进程」，停服务时只停这一个进程。
        self.inner.test_dev_servers.lock().unwrap().insert(stage_run_id, process_id);
        let atp_root = match test_env::prepare_atp_root(dir, &csv_text) {
            Ok(root) => root,
            Err(e) => {
                self.stop_test_dev_server(stage_run_id);
                return Err(e.into());
            }
        };
        Ok(TestPrep::Waiting(ServiceWait {
            process_id,
            port,
            url,
            atp_root,
            atp_command,
            timeout: Duration::from_secs(ready_timeout_s),
        }))
    }
```

- [ ] **Step 5: 实现——锁外等待、锁内启动**

（审查后修订）停服务不按工作区停：`Inner` 里维护 `test_dev_servers: std::sync::Mutex<HashMap<Uuid /*测试尝试*/, Uuid /*开发服务器进程*/>>`，
`prepare_test_env` 起服务后登记；`stop_test_dev_server(stage_run_id)` 取出并移除，再 `tokio::spawn` 调
`launcher.stop_dev_server(id)`，不 await——容器停进程先发 SIGINT 再至少等 2 秒，不能占着全局锁。
表只在内存里，服务重启后为空，不再停。

日志增量读取用 `test_env::ServiceUrlScanner`：只扫新到的完整行（以换行或回车结尾），末尾不完整的一段留到
下次拼起来再扫（超过 1 KB 直接扫完丢掉），候选按出现顺序去重累积。

```rust
    /// 等被测服务就绪：每轮先依次探约定端口的 IPv4/IPv6 地址，再探日志里累积识别到的其余候选；
    /// 开发服务器退出或超时即放弃。每次探测前检查截止时间，并用剩余时间截断探测，
    /// 一轮候选再多也不会远超 `ready_timeout_s`。**不持有引擎锁**。
    async fn wait_for_service(&self, wait: &ServiceWait) -> ServiceState {
        let launcher = &self.inner.launcher;
        let deadline = Instant::now() + wait.timeout;
        let timed_out = || {
            ServiceState::NotReady(format!(
                "{} 秒内 {} 没有响应",
                wait.timeout.as_secs(),
                wait.url
            ))
        };
        let conventional = test_env::port_candidates(wait.port);
        // 日志只读新到的消息（Vite 的 `Local:` 在最前面，不能只看尾部），候选累积在扫描器里。
        let mut offset = 0;
        let mut scanner = ServiceUrlScanner::default();
        loop {
            let (text, next) = launcher.process_output_since(wait.process_id, offset).await;
            offset = next;
            scanner.feed(&text);
            let discovered = scanner
                .found()
                .iter()
                .filter(|url| !conventional.contains(url));
            for url in conventional.iter().chain(discovered) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return timed_out();
                }
                let probe = launcher.probe_http(url);
                if tokio::time::timeout(remaining.min(launcher::PROBE_TIMEOUT), probe)
                    .await
                    .unwrap_or(false)
                {
                    return ServiceState::Ready(url.clone());
                }
            }
            if self.dev_server_exited(wait.process_id).await {
                return ServiceState::NotReady("开发服务器已退出".to_string());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return timed_out();
            }
            tokio::time::sleep(remaining.min(SERVICE_POLL_INTERVAL)).await;
        }
    }

    fn spawn_test_launch(&self, pending: PendingTestLaunch) {
        let service = self.clone();
        tokio::spawn(async move {
            let state = service.wait_for_service(&pending.wait).await;
            let _guard = service.inner.lock.lock().await;
            let (run_id, stage_run_id) = (pending.run_id, pending.stage_run_id);
            if let Err(e) = service.finish_test_launch_locked(pending, state).await {
                tracing::error!("测试阶段启动智能体时出错: {e}");
                service.stop_test_dev_server(stage_run_id);
                // 兜底：错误不是「状态已改变」、尝试仍在等待（running、无会话、运行未结束）时记失败，
                // 否则它会一直卡在 running（没有回调推进，「继续」也不重新启动）。
                if let Err(db_error) = service
                    .fail_waiting_test_attempt_locked(run_id, stage_run_id, &e)
                    .await
                {
                    tracing::error!("记录测试阶段启动失败时出错: {db_error}");
                }
            }
        });
    }

    /// 服务就绪（或确定起不来）之后：尝试仍在等待中才启动智能体，否则只收拾开发服务器。
    async fn finish_test_launch_locked(
        &self,
        pending: PendingTestLaunch,
        state: ServiceState,
    ) -> Result<(), PipelineError> {
        let pool = self.pool();
        let run = self.load_run(pending.run_id).await?;
        let stage_run = PipelineStageRuns::find_by_id(pool, pending.stage_run_id).await?;
        let waiting = stage_run.as_ref().is_some_and(|stage_run| {
            stage_run.status == PipelineStageStatus::Running && stage_run.session_id.is_none()
        });
        let Some(stage_run) = stage_run.filter(|_| waiting && !run.status.is_finished()) else {
            // 只停本尝试起的那个进程，不会误杀同工作区新一轮测试的服务。
            self.stop_test_dev_server(pending.stage_run_id);
            return Ok(());
        };
        if matches!(state, ServiceState::NotReady(_)) {
            self.stop_test_dev_server(stage_run.id);
        }
        let env = TestEnvironment {
            service: state,
            atp_root: Some(pending.wait.atp_root.clone()),
            atp_command: pending.wait.atp_command.clone(),
        };
        if let Err(e) = self
            .start_agent(
                run.id,
                stage_run.id,
                pending.workspace_id,
                &pending.executor_config,
                pending.base.render(Some(&env)),
                pending.plugin_dirs,
            )
            .await
        {
            self.stop_test_dev_server(stage_run.id);
            if !is_state_changed(&e) {
                self.fail_run(&run, &stage_run, &e.to_string()).await?;
            }
        }
        Ok(())
    }
```

核对 `PipelineStageRun` 上会话字段的名字（预期 `session_id: Option<Uuid>`，见 `crates/db/src/models/pipeline.rs` 的结构体定义）与 `PipelineStageRuns::find_by_id` 的返回类型（预期 `Result<Option<PipelineStageRun>, sqlx::Error>`），不一致就按实际改。

取消时也要停开发服务器：`cancel_locked` 里现有的

```rust
        if was_running && let Some(workspace_id) = run.workspace_id {
            self.inner.launcher.stop_workspace(workspace_id).await;
        }
```

改成

```rust
        if let Some(attempt) = running_attempt // 原 was_running，改成记下在跑尝试的 id
            && let Some(workspace_id) = run.workspace_id
        {
            self.inner.launcher.stop_workspace(workspace_id).await;
            // stop_workspace 不停开发服务器；测试尝试起的要一并收掉（含还在等就绪的）。
            self.stop_test_dev_server(attempt);
        }
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p services pipeline:: 2>&1 | tail -30`
Expected: 新增 7 条与原有流水线用例全部通过。原有用例里若有断言整段提示词文本的，按新增的三行更新期望值。

- [ ] **Step 7: 提交**

```bash
git add crates/services/src/services/pipeline/engine.rs crates/services/src/services/pipeline/tests.rs
git commit -m "流水线引擎：测试阶段先备环境、锁外等被测服务就绪，再启动智能体"
```

---

### Task 7: 引擎——测试阶段结束时停掉开发服务器

**Files:**
- Modify: `crates/services/src/services/pipeline/engine.rs`（`finish_stage_locked`，约 285-433 行）
- Modify: `crates/services/src/services/pipeline/tests.rs`

- [ ] **Step 1: 写失败的测试**

```rust
#[tokio::test]
async fn 测试阶段_结束后停掉开发服务器() {
    let 场 = 场景::新建("加法接口").await;
    场.launcher.dev_server.store(true, Ordering::SeqCst);
    场.launcher.ready_urls.lock().unwrap().push("*".to_string());
    let before = 场.走到测试阶段前().await;
    let process = 场.等到开发服务器(0).await;
    场.等到智能体启动数(before + 1).await;
    场.稍候断言没停开发服务器().await;

    场.结束(&场.launcher.last(), true).await;

    场.等到停掉开发服务器(process).await;
    assert_eq!(场.停掉的开发服务器(), vec![process]);
}

#[tokio::test]
async fn 其它阶段结束时不碰开发服务器() {
    let 场 = 场景::新建("加法接口").await;
    场.启动().await.unwrap();
    场.结束(&场.launcher.last(), true).await;
    场.稍候断言没停开发服务器().await;
}
```

（审查后修订：另有「测试阶段_配了检查脚本时检查结束后才停服务」：测试阶段模板带 `checks: ["true"]`，
智能体成功后服务不停，检查脚本结束后才停。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::tests::测试阶段_结束后停掉开发服务器 2>&1 | tail -15`
Expected: 失败，没有要求停任何开发服务器进程。

- [ ] **Step 3: 实现**

`finish_stage_locked` 里，在「用户手动停止」分支**之前**（即拿到 `stage` 之后），把原先在后面计算的
`setup_broken` / `ok` 提前，并加：

```rust
        // 智能体成功且阶段配了检查脚本：接着在同会话跑检查脚本，执行链还没到头。
        let runs_checks = ok
            && !stage.checks.is_empty()
            && process.run_reason != ExecutionProcessRunReason::PipelineStep;
        // 测试阶段执行链的终点（检查脚本结束，或智能体失败、被手动停止、成功且没有检查脚本）：
        // 被测服务不再需要。要跑检查脚本时留着给检查脚本用。
        if stage_run.stage_key == PipelineStageKey::Test && !runs_checks {
            self.stop_test_dev_server(stage_run.id);
        }
```

起检查脚本的分支条件改用 `runs_checks`；起检查脚本失败（`Err` 分支）时也 `stop_test_dev_server(stage_run.id)`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p services pipeline:: 2>&1 | tail -15`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add crates/services/src/services/pipeline/engine.rs crates/services/src/services/pipeline/tests.rs
git commit -m "流水线引擎：测试阶段结束时停掉被测服务"
```

---

### Task 8: 契约文档与关卡解析兼容新字段

**Files:**
- Modify: `docs/superpowers/plans/2026-09-18-pipeline-contract.md`（§4 第 185 行、§5）
- Modify: `crates/services/src/services/pipeline/gates.rs`（只加测试）

- [ ] **Step 1: 写测试（预期直接通过——关卡不读新字段，这条用例是把兼容性钉住）**

`gates.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn 带耗时模块来源与报告路径的新报告照常判定() {
        let text = r#"{
          "total": 2, "passed": 1, "failed": 1,
          "report_path": "atp/test_results/test_report_1.html",
          "cases": [
            {"id":"vk_gen_001","status":"passed","attribution":null,"message":"",
             "duration_s":0.42,"module":"vk_gen","source":"atp"},
            {"id":"service-not-ready","status":"failed","attribution":"code",
             "message":"被测服务未就绪","duration_s":null,"module":null,"source":"atp"}
          ]
        }"#;
        assert!(matches!(
            check_all_cases_passed(text),
            StageVerdict::FailBackTo { stage: PipelineStageKey::Develop, .. }
        ));
    }
```

- [ ] **Step 2: 跑测试**

Run: `cargo test -p services pipeline::gates 2>&1 | tail -10`
Expected: 通过。若因 `deny_unknown_fields` 之类失败，去掉 `TestReport` / `TestCaseResult` 上的该属性（写计划时看到的结构体没有这个属性）。

- [ ] **Step 3: 改契约文档**

§4 里 `test-report.json` 那一段（第 185 行）整段替换为：

```markdown
`test-report.json` 最小结构：`{"total":n,"passed":n,"failed":n,"cases":[{"id":"...","status":"passed|failed","attribution":"code|case|null","message":"..."}]}`；`all_cases_passed` = `failed == 0 && total > 0`。失败时：任一 `attribution == "case"` → 回到 test_design（人工关卡）；否则 → 回到 develop。

**（U4）可选字段，全部向后兼容——旧报告不带也合法，关卡判定不读它们：**

| 位置 | 字段 | 含义 |
|---|---|---|
| 用例 | `duration_s` | 耗时（秒，数字）；不知道写 `null` |
| 用例 | `module` | 模块名（接口路为 atp 的模块名，仓库路可为 `null`） |
| 用例 | `source` | `"atp"`（接口路）或 `"repo"`（仓库测试命令） |
| 顶层 | `report_path` | atp 的 HTML 报告，**相对产出物目录**，必须在 `atp/test_results/` 下且以 `.html` 结尾；界面经 `GET /api/local/pipeline/artifacts/{id}/html-report` 打开 |

任一路「没跑起来」（被测服务未就绪、atp 命令不可用、atp 退出码 2/3、测试命令起不来）必须记成一条 `failed` 用例并在 `message` 写明原因。接口路整体没跑起来时用例编号固定为 `service-not-ready`。
```

§5 末尾（qa 标记表之后）追加：

```markdown
**（U4）测试阶段的提示词多三行**，排在「上一次被打回的意见：」之前，常量同在 `pipeline_prompt.rs`：

```
被测地址：http://127.0.0.1:43127
atp 项目根：/abs/path/.vk/runs/ISS-12/atp
atp 命令：atp
```

- 「被测地址」三种取值：地址（服务已就绪）、`无（原因）`（正当跳过接口路）、`未就绪（原因）`（应该跑但没跑起来）。
- 「atp 项目根」是搭好的目录（`test_cases/vk_gen/vk_gen.csv`），接口路被跳过时为 `无`。
- 模拟器看到「被测地址」以 `未就绪` 开头时，`test-report.json` 写一条 `service-not-ready` 失败用例（归因 code）；其它情况行为不变。
- 模板的测试阶段可写 `ready_timeout_s`（1–600，默认 60）：等被测服务就绪的秒数上限。
```

- [ ] **Step 4: 提交**

```bash
git add docs/superpowers/plans/2026-09-18-pipeline-contract.md crates/services/src/services/pipeline/gates.rs
git commit -m "契约：测试报告的可选字段与测试阶段提示词的三行"
```

---

### Task 9: HTML 报告接口

**Files:**
- Modify: `crates/services/src/services/pipeline/engine.rs`
- Modify: `crates/services/src/services/pipeline/tests.rs`
- Modify: `crates/server/src/routes/local_projects/pipeline.rs`（路由约 345 行、处理函数约 269 行、路由契约测试约 859 行）
- Modify: `packages/web-core/src/entities/pipeline/api/pipelineApi.ts`

- [ ] **Step 1: 写失败的测试（引擎）**

```rust
#[tokio::test]
async fn 报告链接只解析到结果目录里的_html() {
    let 场 = 场景::新建("加法接口").await;
    let before = 场.走到测试阶段前().await;
    let _ = before;
    let dir = artifacts::artifacts_dir(&场.dir.path().join("workspace"), &场.issue.simple_id);
    let results = dir.join("atp/test_results");
    std::fs::create_dir_all(&results).unwrap();
    std::fs::write(results.join("test_report_1.html"), "<html>ok</html>").unwrap();
    std::fs::write(
        dir.join("test-report.json"),
        r#"{"total":1,"passed":1,"failed":0,"report_path":"atp/test_results/test_report_1.html","cases":[{"id":"a","status":"passed","attribution":null,"message":""}]}"#,
    )
    .unwrap();
    场.launcher.write_artifacts.store(false, Ordering::SeqCst);
    场.结束(&场.launcher.last(), true).await;

    let artifact = IssueArtifacts::latest_for_kind(
        场.test_db.pool(),
        场.issue.id,
        ArtifactKind::TestReport,
    )
    .await
    .unwrap()
    .expect("测试报告应已入库");
    let path = 场.service.test_html_report(&artifact).await.unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "<html>ok</html>");
}

#[tokio::test]
async fn 报告没有路径或路径越界时找不到() {
    let 场 = 场景::新建("加法接口").await;
    场.走到测试阶段前().await;
    场.结束(&场.launcher.last(), true).await; // 假启动器写的报告没有 report_path
    let artifact = IssueArtifacts::latest_for_kind(
        场.test_db.pool(),
        场.issue.id,
        ArtifactKind::TestReport,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(
        场.service.test_html_report(&artifact).await,
        Err(PipelineError::NotFound(_))
    ));
}
```

`IssueArtifacts::latest_for_kind` 的参数顺序以 `crates/db/src/models/pipeline.rs:1000` 为准。`write_artifacts` 关掉的时机：必须在测试阶段智能体**启动之后**（启动时假启动器已经写过一份模拟报告，这里再用手写的覆盖它），所以先 `走到测试阶段前()` 再写文件。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::tests::报告 2>&1 | tail -15`
Expected: 编译失败，`test_html_report` 不存在。

- [ ] **Step 3: 实现（引擎）**

`engine.rs` 的 `impl PipelineService` 里（`view_for_run` 附近的只读方法区）加：

```rust
    /// 测试报告对应的 atp HTML 报告文件。报告没写 `report_path`、路径不在
    /// `atp/test_results/` 下、文件不存在，一律按找不到处理。
    pub async fn test_html_report(
        &self,
        artifact: &db::models::pipeline::IssueArtifact,
    ) -> Result<PathBuf, PipelineError> {
        let not_found = || PipelineError::NotFound("完整测试报告".to_string());
        if artifact.summary.kind != db::models::pipeline::ArtifactKind::TestReport {
            return Err(not_found());
        }
        let report_path = serde_json::from_str::<serde_json::Value>(&artifact.content)
            .ok()
            .and_then(|report| report.get("report_path")?.as_str().map(str::to_string))
            .ok_or_else(not_found)?;
        let stage_run = PipelineStageRuns::find_by_id(self.pool(), artifact.summary.stage_run_id)
            .await?
            .ok_or_else(not_found)?;
        let run = self.load_run(stage_run.run_id).await?;
        let root = self.workspace_root(&run).await?.ok_or_else(not_found)?;
        let issue = self.load_issue(run.issue_id).await?;
        let dir = artifacts::artifacts_dir(&root, &issue.simple_id);
        test_env::resolve_html_report(&dir, &report_path).ok_or_else(not_found)
    }
```

- [ ] **Step 4: 路由**

`pipeline.rs` 路由表里 `/artifacts/{id}` 之后加一行：

```rust
        .route("/artifacts/{id}/html-report", &["GET"], get(get_html_report))
```

处理函数（放在 `get_artifact` 之后；`handle_get_artifact` 已做项目归属校验，复用它）。`PipelineError → ApiError` 的转换用文件里第 48-56 行那个现成函数（下面写作 `pipeline_error`，以实际函数名为准）：

```rust
/// atp 的 HTML 报告。内容来自被测仓库里跑出来的文件，按不可信处理：
/// 用 CSP sandbox 把它关进独立源，脚本可以跑（报告的折叠交互要用），但碰不到本站的接口与存储。
async fn get_html_report(
    State(deployment): State<DeploymentImpl>,
    Path(artifact_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let artifact = handle_get_artifact(&deployment.db().pool, artifact_id).await?;
    let path = deployment
        .pipeline()
        .test_html_report(&artifact)
        .await
        .map_err(pipeline_error)?;
    let body = tokio::fs::read(&path).await.map_err(ApiError::Io)?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response())
}
```

`use` 补 `axum::{http::header, response::{IntoResponse, Response}}`（已导入的不重复）。

路由契约测试（约 859 行那张表）加一行 `("/api/local/pipeline/artifacts/{id}/html-report", "GET"),`。

- [ ] **Step 5: 前端地址**

`pipelineApi.ts` 里（`skills: '/api/local/pipeline/skills',` 所在的路径表旁）加导出函数，写法与同文件里带 id 的其它路径保持一致，让路由契约测试的扫描能认出它：

```ts
/** atp 完整 HTML 报告的地址（新标签页打开，不走 JSON 封装）。 */
export function artifactHtmlReportUrl(artifactId: string): string {
  return `/api/local/pipeline/artifacts/${artifactId}/html-report`;
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p services pipeline::tests::报告 2>&1 | tail -10 && cargo test -p server local_projects 2>&1 | tail -15`
Expected: 全部通过（含「前端用到的本地端点都挂上了路由」）。

- [ ] **Step 7: 提交**

```bash
git add crates/services/src/services/pipeline/engine.rs crates/services/src/services/pipeline/tests.rs crates/server/src/routes/local_projects/pipeline.rs packages/web-core/src/entities/pipeline/api/pipelineApi.ts
git commit -m "接口：新增 atp 完整 HTML 报告 GET /api/local/pipeline/artifacts/{id}/html-report"
```

---

### Task 10: 前端——报告解析与「测试」页签

**Files:**
- Modify: `packages/web-core/src/pages/issue-detail/artifactParsers.ts:140-186`
- Modify: `packages/web-core/src/pages/issue-detail/artifactParsers.test.ts`（同目录现有测试文件，名字以实际为准）
- Modify: `packages/web-core/src/pages/issue-detail/ArtifactSection.tsx:109-140`
- Modify: `packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}/common.json`

- [ ] **Step 1: 写失败的测试**

```ts
describe('parseTestReport（U4 新字段）', () => {
  it('读出耗时、模块、来源与报告路径', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 2,
        passed: 1,
        failed: 1,
        report_path: 'atp/test_results/test_report_1.html',
        cases: [
          {
            id: 'vk_gen_001',
            status: 'passed',
            attribution: null,
            message: '',
            duration_s: 0.42,
            module: 'vk_gen',
            source: 'atp',
          },
          {
            id: 'tests::add',
            status: 'failed',
            attribution: 'code',
            message: '断言失败',
            source: 'repo',
          },
        ],
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBe(
      'atp/test_results/test_report_1.html'
    );
    expect(result.value.cases[0]).toMatchObject({
      durationS: 0.42,
      module: 'vk_gen',
      source: 'atp',
    });
    expect(result.value.cases[1]).toMatchObject({
      durationS: null,
      module: null,
      source: 'repo',
    });
  });

  it('旧报告没有新字段时全部为空', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 1,
        passed: 1,
        failed: 0,
        cases: [{ id: 'a', status: 'passed', attribution: null, message: '' }],
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBeNull();
    expect(result.value.cases[0]).toMatchObject({
      durationS: null,
      module: null,
      source: null,
    });
  });

  it('来源不是 atp 或 repo 时当作未知，耗时为负或非数字时为空', () => {
    const result = parseTestReport(
      JSON.stringify({
        total: 1,
        cases: [
          {
            id: 'a',
            status: 'passed',
            source: 'other',
            duration_s: -1,
            module: 3,
          },
        ],
        report_path: 42,
      })
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.reportPath).toBeNull();
    expect(result.value.cases[0]).toMatchObject({
      durationS: null,
      module: '3',
      source: null,
    });
  });
});

describe('formatDuration', () => {
  it('按量级取单位', () => {
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(0.0421)).toBe('42 ms');
    expect(formatDuration(1.234)).toBe('1.23 s');
    expect(formatDuration(75)).toBe('1 min 15 s');
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm --filter web-core exec vitest run src/pages/issue-detail/artifactParsers 2>&1 | tail -20`
Expected: 失败，`reportPath`、`formatDuration` 等不存在。（包名以 `packages/web-core/package.json` 的 `name` 为准。）

- [ ] **Step 3: 实现解析器**

`artifactParsers.ts` 里把 `TestCaseResult`、`TestReport` 与 `parseTestReport` 的 `cases` 映射和返回值换成：

```ts
export type TestSource = 'atp' | 'repo' | null;

export interface TestCaseResult {
  id: string;
  status: TestCaseStatus;
  attribution: TestAttribution;
  message: string;
  /** 耗时（秒）；报告没给或不合法时为 null。 */
  durationS: number | null;
  module: string | null;
  /** 哪一路跑出来的：atp（接口路）或 repo（仓库测试命令）。 */
  source: TestSource;
}

export interface TestReport {
  total: number;
  passed: number;
  failed: number;
  cases: TestCaseResult[];
  /** atp 完整 HTML 报告，相对产出物目录。 */
  reportPath: string | null;
}
```

```ts
  const cases: TestCaseResult[] = rawCases.filter(isRecord).map((c) => {
    const duration = num(c.duration_s);
    return {
      id: text(c.id) ?? '',
      status:
        c.status === 'passed' || c.status === 'failed' ? c.status : 'unknown',
      attribution:
        c.attribution === 'code' || c.attribution === 'case'
          ? c.attribution
          : null,
      message: typeof c.message === 'string' ? c.message : '',
      durationS: duration !== null && duration >= 0 ? duration : null,
      module: text(c.module),
      source: c.source === 'atp' || c.source === 'repo' ? c.source : null,
    };
  });
```

返回的 `value` 里加：

```ts
      reportPath:
        typeof data.report_path === 'string' && data.report_path.trim() !== ''
          ? data.report_path
          : null,
```

文件末尾加：

```ts
/** 耗时的展示文本：不到 1 秒用毫秒，不到 1 分钟用秒，否则分加秒。 */
export function formatDuration(seconds: number | null): string {
  if (seconds === null) return '—';
  if (seconds < 1) return `${Math.round(seconds * 1000)} ms`;
  if (seconds < 60) return `${seconds.toFixed(2)} s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes} min ${Math.round(seconds - minutes * 60)} s`;
}
```

确认 `text()` 对数字 `3` 返回 `'3'`（文件第 118-123 行的辅助函数看起来是数字原样返回——那样的话 `module` 取值处改成 `c.module == null ? null : String(text(c.module) ?? '') || null`，以测试通过为准）。

- [ ] **Step 4: 实现界面**

`ArtifactSection.tsx` 的 `case 'test_report'` 分支：`renderContent`（或该 `switch` 所在的函数）需要拿到产出物 id。看函数签名，把 `summary.id` 作为参数传进来（下面写作 `artifactId`）。分支体改成：

```tsx
    case 'test_report': {
      const result = parseTestReport(content);
      if (!result.ok) return <RawContent content={content} />;
      const report = result.value;
      return (
        <div className="flex flex-col gap-half">
          <div className="flex items-center justify-between gap-base">
            <p className="m-0 text-sm text-normal">
              {t('issueDetail.testSummary', {
                passed: report.passed,
                failed: report.failed,
                total: report.total,
              })}
            </p>
            {report.reportPath !== null && (
              <a
                className="text-sm text-brand hover:underline"
                href={artifactHtmlReportUrl(artifactId)}
                target="_blank"
                rel="noreferrer"
              >
                {t('issueDetail.testFullReport')}
              </a>
            )}
          </div>
          <CsvTableView
            header={[
              t('issueDetail.testCols.case'),
              t('issueDetail.testCols.source'),
              t('issueDetail.testCols.status'),
              t('issueDetail.testCols.duration'),
              t('issueDetail.testCols.attribution'),
              t('issueDetail.testCols.message'),
            ]}
            rows={report.cases.map((testCase) => [
              testCase.id,
              testCase.source ? t(SOURCE_KEYS[testCase.source]) : '—',
              t(CASE_STATUS_KEYS[testCase.status]),
              formatDuration(testCase.durationS),
              testCase.attribution
                ? t(ATTRIBUTION_KEYS[testCase.attribution])
                : '—',
              testCase.message,
            ])}
            emptyText={t('issueDetail.noArtifact')}
          />
        </div>
      );
    }
```

文件顶部（`ATTRIBUTION_KEYS` 旁）加：

```tsx
const SOURCE_KEYS = {
  atp: 'issueDetail.testSource.atp',
  repo: 'issueDetail.testSource.repo',
} as const;
```

并导入 `formatDuration`（来自 `./artifactParsers`）与 `artifactHtmlReportUrl`（来自 `@/entities/pipeline/api/pipelineApi`）。链接的颜色类名用本文件/本目录里其它链接已经在用的那个（参照 `packages/local-web/AGENTS.md` 的设计系统约定），不要自造。

- [ ] **Step 5: 文案**

七种语言的 `common.json`，`issueDetail` 下：`testCols` 里加 `source`、`duration`；与 `testCols` 同级加 `testFullReport` 与 `testSource`。

| 键 | zh-Hans | zh-Hant | en | ja | ko | es | fr |
|---|---|---|---|---|---|---|---|
| `testCols.source` | 来源 | 來源 | Source | 実行元 | 출처 | Origen | Source |
| `testCols.duration` | 耗时 | 耗時 | Duration | 所要時間 | 소요 시간 | Duración | Durée |
| `testFullReport` | 查看完整报告 | 查看完整報告 | View full report | 完全なレポートを見る | 전체 보고서 보기 | Ver informe completo | Voir le rapport complet |
| `testSource.atp` | 接口用例 | 介面用例 | API cases | API ケース | API 케이스 | Casos de API | Cas d’API |
| `testSource.repo` | 仓库测试 | 倉庫測試 | Repo tests | リポジトリのテスト | 저장소 테스트 | Pruebas del repositorio | Tests du dépôt |

- [ ] **Step 6: 跑测试确认通过**

Run: `pnpm --filter web-core exec vitest run src/pages/issue-detail 2>&1 | tail -20 && pnpm run web-core:check 2>&1 | tail -10`
Expected: 测试通过，类型检查无报错。同目录若有 `ArtifactSection` 的渲染测试断言了表头列数，按六列更新。

- [ ] **Step 7: 提交**

```bash
git add packages/web-core/src/pages/issue-detail packages/web-core/src/i18n/locales
git commit -m "界面：测试页签显示来源与耗时，并可打开 atp 完整报告"
```

---

### Task 11: 前端——设置页填 atp 命令

**Files:**
- Modify: `packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx`
- Modify: `packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}/settings.json`

- [ ] **Step 1: 实现**

本节是一个受控输入框加保存按钮，逻辑只有「读配置 → 改 → 存」，沿用 `AgentsSettingsSection.tsx:53,233` 的 `useUserSystem().updateAndSaveConfig` 写法（导入路径照抄那个文件）。

`PipelineSkillsSettingsSection` 函数体里，`useState` 之后加：

```tsx
  const { config, updateAndSaveConfig } = useUserSystem();
  const savedCommand = config?.pipeline_atp_command ?? '';
  const [atpCommand, setAtpCommand] = useState(savedCommand);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    setAtpCommand(savedCommand);
  }, [savedCommand]);

  const saveAtpCommand = async () => {
    setSaving(true);
    try {
      const trimmed = atpCommand.trim();
      await updateAndSaveConfig({
        pipeline_atp_command: trimmed === '' ? null : trimmed,
      });
    } finally {
      setSaving(false);
    }
  };
```

返回的 JSX 里，`readOnly` 那个 `<p>` 之前插入：

```tsx
      <div className="space-y-2">
        <label
          htmlFor="pipeline-atp-command"
          className="text-sm font-medium text-high"
        >
          {t('settings.pipelineSkills.atpCommand.label')}
        </label>
        <div className="flex gap-2">
          <input
            id="pipeline-atp-command"
            className="flex-1 rounded border border-border bg-primary px-2 py-1 font-ibm-plex-mono text-sm text-high"
            value={atpCommand}
            placeholder="atp"
            onChange={(event) => setAtpCommand(event.target.value)}
          />
          <button
            type="button"
            className="rounded border border-border px-3 py-1 text-sm text-high disabled:opacity-50"
            disabled={saving || atpCommand.trim() === savedCommand.trim()}
            onClick={() => void saveAtpCommand()}
          >
            {t('settings.pipelineSkills.atpCommand.save')}
          </button>
        </div>
        <p className="text-xs text-low">
          {t('settings.pipelineSkills.atpCommand.help')}
        </p>
      </div>
```

输入框与按钮优先换成设置页其它分区在用的现成组件（看 `GeneralSettingsSection.tsx` 里文本输入与保存按钮用的是什么），上面的原生元素只是兜底写法。

- [ ] **Step 2: 文案**

`settings.json` 的 `settings.pipelineSkills` 下加 `atpCommand`：

| 键 | zh-Hans | en |
|---|---|---|
| `atpCommand.label` | atp 命令 | atp command |
| `atpCommand.save` | 保存 | Save |
| `atpCommand.help` | 测试阶段用它执行接口用例。留空用 `atp`；没有全局安装时可填 `uv run --project /path/to/atp atp`。 | Used by the test stage to run API cases. Leave empty for `atp`; without a global install use `uv run --project /path/to/atp atp`. |

zh-Hant 用繁体转写（「測試階段用它執行介面用例。留空用 `atp`；沒有全域安裝時可填 …」，按钮「儲存」）；es / fr / ja / ko 各自翻译，命令示例原样保留。

- [ ] **Step 3: 检查**

Run: `pnpm run web-core:check 2>&1 | tail -10 && pnpm run lint 2>&1 | tail -15`
Expected: 无报错（`lint` 会跑 clippy，注意此时不能有别的 cargo 在跑）。

- [ ] **Step 4: 提交**

```bash
git add packages/web-core/src/shared/dialogs/settings/settings/PipelineSkillsSettingsSection.tsx packages/web-core/src/i18n/locales
git commit -m "设置：流水线技能页可填 atp 命令"
```

---

### Task 12: 重写 `atp-run` 技能

> 下面的技能正文是写计划时的草稿；**最终技能以 `assets/pipeline-plugin/skills/atp-run/SKILL.md` 为准**（按 atp 实测与终审意见修正过，例如仓库路失败一律归因 `code`、`no-tests` 计入 `total`），本节不逐字同步。

**Files:**
- Modify: `assets/pipeline-plugin/skills/atp-run/SKILL.md`
- Modify: `crates/services/src/services/pipeline/plugin.rs`（`mod tests`，约 400-440 行）

- [ ] **Step 1: 写失败的测试**

`plugin.rs` 的 `mod tests` 里追加（`skill_text` 是该模块现成的辅助函数）：

```rust
    #[test]
    fn 测试技能认得平台给的三行与固定编号() {
        use executors::pipeline_prompt::{
            ATP_COMMAND_LABEL, ATP_ROOT_LABEL, SERVICE_NOT_READY_CASE_ID,
            SERVICE_NOT_READY_PREFIX, SERVICE_URL_LABEL,
        };
        let text = skill_text("atp-run");
        for needle in [
            SERVICE_URL_LABEL,
            ATP_ROOT_LABEL,
            ATP_COMMAND_LABEL,
            SERVICE_NOT_READY_PREFIX,
            SERVICE_NOT_READY_CASE_ID,
            "legacy run --all",
            "--html-report",
            "report_path",
            "duration_s",
        ] {
            assert!(text.contains(needle), "atp-run 技能正文缺少 {needle}");
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p services pipeline::plugin 2>&1 | tail -15`
Expected: 新用例失败（正文缺少「被测地址：」）。

- [ ] **Step 3: 重写技能**

`## 流水线约定` 一节**一个字都不要动**（`平台技能都带同一段流水线约定` 这条测试逐字比对）。frontmatter、开头引言与 `## 做法` 起的全部内容替换为：

````markdown
---
name: atp-run
description: 流水线「测试」阶段：接口用例用 atp 真跑、仓库测试命令真跑，两路结果合并成 test-report.json
---

# 测试执行（流水线阶段：test）

两路都要真跑，**绝不能编造结果**；任何一路没跑起来都要如实记成失败用例。

## 流水线约定

（保持原文不变）

## 平台给的三行

除了上面的固定行，本阶段的提示词还有三行：

- `被测地址：` 三种取值——
  - 一个地址（如 `http://127.0.0.1:43127`）：被测服务已由平台拉起并确认可访问，**不要自己再起服务**。
  - `无（原因）`：正当跳过接口路（没有可自动化用例，或仓库没配开发服务器脚本）。只跑仓库路，不记失败。
  - `未就绪（原因）`：该跑但服务没起来。接口路记**一条**失败用例（见下），然后照常跑仓库路。
- `atp 项目根：` 平台搭好的目录（里面是 `test_cases/vk_gen/vk_gen.csv`）；接口路被跳过时是 `无`。
- `atp 命令：` 调用 atp 的命令，可能带空格（如 `uv run --project /opt/atp atp`），原样拼在前面用。

## 接口路（被测地址是一个地址时才做）

1. 进到「atp 项目根」，执行：

   ```bash
   cd "<atp 项目根>" && <atp 命令> legacy run --all --base-url "<被测地址>" --out test_results --html-report
   ```

   记下退出码。**不要改 `test_cases/` 里的 CSV**，那是人工确认过的用例。

2. 按退出码处理：

   | 退出码 | 含义 | 做法 |
   |---|---|---|
   | 0 | 全部通过 | 读结果 |
   | 1 | 有失败或未执行 | 读结果 |
   | 2、3、其它 | 没跑起来（参数错、框架错、命令不存在） | 接口路记一条失败用例：`id` 写 `service-not-ready`，`attribution` 写 `code`，`message` 写退出码与输出末尾几行 |

3. 读 `test_results/results.json`，每条用例取 `case_id`、`title`、`module`、`status`、`duration_s`、`message`，映射成报告用例：

   | 报告字段 | 取值 |
   |---|---|
   | `id` | `case_id` |
   | `status` | atp 的通过状态 → `passed`；其余（失败、未执行、跳过、错误）→ `failed` |
   | `message` | atp 的 `message`；通过时写 `""` |
   | `duration_s` | 原样（数字） |
   | `module` | 原样 |
   | `source` | `"atp"` |

   CSV 里 `NOAUTO` 非空（与 atp 一致）的人工用例 atp 不会执行，**不计入报告**。

4. 给每条失败用例判归因。先看 `test_results/` 下结果 CSV 里该用例的 `RootCause` 列与实际返回：
   - `code`：服务返回的内容与期望不符（状态码不对、字段缺失、值不对）、服务报 5xx、连接被拒。
   - `case`：用例自身的问题（请求写错了路径或参数名、期望值与需求/规格矛盾、引用了不存在的变量）。
   - 判不准写 `code`。

5. 在 `test_results/` 下找 `test_report*.html`（取最新的一个）。找到就把顶层 `report_path` 写成 `atp/test_results/<文件名>`；没有就不写这个字段。

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
````

「（保持原文不变）」那一行是给实施者看的占位说明——实际文件里那一节保留现有的 11 条约定原文，不是写这句话。

实施时核对两处 atp 细节并据实改技能正文：`results.json` 里表示通过的 `status` 取值（看 `/Users/admin/work/github/atp/src/atp/results.py:19-36`）、结果 CSV 的落点与文件名（`src/atp/legacy_csv/result_writer.py`）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p services pipeline::plugin 2>&1 | tail -15 && pnpm run pipeline-skills:check`
Expected: 全部通过（`atp-run` 是平台自写技能，不在锁文件里，哈希校验不受影响）。

- [ ] **Step 5: 提交**

```bash
git add assets/pipeline-plugin/skills/atp-run/SKILL.md crates/services/src/services/pipeline/plugin.rs
git commit -m "技能：atp-run 改为接口路与仓库路两路真跑并合并报告"
```

---

### Task 13: 端到端——接口路未就绪时如实标失败

**Files:**
- Modify: `packages/e2e/support/api.ts`
- Modify: `packages/e2e/support/fixtures.ts`
- Modify: `packages/e2e/tests/exceptions.spec.ts`

- [ ] **Step 1: 支撑代码**

`api.ts` 的 `VkApi` 里加（仓库更新接口的路径与请求体以 `crates/server/src/routes/repo.rs` 为准——找处理 `dev_server_script` 的那个 `PUT`/`PATCH`）：

```ts
  /** 给仓库配开发服务器脚本。 */
  async setDevServerScript(repoId: string, script: string): Promise<void> {
    await this.envelope('PUT', `/api/repos/${repoId}`, {
      dev_server_script: script,
    });
  }
```

`fixtures.ts` 的 `createGitRepo` 加一个可选参数，用来在初始提交里带上仓库模板：

```ts
export function createGitRepo(
  dir: string,
  files: Record<string, string> = {}
): string {
  fs.mkdirSync(dir, { recursive: true });
  const git = (...args: string[]) =>
    execFileSync('git', args, { cwd: dir, stdio: 'pipe' });
  git('init', '-b', 'main');
  fs.writeFileSync(path.join(dir, 'README.md'), '# vk e2e fixture\n');
  for (const [name, content] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
    fs.writeFileSync(path.join(dir, name), content);
  }
  git('add', '.');
  // …后面的 commit 不变
```

- [ ] **Step 2: 写用例**

`exceptions.spec.ts` 末尾追加。走人工关卡的写法照抄同文件「[qa:test-fail-case-once]」那条用例（第 106 行起）；下面用 `通过三道人工关卡` 指代那段现成步骤，实施时内联或抽成本文件的辅助函数。

```ts
test('被测服务起不来：测试报告如实标失败并退回开发', async ({ page, api }) => {
  await api.prepareConfig();
  const repoName = `e2e-notready-${Date.now().toString(36)}`;
  // 模板把就绪等待缩到 2 秒；开发服务器脚本只睡觉、不监听端口。
  const template = [
    'version: 1',
    'stages:',
    '  - key: requirement',
    '    skill: vk-requirement',
    '    artifacts: [requirement.md]',
    '    gate: { human: 需求确认 }',
    '  - key: spec',
    '    skill: vk-spec',
    '    artifacts: [spec.md, plan.md]',
    '    gate: { human: 设计规格确认 }',
    '  - key: test_design',
    '    skill: prd2testcase',
    '    artifacts: [test-cases.csv, trace-matrix.md]',
    '    gate: { human: 用例设计确认 }',
    '  - key: develop',
    '    skill: vk-develop',
    '    gate: { auto: checks_passed }',
    '  - key: review',
    '    skill: vk-review',
    '    artifacts: [review.json]',
    '    gate: { auto: no_blocking_findings }',
    '  - key: test',
    '    skill: atp-run',
    '    artifacts: [test-report.json]',
    '    ready_timeout_s: 2',
    '    gate: { auto: all_cases_passed }',
    '    retry: { max_rounds: 1 }',
    '',
  ].join('\n');
  const repo = await api.registerRepo(
    createGitRepo(path.join(REPOS_DIR, repoName), {
      '.vibe/pipeline.yaml': template,
    }),
    repoName
  );
  await api.setDevServerScript(repo.id, 'sleep 600');
  const project = await api.createProject('E2E 被测服务起不来');

  const title = '被测服务起不来';
  const issue = await api.startPipelineDirect({
    projectId: project.id,
    repoId: repo.id,
    title,
  });
  await 通过三道人工关卡(page, api, project.id, issue);

  // 测试阶段：等 2 秒超时 → 模拟器写未就绪报告 → 关卡失败。
  const view = await api.waitForPipeline(issue.id, (v) =>
    v.stages.some(
      (stage) => stage.stage_key === 'test' && stage.status === 'failed'
    )
  );
  const failed = view.stages.find(
    (stage) => stage.stage_key === 'test' && stage.status === 'failed'
  );
  expect(failed?.error ?? failed?.summary ?? '').toContain('service-not-ready');

  // 界面：测试页签能看到这条失败与原因。
  await page.goto(`/issues/${issue.id}`);
  await page.getByRole('tab', { name: '测试' }).click();
  await expect(page.getByText('service-not-ready')).toBeVisible();
  await expect(page.getByText(/被测服务未就绪/)).toBeVisible();
});
```

`startPipelineDirect` 的参数名、返回值，`waitForPipeline` 的第一个参数，需求详情页的地址，阶段视图里放失败摘要的字段名（`error` 还是别的），都以 `support/api.ts`、`support/pages.ts` 与同文件现有用例为准；`path`、`REPOS_DIR`、`createGitRepo` 从 `../support/fixtures`、`../support/env` 导入。`max_rounds: 1` 让测试阶段失败一次就转人工，避免在「开发 → 评审 → 测试」里空转。

- [ ] **Step 3: 跑**

Run: `pnpm run e2e:check 2>&1 | tail -10`（类型检查），然后按 `packages/e2e/package.json` 里的脚本跑 `exceptions.spec.ts`（后台运行，定时看输出；跑之前确认没有别的 cargo 在跑）。
Expected: 新用例与原有 e2e 全部通过。黄金路径用例的仓库没配开发服务器脚本，走「跳过接口路」，行为与之前一致。

- [ ] **Step 4: 提交**

```bash
git add packages/e2e
git commit -m "端到端：被测服务起不来时测试报告如实标失败"
```

---

### Task 14: 收尾与全量校验

- [ ] **Step 1: 格式化**

Run: `pnpm run format`

- [ ] **Step 2: 全量校验（逐条跑，不要并行——同一时间只允许一个 cargo）**

```bash
pnpm run generate-types:check
pnpm run check
pnpm run lint
cargo test --workspace
```

Expected: 全部通过。任何一条失败都先修再往下。

- [ ] **Step 3: 把设计文档状态改成已实施**

`docs/superpowers/specs/2026-09-26-pipeline-test-stage-design.md` 第 3 行的「已确认方向，待实施」改成「已实施（计划见 `docs/superpowers/plans/2026-10-02-pipeline-test-stage.md`，与设计的偏差见该计划 §0）」。

- [ ] **Step 4: 提交并推送、开 PR**

```bash
git add -A
git commit -m "收尾：格式化并标记 U4 设计已实施"
git push -u origin pipeline-test-stage
gh pr create --repo jk-ai-automation/vibe-kanban --base main --title "流水线测试阶段接入真实执行（U4）" --body-file <PR 描述文件>
```

PR 描述写清：两路执行、与设计的偏差（本计划 §0）、人工验收清单（下一步）。

- [ ] **Step 5: 真实验收（人工，非 qa-mode + 真实 Claude Code）**

准备一个带 HTTP 接口的小样例仓库（例如 Node `http` 起一个 `GET /add?a=1&b=2`，按 `PORT` 环境变量监听，`dev_server_script` 填 `node server.js`），在设置页把 atp 命令填成 `uv run --project /Users/admin/work/github/atp atp`。逐项确认并把结果记到本文件末尾：

1. 有接口的需求：测试阶段自动起服务，atp 真跑，报告里每条用例有状态与耗时、来源是「接口用例」，点「查看完整报告」能打开 HTML。
2. 纯函数需求：提示词里「被测地址」是 `无（…）`，只跑仓库测试命令，流水线不中断。
3. 把 `dev_server_script` 改成 `sleep 600`：约 60 秒后测试阶段失败，报告里有 `service-not-ready` 与原因，流水线退回开发。
4. 把 atp 命令填成不存在的命令：接口路记一条失败并写明原因，不会被判通过。
5. 故意让一条用例的期望值与规格矛盾：归因 `case`，退回用例设计并重新过人工关卡。
6. 测试阶段结束后 `ps` 里没有残留的开发服务器进程。

---

## 自查记录

- **设计覆盖**：§4.1 → Task 2（地址识别、端口）、5（起服务、探测）、6（等待）；§4.2 → Task 2（项目根）、5（atp 命令配置）、11（设置页）、12（技能）；§4.3、§4.4 → Task 12；§4.5 → Task 8（契约与 Rust）、10（前端）、9（报告链接）；§6 → 各任务的单测 + Task 13（e2e）+ Task 14（人工验收）；§7 的 5 条验收标准 → Task 14 Step 2 与 Step 5。
- **与设计的偏差**集中在 §0，实施中再发现偏差就追加到那张表。
- **类型一致性**：`ServiceState` / `TestEnvironment`（Task 2）在 Task 3、6 使用；`ready_timeout_s`（Task 4）在 Task 6 使用；启动器的五个新方法（Task 5，审查后为 `start_dev_server` / `stop_dev_server` / `probe_http` / `process_output_since` / `atp_command`）在 Task 6、7 使用；`resolve_html_report`（Task 2）在 Task 9 使用；`artifactHtmlReportUrl`（Task 9）在 Task 10 使用；提示词常量（Task 1）在 Task 2、3、12 使用。
- **写计划时没法确认、已在对应步骤里标出要核对的地方**：几个 db 模型方法的精确签名（Task 5、6、9）、仓库更新接口的路径（Task 13）、atp `results.json` 的通过状态取值（Task 12）、前端现成的输入/按钮组件（Task 11）。
