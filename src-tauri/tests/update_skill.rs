//! 更新部署：替换语义 + 失败不丢内容
//!
//! 最关键的一条是「替换失败后技能不消失」——只放开 MustBeEmpty 守卫而不
//! 把旧内容挪进暂存区，会让更新失败直接删掉用户的技能，比不能更新更糟。

use skill_market_lib::install::{
    backup_library, deploy_to_paths, is_link, make_symlink, symlink_supported, update_deploy,
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

fn skill(root: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let d = root.join(name);
    write(&d, "SKILL.md", &format!("---\nname: demo\n---\n{body}\n"));
    d
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

#[test]
fn 更新替换库内容且_copy_副本跟随变新() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let copy_target = d.path().join("tools").join("skills").join("demo");

    let v1 = skill(d.path(), "v1/demo", "v1");
    deploy_to_paths(&v1, &lib, &[copy_target.clone()], InstallMode::Copy).unwrap();
    assert!(read(&copy_target.join("SKILL.md")).contains("v1"));

    let v2 = skill(d.path(), "v2/demo", "v2 新版");
    update_deploy(
        &v2,
        &lib,
        "demo",
        &[(copy_target.clone(), InstallMode::Copy)],
    )
    .unwrap();

    assert!(read(&lib.join("demo").join("SKILL.md")).contains("v2 新版"));
    assert!(
        read(&copy_target.join("SKILL.md")).contains("v2 新版"),
        "copy 副本必须被替换成新版"
    );
}

#[test]
fn 更新时链接目标不重建但内容自动跟随() {
    if !symlink_supported() {
        eprintln!("跳过：当前环境无权创建目录软链");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let link_target = d.path().join("tools").join("skills").join("demo");

    let v1 = skill(d.path(), "v1/demo", "v1");
    deploy_to_paths(&v1, &lib, &[link_target.clone()], InstallMode::Symlink).unwrap();
    assert!(is_link(&link_target));

    let v2 = skill(d.path(), "v2/demo", "v2 新版");
    update_deploy(
        &v2,
        &lib,
        "demo",
        &[(link_target.clone(), InstallMode::Symlink)],
    )
    .unwrap();

    assert!(is_link(&link_target), "链接本身不该被替换成目录");
    assert!(
        read(&link_target.join("SKILL.md")).contains("v2 新版"),
        "链接应自动跟随新的库内容"
    );
}

#[test]
fn 混合_mode_更新后两者都变新() {
    if !symlink_supported() {
        eprintln!("跳过：当前环境无权创建目录软链");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");

    let link_target = d.path().join("a").join("skills").join("demo");
    let copy_target = d.path().join("b").join("skills").join("demo");

    let v1 = skill(d.path(), "v1/demo", "v1");
    // 同一个技能分别用链接与复制装到两个「工具」
    deploy_to_paths(&v1, &lib, &[link_target.clone()], InstallMode::Symlink).unwrap();
    deploy_to_paths(&v1, &lib, &[copy_target.clone()], InstallMode::Copy).unwrap();

    let v2 = skill(d.path(), "v2/demo", "v2 新版");
    update_deploy(
        &v2,
        &lib,
        "demo",
        &[
            (link_target.clone(), InstallMode::Symlink),
            (copy_target.clone(), InstallMode::Copy),
        ],
    )
    .unwrap();

    assert!(read(&link_target.join("SKILL.md")).contains("v2 新版"));
    assert!(read(&copy_target.join("SKILL.md")).contains("v2 新版"));
}

#[test]
fn 替换失败后技能不消失() {
    // 本票最关键的一条：第二个目标替换失败时，第一个目标的原内容必须还在
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");

    let t1 = d.path().join("a").join("skills").join("demo");
    let t2 = d.path().join("b").join("skills").join("demo");

    let v1 = skill(d.path(), "v1/demo", "v1 原始内容");
    deploy_to_paths(&v1, &lib, &[t1.clone()], InstallMode::Copy).unwrap();
    deploy_to_paths(&v1, &lib, &[t2.clone()], InstallMode::Copy).unwrap();
    let before = read(&t1.join("SKILL.md"));
    assert!(before.contains("v1"));

    // 让第二个目标的替换必然失败：把它的父目录换成普通文件，
    // 使 tempdir_in(parent) 失败——这样第一个已进暂存但尚未替换
    fs::remove_dir_all(t2.parent().unwrap()).unwrap();
    fs::write(t2.parent().unwrap(), "not a dir").unwrap();

    let v2 = skill(d.path(), "v2/demo", "v2 新版");
    let r = update_deploy(
        &v2,
        &lib,
        "demo",
        &[(t1.clone(), InstallMode::Copy), (t2.clone(), InstallMode::Copy)],
    );

    assert!(r.is_err(), "第二个目标失败应让整体失败");
    assert!(t1.join("SKILL.md").is_file(), "第一个目标不能被删空");
    assert_eq!(
        read(&t1.join("SKILL.md")),
        before,
        "第一个目标必须还原成更新前的内容"
    );
}

#[test]
fn 库替换后旧库内容可恢复() {
    // 备份是兜底：更新后旧版本仍能在备份目录里找到
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    let target = d.path().join("t").join("demo");

    let v1 = skill(d.path(), "v1/demo", "v1 原始");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();

    let bak = backup_library(&lib, "demo", &backups).unwrap();
    assert!(bak.is_dir(), "备份目录应存在");
    assert!(
        read(&bak.join("SKILL.md")).contains("v1 原始"),
        "备份里应是旧内容"
    );

    // 更新后旧内容仍可从备份取回
    let v2 = skill(d.path(), "v2/demo", "v2 新版");
    update_deploy(&v2, &lib, "demo", &[(target.clone(), InstallMode::Copy)]).unwrap();
    assert!(read(&lib.join("demo").join("SKILL.md")).contains("v2 新版"));
    assert!(read(&bak.join("SKILL.md")).contains("v1 原始"));
}

#[test]
fn 备份重名时加后缀不覆盖() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    let demo = lib.join("demo");
    fs::create_dir_all(&demo).unwrap();
    write(&demo, "SKILL.md", "x");

    let a = backup_library(&lib, "demo", &backups).unwrap();
    let b = backup_library(&lib, "demo", &backups).unwrap();
    assert_ne!(a, b, "同一秒内两次备份不能互相覆盖");
    assert!(a.is_dir() && b.is_dir());
}

#[test]
fn 更新拒绝非法技能名() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let src = skill(d.path(), "v2/demo", "v2");
    let r = update_deploy(&src, &lib, "../escape", &[]);
    assert!(r.is_err(), "路径穿越式的名字必须被拒绝");
}

#[test]
fn 用_make_symlink_构造的环境仍能识别链接() {
    // 保证 is_link 在两个平台都能识别 make_symlink 造出来的东西
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let dst = d.path().join("dst");
    if !make_symlink(&src, &dst) {
        eprintln!("跳过：当前环境无权创建目录软链");
        return;
    }
    assert!(is_link(&dst));
}
