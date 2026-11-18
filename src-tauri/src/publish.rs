//! 发布：把本地技能推进内网 GitLab
//!
//! 设计要点（对应「取消 MR、直推 master」的决策）：
//!
//! 1. **直推 master** —— 依赖 bot 具备 Maintainer 权限
//! 2. **发布前快照** —— 先把当前 master 的该技能内容备份到本地，
//!    万一推坏了能一键回滚（补偿取消人工审核的风险）
//! 3. **署名声明** —— 内置 Token 意味着 GitLab 只看到 bot，
//!    因此发布人信息写进 commit message 和 frontmatter
//!
//! 为什么不用 MR：内网小团队、无 Runner、技能是 Markdown 而非生产代码，
//! 「人工点 MR」的仪式感大于实际价值。改用「留名 + 可回滚」更可靠。

use crate::frontmatter::SkillMeta;
use crate::gitlab::{GitLabClient, GitLabConfig};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 发布请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    /// 本地技能目录（含 SKILL.md）
    pub local_dir: String,
    /// 目标分类（如「通用」「研发」），为空则用 frontmatter 里的 category
    pub category: Option<String>,
    /// 发布人署名（姓名/工号）——内置 Token 下这是唯一的身份来源
    pub publisher: String,
    /// 提交说明（可选）
    pub message: Option<String>,
}

/// 发布结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    pub name: String,
    pub repo_path: String,
    pub commit_sha: String,
    /// 发布前的快照 id（可用于回滚）
    pub snapshot_id: Option<String>,
    /// 提交信息（给用户看）
    pub commit_message: String,
}

/// 发布前的本地快照
///
/// 目的：直推 master 少了人工门禁，至少保证「推坏了能退回去」。
/// 快照存在本地，不占仓库空间，也不受 Token 权限影响。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub skill_name: String,
    pub repo_path: String,
    pub commit_sha: String,
    pub created_at: i64,
    /// 快照目录的绝对路径
    pub dir: String,
}

