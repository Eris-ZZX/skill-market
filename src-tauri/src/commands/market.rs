//! 技能市场：拉取索引、下载、安装
//!
//! 关键设计：**一次下载整仓 zip，本地解析出所有技能**。
//!
//! 理由：GitLab 的 `archive.zip` 是单个请求，100 个技能的索引只花
//! 2 个请求（head commit + 整仓 zip），而不是 100 个。这对内网 GitLab
//! 非常友好，也不会因为技能多而变慢。
//!
//! 单个技能的安装则用 `archive.zip?path=` 只下那个目录。

use crate::frontmatter::SkillMeta;
use crate::gitlab::{GitLabClient, GitLabConfig};
use crate::install;
use crate::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};
use crate::tools::ToolKind;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::RemoteSkillView;

/// 索引内存缓存（进程内）
static INDEX_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<Vec<RemoteSkillView>>>> =
    std::sync::OnceLock::new();

fn cache() -> &'static std::sync::Mutex<Option<Vec<RemoteSkillView>>> {
    INDEX_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn index_cache_path() -> Result<PathBuf> {
    Ok(crate::storage::app_data_dir()?.join("index.json"))
}

fn head_cache_path() -> Result<PathBuf> {
    Ok(crate::storage::app_data_dir()?.join("head.json"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HeadCache {
    sha: String,
    fetched_at: i64,
}

fn read_head_cache() -> Option<HeadCache> {
    let p = head_cache_path().ok()?;
    let raw = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_head_cache(sha: &str) -> Result<()> {
    let p = head_cache_path()?;
    crate::storage::atomic_write_json(
        &p,
        &HeadCache {
            sha: sha.to_string(),
            fetched_at: chrono::Utc::now().timestamp(),
        },
    )
}

/// 拉取远端索引
///
/// 策略：
/// 1. 查 head commit（1 个请求）
/// 2. 与本地缓存的 head 比对
///    - 相同 → 直接用缓存的索引，结束
///    - 不同 → 下载整仓 zip，本地解析，重建索引
pub async fn fetch_index(cfg: GitLabConfig) -> Result<Vec<RemoteSkillView>> {
    let client = GitLabClient::new(cfg.clone())?;
    let head = client.head_commit().await?;

    // 缓存命中
    if let Some(cached) = read_head_cache() {
        if cached.sha == head {
            if let Some(idx) = load_cached_index().ok().filter(|v: &Vec<_>| !v.is_empty()) {
                log::info!("head 未变（{head}），使用缓存索引（{} 个技能）", idx.len());
                return Ok(idx);
            }
        }
    }

    log::info!("下载仓库归档（head={head}）");
    let zip_bytes = client.download_repo_zip(&head).await?;
    let skills = parse_repo_zip(&zip_bytes)?;
    log::info!("解析出 {} 个技能", skills.len());

    // 写缓存
    crate::storage::atomic_write_json(&index_cache_path()?, &skills)?;
    write_head_cache(&head)?;
    *cache().lock().unwrap() = Some(skills.clone());

    Ok(skills)
}

/// 读取已缓存的索引
pub fn load_cached_index() -> Result<Vec<RemoteSkillView>> {
    {
        let guard = cache().lock().unwrap();
        if let Some(v) = guard.as_ref() {
            return Ok(v.clone());
        }
    }
    let p = index_cache_path()?;
    if !p.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&p)?;
    let list: Vec<RemoteSkillView> = serde_json::from_str(&raw)?;
    *cache().lock().unwrap() = Some(list.clone());
    Ok(list)
}

/// 解析仓库 zip，找出所有含 SKILL.md 的目录
///
/// GitLab 的 archive 会带一层 `repo-branch-sha/` 前缀，按「第一段」剥掉。
fn parse_repo_zip(bytes: &[u8]) -> Result<Vec<RemoteSkillView>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("打开仓库归档失败")?;

    // 先收集「哪些目录含 SKILL.md」
    let mut skill_dirs: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if let Some(dir) = name.strip_suffix("/SKILL.md").or_else(|| name.strip_suffix("SKILL.md")) {
            let dir = dir.trim_end_matches('/');
            if !dir.is_empty() {
                skill_dirs.push(dir.to_string());
            }
        }
    }
    skill_dirs.sort();
    skill_dirs.dedup();

    let mut out = Vec::new();
    for raw_dir in skill_dirs {
        // 剥掉顶层 `repo-branch-sha/` 前缀
        let repo_path = strip_archive_root(&raw_dir);
        if repo_path.is_empty() {
            continue;
        }

        // 用技能目录的内容算哈希（与本地安装后的算法一致）
        let mut hasher_files: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            let Some(rel) = name.strip_prefix(&format!("{raw_dir}/")) else {
                continue;
            };
            if rel.is_empty() || entry.is_dir() {
                continue;
            }
            // 与 hash::hash_dir 的排除规则保持一致
            if rel.split('/').any(|seg| seg.starts_with('.') || seg == "__pycache__") {
                continue;
            }
            if rel.ends_with(".pyc") {
                continue;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            hasher_files.push((rel.to_string(), buf));
        }

        // 解析 SKILL.md 元信息
        let skill_md = hasher_files
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case("SKILL.md"))
            .and_then(|(_, b)| String::from_utf8(b.clone()).ok());

        let dir_name = repo_path.rsplit('/').next().unwrap_or(&repo_path).to_string();
        let meta = match skill_md {
            Some(content) => SkillMeta::from_str_content(&content, Some(&dir_name)),
            None => continue,
        };

        let hash = hash_from_files(&hasher_files, meta.name.clone())?;

        out.push(RemoteSkillView {
            name: meta.name,
            repo_path,
            hash,
            version: meta.version,
            description: meta.description,
            tags: meta.tags,
            category: meta.category,
            last_commit_at: None,
        });
    }

    Ok(out)
}

