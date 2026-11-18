//! 技能内容哈希
//!
//! 用于「有没有新版本」和「本地文件被改过没有」的判定。
//!
//! 核心教训（来自 cc-switch 的 7,937 行实战）：
//! 1. **不能直接对文件原始字节做 hash** —— 工具会回写易变字段
//! 2. 必须剥离 frontmatter 里的 `enabled_at` / `installed_at` 等
//! 3. 必须排除 `__pycache__` / `*.pyc`（跑一次技能就产生）
//! 4. 必须归一换行符（Windows CRLF vs git LF）
//! 5. 必须排序文件列表（文件系统遍历顺序不稳定）
//! 6. 必须把相对路径算进 hash（否则改名检测不出来）

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// 计算目录的内容哈希（更新检测用，宽松模式）
///
/// 排除项：
/// - 所有 `.` 开头的隐藏文件/目录
/// - `__pycache__/` 和 `*.pyc`（Python 运行时生成）
///
/// `tools_that_rewrite_frontmatter` 为真时，会剥离 SKILL.md frontmatter
/// 里的易变字段（千问办公会写 `enabled_at`，WorkBuddy 可能类似）。
pub fn hash_dir(dir: &Path, strip_volatile: bool) -> anyhow::Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for path in &files {
        let rel = path.strip_prefix(dir).unwrap_or(path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        hasher.update(rel_str.as_bytes());
        hasher.update(b"\0");

        let bytes = if strip_volatile && rel_str.eq_ignore_ascii_case("SKILL.md") {
            let raw = fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("读取 {} 失败: {e}", path.display()))?;
            normalized_skill_md(&raw).into_bytes()
        } else {
            normalize_bytes(&fs::read(path)?)
        };

        hasher.update(&bytes);
        hasher.update(b"\0");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// 计算目录的部署完整性哈希（严格模式）
///
/// 与 `hash_dir` 的区别：
/// - **包含**隐藏文件
/// - **包含**空目录（用户删掉一个空目录也算改动）
///
/// 用来判断「目标目录里的技能与 SSOT 副本是否一致」。
pub fn hash_dir_strict(dir: &Path) -> anyhow::Result<String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_tree_entries(dir, &mut entries)?;
    entries.sort();

    let mut hasher = Sha256::new();
    for path in &entries {
        let rel = path.strip_prefix(dir).unwrap_or(path);
        hasher.update(rel.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update(b"\0");

        let meta = fs::symlink_metadata(path)?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            hasher.update(b"link\0");
            if let Ok(target) = fs::read_link(path) {
                hasher.update(target.to_string_lossy().as_bytes());
            }
        } else if ft.is_dir() {
            hasher.update(b"dir\0");
        } else {
            hasher.update(b"file\0");
            hasher.update(&normalize_bytes(&fs::read(path)?));
        }
        hasher.update(b"\0");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// 递归收集用于更新检测的文件（排除隐藏项与 Python 字节码）
fn collect_files(base: &Path, current: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // 隐藏文件/目录一律跳过（.git/.DS_Store/.claude 等）
        if name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            if name == "__pycache__" {
                continue;
            }
            collect_files(base, &path, out)?;
        } else {
            if name.ends_with(".pyc") {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}

/// 递归收集完整树（含隐藏、含空目录），用于严格哈希
fn collect_tree_entries(current: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        out.push(path.clone());
        if path.is_dir() {
            collect_tree_entries(&path, out)?;
        }
    }
    Ok(())
}

/// 文本字节归一化：CRLF / CR 统一成 LF
fn normalize_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\n');
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// 剥离 SKILL.md frontmatter 中的易变字段，并对剩余键排序
///
/// 千问办公会在 SKILL.md 里写入 `enabled_at: <timestamp>`，
/// 不剥离的话每跑一次技能 hash 就变，永远显示「已修改」。
fn normalized_skill_md(raw: &str) -> String {
    /// 工具运行时回写的字段，不代表技能内容变化
    const VOLATILE_KEYS: &[&str] = &[
        "enabled_at",
        "enabled_at_ms",
        "installed_at",
        "enabled",
        "last_opened_at",
        "last_used_at",
        "cli_version",
        "updated_at",
    ];

    let text = raw.trim_start_matches('\u{feff}');
    if !text.starts_with("---") {
        return text.replace("\r\n", "\n");
    }

    // 找 frontmatter 的结束分隔符
    let after_open = &text[3..];
    let Some(close_rel) = after_open.find("\n---") else {
        return text.replace("\r\n", "\n");
    };

    let fm_end = 3 + close_rel;
    let frontmatter = &text[3..fm_end];
    let body = &text[fm_end + 4..]; // 跳过 "\n---"

    // 逐行过滤：删除易变键的整行（含其下的缩进续行）
    let mut kept: Vec<String> = Vec::new();
    let mut skipping_continuation = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        let is_continuation = line.starts_with(' ') || line.starts_with('\t');

        if skipping_continuation && is_continuation {
            continue;
        }
        skipping_continuation = false;

        if !is_continuation {
            if let Some((key, _)) = trimmed.split_once(':') {
                let key = key.trim();
                if VOLATILE_KEYS.contains(&key) {
                    skipping_continuation = true;
                    continue;
                }
            }
        }
        kept.push(line.to_string());
    }

    // 剩余顶层键按字典序排序（保持缩进块整体移动）
    let sorted = sort_frontmatter_blocks(&kept);

    format!("---\n{}\n---\n{}", sorted, body.replace("\r\n", "\n"))
}

/// 把 frontmatter 行按「顶层键 + 其续行」分块，块间按 key 排序
fn sort_frontmatter_blocks(lines: &[String]) -> String {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;

    for line in lines {
        let is_continuation = line.starts_with(' ') || line.starts_with('\t') || line.starts_with('-');
        if is_continuation {
            if let Some((_, body)) = current.as_mut() {
                body.push(line.clone());
                continue;
            }
            // frontmatter 开头就是续行（异常格式），当独立块处理
            blocks.push((line.clone(), vec![]));
            continue;
        }

        let key = line
            .split_once(':')
            .map(|(k, _)| k.trim().to_string())
            .unwrap_or_else(|| line.clone());

        if let Some(done) = current.take() {
            blocks.push(done);
        }
        current = Some((key, vec![line.clone()]));
    }
    if let Some(done) = current {
        blocks.push(done);
    }

    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    blocks
        .into_iter()
        .flat_map(|(_, lines)| lines)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn crlf_and_lf_produce_same_hash() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
        write(b.path(), "SKILL.md", "---\r\nname: x\r\n---\r\nbody\r\n");
        assert_eq!(
            hash_dir(a.path(), false).unwrap(),
            hash_dir(b.path(), false).unwrap()
        );
    }

    #[test]
    fn pycache_is_ignored() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\n---\n");
        write(b.path(), "SKILL.md", "---\nname: x\n---\n");
        write(b.path(), "__pycache__/mod.cpython-311.pyc", "garbage");
        write(b.path(), "run.pyc", "garbage");
        assert_eq!(
            hash_dir(a.path(), false).unwrap(),
            hash_dir(b.path(), false).unwrap()
        );
    }

    #[test]
    fn hidden_files_are_ignored() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\n---\n");
        write(b.path(), "SKILL.md", "---\nname: x\n---\n");
        write(b.path(), ".DS_Store", "junk");
        assert_eq!(
            hash_dir(a.path(), false).unwrap(),
            hash_dir(b.path(), false).unwrap()
        );
    }

    #[test]
    fn enabled_at_is_stripped() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\ndescription: d\n---\nbody\n");
        write(
            b.path(),
            "SKILL.md",
            "---\nname: x\ndescription: d\nenabled_at: 1787556778796\n---\nbody\n",
        );
        assert_eq!(
            hash_dir(a.path(), true).unwrap(),
            hash_dir(b.path(), true).unwrap(),
            "enabled_at 不应影响哈希"
        );
        // 不开启剥离时应该不同
        assert_ne!(
            hash_dir(a.path(), false).unwrap(),
            hash_dir(b.path(), false).unwrap()
        );
    }

    #[test]
    fn key_order_does_not_matter() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\ndescription: d\n---\nbody\n");
        write(b.path(), "SKILL.md", "---\ndescription: d\nname: x\n---\nbody\n");
        assert_eq!(
            hash_dir(a.path(), true).unwrap(),
            hash_dir(b.path(), true).unwrap()
        );
    }

    #[test]
    fn content_change_changes_hash() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\n---\nbody A\n");
        write(b.path(), "SKILL.md", "---\nname: x\n---\nbody B\n");
        assert_ne!(
            hash_dir(a.path(), true).unwrap(),
            hash_dir(b.path(), true).unwrap()
        );
    }

    #[test]
    fn rename_changes_hash() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "a.txt", "same");
        write(b.path(), "b.txt", "same");
        assert_ne!(
            hash_dir(a.path(), true).unwrap(),
            hash_dir(b.path(), true).unwrap()
        );
    }

    #[test]
    fn missing_frontmatter_does_not_panic() {
        let a = tmpdir();
        write(a.path(), "SKILL.md", "# just markdown, no frontmatter\n");
        assert!(hash_dir(a.path(), true).is_ok());
    }

    #[test]
    fn strict_hash_detects_empty_dir_removal() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "x");
        write(b.path(), "SKILL.md", "x");
        fs::create_dir_all(a.path().join("empty")).unwrap();
        // b 没有 empty 目录
        assert_ne!(
            hash_dir_strict(a.path()).unwrap(),
            hash_dir_strict(b.path()).unwrap()
        );
    }
}
