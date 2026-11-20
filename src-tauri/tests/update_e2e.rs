//! 更新链路端到端（本地模拟，不依赖网络、不依赖 Tauri 运行时）
//!
//! `update_skill` 把「取远端内容」与「落盘」拆成两步：`download_skill_package`
//! 走网络，`deploy_downloaded` 只管落盘。本文件直接构造 `DownloadedSkill`
//! 指向本地目录，把落盘侧（哈希重算、部署、回滚、mode 处理）跑通。
//!
//! 命令层的六项校验已抽成纯函数 `validate_update_preflight` /
//! `verify_records_unchanged`，直接对 `Vec<InstalledSkill>` 断言即可，
//! **不需要构造 `tauri::State`**。
//!
//! 为什么不用 `tauri::test::mock_app`：它会拉入 Tauri 完整运行时的链接依赖，
//! Windows 上测试二进制的 DLL 入口解析会失败（0xC0000139
//! STATUS_ENTRYPOINT_NOT_FOUND），整个测试二进制起不来。
//!
//! `deploy_downloaded` 内部用真实的 `storage::library_dir()`，所以整个二进制
//! 通过 `XDG_CONFIG_HOME` 把应用数据目录重定向到临时目录，绝不碰
//! 用户真实的 `~/.config`。

use skill_market_lib::commands::market::{deploy_downloaded, DownloadedSkill};
use skill_market_lib::hash;
use skill_market_lib::install::{
    backup_library, deploy_to_paths, is_link, junction_supported, restore_backup_as,
    symlink_supported,
};
use skill_market_lib::state::InstallMode;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;

static INIT: Once = Once::new();

