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

/// 可自动化用例数：`NOAUTO` 为空的行按 `CaseID` 去重（同一编号多行是多个步骤）。
/// 「空」的口径与 atp 一致（`legacy_csv/matching.py` 的 `is_empty_value`）：去首尾空白、
/// 转小写后是 `""`、`nan` 或 `none`；其它任何非空值（`Y`、`N`、`是`……）都算人工步骤。
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
        let manual = is_manual_step(record.get(noauto).unwrap_or_default());
        if !id.is_empty() && !manual {
            cases.insert(id.to_string());
        }
    }
    Ok(cases.len())
}

/// atp 跳过手工步骤的判定：`NOAUTO` 去首尾空白、转小写后不是 `""` / `nan` / `none`。
fn is_manual_step(noauto: &str) -> bool {
    !matches!(noauto.trim().to_lowercase().as_str(), "" | "nan" | "none")
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

/// 不完整的行最多留这么多字节等后续输出；更长的（进度条之类）直接认完丢掉。
const MAX_PARTIAL_LINE: usize = 1024;

/// 增量识别开发服务器日志里的地址候选：每次只扫新到的输出，按出现顺序去重累积。
/// 只扫完整的行（以换行或回车结尾）；末尾不完整的一段留到下次与新输出拼起来再扫，
/// 免得把被截断的 `localhost:51` 当成候选。
#[derive(Debug, Default)]
pub struct ServiceUrlScanner {
    partial: String,
    found: Vec<String>,
}

impl ServiceUrlScanner {
    pub fn feed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.partial.push_str(text);
        let complete_end = self.partial.rfind(['\n', '\r']).map_or(0, |at| at + 1);
        let scan_end = if self.partial.len() - complete_end > MAX_PARTIAL_LINE {
            self.partial.len()
        } else {
            complete_end
        };
        for url in service_url_candidates(&self.partial[..scan_end]) {
            if !self.found.contains(&url) {
                self.found.push(url);
            }
        }
        self.partial.drain(..scan_end);
    }

    /// 到目前为止认出的候选，按出现顺序。
    pub fn found(&self) -> &[String] {
        &self.found
    }
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

/// 向系统要一个当前空闲的回环端口。拿到后立刻释放，存在被别人抢走的极小窗口。
/// 抢走的后果有两种（概率都极低）：开发服务器起不来 → 就绪探测超时 → 报告如实记失败；
/// 或者抢走端口的进程会回 HTTP，探测命中它而被误判为就绪，接口用例打到错的服务上失败。
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

#[cfg(test)]
mod tests {
    use executors::pipeline_prompt::MOCK_TEST_CASES_HEADER;

    use super::*;

    #[test]
    fn 增量识别_跨两段输出的行拼起来再认() {
        let mut scanner = ServiceUrlScanner::default();
        scanner.feed("ECONNREFUSED 127.0.0.1:5432\n  ➜  Local:   http://local");
        assert_eq!(scanner.found(), ["http://127.0.0.1:5432"]);
        scanner.feed("host:5173/\n");
        assert_eq!(
            scanner.found(),
            [
                "http://127.0.0.1:5432",
                "http://127.0.0.1:5173",
                "http://[::1]:5173"
            ]
        );
    }

    #[test]
    fn 增量识别_不完整的行先不认_重复地址只记一次() {
        let mut scanner = ServiceUrlScanner::default();
        scanner.feed("listening on http://localhost:51");
        assert!(scanner.found().is_empty(), "{:?}", scanner.found());
        scanner.feed("73\nagain http://127.0.0.1:5173\r");
        assert_eq!(
            scanner.found(),
            ["http://127.0.0.1:5173", "http://[::1]:5173"]
        );
    }

    #[test]
    fn 增量识别_超长的不完整行也会被认并丢弃() {
        let mut scanner = ServiceUrlScanner::default();
        scanner.feed(&format!("http://localhost:3000 {}", "x".repeat(2000)));
        assert_eq!(
            scanner.found(),
            ["http://127.0.0.1:3000", "http://[::1]:3000"]
        );
        scanner.feed("y\n");
        assert_eq!(scanner.found().len(), 2);
    }

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
        let text = csv_with(&[
            ("a_001", ""),
            ("a_001", ""),
            ("a_002", "Y"),
            ("a_003", " y "),
        ]);
        assert_eq!(count_automatable_cases(&text), Ok(1));
    }

    #[test]
    fn 人工用例判定与_atp_一致_非空即人工() {
        for manual in ["N", "n", "是", "manual", "0", "Y", " N "] {
            assert_eq!(
                count_automatable_cases(&csv_with(&[("a_001", manual)])),
                Ok(0),
                "NOAUTO={manual:?} 应算人工"
            );
        }
        for automatable in ["", "  ", "nan", "NaN", " NAN ", "none", "None", "NONE"] {
            assert_eq!(
                count_automatable_cases(&csv_with(&[("a_001", automatable)])),
                Ok(1),
                "NOAUTO={automatable:?} 应算自动化"
            );
        }
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
