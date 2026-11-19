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

/// 计算目录的内容哈希（更新检测用）
///
/// 排除项见 [`should_skip`]。
pub fn hash_dir(dir: &Path) -> anyhow::Result<String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files(dir, dir, &mut paths)?;

    let files: Vec<(String, Vec<u8>)> = paths
        .iter()
        .map(|path| {
            let rel = path.strip_prefix(dir).unwrap_or(path);
            Ok((
                rel.to_string_lossy().replace('\\', "/"),
                fs::read(path)?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;

    hash_files(&files)
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

/// 排除判定：任一路径分段是隐藏项或 `__pycache__`，或文件名以 `.pyc` 结尾
///
/// 入参可以是单段文件名（目录遍历时）或完整相对路径（远端归档解析时），
/// 两种形态都必须判对：只看最后一段会让 `.hidden/x` 这类深层隐藏文件漏过。
pub fn should_skip(name: &str) -> bool {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    name.split('/')
        .any(|seg| seg.starts_with('.') || seg == "__pycache__")
        || leaf.ends_with(".pyc")
}

/// 对内存中的文件集合算指纹
///
/// 本地目录和远端索引都走这里，两侧算法无法再各自漂移。
/// `files` 的相对路径必须是 `/` 分隔、无前导 `./` 的形式。
pub fn hash_files(files: &[(String, Vec<u8>)]) -> anyhow::Result<String> {
    // 按字符串逐字节排序。不能用 PathBuf：它按路径分量比较，
    // 会让 `a/b.md` 与 `a-x.md` 这类同一目录下两条路径的顺序反过来。
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (rel, bytes) in sorted {
        // 相对路径也参与哈希，否则改名检测不出来
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(normalize_file_bytes(rel, bytes));
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 单个文件在进指纹前的规范化
///
/// `SKILL.md` 走 frontmatter 规范化；其余文件只归一换行符。
pub fn normalize_file_bytes(name: &str, bytes: &[u8]) -> Vec<u8> {
    if name.rsplit('/').next().unwrap_or(name).eq_ignore_ascii_case("SKILL.md") {
        match std::str::from_utf8(bytes) {
            Ok(s) => normalize_skill_md(s).into_bytes(),
            // 非 UTF-8 的 SKILL.md 无法解析 frontmatter，至少保证换行符一致
            Err(_) => normalize_bytes(bytes),
        }
    } else {
        normalize_bytes(bytes)
    }
}

/// 递归收集用于更新检测的文件（排除见 [`should_skip`]）
fn collect_files(base: &Path, current: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if should_skip(&name) {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else {
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
///
/// 远端索引和本地目录都必须走这里——两侧用不同规则算出的指纹永远对不上。
pub fn normalize_skill_md(raw: &str) -> String {
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

    // 先归一换行符，否则 `lines()` 剥离前导分隔符的 `\r` 后会残留一个空行，
    // 且 CRLF 文件里 frontmatter 末行会带着 `\r` 进入排序，两种换行算出不同指纹。
    let text = text.replace("\r\n", "\n");
    let after_open = &text[3..];
    let Some(close_rel) = after_open.find("\n---") else {
        return text;
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

    format!("---\n{sorted}\n---\n{body}")
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
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
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
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
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
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
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
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap(),
            "enabled_at 不应影响哈希"
        );
    }

    #[test]
    fn key_order_does_not_matter() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\ndescription: d\n---\nbody\n");
        write(b.path(), "SKILL.md", "---\ndescription: d\nname: x\n---\nbody\n");
        assert_eq!(
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
        );
    }

    #[test]
    fn content_change_changes_hash() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "SKILL.md", "---\nname: x\n---\nbody A\n");
        write(b.path(), "SKILL.md", "---\nname: x\n---\nbody B\n");
        assert_ne!(
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
        );
    }

    #[test]
    fn rename_changes_hash() {
        let a = tmpdir();
        let b = tmpdir();
        write(a.path(), "a.txt", "same");
        write(b.path(), "b.txt", "same");
        assert_ne!(
            hash_dir(a.path()).unwrap(),
            hash_dir(b.path()).unwrap()
        );
    }

    #[test]
    fn missing_frontmatter_does_not_panic() {
        let a = tmpdir();
        write(a.path(), "SKILL.md", "# just markdown, no frontmatter\n");
        assert!(hash_dir(a.path()).is_ok());
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
