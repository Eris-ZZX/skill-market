//! 重新同步：修复断链与技能消失
//!
//! 核心风险是「修坏了」——用户手改过的副本绝不能因为一次「修复」被覆盖。
//! 这里用 tempdir + 显式路径注入验证，不依赖环境变量（#30 的教训），
//! 也不碰真实 AppData。

use skill_market_lib::hash;
use skill_market_lib::install::{
    is_link, junction_supported, resync_all, resync_one, symlink_supported, ResyncAction,
};
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};
use std::fs;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

/// 造一份库内容，返回它的哈希
fn seed_library(lib: &Path, name: &str) -> String {
    write(
        &lib.join(name),
        "SKILL.md",
        &format!("---\nname: {name}\ndescription: d\nversion: 1.0.0\n---\nbody\n"),
    );
    write(&lib.join(name), "scripts/run.py", "print(1)\n");
    hash::hash_dir(&lib.join(name)).unwrap()
}

fn rec(name: &str, tool: &str, target: &str, mode: InstallMode, h: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: target.into(),
        mode,
        installed_hash: h.into(),
        installed_version: Some("1.0.0".into()),
        source: SkillSource {
            kind: SourceKind::Gitlab,
            repo_url: None,
            repo_path: Some(format!("skills/通用/{name}")),
            branch: None,
            commit: None,
        },
        installed_at: 100,
        ignored_remote_hash: None,
    }
}

/// 链接类 mode；环境不支持时返回 None
fn link_mode() -> Option<InstallMode> {
    if junction_supported() {
        Some(InstallMode::Junction)
    } else if symlink_supported() {
        Some(InstallMode::Symlink)
    } else {
        None
    }
}

#[test]
fn copy_副本缺失时重建且哈希与库一致() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    // 目标不存在（模拟被手动删除）

    let (action, _) = resync_one("demo", &target, InstallMode::Copy, &h, &lib);

    assert_eq!(action, ResyncAction::Repaired);
    assert!(target.join("SKILL.md").is_file(), "应已重建");
    assert_eq!(
        hash::hash_dir(&target).unwrap(),
        h,
        "重建后必须与库内容一致"
    );
    assert_eq!(read(&target.join("scripts/run.py")), "print(1)\n");
}

#[test]
fn 链接缺失时按记录的_mode_重建() {
    let Some(mode) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");

    let (action, _) = resync_one("demo", &target, mode, &h, &lib);

    assert_eq!(action, ResyncAction::Repaired);
    assert!(is_link(&target), "应按记录的链接模式重建，而不是复制");
    assert_eq!(hash::hash_dir(&target).unwrap(), h);
}

#[test]
fn 断链被摘掉后重建() {
    let Some(mode) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    fs::create_dir_all(target.parent().unwrap()).unwrap();

    // 造一个指向已删除位置的链接 = 断链
    let gone = d.path().join("gone");
    fs::create_dir_all(&gone).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&gone, &target).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&gone, &target).unwrap();
    fs::remove_dir_all(&gone).unwrap();
    assert!(!target.exists() && is_link(&target), "前置：应是断链");

    let (action, _) = resync_one("demo", &target, mode, &h, &lib);

    assert_eq!(action, ResyncAction::Repaired);
    assert_eq!(hash::hash_dir(&target).unwrap(), h, "重建后应可读且一致");
}

#[test]
fn 副本被改过时保护性跳过且不覆盖() {
    // 本票最重要的一条：用户手改过的内容不能被「修复」覆盖
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    write(&target, "SKILL.md", "---\nname: demo\n---\n我手改的内容\n");
    let before = read(&target.join("SKILL.md"));
    let h_before = hash::hash_dir(&target).unwrap();

    let (action, msg) = resync_one("demo", &target, InstallMode::Copy, &h, &lib);

    assert_eq!(action, ResyncAction::SkippedModified);
    assert!(msg.contains("修改"), "应说明是因为被修改: {msg}");
    assert_eq!(
        read(&target.join("SKILL.md")),
        before,
        "内容绝对不能被覆盖"
    );
    assert_eq!(
        hash::hash_dir(&target).unwrap(),
        h_before,
        "指纹也不该变"
    );
}

