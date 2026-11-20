//! 部署引擎
//!
//! 把一个技能目录放进目标工具的技能目录，并记录安装状态。
//!
//! 两条硬约束：
//! - **R1**：只写已存在的工具目录，绝不创建工具根目录
//! - **R4**：Cursor 的 `skills-cursor/` 等受保护子目录绝对不写
//!
//! SSOT：技能文件只存一份在 `library/<name>/`，工具目录里放链接指过去。
//! 这样同一技能装到多个工具不占多份空间，更新时也只改库里那一份。

use crate::hash;
use crate::state::{InstallMode, InstalledSkill, SkillSource};
use crate::tools::{assert_writable, prepare_target, ToolKind};
use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// 校验并规范化技能目录名
///
/// 显式拒绝两种分隔符：反斜杠在 Linux/macOS 上不是分隔符，会被当成合法
/// 单段名放行，但同一个值同步到 Windows 就变成嵌套路径。不能依赖
/// `components()` 的平台语义。
pub fn sanitize_install_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    let path = Path::new(trimmed);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => {
            let normalized = name.to_string_lossy().trim().to_string();
            if normalized.is_empty()
                || normalized == "."
                || normalized == ".."
                || normalized.starts_with('.')
            {
                None
            } else {
                Some(normalized)
            }
        }
        _ => None,
    }
}

/// 要求传入的目录名本身就是合法值（不做规范化，避免静默改正）
pub fn require_valid_directory(directory: &str) -> Result<String> {
    match sanitize_install_name(directory) {
        Some(n) if n == directory => Ok(n),
        _ => bail!("非法技能目录名（疑似路径穿越）: {directory:?}"),
    }
}

/// 两条路径是否指向同一位置（含软链解析后）
fn paths_alias(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    matches!((left.canonicalize(), right.canonicalize()),
        (Ok(l), Ok(r)) if l == r)
}

/// 库目录不能与工具技能目录重合，否则链接指向自己
pub fn ensure_distinct_skill_roots(ssot: &Path, tool_root: &Path, tool: &str) -> Result<()> {
    if paths_alias(ssot, tool_root) {
        bail!(
            "技能库目录不能与 {} 的技能目录相同: {}",
            tool,
            ssot.display()
        );
    }
    Ok(())
}

/// 把技能写进库，并为每个目标建链接/复制
///
/// 库根与目标路径都由调用方给出：生产代码传 `storage::library_dir()` 与
/// 各工具的 skills 目录，测试传 tempdir。
pub fn deploy_to_paths(
    source_dir: &Path,
    library_root: &Path,
    targets: &[PathBuf],
    mode: InstallMode,
) -> Result<(PathBuf, Vec<InstallMode>)> {
    if !source_dir.join("SKILL.md").is_file() {
        bail!("来源目录下没有 SKILL.md: {}", source_dir.display());
    }
    let name = source_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_name = require_valid_directory(&name)?;

    let ssot = prepare_library(source_dir, library_root, &dir_name)?;
    let deployments: Vec<Deployment> = targets
        .iter()
        .map(|t| Deployment {
            tool: ToolKind::Cursor,
            target: t.as_path(),
        })
        .collect();
    let used = deploy_many(&ssot, &deployments, mode, DeployPolicy::MustBeEmpty)?;
    Ok((ssot, used))
}

