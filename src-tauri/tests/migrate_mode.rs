//! 同步方式批量转换
//!
//! 核心风险是「转换 = 删副本」。用户手改过的副本一旦被删就找不回来，
//! 所以哈希保护必须排在所有写操作之前——这一条单独有测试。
//!
//! 全部用 tempdir + 显式路径，不依赖环境变量、不碰真实 AppData。
//! junction/symlink 用能力探测分叉（Linux 无 junction，Windows 都有）。

use skill_market_lib::hash;
use skill_market_lib::install::{
    is_link, junction_supported, migrate_all, migrate_one, symlink_supported, MigrateAction,
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

/// 造库内容，返回哈希
fn seed_library(lib: &Path, name: &str, body: &str) -> String {
    write(
        &lib.join(name),
        "SKILL.md",
        &format!("---\nname: {name}\ndescription: d\nversion: 1.0.0\n---\n{body}\n"),
    );
    write(&lib.join(name), "scripts/run.py", "print(1)\n");
    hash::hash_dir(&lib.join(name)).unwrap()
}

/// 把库内容复制成一个 copy 部署（模拟已用 copy 装过）
fn deploy_copy(ssot: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for e in fs::read_dir(ssot).unwrap().flatten() {
        let to = target.join(e.file_name());
        if e.path().is_dir() {
            deploy_copy(&e.path(), &to);
        } else {
            fs::copy(e.path(), &to).unwrap();
        }
    }
}

fn rec(name: &str, tool: &str, target: &Path, mode: InstallMode, h: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: target.to_string_lossy().to_string(),
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
        installed_at: 1,
        ignored_remote_hash: None,
    }
}

/// 链接类目标方式；环境都不支持时返回 None
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
fn copy_转链接成功且内容不变() {
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    deploy_copy(&lib.join("demo"), &target);
    assert!(!is_link(&target), "前置：应是实体副本");

    let (action, used, freed, msg) = migrate_one(
        "demo",
        &target,
        InstallMode::Copy,
        &h,
        link,
        &lib,
    );

    assert_eq!(action, MigrateAction::Migrated, "{msg}");
    assert_eq!(used, link, "实际方式应是目标方式");
    assert!(is_link(&target), "转换后应是链接");
    assert_eq!(hash::hash_dir(&target).unwrap(), h, "内容必须可读且一致");
    assert!(freed > 0, "copy→链接应释放空间，实际 {freed}");
}

#[test]
fn 链接转_copy_成功() {
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    // 先建成链接
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let (a, _, _, m) = migrate_one("demo", &target, InstallMode::Copy, &h, link, &lib);
    assert_eq!(a, MigrateAction::Failed, "目标不存在时应失败: {m}");

    // 正经路径：先 copy 再转链接，然后转回去
    deploy_copy(&lib.join("demo"), &target);
    let (a1, _, _, _) = migrate_one("demo", &target, InstallMode::Copy, &h, link, &lib);
    assert_eq!(a1, MigrateAction::Migrated);

    let (a2, used2, freed2, msg2) = migrate_one(
        "demo",
        &target,
        link,
        &h,
        InstallMode::Copy,
        &lib,
    );
    assert_eq!(a2, MigrateAction::Migrated, "{msg2}");
    assert_eq!(used2, InstallMode::Copy);
    assert!(!is_link(&target), "转换后应是实体目录");
    assert_eq!(hash::hash_dir(&target).unwrap(), h);
    assert_eq!(freed2, 0, "反向转换不释放空间");
}

#[test]
fn 已是目标方式时_skipped() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("demo");
    deploy_copy(&lib.join("demo"), &target);

    let (action, _, freed, _) = migrate_one(
        "demo",
        &target,
        InstallMode::Copy,
        &h,
        InstallMode::Copy,
        &lib,
    );
    assert_eq!(action, MigrateAction::Skipped);
    assert_eq!(freed, 0);
}

#[test]
fn 副本被改过时保护性跳过且内容完整保留() {
    // 本票最重要的一条：转换要删副本，改过的副本删了就找不回来
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("skills").join("demo");
    deploy_copy(&lib.join("demo"), &target);
    // 用户手改
    write(&target, "SKILL.md", "---\nname: demo\n---\n我改过\n");
    let before = read(&target.join("SKILL.md"));
    let h_before = hash::hash_dir(&target).unwrap();

    let (action, used, freed, msg) =
        migrate_one("demo", &target, InstallMode::Copy, &h, link, &lib);

    assert_eq!(action, MigrateAction::SkippedModified);
    assert_eq!(used, InstallMode::Copy, "方式不该变");
    assert_eq!(freed, 0, "没转换就不该报释放空间");
    assert!(msg.contains("修改"), "应说明原因: {msg}");
    assert!(!is_link(&target), "不能变成链接");
    assert_eq!(read(&target.join("SKILL.md")), before, "内容绝不能动");
    assert_eq!(hash::hash_dir(&target).unwrap(), h_before);
}