fn snapshots_root() -> Result<std::path::PathBuf> {
    let dir = crate::storage::app_data_dir()?.join("snapshots");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn snapshot_index_path() -> Result<std::path::PathBuf> {
    Ok(snapshots_root()?.join("index.json"))
}

/// 读取所有快照（最多保留的由 prune 控制）
pub fn list_snapshots() -> Result<Vec<Snapshot>> {
    let p = snapshot_index_path()?;
    if !p.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(p)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_snapshots(list: Vec<Snapshot>) -> Result<()> {
    crate::storage::atomic_write_json(&snapshot_index_path()?, &list)
}

/// 每个技能最多保留的快照数
const MAX_SNAPSHOTS_PER_SKILL: usize = 10;

/// 把一份技能内容存为快照
fn create_snapshot(
    skill_name: &str,
    repo_path: &str,
    commit_sha: &str,
    source_dir: &Path,
) -> Result<Snapshot> {
    let id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        &commit_sha[..8.min(commit_sha.len())]
    );
    let dir = snapshots_root()?.join(&id);
    std::fs::create_dir_all(&dir)?;
    copy_dir(source_dir, &dir)?;

    let snap = Snapshot {
        id: id.clone(),
        skill_name: skill_name.to_string(),
        repo_path: repo_path.to_string(),
        commit_sha: commit_sha.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        dir: dir.to_string_lossy().to_string(),
    };

    // 写入索引并做淘汰
    let mut all = list_snapshots().unwrap_or_default();
    all.push(snap.clone());

    // 同一技能只保留最近 N 个
    let mut by_skill: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, s) in all.iter().enumerate() {
        by_skill.entry(s.skill_name.clone()).or_default().push(i);
    }
    let mut drop_idx: Vec<usize> = Vec::new();
    for idxs in by_skill.values() {
        if idxs.len() > MAX_SNAPSHOTS_PER_SKILL {
            // 按 created_at 排序，标记最旧的
            let mut sorted = idxs.clone();
            sorted.sort_by_key(|&i| all[i].created_at);
            let excess = sorted.len() - MAX_SNAPSHOTS_PER_SKILL;
            for &i in sorted.iter().take(excess) {
                drop_idx.push(i);
            }
        }
    }
    drop_idx.sort_unstable();
    drop_idx.reverse();
    for i in drop_idx {
        let removed = all.remove(i);
        let _ = std::fs::remove_dir_all(&removed.dir);
    }

    save_snapshots(all)?;
    Ok(snap)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let e = entry?;
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 发布一个技能
///
/// 流程：
/// 1. 校验本地目录、解析 frontmatter
/// 2. 生成/补全 `metadata.skillmarket`（作者、分类、owner）
/// 3. 对当前 master 上的同名技能做快照（若存在）
/// 4. 用 GitLab Commits API 一次提交所有文件（新建或更新）
///
/// 注意：Commits API 支持 `actions` 数组，一次请求提交多文件，
/// 避免多次往返导致中间状态不一致。
pub async fn publish(cfg: GitLabConfig, req: PublishRequest) -> Result<PublishResult> {
    let local = Path::new(&req.local_dir);
    if !local.join("SKILL.md").is_file() {
        bail!("{} 下没有 SKILL.md", local.display());
    }
    if req.publisher.trim().is_empty() {
        bail!("请填写发布人（姓名或工号）");
    }

    // 1. 解析元信息
    let mut meta = SkillMeta::from_dir(local)?;
    if !SkillMeta::is_valid_name(&meta.name) {
        bail!(
            "技能名不合法：{:?}（要求小写字母、数字、连字符，且不以连字符开头/结尾）",
            meta.name
        );
    }

    // 2. 分类：显式指定 > frontmatter > 默认「通用」
    let category = req
        .category
        .clone()
        .or(meta.category.clone())
        .unwrap_or_else(|| "通用".to_string());

    if category.contains('/') || category.contains('\\') || category.contains("..") {
        bail!("分类名含非法字符: {category}");
    }

    let repo_path = format!("skills/{}/{}", category, meta.name);

    // 3. 查当前 master 上的同名技能，为发布前快照做准备
    let client = GitLabClient::new(cfg.clone())?;
    let head = client.head_commit().await?;

    let mut snapshot_id = None;
    if let Ok(existing) = client.download_skill_zip(&repo_path, &head).await {
        if let Ok(tmp) = tempfile::tempdir() {
            if crate::commands::market::extract_zip(&existing, tmp.path()).is_ok() {
                if let Ok(dir) = crate::commands::market::find_skill_dir(tmp.path()) {
                    if let Ok(s) = create_snapshot(&meta.name, &repo_path, &head, &dir) {
                        log::info!("已创建发布前快照: {}", s.id);
                        snapshot_id = Some(s.id);
                    }
                }
            }
        }
    }

    // 4. 收集要提交的文件
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_files(local, local, &mut files)?;
    if files.is_empty() {
        bail!("目录里没有可提交的文件");
    }

    // 对 SKILL.md 补全元信息
    let mut final_files: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    for (rel, bytes) in files {
        if rel.eq_ignore_ascii_case("SKILL.md") {
            let text = String::from_utf8(bytes.clone()).context("SKILL.md 不是合法 UTF-8")?;
            let patched = inject_meta(&text, &category, &req.publisher);
            final_files.push((rel, patched.into_bytes()));
        } else {
            final_files.push((rel, bytes));
        }
    }
    // 更新内存里的 meta（供返回用）
    meta.category = Some(category.clone());

    // 5. 提交
    let commit_message = build_commit_message(&req, &meta.name);
    let sha = client
        .commit_files(&repo_path, &final_files, &commit_message, &req.publisher)
        .await?;

    Ok(PublishResult {
        name: meta.name,
        repo_path,
        commit_sha: sha,
        snapshot_id,
        commit_message,
    })
}

fn build_commit_message(req: &PublishRequest, name: &str) -> String {
    let base = req
        .message
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("更新技能 {name}"));
    // 署名放进标题，让 GitLab 提交列表一眼可见
    format!("[发布人: {}] {}", req.publisher.trim(), base)
}

