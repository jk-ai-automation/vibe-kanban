//! 流水线技能插件：编进二进制、按内容哈希解压到数据目录，会话级注入 Claude Code。
//!
//! 插件只存在于两个地方：本仓库的 `assets/pipeline-plugin/`（源码）与
//! `asset_dir()/pipeline-plugin/<内容哈希>/`（解压产物）。**绝不写进用户仓库，也绝不动
//! `~/.claude`**——注入走 `claude --plugin-dir`，只对当次会话生效。
//!
//! rust-embed 默认不开 `debug-embed`：debug 构建下 `iter()`/`get()` 走文件系统实时读，
//! release 才真嵌入。两种模式下本模块的行为一致（哈希都按当下的内容算）。

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

use db::models::pipeline::PipelineStageKey;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::template::builtin_template;

/// `assets/pipeline-plugin` 本身就是一个 Claude Code 插件根目录
/// （里面直接是 `.claude-plugin/plugin.json` 与 `skills/`）。
#[derive(RustEmbed)]
#[folder = "../../assets/pipeline-plugin"]
pub struct PipelinePluginAssets;

/// 插件名，与 `.claude-plugin/plugin.json` 的 `name` 一致；技能按 `vk-pipeline:<技能名>` 引用。
pub const PLUGIN_NAME: &str = "vk-pipeline";
pub const LOCK_FILE: &str = "skills.lock.json";
/// 解压完成标记：存在即认为该哈希目录完整可用。
pub const READY_MARKER: &str = ".vk-plugin-ready";
const SKILLS_PREFIX: &str = "skills/";
const SKILL_FILE: &str = "SKILL.md";

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 整个插件的内容哈希（路径 + 长度 + 字节，按路径排序）。目录名用它，内容一变就换目录。
pub fn content_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut paths: Vec<String> = PipelinePluginAssets::iter()
            .map(|path| path.to_string())
            .collect();
        paths.sort();
        let mut hasher = Sha256::new();
        for path in &paths {
            let file = PipelinePluginAssets::get(path).expect("嵌入文件必然存在");
            hasher.update(path.as_bytes());
            hasher.update([0u8]);
            hasher.update((file.data.len() as u64).to_le_bytes());
            hasher.update(file.data.as_ref());
        }
        hex_lower(&hasher.finalize())
    })
    .clone()
}

/// 相对、不含 `..`、不是绝对路径、没有盘符。
fn is_safe_relative(path: &Path) -> bool {
    let mut any = false;
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return false;
        }
        any = true;
    }
    any
}

/// 解压到 `<root>/pipeline-plugin/<内容哈希>/` 并返回该目录。
///
/// 幂等：标记文件在就直接返回。先写同级临时目录再整体改名，别的进程看不到写了一半的插件；
/// 改名失败而目标已就绪（另一个进程抢先完成）时按成功处理。
pub fn ensure_extracted_in(root: &Path) -> std::io::Result<PathBuf> {
    let base = root.join("pipeline-plugin");
    let hash = content_hash();
    let target = base.join(&hash);
    if target.join(READY_MARKER).is_file() {
        return Ok(target);
    }
    std::fs::create_dir_all(&base)?;
    let staging = base.join(format!(".tmp-{hash}-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    for path in PipelinePluginAssets::iter() {
        let rel = Path::new(path.as_ref());
        if !is_safe_relative(rel) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("嵌入资源路径不安全：{}", path.as_ref()),
            ));
        }
        let file = PipelinePluginAssets::get(path.as_ref()).expect("嵌入文件必然存在");
        let dest = staging.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, file.data.as_ref())?;
    }
    std::fs::write(staging.join(READY_MARKER), hash.as_bytes())?;
    if let Err(e) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_dir_all(&staging);
        if !target.join(READY_MARKER).is_file() {
            return Err(e);
        }
    }
    Ok(target)
}

/// 解压到数据目录（`VK_ASSET_DIR` 可覆盖）。整个进程只做一次，失败也只记一次。
pub fn ensure_extracted() -> Result<PathBuf, String> {
    static DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    DIR.get_or_init(|| {
        ensure_extracted_in(&utils::assets::asset_dir())
            .map_err(|e| format!("解压流水线技能插件失败：{e}"))
    })
    .clone()
}

