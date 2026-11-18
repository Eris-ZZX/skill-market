//! 部署引擎
//!
//! 把一个技能目录放进目标工具的技能目录，并记录安装状态。
//!
//! 两条硬约束：
//! - **R1**：只写已存在的工具目录，绝不创建工具根目录
//! - **R4**：Cursor 的 `skills-cursor/` 等受保护子目录绝对不写
//!
//! 默认 copy（独立副本）。junction 是高级选项，且必须运行时探测可用性 ——
//! 本机开发者模式开着能建成功，不代表同事的电脑也能。

use crate::hash;
use crate::state::{InstallMode, InstalledSkill, SkillSource};
use crate::tools::{assert_writable, prepare_target, ToolKind};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// 安装一个技能
///
/// `source_dir` 是解压好的技能目录（已在临时目录里，含 SKILL.md）
pub fn install(
    tool: ToolKind,
    name: &str,
    source_dir: &Path,
    mode: InstallMode,
    source: SkillSource,
    version: Option<String>,
    allow_create_skills_dir: bool,
) -> Result<InstalledSkill> {
    // 校验来源目录
    if !source_dir.join("SKILL.md").is_file() {
        bail!("来源目录下没有 SKILL.md: {}", source_dir.display());
    }

    // R1：探测工具根目录，未安装则直接报错
    let skills_root = prepare_target(tool, allow_create_skills_dir)?;
    let target = skills_root.join(name);

    // R4：受保护目录断言
    assert_writable(tool, &target)?;

    // 目标已存在时的处理
    if target.exists() {
        return Err(anyhow::anyhow!(
            "{} 下已存在同名技能 {}，请先卸载或改名",
            tool.display_name(),
            name
        ));
    }

    let source_hash = hash::hash_dir(source_dir, tool.writes_volatile_fields())?;

    match mode {
        InstallMode::Copy => {
            copy_dir_recursive(source_dir, &target)?;
        }
        InstallMode::Junction => {
            // junction 源统一放在本地库，避免指向临时目录
            let lib = crate::storage::library_dir()?.join(name);
            if lib.exists() {
                std::fs::remove_dir_all(&lib).ok();
            }
            copy_dir_recursive(source_dir, &lib)?;
            create_junction(&lib, &target)?;
        }
    }

    // 装完再算一次（copy 后内容应当一致；junction 会跟随源）
    let installed_hash = hash::hash_dir(&target, tool.writes_volatile_fields())?;
    if installed_hash != source_hash {
        // 不致命，但记录差异以便排查
        log::warn!(
            "安装后哈希不一致（源 {source_hash} vs 目标 {installed_hash}），可能存在换行符转换"
        );
    }

    Ok(InstalledSkill {
        name: name.to_string(),
        tool: tool.key().to_string(),
        target_path: target.to_string_lossy().to_string(),
        mode,
        installed_hash,
        installed_version: version,
        source,
        installed_at: chrono::Utc::now().timestamp(),
    })
}

/// 卸载
pub fn uninstall(tool: ToolKind, name: &str) -> Result<()> {
    let skills_root = tool
        .skills_dir()
        .ok_or_else(|| anyhow::anyhow!("无法定位 {} 的技能目录", tool.display_name()))?;
    let target = skills_root.join(name);

    assert_writable(tool, &target)?;

    if !target.exists() {
        return Ok(());
    }

    // junction 要用 remove_dir（不递归删内容），普通目录用 remove_dir_all
    let meta = std::fs::symlink_metadata(&target)?;
    if meta.file_type().is_symlink() || is_junction(&target) {
        #[cfg(windows)]
        {
            std::fs::remove_dir(&target)?;
        }
        #[cfg(not(windows))]
        {
            std::fs::remove_file(&target)?;
        }
    } else {
        std::fs::remove_dir_all(&target)?;
    }
    Ok(())
}

/// 计算某个已装技能的当前本地哈希
pub fn local_hash(tool: ToolKind, target_path: &str) -> Result<String> {
    let p = Path::new(target_path);
    if !p.exists() {
        bail!("目标目录不存在: {target_path}");
    }
    hash::hash_dir(p, tool.writes_volatile_fields())
}

/// 递归复制目录
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("复制 {} 失败", from.display()))?;
        }
    }
    Ok(())
}

/// 创建目录 junction
///
/// 注意：junction 不需要管理员权限，但**部分企业安全策略会禁止**
/// reparse point 创建。调用前应先 `junction_supported()` 探测。
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

#[cfg(windows)]
fn is_junction(p: &Path) -> bool {
    // junction 在文件系统层面表现为目录重解析点
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
        || is_reparse_point(p)
}

#[cfg(not(windows))]
fn is_junction(_p: &Path) -> bool {
    false
}

#[cfg(windows)]
fn is_reparse_point(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(p)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
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
        let source = SkillSource {
            kind: crate::state::SourceKind::Local,
            repo_url: None,
            repo_path: None,
            branch: None,
            commit: None,
        };
        let r = install(
            ToolKind::Cursor,
            "x",
            src.path(),
            InstallMode::Copy,
            source,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("SKILL.md"));
    }

    #[test]
    fn junction_probe_does_not_panic() {
        // 结果取决于环境，只要求不 panic
        let _ = junction_supported();
    }
}