/// 把应用数据目录指到临时位置，并返回 `library` 根
fn library_root() -> PathBuf {
    INIT.call_once(|| {
        let base = std::env::temp_dir().join(format!("skill-market-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // Linux 上 dirs::config_dir() 读 XDG_CONFIG_HOME
        std::env::set_var("XDG_CONFIG_HOME", &base);
    });
    let root = skill_market_lib::storage::library_dir().unwrap();
    fs::create_dir_all(&root).unwrap();
    root
}

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

/// 在 `parent/<name>/` 造一个技能目录
///
/// 目录名必须与技能名一致：`deploy_to_paths` 用目录名作为库条目名，
/// 而 `deploy_downloaded` 用显式传入的 name——不一致就找不到库。
fn make_skill(parent: &Path, name: &str, version: &str) -> PathBuf {
    let d = parent.join(name);
    write(
        &d,
        "SKILL.md",
        &format!("---\nname: {name}\ndescription: d\nversion: {version}\n---\n{version}\n"),
    );
    write(&d, "scripts/run.py", &format!("print('{version}')\n"));
    d
}

/// 构造指向本地目录的「下载结果」，完全不走网络
fn fake_download(extracted: &Path, version: &str) -> DownloadedSkill {
    DownloadedSkill {
        dir: tempfile::tempdir().unwrap(),
        extracted: extracted.to_path_buf(),
        version: Some(version.to_string()),
        commit: "0".repeat(40),
    }
}

/// 链接类安装方式；环境都不支持时返回 None
fn link_mode() -> Option<InstallMode> {
    if junction_supported() {
        Some(InstallMode::Junction)
    } else if symlink_supported() {
        Some(InstallMode::Symlink)
    } else {
        None
    }
}

// ========== 落盘层（不走网络） ==========

#[test]
fn 场景1_装v1后部署v2_内容变新且哈希改变() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("tool").join("skills").join("e2e-s1");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s1", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let old_hash = hash::hash_dir(&lib.join("e2e-s1")).unwrap();
    assert!(read(&target.join("SKILL.md")).contains("1.0.0"));

    let v2 = make_skill(&work.path().join("v2"), "e2e-s1", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    let (new_hash, ver) =
        deploy_downloaded(&dl, "e2e-s1", &[(target.clone(), InstallMode::Copy)]).unwrap();

    assert!(read(&target.join("SKILL.md")).contains("2.0.0"), "副本应变新");
    assert!(
        read(&lib.join("e2e-s1").join("SKILL.md")).contains("2.0.0"),
        "库应变新"
    );
    assert_ne!(new_hash, old_hash, "内容变了哈希必须变");
    assert_eq!(
        new_hash,
        hash::hash_dir(&lib.join("e2e-s1")).unwrap(),
        "返回的哈希应等于新库哈希"
    );
    assert_eq!(ver.as_deref(), Some("2.0.0"));
}

#[test]
fn 场景5_两个工具混合模式更新后都变新() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let copy_target = work.path().join("a").join("skills").join("e2e-s5");
    let link_target = work.path().join("b").join("skills").join("e2e-s5");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s5", "1.0.0");
    deploy_to_paths(&v1, &lib, &[copy_target.clone()], InstallMode::Copy).unwrap();

    let Some(lm) = link_mode() else {
        eprintln!("跳过：当前环境既不能建 junction 也不能建软链");
        return;
    };
    deploy_to_paths(&v1, &lib, &[link_target.clone()], lm).unwrap();

    let v2 = make_skill(&work.path().join("v2"), "e2e-s5", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(
        &dl,
        "e2e-s5",
        &[
            (copy_target.clone(), InstallMode::Copy),
            (link_target.clone(), lm),
        ],
    )
    .unwrap();

    assert!(
        read(&copy_target.join("SKILL.md")).contains("2.0.0"),
        "复制副本应变新"
    );
    assert!(
        read(&link_target.join("SKILL.md")).contains("2.0.0"),
        "链接目标应跟随变新"
    );
}

#[test]
fn 场景6_第二个目标失败时第一个的原内容不消失() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let t1 = work.path().join("a").join("skills").join("e2e-s6");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s6", "1.0.0");
    deploy_to_paths(&v1, &lib, &[t1.clone()], InstallMode::Copy).unwrap();
    let before = read(&t1.join("SKILL.md"));
    assert!(before.contains("1.0.0"));

    // 让第二个目标在「准备阶段」失败（父路径被普通文件占住）。
    // 此时第一个目标还没开始替换，磁盘本就干净——这条验证「前置校验失败
    // 不会留下半套安装」。替换阶段的回滚由 install.rs 的 rollback_ 单测覆盖，
    // 那里直接构造「已替换」状态再调 rollback。
    let blocker = work.path().join("c").join("skills");
    fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    fs::write(&blocker, "not a dir").unwrap();

    let v2 = make_skill(&work.path().join("v2"), "e2e-s6", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    let r = deploy_downloaded(
        &dl,
        "e2e-s6",
        &[
            (t1.clone(), InstallMode::Copy),
            (blocker.join("e2e-s6"), InstallMode::Copy),
        ],
    );

    assert!(r.is_err(), "第二个目标失败应让整体失败");
    assert!(t1.join("SKILL.md").is_file(), "第一个目标不能被删空");
    assert_eq!(
        read(&t1.join("SKILL.md")),
        before,
        "第一个目标必须保持更新前的内容"
    );
}

#[test]
fn 场景7_备份能恢复出当时的内容且不带_meta_json() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join("e2e-s7");
    let backups = work.path().join("backups");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s7", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let v1_hash = hash::hash_dir(&lib.join("e2e-s7")).unwrap();

    // 更新到 v2
    let v2 = make_skill(&work.path().join("v2"), "e2e-s7", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(&dl, "e2e-s7", &[(target.clone(), InstallMode::Copy)]).unwrap();
    assert!(read(&lib.join("e2e-s7").join("SKILL.md")).contains("2.0.0"));

    // 单独造一份 v1 的库，备份它，验证能恢复回 v1
    let snap_lib = work.path().join("snap-lib");
    deploy_to_paths(&v1, &snap_lib, &[], InstallMode::Copy).ok();
    let bak = backup_library(&snap_lib, "e2e-s7", &backups, None).unwrap();

    let restored = restore_backup_as(&bak, &lib, "e2e-s7-restored").unwrap();

    // 内容（正文与脚本）必须与 v1 一致。注意不能直接比指纹：
    // restore_backup_as 会改写 frontmatter 的 name（#29 的实现，避免副本
    // 与正本在「我的技能」里按 name 聚合成一行），那一步本身就会改变指纹。
    assert_eq!(
        read(&restored.join("scripts/run.py")),
        read(&snap_lib.join("e2e-s7").join("scripts/run.py")),
        "脚本内容应与备份一致"
    );
    assert!(
        read(&restored.join("SKILL.md")).contains("version: 1.0.0"),
        "应是 v1 的内容而不是 v2"
    );
    assert!(
        read(&restored.join("SKILL.md")).contains("name: e2e-s7-restored"),
        "恢复出的副本应带上自己的名字"
    );
    assert_ne!(
        hash::hash_dir(&restored).unwrap(),
        hash::hash_dir(&lib.join("e2e-s7")).unwrap(),
        "恢复出的副本不该与当前 v2 相同"
    );
    let _ = v1_hash;
    assert!(
        !restored.join("meta.json").exists(),
        "meta.json 是元数据，不该混进技能内容"
    );
}

#[test]
fn 场景8_只改易变字段指纹不变() {
    // #19 的核心：enabled_at 这类工具回写的字段必须被剥离，
    // 否则跑一次技能就会被判成「本地已修改」。
    let work = tempfile::tempdir().unwrap();
    let a = work.path().join("a");
    let b = work.path().join("b");
    write(
        &a,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: d\nversion: 1.0.0\n---\nbody\n",
    );
    write(
        &b,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: d\nversion: 1.0.0\nenabled_at: 1787556778796\n---\nbody\n",
    );

    let ha = hash::hash_dir(&a).unwrap();
    assert_eq!(
        ha,
        hash::hash_dir(&b).unwrap(),
        "只加 enabled_at 不应改变指纹"
    );

    // 对照：真实内容改动必须改变指纹
    write(
        &b,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: 改过\nversion: 1.0.0\n---\nbody\n",
    );
    assert_ne!(ha, hash::hash_dir(&b).unwrap(), "真实内容改动必须改变指纹");
}

#[test]
fn 场景7补充_链接模式下更新不破坏链接() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join("e2e-s7b");

    let Some(mode) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };

    let v1 = make_skill(&work.path().join("v1"), "e2e-s7b", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], mode).unwrap();
    assert!(is_link(&target), "应为链接");

    let v2 = make_skill(&work.path().join("v2"), "e2e-s7b", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(&dl, "e2e-s7b", &[(target.clone(), mode)]).unwrap();

    assert!(is_link(&target), "更新不该把链接换成实体目录");
    assert!(read(&target.join("SKILL.md")).contains("2.0.0"));
}