/// 更新一个已存在的技能：替换库内容，并按每条记录自己的 mode 处理工具目录
///
/// `targets` 是 `(工具目录里的落点, 该记录实际使用的 mode)`。链接模式的落点
/// 不用重建（它跟着库走），只有 Copy 模式的独立副本需要替换。
///
/// 返回新的库路径。
pub fn update_deploy(
    source_dir: &Path,
    library_root: &Path,
    dir_name: &str,
    targets: &[(PathBuf, InstallMode)],
) -> Result<PathBuf> {
    // 校验必须在这里做而不是只靠调用方：下面所有路径都用 dir_name 拼，
    // 漏一次就是任意目录写入。
    require_valid_directory(dir_name)?;

    let ssot = prepare_library(source_dir, library_root, dir_name)?;

    // 只有 copy 副本需要替换；链接会跟着库自动变
    let deployments: Vec<Deployment> = targets
        .iter()
        .filter(|(_, m)| !matches!(m, InstallMode::Junction | InstallMode::Symlink))
        .map(|(p, _)| Deployment {
            tool: ToolKind::Cursor,
            target: p.as_path(),
        })
        .collect();

    if !deployments.is_empty() {
        deploy_many(&ssot, &deployments, InstallMode::Copy, DeployPolicy::Replace)?;
    }
    Ok(ssot)
}

/// 创建目录软链；返回是否成功（测试用于探测环境）
pub fn make_symlink(src: &Path, dst: &Path) -> bool {
    create_symlink(src, dst).is_ok()
}

/// 移除一个已安装目标：链接只摘链接，普通目录递归删
///
/// 链接必须用 remove_file/remove_dir 摘掉而不递归，否则会删到库里的真身。
pub fn remove_installed(target: &Path) -> Result<()> {
    if !target.exists() && !is_link(target) {
        return Ok(());
    }
    if is_link(target) {
        #[cfg(windows)]
        {
            std::fs::remove_dir(target)?;
        }
        #[cfg(not(windows))]
        {
            // 目录软链在 Unix 上是普通文件条目，remove_dir 会失败
            std::fs::remove_file(target)?;
        }
    } else {
        std::fs::remove_dir_all(target)?;
    }
    Ok(())
}

/// 安装一个技能到单个工具
///
/// `library_root` 由调用方传入而不是在这里读 `storage::library_dir()`，
/// 这样测试可以用 tempdir 隔离，不碰真实的 `%APPDATA%`。
pub fn install(
    tool: ToolKind,
    name: &str,
    source_dir: &Path,
    mode: InstallMode,
    source: SkillSource,
    version: Option<String>,
    allow_create_skills_dir: bool,
    library_root: &Path,
) -> Result<InstalledSkill> {
    let mut recs = install_many(
        name,
        source_dir,
        mode,
        &[tool],
        source,
        version,
        allow_create_skills_dir,
        library_root,
    )?;
    recs.pop()
        .ok_or_else(|| anyhow::anyhow!("安装没有产生记录"))
}

/// 一次部署的目标
pub struct Deployment<'a> {
    pub tool: ToolKind,
    pub target: &'a Path,
}

/// 安装一个技能到多个工具（SSOT 模式）
///
/// 先把技能写进库，再为每个工具建链接或复制。多目标走「先全部准备、
/// 再逐个替换、失败反向回滚」，避免装到一半留下混合状态。
pub fn install_many(
    name: &str,
    source_dir: &Path,
    mode: InstallMode,
    tools: &[ToolKind],
    source: SkillSource,
    version: Option<String>,
    allow_create_skills_dir: bool,
    library_root: &Path,
) -> Result<Vec<InstalledSkill>> {
    if tools.is_empty() {
        bail!("至少要选一个目标工具");
    }
    if !source_dir.join("SKILL.md").is_file() {
        bail!("来源目录下没有 SKILL.md: {}", source_dir.display());
    }
    let dir_name = require_valid_directory(name)?;

    // R1 + R4：先把所有目标算出来并断言，再动任何文件
    let mut targets = Vec::with_capacity(tools.len());
    for tool in tools {
        let skills_root = prepare_target(*tool, allow_create_skills_dir)?;
        let target = skills_root.join(&dir_name);
        assert_writable(*tool, &target)?;
        ensure_distinct_skill_roots(
            &library_root.join(&dir_name),
            &skills_root,
            tool.display_name(),
        )?;
        targets.push(target);
    }

    let ssot = prepare_library(source_dir, library_root, &dir_name)?;
    let source_hash = hash::hash_dir(&ssot)?;

    let deployments: Vec<Deployment> = targets
        .iter()
        .zip(tools.iter())
        .map(|(target, tool)| Deployment {
            tool: *tool,
            target: target.as_path(),
        })
        .collect();

    let used = deploy_many(&ssot, &deployments, mode, DeployPolicy::MustBeEmpty)?;

    let mut out = Vec::with_capacity(used.len());
    for (i, tool) in tools.iter().enumerate() {
        let target = &targets[i];
        // 链接模式算出来的哈希跟随库；copy 模式是独立副本，也应一致
        let installed_hash = hash::hash_dir(target).unwrap_or_else(|_| source_hash.clone());
        if installed_hash != source_hash {
            log::warn!(
                "安装后哈希不一致（源 {source_hash} vs 目标 {installed_hash}），可能存在换行符转换"
            );
        }
        out.push(InstalledSkill {
            name: dir_name.clone(),
            tool: tool.key().to_string(),
            target_path: target.to_string_lossy().to_string(),
            mode: used[i],
            installed_hash,
            installed_version: version.clone(),
            source: source.clone(),
            installed_at: chrono::Utc::now().timestamp(),
            ignored_remote_hash: None,
        });
    }
    Ok(out)
}