/// `skills/<name>/SKILL.md` → `Some(name)`；其余 → `None`。
fn skill_name_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix(SKILLS_PREFIX)?;
    let (name, file) = rest.split_once('/')?;
    (file == SKILL_FILE && !name.is_empty()).then(|| name.to_string())
}

/// 插件内的全部技能名（按名字排序）。
pub fn skill_names() -> &'static BTreeSet<String> {
    static NAMES: OnceLock<BTreeSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        PipelinePluginAssets::iter()
            .filter_map(|path| skill_name_of(path.as_ref()))
            .collect()
    })
}

/// 模板里的技能名 → 提示词里写的引用名（设计 §3.2 第 4 条）。
///
/// - 名字里已经有 `:`（自带命名空间）：原样。
/// - 是插件内技能：加 `vk-pipeline:` 前缀。
/// - 其余：原样（视为用户仓库 `.claude/skills/` 里自带的技能）。
pub fn qualify_skill(skill: &str) -> String {
    let name = skill.trim();
    if name.contains(':') || !skill_names().contains(name) {
        return name.to_string();
    }
    format!("{PLUGIN_NAME}:{name}")
}

// ---------------------------------------------------------------------------
// 锁文件与技能清单（设置页用；只读嵌入内容，不依赖解压）
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Lock {
    sources: BTreeMap<String, LockSource>,
    skills: Vec<LockSkill>,
}