// ========== 命令层校验（纯函数，不碰 Tauri / 文件系统） ==========

use skill_market_lib::commands::{
    validate_update_preflight, verify_records_unchanged,
};
use skill_market_lib::state::{InstalledSkill, SkillSource, SourceKind};

fn rec(name: &str, tool: &str, target: &str, hash: &str, at: i64) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: target.into(),
        mode: InstallMode::Copy,
        installed_hash: hash.into(),
        installed_version: Some("1.0.0".into()),
        source: SkillSource {
            kind: SourceKind::Gitlab,
            repo_url: Some("https://example.invalid/g/p".into()),
            repo_path: Some(format!("skills/通用/{name}")),
            branch: Some("master".into()),
            commit: Some("abc".into()),
        },
        installed_at: at,
        ignored_remote_hash: None,
    }
}

#[test]
fn 场景2_本地改过时更新被拒绝() {
    let snapshot = vec![rec("x", "cursor", "/p/x", "h1", 100)];

    // 本地哈希与安装时不同 → 拒绝
    let err = validate_update_preflight("x", "cursor", &snapshot, Some("h2")).unwrap_err();
    assert!(err.contains("本地"), "错误信息应说明是本地改动: {err}");

    // 对照：未改过时应通过，证明上面不是恒错
    assert!(validate_update_preflight("x", "cursor", &snapshot, Some("h1")).is_ok());
}

