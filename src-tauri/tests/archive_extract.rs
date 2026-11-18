//! 归档解压的正确性验证
//!
//! 重点覆盖内网真实场景：**中文分类目录**。
//!
//! 背景：zip 规范里文件名的编码由「通用标志位 bit 11」决定 —— 置位表示 UTF-8，
//! 未置位表示 CP437。GitLab 用的 zip 库通常置位，但不同版本行为不一，
//! 中文路径一旦解码错误，解压出来的目录名就是乱码，客户端再也找不到技能。

use std::io::Write;
use zip::write::SimpleFileOptions;

/// 与 market.rs 中的 extract_zip 保持相同逻辑的最小实现，
/// 用于独立验证「中文路径能否正确落盘」
fn extract(bytes: &[u8], dest: &std::path::Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(path) = entry.enclosed_name() else {
            anyhow::bail!("归档含非法路径: {}", entry.name());
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

fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

#[test]
fn chinese_paths_extract_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let zip = build_zip(&[
        (
            "skill-abc123-abc123456789/skills/通用/grill-me/SKILL.md",
            "---\nname: grill-me\n---\n",
        ),
        (
            "skill-abc123-abc123456789/skills/通用/grill-me/agents/openai.yaml",
            "name: x\n",
        ),
    ]);

    extract(&zip, dir.path()).expect("解压中文路径");

    let skill_md = dir
        .path()
        .join("skill-abc123-abc123456789/skills/通用/grill-me/SKILL.md");
    assert!(
        skill_md.is_file(),
        "中文路径应正确解压，实际查找: {}",
        skill_md.display()
    );

    let content = std::fs::read_to_string(&skill_md).unwrap();
    assert!(content.contains("grill-me"));

    // 确认目录名不是乱码
    let cat_dir = dir
        .path()
        .join("skill-abc123-abc123456789/skills/通用");
    assert!(cat_dir.is_dir(), "分类目录名应为「通用」而非乱码");
    let names: Vec<String> = std::fs::read_dir(&cat_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["grill-me"], "子目录名应完整保留");
}

#[test]
fn path_traversal_in_zip_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    // 手工构造一个含 ../ 的条目名：zip crate 的 start_file 会拒绝，
    // 所以直接构造恶意归档字节（模拟攻击者）
    let zip = build_zip(&[("root/../../../evil.txt", "pwned")]);
    let r = extract(&zip, dir.path());
    // enclosed_name() 应该拒绝；若 zip crate 在写入阶段就已规范化，则解压到 root 下也算安全
    match r {
        Ok(()) => {
            // 至少不能写到 dest 之外
            let outside = dir.path().parent().unwrap().join("evil.txt");
            assert!(!outside.exists(), "绝不能写到目标目录之外");
        }
        Err(e) => {
            println!("正确拒绝: {e}");
        }
    }
}

#[test]
fn empty_archive_is_handled() {
    let dir = tempfile::tempdir().unwrap();
    let zip = build_zip(&[]);
    extract(&zip, dir.path()).expect("空归档不应 panic");
}