/// 剥掉 GitLab 归档自带的顶层目录前缀
/// GitLab 的归档根目录格式是 `<项目名>-<sha前8位>-<完整sha>/`，例如：
/// `skill-369d2a1a-369d2a1dcd209de95faaba529a2ed5cd32d6ca17/`
///
/// 注意 GitHub 是 `<repo>-<branch>/`，两者不同但「剥第一段」的规则通用。
/// 只有当路径不含斜杠时（理论上不该出现）才返回空。
fn strip_archive_root(p: &str) -> String {
    match p.split_once('/') {
        Some((_, rest)) => rest.to_string(),
        None => String::new(),
    }
}

/// 对内存中的文件集合算哈希
///
/// 必须与 `hash::hash_dir` 的算法完全一致，否则本地算出来的
/// 和远端对不上，会全员误报「有更新」。
fn hash_from_files(files: &[(String, Vec<u8>)], _name: String) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut sorted: Vec<_> = files.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (rel, bytes) in sorted {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        let normalized = if rel.eq_ignore_ascii_case("SKILL.md") {
            // 与本地 hash 保持相同的 frontmatter 规范化
            match String::from_utf8(bytes.clone()) {
                Ok(s) => normalize_md(&s).into_bytes(),
                Err(_) => normalize_newlines(&bytes),
            }
        } else {
            normalize_newlines(&bytes)
        };
        hasher.update(&normalized);
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn normalize_newlines(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' {
            out.push(b'\n');
            if i + 1 < b.len() && b[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            i += 1;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// frontmatter 规范化（与 hash 模块保持一致）
fn normalize_md(raw: &str) -> String {
    // 复用 hash 模块的逻辑：这里简化实现，剥离易变字段并按 key 排序
    // 完整实现见 hash::normalized_skill_md（该函数为私有，这里做等价处理）
    let text = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if !text.starts_with("---") {
        return text;
    }
    let after = &text[3..];
    let Some(close) = after.find("\n---") else {
        return text;
    };
    let fm = &after[..close];
    let body = &after[close + 4..];

    const VOLATILE: &[&str] = &[
        "enabled_at",
        "enabled_at_ms",
        "installed_at",
        "enabled",
        "last_opened_at",
        "last_used_at",
        "cli_version",
        "updated_at",
    ];

    let mut kept: Vec<String> = Vec::new();
    let mut skip = false;
    for line in fm.lines() {
        let cont = line.starts_with(' ') || line.starts_with('\t');
        if skip && cont {
            continue;
        }
        skip = false;
        if !cont {
            if let Some((k, _)) = line.trim_start().split_once(':') {
                if VOLATILE.contains(&k.trim()) {
                    skip = true;
                    continue;
                }
            }
        }
        kept.push(line.to_string());
    }

    // 按顶层 key 排序
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    for line in kept {
        let cont = line.starts_with(' ') || line.starts_with('\t') || line.starts_with('-');
        if cont {
            if let Some((_, v)) = cur.as_mut() {
                v.push(line);
                continue;
            }
            blocks.push((line.clone(), vec![]));
            continue;
        }
        let key = line
            .split_once(':')
            .map(|(k, _)| k.trim().to_string())
            .unwrap_or_else(|| line.clone());
        if let Some(done) = cur.take() {
            blocks.push(done);
        }
        cur = Some((key, vec![line]));
    }
    if let Some(done) = cur {
        blocks.push(done);
    }
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    let sorted = blocks
        .into_iter()
        .flat_map(|(_, v)| v)
        .collect::<Vec<_>>()
        .join("\n");

    format!("---\n{sorted}\n---\n{body}")
}

/// 从仓库安装一个技能到多个工具
pub async fn install_from_repo(
    cfg: GitLabConfig,
    name: &str,
    repo_path: &str,
    tools: &[String],
    allow_create_skills_dir: bool,
) -> Result<Vec<InstalledSkill>> {
    if tools.is_empty() {
        bail!("至少要选一个目标工具");
    }

    let client = GitLabClient::new(cfg.clone())?;
    let head = client.head_commit().await?;
    let zip_bytes = client.download_skill_zip(repo_path, &head).await?;

    // 解压到临时目录
    let tmp = tempfile::tempdir()?;
    extract_zip(&zip_bytes, tmp.path())?;

    // GitLab 归档带一层 `repo-branch-sha/`，剥掉
    let extracted = find_skill_dir(tmp.path())?;

    // 从实际文件解析版本号 —— 之前这里传了 None，导致 state.json 里
    // installedVersion 永远为空，界面上看不到版本。必须从 SKILL.md 读。
    let version = crate::frontmatter::SkillMeta::from_dir(&extracted)
        .ok()
        .and_then(|m| m.version);

    let mut results = Vec::new();
    for tool_key in tools {
        let kind = ToolKind::from_key(tool_key)
            .ok_or_else(|| anyhow::anyhow!("未知工具: {tool_key}"))?;

        let rec = install::install(
            kind,
            name,
            &extracted,
            InstallMode::Copy,
            SkillSource {
                kind: SourceKind::Gitlab,
                repo_url: Some(format!(
                    "{}/{}",
                    cfg.base_url.trim_end_matches('/'),
                    cfg.project_path
                )),
                repo_path: Some(repo_path.to_string()),
                branch: Some(cfg.branch.clone()),
                commit: Some(head.clone()),
            },
            version.clone(),
            allow_create_skills_dir,
        )?;
        results.push(rec);
    }

    Ok(results)
}

pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(path) = entry.enclosed_name() else {
            bail!("归档含非法路径: {}", entry.name());
        };
        let out = dest.join(path);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

/// 在解压结果里找到真正的技能目录（含 SKILL.md 的那一层）
pub fn find_skill_dir(root: &Path) -> Result<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        if dir.join("SKILL.md").is_file() {
            return Some(dir.to_path_buf());
        }
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            if entry.path().is_dir() {
                if let Some(found) = walk(&entry.path(), depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(root, 0).ok_or_else(|| anyhow::anyhow!("归档里没找到含 SKILL.md 的目录"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_root_removes_first_segment() {
        assert_eq!(
            strip_archive_root("repo-main-abc123/skills/销售/sales-report"),
            "skills/销售/sales-report"
        );
        assert_eq!(strip_archive_root("single"), "");
    }

    #[test]
    fn hash_is_stable_regardless_of_newlines() {
        let lf = vec![("SKILL.md".to_string(), b"---\nname: x\n---\n".to_vec())];
        let crlf = vec![(
            "SKILL.md".to_string(),
            b"---\r\nname: x\r\n---\r\n".to_vec(),
        )];
        assert_eq!(
            hash_from_files(&lf, "x".into()).unwrap(),
            hash_from_files(&crlf, "x".into()).unwrap()
        );
    }

    #[test]
    fn hash_ignores_volatile_fields() {
        let a = vec![(
            "SKILL.md".to_string(),
            b"---\nname: x\ndescription: d\n---\n".to_vec(),
        )];
        let b = vec![(
            "SKILL.md".to_string(),
            b"---\nname: x\ndescription: d\nenabled_at: 123\n---\n".to_vec(),
        )];
        assert_eq!(
            hash_from_files(&a, "x".into()).unwrap(),
            hash_from_files(&b, "x".into()).unwrap()
        );
    }
}