/// 把技能写进库；库里已有同名副本时比较内容，不同则替换
///
/// 返回库里的技能目录。
fn prepare_library(source_dir: &Path, library_root: &Path, dir_name: &str) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(library_root)
        .with_context(|| format!("创建技能库目录失败: {}", library_root.display()))?;
    let ssot = library_root.join(dir_name);

    if ssot.is_dir() {
        let same = hash::hash_dir(source_dir).ok() == hash::hash_dir(&ssot).ok();
        if same {
            return Ok(ssot);
        }
    }

    // 用暂存 + rename 替换库内容，避免写一半留下残库
    let staging = tempfile::Builder::new()
        .prefix(".skill-ssot-")
        .tempdir_in(library_root)?;
    let fresh = staging.path().join("new");
    copy_dir_recursive(source_dir, &fresh)?;

    if ssot.exists() || is_link(&ssot) {
        let old = staging.path().join("old");
        std::fs::rename(&ssot, &old).with_context(|| {
            format!("移动旧库失败: {}", ssot.display())
        })?;
    }
    if let Err(e) = std::fs::rename(&fresh, &ssot) {
        bail!("写入技能库失败: {e}");
    }
    Ok(ssot)
}

/// 备份元数据
///
/// 与内容一起落盘，用于「用远端」与「另存一份」之后能找回原样。
/// 反序列化后其中的 `name` 是不可信输入，拼路径前必须重新校验。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupMeta {
    pub name: String,
    pub tool: String,
    pub installed_hash: String,
    pub installed_version: Option<String>,
    pub repo_path: Option<String>,
    pub backup_created_at: i64,
    pub source_path: String,
}

/// 每个技能最多保留的备份份数
pub const BACKUP_RETAIN_COUNT: usize = 20;

/// 备份目录根：`<app_data>/backups`
pub fn backups_root(app_data: &Path) -> PathBuf {
    app_data.join("backups")
}

/// 把当前库内容备份到 `<root>/<YYYYMMDD_HHMMSS>_<name>/`，并写 meta.json
///
/// 备份是兜底而非前置条件：失败只返回 Err 供调用方 warn，不中断更新。
pub fn backup_library(
    library_root: &Path,
    name: &str,
    backups: &Path,
    meta: Option<&BackupMeta>,
) -> Result<PathBuf> {
    let ssot = library_root.join(name);
    if !ssot.is_dir() {
        bail!("技能库里没有 {name}，无需备份");
    }
    std::fs::create_dir_all(backups)?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let mut dest = backups.join(format!("{stamp}_{name}"));
    let mut n = 1;
    while dest.exists() {
        dest = backups.join(format!("{stamp}_{name}_{n}"));
        n += 1;
    }
    copy_dir_recursive(&ssot, &dest)?;

    if let Some(meta) = meta {
        let with_path = BackupMeta {
            source_path: dest.to_string_lossy().to_string(),
            ..meta.clone()
        };
        std::fs::write(
            dest.join("meta.json"),
            serde_json::to_vec_pretty(&with_path)?,
        )?;
    }

    prune_backups(backups, name, BACKUP_RETAIN_COUNT);
    Ok(dest)
}

