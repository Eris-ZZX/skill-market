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
use crate::hash;
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
            if hash::should_skip(rel) {
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

        let hash = hash::hash_files(&hasher_files)?;

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

/// 从仓库安装一个技能到多个工具
pub async fn install_from_repo(
    cfg: GitLabConfig,
    name: &str,
    repo_path: &str,
    tools: &[String],
    mode: InstallMode,
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

    let kinds: Vec<ToolKind> = tools
        .iter()
        .map(|k| ToolKind::from_key(k).ok_or_else(|| anyhow::anyhow!("未知工具: {k}")))
        .collect::<Result<_>>()?;

    let library_root = crate::storage::library_dir()?;
    install::install_many(
        name,
        &extracted,
        mode,
        &kinds,
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
        version,
        allow_create_skills_dir,
        &library_root,
    )
}

/// 下载到的技能包（尚未落地）
pub struct DownloadedSkill {
    pub dir: tempfile::TempDir,
    pub extracted: PathBuf,
    pub version: Option<String>,
    pub commit: String,
}

/// 只下载并解压，不碰本地任何文件
///
/// 拆成两步是为了让调用方能在「下载完成」与「开始改本地」之间插入
/// 记录校验——下载耗时几十秒，期间用户可能已经卸载了这个技能。
pub async fn download_skill_package(
    cfg: GitLabConfig,
    repo_path: &str,
) -> Result<DownloadedSkill> {
    let client = GitLabClient::new(cfg)?;
    let head = client.head_commit().await?;
    let zip_bytes = client.download_skill_zip(repo_path, &head).await?;

    let tmp = tempfile::tempdir()?;
    extract_zip(&zip_bytes, tmp.path())?;
    let extracted = find_skill_dir(tmp.path())?;
    let version = crate::frontmatter::SkillMeta::from_dir(&extracted)
        .ok()
        .and_then(|m| m.version);

    Ok(DownloadedSkill {
        dir: tmp,
        extracted,
        version,
        commit: head,
    })
}

/// 把下载到的内容替换进库，并按每条记录自己的 mode 处理工具目录
///
/// 返回新的内容哈希与新版本号。
pub fn deploy_downloaded(
    dl: &DownloadedSkill,
    name: &str,
    targets: &[(PathBuf, InstallMode)],
) -> Result<(String, Option<String>)> {
    // 危险操作前校验名字：后面所有路径都用它拼
    crate::install::require_valid_directory(name)?;
    let _ = &dl.dir; // 临时目录守卫必须活到部署结束

    let library_root = crate::storage::library_dir()?;
    crate::install::update_deploy(&dl.extracted, &library_root, name, targets)?;

    let new_hash = crate::hash::hash_dir(&library_root.join(name))?;
    Ok((new_hash, dl.version.clone()))
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
            hash::hash_files(&lf).unwrap(),
            hash::hash_files(&crlf).unwrap()
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
            hash::hash_files(&a).unwrap(),
            hash::hash_files(&b).unwrap()
        );
    }
}