/// 在 SKILL.md 的 frontmatter 里写入/更新 metadata.skillmarket
fn inject_meta(raw: &str, category: &str, publisher: &str) -> String {
    let text = raw.trim_start_matches('\u{feff}');

    if !text.starts_with("---") {
        // 无 frontmatter：补一个最小的
        return format!(
            "---\nname: {}\nmetadata:\n  skillmarket:\n    category: {}\n    authors:\n      - {}\n---\n{}",
            "unknown", category, publisher, text
        );
    }

    let after = &text[3..];
    let Some(close) = after.find("\n---") else {
        return text.to_string();
    };
    let fm = &after[..close];
    let body = &after[close + 4..];

    // 已有 metadata.skillmarket 就不动，避免覆盖用户手填的内容
    if fm.contains("skillmarket") {
        return text.to_string();
    }

    let mut extra = String::from("\nmetadata:\n  skillmarket:\n");
    extra.push_str(&format!("    category: {category}\n"));
    extra.push_str("    authors:\n");
    extra.push_str(&format!("      - {}\n", publisher.trim()));

    format!("---{}{}\n---\n{}", fm, extra.trim_end_matches('\n'), body)
}

/// 递归收集文件，返回 (仓库内相对路径, 内容)
///
/// 排除规则与哈希保持一致：隐藏文件、__pycache__、*.pyc 不进仓库。
fn collect_files(base: &Path, current: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "__pycache__" || name.ends_with(".pyc") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, std::fs::read(&path)?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_message_has_publisher() {
        let req = PublishRequest {
            local_dir: "x".into(),
            category: None,
            publisher: "张三/1234".into(),
            message: Some("修复触发条件".into()),
        };
        let msg = build_commit_message(&req, "demo");
        assert!(msg.contains("张三/1234"));
        assert!(msg.contains("修复触发条件"));
        assert!(msg.starts_with("[发布人:"));
    }

    #[test]
    fn commit_message_defaults_to_update_title() {
        let req = PublishRequest {
            local_dir: "x".into(),
            category: None,
            publisher: "李四".into(),
            message: None,
        };
        let msg = build_commit_message(&req, "sales-report");
        assert_eq!(msg, "[发布人: 李四] 更新技能 sales-report");
    }

    #[test]
    fn inject_meta_adds_block_when_missing() {
        let raw = "---\nname: demo\ndescription: d\n---\nbody\n";
        let out = inject_meta(raw, "销售", "张三");
        assert!(out.contains("metadata:"));
        assert!(out.contains("skillmarket:"));
        assert!(out.contains("category: 销售"));
        assert!(out.contains("- 张三"));
    }

    #[test]
    fn inject_meta_preserves_existing() {
        let raw = "---\nname: demo\nmetadata:\n  skillmarket:\n    category: 已有\n---\nbody\n";
        let out = inject_meta(raw, "销售", "张三");
        assert_eq!(out, raw, "已有 skillmarket 块时不应改动");
    }

    #[test]
    fn collect_files_skips_hidden_and_pycache() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("__pycache__")).unwrap();
        std::fs::write(base.join("SKILL.md"), "x").unwrap();
        std::fs::write(base.join(".DS_Store"), "junk").unwrap();
        std::fs::write(base.join("__pycache__/a.pyc"), "junk").unwrap();
        std::fs::write(base.join("run.pyc"), "junk").unwrap();
        std::fs::create_dir_all(base.join("scripts")).unwrap();
        std::fs::write(base.join("scripts/a.py"), "print(1)").unwrap();

        let mut out = Vec::new();
        collect_files(base, base, &mut out).unwrap();
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"SKILL.md"));
        assert!(names.contains(&"scripts/a.py"));
        assert!(!names.iter().any(|n| n.contains(".DS_Store")));
        assert!(!names.iter().any(|n| n.contains("pyc")));
    }
}
