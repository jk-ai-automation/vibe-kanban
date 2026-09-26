//! 产出物读盘与入库（设计 §5、§6.2；契约 §4）。
//!
//! 产出物目录 = `<工作区目录>/.vk/runs/<simple_id>/`。工作区目录下每个仓库是子目录
//! （`crates/workspace-manager/src/workspace_manager.rs` 的 `workspace_dir.join(&repo.name)`），
//! 这个位置不在任何 git 仓库里。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use db::models::pipeline::{ArtifactKind, IssueArtifacts, truncate_for_storage};
use sqlx::SqlitePool;
use uuid::Uuid;

pub fn artifacts_dir(workspace_root: &Path, simple_id: &str) -> PathBuf {
    workspace_root
        .join(".vk")
        .join("runs")
        .join(sanitize_segment(simple_id))
}

/// 相对工作区根目录的路径，存进 `issue_artifacts.rel_path`。
pub fn relative_path(simple_id: &str, file_name: &str) -> String {
    format!(".vk/runs/{}/{}", sanitize_segment(simple_id), file_name)
}

/// 目录名只保留 ASCII 字母数字、`-`、`_`，其余替换成 `_`，防止路径穿越。
pub fn sanitize_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// 目录里已有的产出物文件名（不含 `*.prev` 旧版本与子目录），排序后返回。
pub fn list_existing(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.ends_with(".prev"))
        .collect();
    names.sort();
    names
}

/// 产出物只能是目录下的纯文件名：不含路径分隔符，不是空串、`.` 或 `..`。
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// 阶段启动前把它声明的旧产出物改名为 `<name>.prev`（已有的 `.prev` 覆盖）：
/// 本轮判定只看本轮新写的文件，不会把上一轮的旧文件当成结果。不存在的跳过。
pub fn retire_declared(dir: &Path, declared: &[String]) -> std::io::Result<()> {
    for name in declared.iter().filter(|name| is_plain_file_name(name)) {
        let path = dir.join(name);
        if path.is_file() {
            std::fs::rename(&path, dir.join(format!("{name}.prev")))?;
        }
    }
    Ok(())
}

/// 读阶段声明的产出物：存在且非空白的才收进来。非 UTF-8 按有损转换。
/// 不是纯文件名的声明（模板写了路径）一律不读，防止读到产出物目录之外的文件。
pub fn read_declared(dir: &Path, declared: &[String]) -> BTreeMap<String, String> {
    declared
        .iter()
        .filter(|name| is_plain_file_name(name))
        .filter_map(|name| {
            let bytes = std::fs::read(dir.join(name)).ok()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            (!text.trim().is_empty()).then(|| (name.clone(), text))
        })
        .collect()
}

/// 读盘并入库：与该 kind 最近一版（截断后）内容相同的不重复入库；不认识的文件名只读不入库。
/// 返回读到的全文，给关卡判定用（判定看磁盘全文，不看库里截断后的内容）。
pub async fn ingest_stage_artifacts(
    pool: &SqlitePool,
    issue_id: Uuid,
    simple_id: &str,
    stage_run_id: Uuid,
    dir: &Path,
    declared: &[String],
) -> Result<BTreeMap<String, String>, sqlx::Error> {
    let present = read_declared(dir, declared);
    for (name, content) in &present {
        let Some(kind) = ArtifactKind::from_file_name(name) else {
            continue;
        };
        let (stored, _) = truncate_for_storage(content);
        if let Some(latest) = IssueArtifacts::latest_for_kind(pool, issue_id, kind).await?
            && latest.content == stored
        {
            continue;
        }
        IssueArtifacts::insert_version(
            pool,
            issue_id,
            stage_run_id,
            kind,
            &relative_path(simple_id, name),
            content,
        )
        .await?;
    }
    Ok(present)
}

#[cfg(test)]
mod tests {
    use db::{
        models::pipeline::{ArtifactKind, IssueArtifacts, PipelineStageKey},
        test_support::TestDb,
    };

    use super::*;
    use crate::services::pipeline::test_support::{准备运行与阶段, 准备需求};

    #[test]
    fn 产出物目录在工作区根下的_vk_runs() {
        assert_eq!(
            artifacts_dir(Path::new("/ws"), "VK-12"),
            PathBuf::from("/ws/.vk/runs/VK-12")
        );
        assert_eq!(sanitize_segment("../x y"), "___x_y");
        assert_eq!(sanitize_segment(""), "_");
        assert_eq!(relative_path("VK-12", "spec.md"), ".vk/runs/VK-12/spec.md");
    }

