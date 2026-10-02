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

/// 可自动化用例数：`NOAUTO` 不是 `Y` 的行按 `CaseID` 去重（同一编号多行是多个步骤）。
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
        r"(?i)(?:^|[^A-Za-z0-9_.\-])(?:(https?)://)?(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):(\d{2,5})\b",
    )
    .unwrap()
});

/// 从开发服务器日志里找第一个本机地址，统一成 `scheme://127.0.0.1:port`。
/// 是前端 `usePreviewUrl` 识别规则的后端简化版：只要回环地址，不要局域网 IP。
pub fn detect_service_url(log: &str) -> Option<String> {
    let clean = ANSI.replace_all(log, "");
    let found = LOCAL_URL.captures(&clean)?;
    let scheme = found
        .get(1)
        .map(|scheme| scheme.as_str().to_ascii_lowercase())
        .unwrap_or_else(|| "http".to_string());
    let port: u16 = found.get(2)?.as_str().parse().ok()?;
    Some(format!("{scheme}://127.0.0.1:{port}"))
}

/// 在产出物目录下搭 atp 项目根，返回根目录。上一轮的结果目录整个删掉：
/// 本轮判定只认本轮跑出来的结果。
pub fn prepare_atp_root(artifacts_dir: &Path, csv_text: &str) -> std::io::Result<PathBuf> {
    let root = artifacts_dir.join(ATP_DIR);
    let results = root.join(ATP_RESULTS_DIR);
    if results.exists() {
        std::fs::remove_dir_all(&results)?;
    }
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
    let allowed = artifacts_dir
        .join(ATP_DIR)
        .join(ATP_RESULTS_DIR)
        .canonicalize()
        .ok()?;
    let resolved = artifacts_dir.join(relative).canonicalize().ok()?;
    (resolved.starts_with(&allowed) && resolved.is_file()).then_some(resolved)
}

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

    #[test]
    fn 从日志里识别本机地址() {
        for (log, expected) in [
            ("  ➜  Local:   http://localhost:5173/", Some("http://127.0.0.1:5173")),
            ("Listening on 0.0.0.0:8080", Some("http://127.0.0.1:8080")),
            ("server at https://127.0.0.1:8443/app", Some("https://127.0.0.1:8443")),
            ("\u{1b}[32mready\u{1b}[0m http://[::1]:3000", Some("http://127.0.0.1:3000")),
            ("proxy 4000.localhost:3009 up", None),
            ("see https://example.com:443/docs", None),
            ("nothing here", None),
        ] {
            assert_eq!(detect_service_url(log).as_deref(), expected, "{log}");
        }
    }

    #[test]
    fn 多个地址取第一个() {
        let log = "Local: http://localhost:3000\nNetwork: http://127.0.0.1:3001";
        assert_eq!(detect_service_url(log).as_deref(), Some("http://127.0.0.1:3000"));
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
}