#[test]
fn 库内容缺失时失败() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    fs::create_dir_all(&lib).unwrap();
    // 目标存在且有内容，但库里没有同名技能 -> 没有重建源
    let target: PathBuf = d.path().join("t").join("demo");
    write(&target, "SKILL.md", "---\nname: demo\n---\nx\n");
    let h = hash::hash_dir(&target).unwrap();

    // mode 与目标相同会先命中 skipped，所以用一个不同的目标方式
    let (action, _, _, msg) = migrate_one(
        "demo",
        &target,
        InstallMode::Copy,
        &h,
        InstallMode::Junction,
        &lib,
    );
    assert_eq!(action, MigrateAction::Failed, "{msg}");
    assert!(msg.contains("重新安装"), "{msg}");
    // 失败时不得动目标
    assert!(target.join("SKILL.md").is_file());
}

#[test]
fn 一条失败不影响其余() {
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h_ok = seed_library(&lib, "ok", "a");
    let h_mod = seed_library(&lib, "modified", "b");

    let t_ok: PathBuf = d.path().join("a").join("ok");
    let t_missing: PathBuf = d.path().join("b").join("gone"); // 库没有 gone
    let t_mod: PathBuf = d.path().join("c").join("modified");
    deploy_copy(&lib.join("ok"), &t_ok);
    deploy_copy(&lib.join("modified"), &t_mod);
    write(&t_mod, "SKILL.md", "---\nname: modified\n---\n手改\n");

    let records = vec![
        rec("ok", "claude", &t_ok, InstallMode::Copy, &h_ok),
        rec("gone", "claude", &t_missing, InstallMode::Copy, "x"),
        rec("modified", "cursor", &t_mod, InstallMode::Copy, &h_mod),
    ];
    let r = migrate_all(&records, &lib, link);

    assert_eq!(r.migrated.len(), 1, "只有 ok 该转换");
    assert_eq!(r.migrated[0].0, "ok");
    assert_eq!(r.failed.len(), 1, "gone 应失败");
    assert_eq!(r.skipped_modified.len(), 1, "modified 应保护性跳过");
    // 失败与跳过都没妨碍 ok
    assert!(is_link(&t_ok));
    assert!(read(&t_mod.join("SKILL.md")).contains("手改"));
}

#[test]
fn freed_bytes_只统计真正释放的副本大小() {
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h1 = seed_library(&lib, "big", "x");
    let h2 = seed_library(&lib, "small", "y");

    let t1: PathBuf = d.path().join("a").join("big");
    let t2: PathBuf = d.path().join("b").join("small");
    deploy_copy(&lib.join("big"), &t1);
    deploy_copy(&lib.join("small"), &t2);

    // 记录副本的实际大小
    let expected: u64 = [&t1, &t2]
        .iter()
        .map(|p| {
            fs::read_dir(p)
                .unwrap()
                .flatten()
                .map(|e| {
                    if e.path().is_dir() {
                        fs::read_dir(e.path())
                            .unwrap()
                            .flatten()
                            .map(|x| fs::metadata(x.path()).unwrap().len())
                            .sum::<u64>()
                    } else {
                        fs::metadata(e.path()).unwrap().len()
                    }
                })
                .sum::<u64>()
        })
        .sum();

    let records = vec![
        rec("big", "claude", &t1, InstallMode::Copy, &h1),
        rec("small", "cursor", &t2, InstallMode::Copy, &h2),
    ];
    let r = migrate_all(&records, &lib, link);

    assert_eq!(r.migrated.len(), 2);
    assert_eq!(
        r.freed_bytes, expected,
        "释放量应等于两个副本的实际占用"
    );
}

#[test]
fn 库内容在转换前后哈希不变() {
    // 转换只改工具目录的部署方式，绝不碰库
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("demo");
    deploy_copy(&lib.join("demo"), &target);

    let before = hash::hash_dir(&lib.join("demo")).unwrap();
    let (a, _, _, _) = migrate_one("demo", &target, InstallMode::Copy, &h, link, &lib);
    assert_eq!(a, MigrateAction::Migrated);

    assert_eq!(
        hash::hash_dir(&lib.join("demo")).unwrap(),
        before,
        "库内容必须完全不变"
    );
}

#[test]
fn 批量转换后记录的_mode_是实际采用值() {
    let Some(link) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let h = seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("demo");
    deploy_copy(&lib.join("demo"), &target);

    let records = vec![rec("demo", "cursor", &target, InstallMode::Copy, &h)];
    let r = migrate_all(&records, &lib, link);

    assert_eq!(r.migrated.len(), 1);
    let (name, tool, from, to) = &r.migrated[0];
    assert_eq!(name, "demo");
    assert_eq!(tool, "cursor");
    assert_eq!(*from, InstallMode::Copy, "原方式应如实记录");
    assert_eq!(*to, link, "新方式应是实际采用的（不是用户请求的字面值）");
}

#[test]
fn 空记录列表返回空报告() {
    let d = tempfile::tempdir().unwrap();
    let r = migrate_all(&[], &d.path().join("library"), InstallMode::Copy);
    assert!(r.migrated.is_empty());
    assert!(r.skipped.is_empty());
    assert!(r.skipped_modified.is_empty());
    assert!(r.failed.is_empty());
    assert_eq!(r.freed_bytes, 0);
}

#[test]
fn 非法技能名直接失败() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    seed_library(&lib, "demo", "v1");
    let target: PathBuf = d.path().join("t").join("evil");

    let (action, _, _, msg) = migrate_one(
        "../evil",
        &target,
        InstallMode::Copy,
        "h",
        InstallMode::Junction,
        &lib,
    );
    assert_eq!(action, MigrateAction::Failed);
    assert!(msg.contains("非法"), "{msg}");
}