    #[test]
    fn 已有产出物清单排序且不含旧版本() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["spec.md", "requirement.md", "review.json.prev"] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(list_existing(dir.path()), vec!["requirement.md", "spec.md"]);
        assert!(list_existing(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn 只读声明过且非空的产出物() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# 规格").unwrap();
        std::fs::write(dir.path().join("plan.md"), "  \n").unwrap();
        std::fs::write(dir.path().join("other.md"), "不该读").unwrap();
        let declared = vec![
            "spec.md".to_string(),
            "plan.md".to_string(),
            "x.md".to_string(),
        ];
        let read = read_declared(dir.path(), &declared);
        assert_eq!(read.keys().collect::<Vec<_>>(), vec!["spec.md"]);
    }

    #[test]
    fn 启动前把本阶段旧产出物改名为_prev() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("review.json"), "新旧").unwrap();
        std::fs::write(dir.path().join("review.json.prev"), "更旧").unwrap();
        std::fs::write(dir.path().join("spec.md"), "别的阶段").unwrap();
        let declared = vec![
            "review.json".to_string(),
            "missing.md".to_string(),
            "../spec.md".to_string(),
        ];
        retire_declared(dir.path(), &declared).unwrap();
        assert!(!dir.path().join("review.json").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("review.json.prev")).unwrap(),
            "新旧",
            "已有的 *.prev 被覆盖"
        );
        assert!(dir.path().join("spec.md").exists(), "未声明的不动");
        assert!(!dir.path().join("missing.md.prev").exists());
        retire_declared(&dir.path().join("none"), &declared).unwrap();
    }

    #[test]
    fn 声明的文件名带路径时不读() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(root.path().join("secret.md"), "目录外").unwrap();
        std::fs::write(dir.join("sub").join("a.md"), "子目录").unwrap();
        std::fs::write(dir.join("ok.md"), "正常").unwrap();
        let declared = vec![
            "../secret.md".to_string(),
            "sub/a.md".to_string(),
            "sub\\a.md".to_string(),
            "..".to_string(),
            ".".to_string(),
            "".to_string(),
            root.path().join("secret.md").to_string_lossy().into_owned(),
            "ok.md".to_string(),
        ];
        let read = read_declared(&dir, &declared);
        assert_eq!(read.keys().collect::<Vec<_>>(), vec!["ok.md"]);
    }

    #[tokio::test]
    async fn 入库时内容未变不重复写版本() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "入库").await;
        let (_run, stage) = 准备运行与阶段(&test_db, &issue, PipelineStageKey::Spec).await;
        let dir = tempfile::tempdir().unwrap();
        let declared = vec!["spec.md".to_string(), "plan.md".to_string()];

        std::fs::write(dir.path().join("spec.md"), "第一版").unwrap();
        let read = ingest_stage_artifacts(
            test_db.pool(),
            issue.id,
            &issue.simple_id,
            stage.id,
            dir.path(),
            &declared,
        )
        .await
        .unwrap();
        assert_eq!(read.len(), 1, "plan.md 不存在，不读");

        ingest_stage_artifacts(
            test_db.pool(),
            issue.id,
            &issue.simple_id,
            stage.id,
            dir.path(),
            &declared,
        )
        .await
        .unwrap();
        std::fs::write(dir.path().join("spec.md"), "第二版").unwrap();
        ingest_stage_artifacts(
            test_db.pool(),
            issue.id,
            &issue.simple_id,
            stage.id,
            dir.path(),
            &declared,
        )
        .await
        .unwrap();

        let all = IssueArtifacts::list_summaries_for_issue(test_db.pool(), issue.id)
            .await
            .unwrap();
        assert_eq!(all.len(), 2, "相同内容只入库一次");
        assert_eq!(all[1].version, 2);
        assert_eq!(all[1].kind, ArtifactKind::Spec);
        assert_eq!(
            all[1].rel_path,
            format!(".vk/runs/{}/spec.md", sanitize_segment(&issue.simple_id))
        );
    }

    #[tokio::test]
    async fn 不认识的文件名只读不入库() {
        let test_db = TestDb::new().await;
        let issue = 准备需求(&test_db, "未知文件").await;
        let (_run, stage) = 准备运行与阶段(&test_db, &issue, PipelineStageKey::Develop).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "随手记").unwrap();
        let declared = vec!["notes.md".to_string()];

        let read = ingest_stage_artifacts(
            test_db.pool(),
            issue.id,
            &issue.simple_id,
            stage.id,
            dir.path(),
            &declared,
        )
        .await
        .unwrap();
        assert_eq!(read.len(), 1, "读到的全文照样交给关卡判定");
        assert!(
            IssueArtifacts::list_summaries_for_issue(test_db.pool(), issue.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