/// 找一个不冲突的同级名字：`<base>` 被占就试 `<base>-2`、`-3`…，最多 10 次
///
/// 绝不静默覆盖：重名到上限就报错，让用户自己决定怎么处理。
pub fn unique_sibling_name(library_root: &Path, base: &str) -> Result<String> {
    if !library_root.join(base).exists() && !is_link(&library_root.join(base)) {
        return Ok(base.to_string());
    }
    for n in 2..=10 {
        let candidate = format!("{base}-{n}");
        let p = library_root.join(&candidate);
        if !p.exists() && !is_link(&p) {
            return Ok(candidate);
        }
    }
    bail!("{base} 及其变体都已存在（试到 -10），请先清理后重试");
}

/// 只保留最近的 N 份，超出删最旧
fn prune_backups(backups: &Path, name: &str, keep: usize) {
    let Ok(rd) = std::fs::read_dir(backups) else {
        return;
    };
    // 目录名形如 `20250101_120000_<name>` 或 `..._<name>_2`；时间戳前缀
    // 保证字典序即时间序，所以排序后取前面删掉就是删最旧
    let suffix = format!("_{name}");
    let numbered = format!("_{name}_");
    let mut mine: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with(&suffix) || n.contains(&numbered)
        })
        .map(|e| e.path())
        .collect();
    mine.sort();
    let excess = mine.len().saturating_sub(keep);
    for old in mine.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(old);
    }
}

/// 读出备份的 meta.json，并校验其中的目录名
///
/// meta.json 可能是手工放置或来自不可信备份，`name` 直接 join 会穿越出
/// 库目录写任意位置，所以必须重新校验。
pub fn read_backup_meta(backup_dir: &Path) -> Result<BackupMeta> {
    let raw = std::fs::read(backup_dir.join("meta.json"))
        .with_context(|| format!("备份缺少 meta.json: {}", backup_dir.display()))?;
    let meta: BackupMeta = serde_json::from_slice(&raw)?;
    require_valid_directory(&meta.name)?;
    Ok(meta)
}

/// 把备份内容恢复成一个新的技能目录 `<library>/<target_name>`
///
/// 目标已存在时拒绝，绝不静默覆盖。
pub fn restore_backup_as(
    backup_dir: &Path,
    library_root: &Path,
    target_name: &str,
) -> Result<PathBuf> {
    require_valid_directory(target_name)?;
    let dest = library_root.join(target_name);
    if dest.exists() || is_link(&dest) {
        bail!("目标已存在，不能覆盖: {}", dest.display());
    }
    // 备份里如果有 meta.json，它是元数据不是技能内容，不该带进新技能
    copy_dir_recursive_skipping(backup_dir, &dest, &["meta.json"])?;

    // frontmatter 里的 name 还写着原名，而「我的技能」按 name 聚合。
    // 不改的话另存出来的副本会和正本合并成一行，用户看不到两份。
    if let Err(e) = rewrite_frontmatter_name(&dest.join("SKILL.md"), target_name) {
        // 改名失败不致命：内容已经保住，只是界面里可能合并显示
        log::warn!("改写 {target_name} 的 frontmatter name 失败: {e}");
    }
    Ok(dest)
}

