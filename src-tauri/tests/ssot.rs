//! SSOT 部署：单一副本 + 链接/复制
//!
//! 用 tempdir 同时充当「库」与「工具技能目录」，不碰真实 %APPDATA%。
//! `deploy_to_paths` 接受显式路径，正是为了让这些用例能脱离 `ToolKind`
//! 写死的 home 目录。

use skill_market_lib::install::{
    deploy_to_paths, ensure_distinct_skill_roots, is_link, make_symlink, remove_installed,
    sanitize_install_name, symlink_supported,
};
use skill_market_lib::state::InstallMode;
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

/// 造一个技能目录，目录名就是技能名
fn skill(root: &Path, name: &str) -> std::path::PathBuf {
    let d = root.join(name);
    write(&d, "SKILL.md", "---\nname: demo\n---\nbody\n");
    write(&d, "scripts/run.py", "print(1)\n");
    d
}

#[test]
fn 八个非法目录名全被拒绝() {
    let bad = [
        "..",
        "../../",
        "a/b",
        "a\\b",
        "",
        ".hidden",
        "C:\\evil",
        "/etc",
    ];
    for name in bad {
        assert!(sanitize_install_name(name).is_none(), "{name:?} 应被拒绝");
    }
}

#[test]
fn 合法目录名通过() {
    for name in ["demo", "sales-report", "技能名", "a1"] {
        assert_eq!(sanitize_install_name(name).as_deref(), Some(name));
    }
    assert_eq!(sanitize_install_name("  demo  ").as_deref(), Some("demo"));
}

#[test]
fn 库目录与工具目录重合时报错() {
    let d = tempfile::tempdir().unwrap();
    let same = d.path().join("skills");
    fs::create_dir_all(&same).unwrap();
    assert!(
        ensure_distinct_skill_roots(&same, &same, "Cursor").is_err(),
        "库目录与技能目录相同时必须报错"
    );

    let other = d.path().join("other");
    fs::create_dir_all(&other).unwrap();
    assert!(ensure_distinct_skill_roots(&same, &other, "Cursor").is_ok());
}

#[test]
fn symlink_部署后工具目录指向库且内容可读() {
    // Linux 上 junction 不可用，用 symlink 代替；Windows 下是等价的目录软链
    if !symlink_supported() {
        eprintln!("跳过：当前环境无权创建目录软链");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let src = skill(d.path(), "demo");
    let lib = d.path().join("library");
    let target = d.path().join("tools").join("skills").join("demo");

    let (ssot, used) =
        deploy_to_paths(&src, &lib, &[target.clone()], InstallMode::Symlink).unwrap();

    assert_eq!(used, vec![InstallMode::Symlink]);
    assert!(is_link(&target), "工具目录里应是链接");
    assert!(target.join("SKILL.md").is_file(), "通过链接能读到内容");
    assert!(ssot.join("SKILL.md").is_file(), "库里有唯一副本");

    // 改库里的文件，工具目录应同步看到
    fs::write(ssot.join("SKILL.md"), "---\nname: demo\n---\n改过\n").unwrap();
    let seen = fs::read_to_string(target.join("SKILL.md")).unwrap();
    assert!(seen.contains("改过"), "链接应反映库里的最新内容");
}

#[test]
fn 显式选链接但环境不支持时报错不降级() {
    let d = tempfile::tempdir().unwrap();
    let src = skill(d.path(), "demo");
    let lib = d.path().join("library");
    // 目标父路径是个文件，create_dir_all 必然失败
    let blocker = d.path().join("blocked");
    fs::write(&blocker, "not a dir").unwrap();
    let target = blocker.join("demo");

    let r = deploy_to_paths(&src, &lib, &[target.clone()], InstallMode::Symlink);
    assert!(r.is_err(), "显式 Symlink 失败必须报错");
    assert!(!target.exists(), "失败后不应留下复制品");
}

#[test]
fn 多目标部署第二个失败时第一个被回滚() {
    let d = tempfile::tempdir().unwrap();
    let src = skill(d.path(), "demo");
    let lib = d.path().join("library");

    let ok_target = d.path().join("a").join("skills").join("demo");
    // 第二个目标的父路径是个文件
    let blocker = d.path().join("b");
    fs::write(&blocker, "not a dir").unwrap();
    let bad_target = blocker.join("skills").join("demo");

    let r = deploy_to_paths(
        &src,
        &lib,
        &[ok_target.clone(), bad_target],
        InstallMode::Copy,
    );
    assert!(r.is_err(), "第二个目标失败应让整体失败");
    assert!(
        !ok_target.exists(),
        "已装的第一个目标必须被回滚干净，实际仍在: {}",
        ok_target.display()
    );
}

#[test]
fn 卸载链接不删库里的真身() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let demo = lib.join("demo");
    fs::create_dir_all(&demo).unwrap();
    write(&demo, "SKILL.md", "---\nname: demo\n---\nbody\n");

    let tool_skills = d.path().join("tool-skills");
    fs::create_dir_all(&tool_skills).unwrap();
    let link = tool_skills.join("demo");
    if !make_symlink(&demo, &link) {
        eprintln!("跳过：当前环境无权创建目录软链");
        return;
    }

    assert!(is_link(&link));
    remove_installed(&link).unwrap();

    assert!(!is_link(&link) && !link.exists(), "链接应已移除");
    assert!(demo.join("SKILL.md").is_file(), "库里的真身必须还在");
}

#[test]
fn 老格式_copy_与_junction_仍能反序列化() {
    assert_eq!(
        serde_json::from_str::<InstallMode>("\"copy\"").unwrap(),
        InstallMode::Copy
    );
    assert_eq!(
        serde_json::from_str::<InstallMode>("\"junction\"").unwrap(),
        InstallMode::Junction
    );
    for m in [InstallMode::Symlink, InstallMode::Auto] {
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<InstallMode>(&s).unwrap(), m);
    }
}

#[test]
fn auto_在无_junction_的平台上回退到_copy() {
    let d = tempfile::tempdir().unwrap();
    let src = skill(d.path(), "demo");
    let lib = d.path().join("library");
    let target = d.path().join("t").join("skills").join("demo");

    let (_ssot, used) =
        deploy_to_paths(&src, &lib, &[target.clone()], InstallMode::Auto).unwrap();

    // Linux 上 junction 不可用 → 必须回退 copy 且真的复制了文件
    assert_eq!(used, vec![InstallMode::Copy]);
    assert!(!is_link(&target));
    assert!(target.join("SKILL.md").is_file());
}

#[test]
fn 库已有同名副本内容相同则不重复写() {
    let d = tempfile::tempdir().unwrap();
    let src = skill(d.path(), "demo");
    let lib = d.path().join("library");
    let t1 = d.path().join("t1").join("demo");

    let (ssot, _) = deploy_to_paths(&src, &lib, &[t1], InstallMode::Copy).unwrap();
    let first = fs::read_to_string(ssot.join("SKILL.md")).unwrap();

    // 再装一次到另一个目标：库内容未变，应被复用
    let t2 = d.path().join("t2").join("demo");
    let (ssot2, _) = deploy_to_paths(&src, &lib, &[t2], InstallMode::Copy).unwrap();
    assert_eq!(ssot, ssot2);
    assert_eq!(first, fs::read_to_string(ssot2.join("SKILL.md")).unwrap());
}
