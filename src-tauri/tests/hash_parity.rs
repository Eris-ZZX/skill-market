//! 跨模块哈希一致性
//!
//! 远端索引（`hash::hash_files`，输入是内存里的文件集合）和本地目录
//! （`hash::hash_dir`，输入是磁盘目录）必须算出完全相同的指纹，
//! 否则同一份技能什么都没改也会被判定为「可更新」。
//!
//! 这里的每一个用例都同时走两条路径再比对，任何一侧规则漂移都会红。

use skill_market_lib::hash;
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content.as_bytes()).unwrap();
}

/// 模拟远端索引路径：遍历目录收集文件，排除规则与 `parse_repo_zip` 一致，
/// 然后交给 `hash_files`。
fn remote_hash(dir: &Path) -> String {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(cur).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if hash::should_skip(&name) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    walk(dir, dir, &mut files);
    hash::hash_files(&files).unwrap()
}

/// 断言两条路径在同一个目录上算出相同的指纹
fn assert_parity(label: &str, dir: &Path) -> String {
    let local = hash::hash_dir(dir).unwrap();
    let remote = remote_hash(dir);
    assert_eq!(local, remote, "{label}：本地与远端指纹不一致");
    local
}

#[test]
fn 只有_skill_md_的普通技能() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "SKILL.md",
        "---\nname: sales-report\ndescription: 生成销售周报\nversion: 1.2.0\n---\n# 正文\n",
    );
    assert_parity("只有 SKILL.md", d.path());
}

#[test]
fn 多层子目录() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(d.path(), "scripts/run.py", "print(1)\n");
    write(d.path(), "scripts/lib/util.py", "def f():\n    return 1\n");
    write(d.path(), "references/doc.md", "# doc\n");
    assert_parity("多层子目录", d.path());
}

#[test]
fn pycache_与_pyc_被排除() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(d.path(), "run.pyc", "garbage");
    write(d.path(), "__pycache__/mod.cpython-311.pyc", "garbage");
    write(d.path(), "scripts/__pycache__/a.pyc", "garbage");
    let with_junk = assert_parity("含 __pycache__", d.path());

    // 排除必须真的生效：拿掉这些垃圾文件后指纹应保持不变
    let clean = tempfile::tempdir().unwrap();
    write(clean.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    let without_junk = assert_parity("无 __pycache__", clean.path());
    assert_eq!(with_junk, without_junk, "__pycache__ / *.pyc 应被排除");
}

#[test]
fn 隐藏文件被排除() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(d.path(), ".DS_Store", "junk");
    write(d.path(), ".hidden/x", "junk");
    write(d.path(), ".git/config", "junk");
    let with_junk = assert_parity("含隐藏项", d.path());

    let clean = tempfile::tempdir().unwrap();
    write(clean.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    let without_junk = assert_parity("无隐藏项", clean.path());
    assert_eq!(with_junk, without_junk, "隐藏文件 / 目录应被排除");
}

#[test]
fn crlf_与_lf_混用() {
    let lf = tempfile::tempdir().unwrap();
    write(lf.path(), "SKILL.md", "---\nname: x\ndescription: d\n---\nbody\n");
    write(lf.path(), "scripts/run.py", "a = 1\nb = 2\n");

    let mixed = tempfile::tempdir().unwrap();
    write(
        mixed.path(),
        "SKILL.md",
        "---\r\nname: x\r\ndescription: d\r\n---\r\nbody\r\n",
    );
    write(mixed.path(), "scripts/run.py", "a = 1\r\nb = 2\n");

    let a = assert_parity("纯 LF", lf.path());
    let b = assert_parity("CRLF 混用", mixed.path());
    assert_eq!(a, b, "换行符差异不应影响指纹");
}

#[test]
fn frontmatter_易变字段被剥离() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "SKILL.md",
        "---\nname: x\ndescription: d\nenabled_at: 1787556778796\ncli_version: \">=1.0.15\"\n---\nbody\n",
    );
    assert_parity("含 enabled_at / cli_version", d.path());

    // 语义断言：带易变字段与不带易变字段两份内容，指纹必须相等
    let plain = vec![(
        "SKILL.md".to_string(),
        b"---\nname: x\ndescription: d\n---\nbody\n".to_vec(),
    )];
    let volatile = vec![(
        "SKILL.md".to_string(),
        b"---\nname: x\ndescription: d\nenabled_at: 1787556778796\ncli_version: \">=1.0.15\"\n---\nbody\n"
            .to_vec(),
    )];
    assert_eq!(
        hash::hash_files(&plain).unwrap(),
        hash::hash_files(&volatile).unwrap(),
        "易变字段必须被剥离"
    );
}

#[test]
fn frontmatter_键顺序无关() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "SKILL.md", "---\nname: x\ndescription: d\n---\nbody\n");
    write(b.path(), "SKILL.md", "---\ndescription: d\nname: x\n---\nbody\n");

    let ha = assert_parity("name 在前", a.path());
    let hb = assert_parity("description 在前", b.path());
    assert_eq!(ha, hb, "frontmatter 键顺序不应影响指纹");
}

#[test]
fn 排序分歧_readme_md_与_readme_目录() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(d.path(), "readme.md", "# 项目说明\n");
    write(d.path(), "readme/note.md", "# 补充说明\n");
    // 这两条路径按字符串排序与按路径分量排序的顺序相反
    assert_parity("readme.md 与 readme/note.md", d.path());
}

#[test]
fn 排序分歧_a_目录_与_a_x_文件() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(d.path(), "a/b.md", "# b\n");
    write(d.path(), "a-x.md", "# a-x\n");
    // `a/b.md` 与 `a-x.md` 在两种排序下顺序相反
    assert_parity("a/b.md 与 a-x.md", d.path());
}

#[test]
fn 无_frontmatter_不崩溃() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "# 只有正文，没有 frontmatter\n");
    assert_parity("无 frontmatter", d.path());
}

#[test]
fn 只改_version_会改变指纹() {
    // 产品决定：版本号算内容的一部分，改 version 必须提示可更新
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "SKILL.md", "---\nname: x\nversion: 1.2.0\n---\nbody\n");
    write(b.path(), "SKILL.md", "---\nname: x\nversion: 1.2.1\n---\nbody\n");

    let ha = assert_parity("version 1.2.0", a.path());
    let hb = assert_parity("version 1.2.1", b.path());
    assert_ne!(ha, hb, "version 必须参与哈希");
}

#[test]
fn 本地手改内容会改变指纹() {
    // 支撑验收 2：改 description 后应能识别为「已修改」
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\nname: x\ndescription: 旧描述\n---\nbody\n");
    let before = assert_parity("改动前", d.path());

    write(d.path(), "SKILL.md", "---\nname: x\ndescription: 新描述\n---\nbody\n");
    let after = assert_parity("改动后", d.path());
    assert_ne!(before, after, "内容改动必须改变指纹");
}

#[test]
fn 相对路径参与哈希() {
    // 支撑「改文件名也能被检测到」
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "a.txt", "same");
    write(b.path(), "b.txt", "same");
    let ha = assert_parity("a.txt", a.path());
    let hb = assert_parity("b.txt", b.path());
    assert_ne!(ha, hb, "相对路径必须参与哈希");
}

#[test]
fn 临时目录本身不作为相对路径的一部分() {
    // 同一份内容放在两个不同名字的临时目录里，指纹必须相同
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    write(b.path(), "SKILL.md", "---\nname: x\n---\nbody\n");
    assert_eq!(
        assert_parity("目录 A", a.path()),
        assert_parity("目录 B", b.path())
    );
}