/// 把 SKILL.md frontmatter 里的 `name:` 改成指定值
fn rewrite_frontmatter_name(skill_md: &Path, new_name: &str) -> Result<()> {
    let raw = std::fs::read_to_string(skill_md)?;
    let text = raw.trim_start_matches('\u{feff}');
    if !text.starts_with("---") {
        bail!("没有 frontmatter，跳过");
    }
    let after = &text[3..];
    let Some(close) = after.find("\n---") else {
        bail!("frontmatter 未闭合，跳过");
    };
    let fm = &after[..close];
    let body = &after[close + 4..];

    let mut replaced = false;
    let lines: Vec<String> = fm
        .lines()
        .map(|line| {
            // 只替换顶层 name，不碰 metadata 里的同名键
            let is_top_level = !line.starts_with(' ') && !line.starts_with('\t');
            if is_top_level && line.split_once(':').map(|(k, _)| k.trim()) == Some("name") {
                replaced = true;
                format!("name: {new_name}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !replaced {
        bail!("frontmatter 里没有 name，跳过");
    }
    std::fs::write(
        skill_md,
        format!("---\n{}\n---\n{}", lines.join("\n"), body),
    )?;
    Ok(())
}

/// 一个已暂存、等待替换的目标
struct Staged {
    target: std::path::PathBuf,
    staging: tempfile::TempDir,
    used: InstallMode,
}

/// 部署策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployPolicy {
    /// 目标必须不存在（安装语义）
    MustBeEmpty,
    /// 允许替换已存在的目标（更新语义）
    ///
    /// 旧内容会被挪进暂存区的 `old/` 而不是删除：更新失败时得能还原，
    /// 否则「更新失败 = 技能消失」比不能更新更糟。
    Replace,
}

/// 多目标部署：先全部准备，再逐个替换，失败反向回滚
///
/// 返回每个目标实际采用的方式（Auto 会解析成具体的一种）。
fn deploy_many(
    ssot: &Path,
    deployments: &[Deployment<'_>],
    mode: InstallMode,
    policy: DeployPolicy,
) -> Result<Vec<InstallMode>> {
    let mut staged: Vec<Staged> = Vec::with_capacity(deployments.len());
    for d in deployments {
        let occupied = d.target.exists() || is_link(d.target);
        if occupied && policy == DeployPolicy::MustBeEmpty {
            bail!(
                "{} 下已存在同名技能 {}，请先卸载或改名",
                d.tool.display_name(),
                d.target
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            );
        }
        // 暂存目录必须与目标同父目录：跨盘 rename 会失败
        let parent = d.target.parent().context("目标路径没有父目录")?;
        std::fs::create_dir_all(parent)?;
        let staging = tempfile::Builder::new()
            .prefix(".skill-deploy-")
            .tempdir_in(parent)?;
        let replacement = staging.path().join("new");

        let used = match mode {
            InstallMode::Copy => {
                copy_dir_recursive(ssot, &replacement)?;
                InstallMode::Copy
            }
            InstallMode::Junction => {
                create_junction(ssot, &replacement)?;
                InstallMode::Junction
            }
            InstallMode::Symlink => {
                create_symlink(ssot, &replacement)?;
                InstallMode::Symlink
            }
            InstallMode::Auto => {
                // 只有 Auto 可以降级；显式选链接时降级等于骗用户「省了空间」
                if create_junction(ssot, &replacement).is_ok() {
                    InstallMode::Junction
                } else {
                    if replacement.exists() || is_link(&replacement) {
                        let _ = std::fs::remove_dir(&replacement);
                    }
                    copy_dir_recursive(ssot, &replacement)?;
                    InstallMode::Copy
                }
            }
        };
        staged.push(Staged {
            target: d.target.to_path_buf(),
            staging,
            used,
        });
    }

    // 全部就绪后才开始替换：中途失败时前面已装的能整体撤回
    let mut changed = 0usize;
    for entry in &staged {
        let replacement = entry.staging.path().join("new");
        let previous = entry.staging.path().join("old");

        // Replace 策略下把旧内容挪进暂存区而不是删除，失败才好还原
        let had_previous = if policy == DeployPolicy::Replace
            && (entry.target.exists() || is_link(&entry.target))
        {
            if let Err(e) = std::fs::rename(&entry.target, &previous) {
                rollback(&staged[..changed]);
                return Err(anyhow::anyhow!(
                    "移除旧版本 {} 失败: {e}",
                    entry.target.display()
                ));
            }
            true
        } else {
            false
        };

        if let Err(e) = std::fs::rename(&replacement, &entry.target) {
            // 先把这个目标还原，再撤回前面已替换的
            if had_previous {
                let _ = std::fs::rename(&previous, &entry.target);
            }
            rollback(&staged[..changed]);
            return Err(anyhow::anyhow!(
                "部署到 {} 失败: {e}",
                entry.target.display()
            ));
        }
        changed += 1;
    }

    Ok(staged.into_iter().map(|s| s.used).collect())
}

/// 反向撤销已完成的替换
///
/// 暂存区里的 `old/` 是原内容，必须挪回去；直接删目标等于把技能弄丢。
fn rollback(done: &[Staged]) {
    for entry in done.iter().rev() {
        let previous = entry.staging.path().join("old");
        remove_installed(&entry.target).ok();
        if previous.exists() || is_link(&previous) {
            let _ = std::fs::rename(&previous, &entry.target);
        }
    }
}

/// 卸载
///
/// 链接只删链接本身（`remove_dir`，不递归），否则会删到库里的真身。
pub fn uninstall(tool: ToolKind, name: &str) -> Result<()> {
    let skills_root = tool
        .skills_dir()
        .ok_or_else(|| anyhow::anyhow!("无法定位 {} 的技能目录", tool.display_name()))?;
    let target = skills_root.join(name);

    assert_writable(tool, &target)?;
    remove_installed(&target)
}

/// 计算某个已装技能的当前本地哈希
pub fn local_hash(_tool: ToolKind, target_path: &str) -> Result<String> {
    let p = Path::new(target_path);
    if !p.exists() {
        bail!("目标目录不存在: {target_path}");
    }
    // 指纹规则已与工具无关：所有工具的 SKILL.md 都走同一套规范化
    hash::hash_dir(p)
}

/// 递归复制目录
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    copy_dir_recursive_skipping(src, dst, &[])
}

/// 复制目录，跳过顶层指定的文件名（用于排除 meta.json 这类元数据）
fn copy_dir_recursive_skipping(src: &Path, dst: &Path, skip: &[&str]) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name.to_string_lossy() == *s) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("复制 {} 失败", from.display()))?;
        }
    }
    Ok(())
}