#[test]
fn 库内容缺失时失败() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    fs::create_dir_all(&lib).unwrap();
    let target: PathBuf = d.path().join("t").join("skills").join("demo");

    let (action, msg) = resync_one("demo", &target, InstallMode::Copy, "h", &lib);

    assert_eq!(action, ResyncAction::Failed);
    assert!(msg.contains("重新安装"), "应提示需要重新安装: {msg}");
    assert!(!target.exists(), "库没有内容时不该凭空建出东西");
}

#[test]
fn 全部正常时全部_skipped() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    // 先正常复制一份
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::copy(
        lib.join("demo").join("SKILL.md"),
        target.join("SKILL.md"),
    )
    .unwrap();
    fs::create_dir_all(target.join("scripts")).unwrap();
    fs::copy(
        lib.join("demo").join("scripts/run.py"),
        target.join("scripts/run.py"),
    )
    .unwrap();
    assert_eq!(hash::hash_dir(&target).unwrap(), h);

    let (action, _) = resync_one("demo", &target, InstallMode::Copy, &h, &lib);
    assert_eq!(action, ResyncAction::Skipped);
}

#[test]
fn 部分失败不影响其余() {
    // 三条记录：一条修复、一条库缺失失败、一条被改过跳过
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h_ok = seed_library(&lib, "ok");
    let h_mod = seed_library(&lib, "modified");

    let t_ok = d.path().join("a").join("ok");
    let t_missing = d.path().join("b").join("gone"); // 库没有 gone
    let t_mod = d.path().join("c").join("modified");
    write(&t_mod, "SKILL.md", "---\nname: modified\n---\n手改\n");

    let records = vec![
        rec("ok", "claude", &t_ok.to_string_lossy(), InstallMode::Copy, &h_ok),
        rec("gone", "claude", &t_missing.to_string_lossy(), InstallMode::Copy, "whatever"),
        rec("modified", "cursor", &t_mod.to_string_lossy(), InstallMode::Copy, &h_mod),
    ];

    let r = resync_all(&records, &lib);

    assert_eq!(r.repaired.len(), 1, "只有 ok 该被修复");
    assert_eq!(r.repaired[0].0, "ok");
    assert_eq!(r.failed.len(), 1, "gone 应失败");
    assert_eq!(r.failed[0].0, "gone");
    assert_eq!(r.skipped_modified.len(), 1, "modified 应保护性跳过");
    assert_eq!(r.skipped_modified[0].0, "modified");
    // 关键：失败与跳过都没影响 ok 的修复
    assert!(t_ok.join("SKILL.md").is_file());
    assert!(!t_missing.exists(), "库缺失的记录不该凭空建出来");
    assert!(read(&t_mod.join("SKILL.md")).contains("手改"));
}

#[test]
fn 重建后内容与库一致且哈希相等() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");

    let (action, _) = resync_one("demo", &target, InstallMode::Copy, &h, &lib);
    assert_eq!(action, ResyncAction::Repaired);

    // 逐文件比对，而不只是比总哈希
    assert_eq!(
        read(&target.join("SKILL.md")),
        read(&lib.join("demo").join("SKILL.md"))
    );
    assert_eq!(
        read(&target.join("scripts/run.py")),
        read(&lib.join("demo").join("scripts/run.py"))
    );
    assert_eq!(hash::hash_dir(&target).unwrap(), hash::hash_dir(&lib.join("demo")).unwrap());
}

#[test]
fn 非法技能名直接失败不碰任何文件() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    seed_library(&lib, "demo");
    let target: PathBuf = d.path().join("t").join("evil");

    let (action, msg) = resync_one("../evil", &target, InstallMode::Copy, "h", &lib);
    assert_eq!(action, ResyncAction::Failed);
    assert!(msg.contains("非法"), "{msg}");
    assert!(!target.exists());
}

#[test]
fn 空的记录列表返回空报告() {
    let d = tempfile::tempdir().unwrap();
    let r = resync_all(&[], &d.path().join("library"));
    assert!(r.repaired.is_empty());
    assert!(r.skipped.is_empty());
    assert!(r.skipped_modified.is_empty());
    assert!(r.failed.is_empty());
}