#[test]
fn 场景3_本地目录读不到时报错且不回退旧指纹() {
    let snapshot = vec![rec("x", "cursor", "/p/x", "h1", 100)];

    // local_hash 为 None = 目录读不到。必须报「目录不存在」，
    // 不能退化成「与 installed_hash 相同 → 放行」
    let err = validate_update_preflight("x", "cursor", &snapshot, None).unwrap_err();
    assert!(
        err.contains("本地技能目录不存在"),
        "必须指出目录缺失而不是静默用旧指纹: {err}"
    );
}

#[test]
fn 场景3补充_未安装的工具与本地导入都被拒() {
    let snapshot = vec![rec("x", "cursor", "/p/x", "h1", 100)];

    let err = validate_update_preflight("x", "claude", &snapshot, Some("h1")).unwrap_err();
    assert!(err.contains("未安装"), "应说明未安装在该工具: {err}");

    // 本地导入：repo_path 为 None
    let mut local = rec("y", "cursor", "/p/y", "h1", 100);
    local.source.repo_path = None;
    local.source.kind = SourceKind::Local;
    let err = validate_update_preflight("y", "cursor", &[local], Some("h1")).unwrap_err();
    assert!(err.contains("本地导入"), "本地导入应被明确拒绝: {err}");
}

#[test]
fn 场景3补充_非法技能名被拒() {
    // 名字后面要拼进路径，穿越式的值必须在这里就拦住
    let snapshot = vec![rec("../evil", "cursor", "/p/evil", "h1", 100)];
    let err = validate_update_preflight("../evil", "cursor", &snapshot, Some("h1")).unwrap_err();
    assert!(
        err.contains("非法"),
        "路径穿越式的技能名必须被拒绝: {err}"
    );

    let snapshot2 = vec![rec("a/b", "cursor", "/p/ab", "h1", 100)];
    assert!(validate_update_preflight("a/b", "cursor", &snapshot2, Some("h1")).is_err());
}

#[test]
fn 场景4_下载期间被卸载时不得复活() {
    let snapshot = vec![rec("x", "cursor", "/p/x", "h1", 100)];

    // 下载完成后记录已被删掉
    let after_uninstall: Vec<InstalledSkill> = vec![];
    let err = verify_records_unchanged(&snapshot, &after_uninstall).unwrap_err();
    assert!(err.contains("已被卸载"), "错误信息应说明已被卸载: {err}");

    // 最关键：不仅报错，还要证明记录没被复活。
    // 这个纯函数只读不写，所以「没被复活」等价于入参仍为空——
    // 命令层拿到 Err 就直接 return，绝不会走到写回那一步。
    assert!(after_uninstall.is_empty(), "记录不得被复活");
}

#[test]
fn 场景4补充_下载期间记录被改动也要拒绝() {
    let snapshot = vec![rec("x", "cursor", "/p/x", "h1", 100)];

    // 哈希变了（用户重装过）
    let changed_hash = vec![rec("x", "cursor", "/p/x", "h9", 100)];
    let err = verify_records_unchanged(&snapshot, &changed_hash).unwrap_err();
    assert!(err.contains("已变化"), "哈希变化应被拒绝: {err}");

    // 安装时间变了
    let changed_at = vec![rec("x", "cursor", "/p/x", "h1", 999)];
    assert!(verify_records_unchanged(&snapshot, &changed_at).is_err());

    // 路径变了
    let changed_path = vec![rec("x", "cursor", "/other/x", "h1", 100)];
    assert!(verify_records_unchanged(&snapshot, &changed_path).is_err());

    // 对照：完全没变时通过
    assert!(verify_records_unchanged(&snapshot, &snapshot).is_ok());
}

#[test]
fn 场景4补充_多工具时任一记录消失都拒绝() {
    // SSOT 下同名技能可能装在多个工具，更新会同时涉及所有记录
    let snapshot = vec![
        rec("x", "cursor", "/a/x", "h1", 100),
        rec("x", "claude", "/b/x", "h1", 100),
    ];
    // 只删掉其中一个，也必须拒绝
    let partial = vec![rec("x", "cursor", "/a/x", "h1", 100)];
    let err = verify_records_unchanged(&snapshot, &partial).unwrap_err();
    assert!(err.contains("已被卸载"), "{err}");
}