/// 该路径是否为软链或 Windows 重解析点
pub fn is_link(p: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

/// 创建目录 junction（Windows 专用，不需要开发者模式）
#[cfg(windows)]
fn create_junction(src: &Path, dst: &Path) -> Result<()> {
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dst)
        .arg(src)
        .output()
        .context("执行 mklink 失败")?;
    if !out.status.success() {
        bail!(
            "创建 junction 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn create_junction(_src: &Path, _dst: &Path) -> Result<()> {
    bail!("junction 仅在 Windows 上受支持")
}

/// 创建目录软链（Windows 上需开发者模式或管理员权限）
#[cfg(windows)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
        .with_context(|| format!("创建目录软链失败: {} -> {}", dst.display(), src.display()))
}

#[cfg(not(windows))]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst)
        .with_context(|| format!("创建目录软链失败: {} -> {}", dst.display(), src.display()))
}

/// 运行时探测：当前环境能否创建 junction
///
/// 用临时目录试建一次，成功即认为可用。
pub fn junction_supported() -> bool {
    let Ok(tmp) = tempfile::tempdir() else {
        return false;
    };
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    if std::fs::create_dir_all(&src).is_err() {
        return false;
    }
    let ok = create_junction(&src, &dst).is_ok();
    if ok {
        let _ = std::fs::remove_dir(&dst);
    }
    ok
}