#[derive(Debug, Deserialize)]
struct LockSource {
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LockSkill {
    name: String,
    source: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

/// 锁文件解析结果。解析不了时返回 None（技能清单退化成「全部算平台自写」，不影响流水线）。
fn lock() -> Option<&'static Lock> {
    static LOCK: OnceLock<Option<Lock>> = OnceLock::new();
    LOCK.get_or_init(|| {
        let file = PipelinePluginAssets::get(LOCK_FILE)?;
        match serde_json::from_slice::<Lock>(file.data.as_ref()) {
            Ok(lock) => Some(lock),
            Err(e) => {
                tracing::warn!("流水线技能锁文件解析失败，技能清单将不显示来源：{e}");
                None
            }
        }
    })
    .as_ref()
}

/// 平台自写技能在清单里的来源名。
pub const SOURCE_PLATFORM: &str = "platform";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PipelineSkillInfo {
    /// 目录名，例：`vk-requirement`
    pub name: String,
    /// 提示词里写的引用名，例：`vk-pipeline:vk-requirement`
    pub qualified_name: String,
    /// `platform`（平台自写）或锁文件里的来源名（`superpowers` / `atp`）
    pub source: String,
    /// 来源的发布版本或提交号；平台自写为 null
    pub source_version: Option<String>,
    /// 来源许可证；平台自写为 null
    pub license: Option<String>,
    /// 内置模板里用在哪个阶段；没被用到为 null
    pub stage: Option<PipelineStageKey>,
    /// 该技能目录下的文件数（外来技能取锁文件白名单，平台技能恒为 1）
    #[ts(type = "number")]
    pub file_count: i64,
}

/// 设置页「流水线技能」的数据。只读嵌入内容，不依赖解压是否成功。
pub fn skills_view() -> Vec<PipelineSkillInfo> {
    let lock = lock();
    let template = builtin_template();
    skill_names()
        .iter()
        .map(|name| {
            let entry = lock.and_then(|l| l.skills.iter().find(|skill| &skill.name == name));
            let source = entry
                .map(|skill| skill.source.clone())
                .unwrap_or_else(|| SOURCE_PLATFORM.to_string());
            let origin = entry.and_then(|skill| lock.and_then(|l| l.sources.get(&skill.source)));
            PipelineSkillInfo {
                name: name.clone(),
                qualified_name: format!("{PLUGIN_NAME}:{name}"),
                source,
                source_version: origin.and_then(|o| o.release.clone().or_else(|| o.commit.clone())),
                license: origin.and_then(|o| o.license.clone()),
                stage: template
                    .stages
                    .iter()
                    .find(|stage| &stage.skill == name)
                    .map(|stage| stage.key),
                file_count: entry.map_or(1, |skill| skill.files.len() as i64),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解压幂等且只写在给定根目录下() {
        let root = tempfile::tempdir().unwrap();
        let first = ensure_extracted_in(root.path()).unwrap();
        assert!(
            first.starts_with(root.path()),
            "{first:?} 必须在给定根目录下"
        );
        assert_eq!(
            first.file_name().unwrap().to_str().unwrap(),
            content_hash(),
            "目录名就是内容哈希"
        );
        assert!(first.join(".claude-plugin/plugin.json").is_file());
        assert!(first.join("skills/vk-requirement/SKILL.md").is_file());
        assert!(first.join(READY_MARKER).is_file());

        // 落盘的每一个文件都必须在根目录下（技能文件绝不外泄）。
        let mut count = 0usize;
        let mut stack = vec![root.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                assert!(path.starts_with(root.path()), "{path:?} 写到了根目录外");
                if path.is_dir() {
                    stack.push(path);
                } else {
                    count += 1;
                }
            }
        }
        assert!(count > 10, "解压出的文件数太少：{count}");

        // 幂等：再来一次不报错、目录不变、没有残留的临时目录。
        let second = ensure_extracted_in(root.path()).unwrap();
        assert_eq!(first, second);
        let leftovers: Vec<_> = std::fs::read_dir(root.path().join("pipeline-plugin"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时目录：{leftovers:?}");
    }

    #[test]
    fn 内容哈希稳定且内容变化时改变() {
        assert_eq!(content_hash(), content_hash());
        assert_eq!(content_hash().len(), 64);
        assert!(content_hash().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn 嵌入资源的路径都是安全的相对路径() {
        for path in PipelinePluginAssets::iter() {
            let path = path.to_string();
            assert!(
                is_safe_relative(std::path::Path::new(&path)),
                "不安全的嵌入路径：{path}"
            );
        }
    }

    #[test]
    fn 技能名解析三种情形() {
        assert_eq!(
            qualify_skill("vk-requirement"),
            "vk-pipeline:vk-requirement"
        );
        assert_eq!(qualify_skill("  vk-review  "), "vk-pipeline:vk-review");
        assert_eq!(
            qualify_skill("superpowers:brainstorming"),
            "superpowers:brainstorming",
            "已带命名空间的原样"
        );
        assert_eq!(
            qualify_skill("我们仓库自带的技能"),
            "我们仓库自带的技能",
            "不在插件里的原样"
        );
    }

    fn skill_text(name: &str) -> String {
        let file = PipelinePluginAssets::get(&format!("skills/{name}/{SKILL_FILE}"))
            .unwrap_or_else(|| panic!("技能 {name} 没有 SKILL.md"));
        String::from_utf8(file.data.to_vec())
            .unwrap_or_else(|_| panic!("技能 {name} 的 SKILL.md 不是 UTF-8"))
    }

    /// frontmatter 的 `key: value`（只取顶层、不解析嵌套 YAML——技能的 frontmatter 就这两个键）。
    fn frontmatter(name: &str) -> BTreeMap<String, String> {
        let text = skill_text(name);
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("---"),
            "技能 {name} 的 SKILL.md 第一行必须是 ---"
        );
        let mut map = BTreeMap::new();
        for line in lines {
            if line.trim() == "---" {
                return map;
            }
            if let Some((key, value)) = line.split_once(": ") {
                map.insert(
                    key.trim().to_string(),
                    value.trim().trim_matches('"').to_string(),
                );
            }
        }
        panic!("技能 {name} 的 frontmatter 没有闭合的 ---")
    }

    /// 「## 流水线约定」到下一个 `## ` 之间的正文。
    fn convention_section(name: &str) -> String {
        let text = skill_text(name);
        let body = text
            .split_once("## 流水线约定")
            .unwrap_or_else(|| panic!("技能 {name} 没有「## 流水线约定」一节"))
            .1;
        body.split("\n## ")
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    #[test]
    fn 每个技能的_frontmatter_合法且_name_与目录一致() {
        assert!(skill_names().len() >= 14, "技能太少：{:?}", skill_names());
        for name in skill_names() {
            let front = frontmatter(name);
            assert_eq!(
                front.get("name").map(String::as_str),
                Some(name.as_str()),
                "技能 {name} 的 frontmatter name 与目录名不一致"
            );
            let description = front
                .get("description")
                .unwrap_or_else(|| panic!("技能 {name} 缺 description"));
            assert!(
                !description.trim().is_empty(),
                "技能 {name} 的 description 是空的"
            );
        }
    }

    #[test]
    fn 平台技能都带同一段流水线约定() {
        let baseline = convention_section("vk-requirement");
        assert!(
            baseline.contains("不要向人提问"),
            "约定段内容不对：{baseline}"
        );
        for name in [
            "vk-spec",
            "vk-develop",
            "vk-review",
            "vk-deliver",
            "atp-run",
            "prd2testcase",
        ] {
            assert_eq!(
                convention_section(name),
                baseline,
                "技能 {name} 的「流水线约定」与 vk-requirement 不一致"
            );
        }
    }

    #[test]
    fn 平台技能正文提到的产出文件名与契约一致() {
        for stage in builtin_template().stages {
            if stage.artifacts.is_empty() {
                continue; // develop 阶段不产出文件
            }
            let text = skill_text(&stage.skill);
            for artifact in &stage.artifacts {
                assert!(
                    text.contains(artifact.as_str()),
                    "技能 {} 的正文没提到它必须产出的 {}（契约 §4）",
                    stage.skill,
                    artifact
                );
            }
        }
    }

    #[test]
    fn 锁文件里的哈希与插件里的内容一致() {
        let lock = lock().expect("skills.lock.json 必须能解析");
        assert!(!lock.skills.is_empty(), "锁文件里一个外来技能都没有");
        for skill in &lock.skills {
            assert!(
                lock.sources.contains_key(&skill.source),
                "技能 {} 的来源 {} 不在 sources 里",
                skill.name,
                skill.source
            );
            for (file, want) in &skill.files {
                let path = format!("skills/{}/{}", skill.name, file);
                let embedded = PipelinePluginAssets::get(&path)
                    .unwrap_or_else(|| panic!("{path} 在锁文件里但不在插件里"));
                let mut hasher = Sha256::new();
                hasher.update(embedded.data.as_ref());
                assert_eq!(
                    &hex_lower(&hasher.finalize()),
                    want,
                    "{path} 与锁文件不一致：跑 node scripts/sync-pipeline-skills.mjs --update"
                );
            }
        }
    }

    #[test]
    fn 已验证支持_plugin_dir_的_claude_版本没有变() {
        // 平台跑的是 `npx -y @anthropic-ai/claude-code@<版本>`。这个版本号一变就要重新验
        // 一遍 --plugin-dir（计划 §4 待核实项 A 的验收脚本），所以在这里钉住。
        // 若因文件移动导致 include_str! 编译失败，按新路径改这里并重新验收。
        let source = include_str!("../../../../executors/src/executors/claude.rs");
        assert!(
            source.contains("@anthropic-ai/claude-code@2.1.274"),
            "Claude Code 的固定版本变了：请重新验证 --plugin-dir 支持情况，再更新本测试与计划 §2.1"
        );
    }

    #[test]
    fn 内置模板的七个技能都在插件里() {
        let missing: Vec<String> = builtin_template()
            .stages
            .iter()
            .filter(|stage| !skill_names().contains(&stage.skill))
            .map(|stage| format!("{}（阶段 {}）", stage.skill, stage.key.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "内置模板用到但插件里没有的技能：{missing:?}"
        );
        for stage in builtin_template().stages {
            assert!(
                qualify_skill(&stage.skill).starts_with("vk-pipeline:"),
                "{} 应该带插件前缀",
                stage.skill
            );
        }
    }
}