/// 运行时探测：当前环境能否创建目录软链
pub fn symlink_supported() -> bool {
    let Ok(tmp) = tempfile::tempdir() else {
        return false;
    };
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    if std::fs::create_dir_all(&src).is_err() {
        return false;
    }
    let ok = create_symlink(&src, &dst).is_ok();
    if ok {
        let _ = std::fs::remove_dir(&dst);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 直接构造「第一个已替换、第二个替换失败」的状态，验证回滚把旧内容挪回来
    ///
    /// 这里不能用 update_deploy：它一旦准备阶段失败就根本走不到回滚，
    /// 测不到本票最关键的那条路径。
    #[test]
    fn rollback_把暂存区的旧内容还原回目标() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("skills").join("demo");
        write_target(&target, "旧内容");

        // 模拟替换完成：目标换成新内容，原内容被挪进 staging/old
        let staging = tempfile::Builder::new()
            .prefix(".skill-deploy-")
            .tempdir_in(target.parent().unwrap())
            .unwrap();
        std::fs::rename(&target, staging.path().join("old")).unwrap();
        write_target(&target, "新内容");

        let staged = vec![Staged {
            target: target.clone(),
            staging,
            used: InstallMode::Copy,
        }];

        rollback(&staged);

        assert!(target.is_dir(), "目标必须被还原而不是删空");
        let restored = std::fs::read_to_string(target.join("SKILL.md")).unwrap();
        assert!(
            restored.contains("旧内容"),
            "回滚要挪回暂存区的 old，不能只删新装的；实际: {restored}"
        );
        assert!(
            !restored.contains("新内容"),
            "回滚后不该留下新装的内容；实际: {restored}"
        );
    }

    /// 原本就没有目标的回滚仍应保持「删掉新装的」行为
    #[test]
    fn rollback_对原本不存在目标时只删除() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("skills").join("demo");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        write_target(&target, "新装的内容");

        let staging = tempfile::Builder::new()
            .prefix(".skill-deploy-")
            .tempdir_in(target.parent().unwrap())
            .unwrap();
        let staged = vec![Staged {
            target: target.clone(),
            staging,
            used: InstallMode::Copy,
        }];

        rollback(&staged);
        assert!(!target.exists(), "本来就没有的东西，回滚后也不该留下");
    }

    fn write_target(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = std::fs::File::create(dir.join("SKILL.md")).unwrap();
        f.write_all(format!("---\nname: demo\n---\n{body}\n").as_bytes())
            .unwrap();
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn source() -> SkillSource {
        SkillSource {
            kind: crate::state::SourceKind::Local,
            repo_url: None,
            repo_path: None,
            branch: None,
            commit: None,
        }
    }

    #[test]
    fn copy_recursive_preserves_structure() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "SKILL.md", "---\nname: x\n---\n");
        write(src.path(), "scripts/run.py", "print(1)");
        write(src.path(), "references/doc.md", "# doc");

        let target = dst.path().join("out");
        copy_dir_recursive(src.path(), &target).unwrap();

        assert!(target.join("SKILL.md").is_file());
        assert!(target.join("scripts/run.py").is_file());
        assert!(target.join("references/doc.md").is_file());
    }

    #[test]
    fn install_rejects_source_without_skill_md() {
        let src = tempfile::tempdir().unwrap();
        let lib = tempfile::tempdir().unwrap();
        let r = install(
            ToolKind::Cursor,
            "x",
            src.path(),
            InstallMode::Copy,
            source(),
            None,
            false,
            lib.path(),
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("SKILL.md"));
    }

    #[test]
    fn junction_probe_does_not_panic() {
        // 结果取决于环境，只要求不 panic
        let _ = junction_supported();
        let _ = symlink_supported();
    }
}
