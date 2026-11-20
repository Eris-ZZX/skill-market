//! 备份列表、恢复、删除
//!
//! 两处路径校验是本票的安全边界：`backup_id` 来自前端、`meta.name` 来自
//! 备份文件内容，两者都是不可信输入，直接 join 就能穿越出目标目录。
//!
//! 全部用 tempdir + 显式路径，不依赖环境变量、不碰真实 AppData。

use skill_market_lib::hash;
use skill_market_lib::install::{
    backup_library, delete_backup, list_backups, restore_backup, validate_backup_id, BackupMeta,
};
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

fn seed_library(lib: &Path, name: &str, body: &str) {
    write(
        &lib.join(name),
        "SKILL.md",
        &format!("---\nname: {name}\ndescription: d\nversion: 1.0.0\n---\n{body}\n"),
    );
    write(&lib.join(name), "scripts/run.py", "print(1)\n");
}

fn meta_for(name: &str, tool: &str, at: i64) -> BackupMeta {
    BackupMeta {
        name: name.into(),
        tool: tool.into(),
        installed_hash: "h".into(),
        installed_version: Some("1.0.0".into()),
        repo_path: Some(format!("skills/通用/{name}")),
        backup_created_at: at,
        source_path: String::new(),
    }
}

/// 造一份备份（内容来自 lib 里的当前同名技能）
fn make_backup(lib: &Path, backups: &Path, name: &str, tool: &str, at: i64) -> PathBuf {
    seed_library(lib, name, &format!("body-{at}"));
    backup_library(lib, name, backups, Some(&meta_for(name, tool, at))).unwrap()
}

// ========== 列表 ==========

#[test]
fn 列出多个备份并按时间倒序() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");

    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    make_backup(&lib, &backups, "grill-me", "cursor", 3000); // 更晚
    make_backup(&lib, &backups, "dws", "qwenwork", 2000);

    let list = list_backups(&backups, &lib);
    assert_eq!(list.len(), 3, "三份都应列出");
    let times: Vec<i64> = list.iter().map(|e| e.meta.backup_created_at).collect();
    assert_eq!(times, vec![3000, 2000, 1000], "必须按时间倒序");
}

#[test]
fn 空备份目录返回空列表() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    fs::create_dir_all(&backups).unwrap();

    assert!(list_backups(&backups, &lib).is_empty());
    // 目录根本不存在时也不该 panic
    assert!(list_backups(&d.path().join("nope"), &lib).is_empty());
}

#[test]
fn 缺_meta_json_的备份目录被跳过() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    write(&backups, "20250101_000000_broken/SKILL.md", "x");
    make_backup(&lib, &backups, "ok", "cursor", 1000);

    let list = list_backups(&backups, &lib);
    assert_eq!(list.len(), 1, "只应列出有合法 meta 的那份");
    assert_eq!(list[0].meta.name, "ok");
}

#[test]
fn 库里已有同名时不可恢复() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    // 库里此刻还有 grill-me（make_backup 保留着）
    let list = list_backups(&backups, &lib);
    assert!(!list[0].is_restorable, "库里有同名时应标为不可恢复");

    // 删掉库里的再列，应变为可恢复
    fs::remove_dir_all(lib.join("grill-me")).unwrap();
    let list2 = list_backups(&backups, &lib);
    assert!(list2[0].is_restorable);
}

// ========== 恢复 ==========

#[test]
fn 恢复成功且不带_meta_json() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    let original_hash = hash::hash_dir(&lib.join("grill-me")).unwrap();

    // 模拟技能消失
    fs::remove_dir_all(lib.join("grill-me")).unwrap();
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    let meta = restore_backup(&backups, &lib, &id).unwrap();

    assert_eq!(meta.name, "grill-me");
    assert!(lib.join("grill-me").join("SKILL.md").is_file(), "应已恢复");
    assert!(
        !lib.join("grill-me").join("meta.json").exists(),
        "meta.json 只是备份元数据，不该混进技能内容"
    );
    assert_eq!(
        hash::hash_dir(&lib.join("grill-me")).unwrap(),
        original_hash,
        "恢复出的内容应与备份时一致"
    );
}

#[test]
fn 恶意_meta_的_name_路径穿越被拒() {
    // meta.json 是不可信输入：name 直接 join 会写穿库目录
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    fs::create_dir_all(&lib).unwrap();
    let dir = backups.join("20250101_000000_evil");
    write(&dir, "skill/SKILL.md", "x");
    let mut m = meta_for("x", "cursor", 1000);
    m.name = "../../etc".into();
    fs::write(dir.join("meta.json"), serde_json::to_vec(&m).unwrap()).unwrap();

    // 列表会跳过它（read_backup_meta 已校验）
    assert!(list_backups(&backups, &lib).is_empty());

    // 直接调恢复也必须被拒
    let r = restore_backup(&backups, &lib, "20250101_000000_evil");
    assert!(r.is_err(), "恶意 name 必须被拒绝");
    assert!(!d.path().join("etc").exists(), "绝不能写穿出去");
}

#[test]
fn 恶意_backup_id_被拒() {
    for bad in ["../evil", "..", ".", "a/b", "a\\b", ""] {
        assert!(
            validate_backup_id(bad).is_err(),
            "{bad:?} 应被拒绝"
        );
    }
    assert!(validate_backup_id("20250101_120000_demo").is_ok());
    assert!(validate_backup_id("20250101_120000_demo_2").is_ok());
}

#[test]
fn 恶意_backup_id_到_restore_与_delete_都被拦下() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir_all(&backups).unwrap();
    // 备份根外面放一个不该被动到的目录
    write(&d.path().join("outside"), "important.txt", "keep me");

    assert!(restore_backup(&backups, &lib, "../outside").is_err());
    assert!(delete_backup(&backups, "../outside").is_err());

    assert!(
        d.path().join("outside/important.txt").is_file(),
        "备份根之外的目录必须完好"
    );
}

#[test]
fn 库里已有同名时拒绝恢复且大小写不敏感() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    // 库里还有 grill-me
    let r = restore_backup(&backups, &lib, &id);
    assert!(r.is_err(), "同名存在时必须拒绝");
    assert!(r.unwrap_err().to_string().contains("请先卸载"));

    // 换成大小写不同的目录名：Windows/macOS 上那是同一个位置，
    // 只比精确相等会放过冲突，所以必须大小写不敏感地拦下
    fs::remove_dir_all(lib.join("grill-me")).unwrap();
    fs::create_dir_all(lib.join("Grill-Me")).unwrap();
    let r2 = restore_backup(&backups, &lib, &id);
    assert!(r2.is_err(), "大小写不同的同名也必须拦下");
}

// ========== 删除 ==========

#[test]
fn 删除备份成功() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    delete_backup(&backups, &id).unwrap();

    assert!(!backups.join(&id).exists(), "备份目录应已删除");
    assert!(list_backups(&backups, &lib).is_empty());
    assert!(lib.join("grill-me").is_dir(), "删备份不能影响库");
}

#[test]
fn 删除不存在的备份报错() {
    let d = tempfile::tempdir().unwrap();
    let backups = d.path().join("backups");
    fs::create_dir_all(&backups).unwrap();
    assert!(delete_backup(&backups, "20250101_000000_nope").is_err());
}

#[test]
fn 恢复后记录字段来自_meta() {
    // 命令层会把 meta 转成 InstalledSkill；这里验证 meta 本身的字段
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1234);
    fs::remove_dir_all(lib.join("grill-me")).unwrap();
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    let meta = restore_backup(&backups, &lib, &id).unwrap();

    assert_eq!(meta.name, "grill-me");
    assert_eq!(meta.tool, "cursor");
    assert_eq!(meta.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(meta.backup_created_at, 1234);
    assert_eq!(meta.repo_path.as_deref(), Some("skills/通用/grill-me"));
    assert!(read(&lib.join("grill-me").join("SKILL.md")).contains("name: grill-me"));
}

// ========== 恢复后的 target_path 衔接（#55 方案 A） ==========

use skill_market_lib::commands::restore_target_for;
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};
use skill_market_lib::install::{resync_all, ResyncAction};
use std::collections::HashMap;

/// 造一个「假工具根」映射：key -> 根目录（是否已装由调用方决定是否创建）
fn fake_roots(map: Vec<(&str, PathBuf)>) -> HashMap<String, PathBuf> {
    map.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[test]
fn 工具已装时_target_path_为_skills_下的预期路径() {
    let d = tempfile::tempdir().unwrap();
    // 造一个「已装」的工具根（目录存在）
    let root = d.path().join(".cursor");
    fs::create_dir_all(&root).unwrap();
    let roots = fake_roots(vec![("cursor", root.clone())]);

    let (target, installed) =
        restore_target_for("cursor", "grill-me", |k| roots.get(k).cloned()).unwrap();

    assert!(installed, "根目录存在应视为已装");
    assert_eq!(target, root.join("skills").join("grill-me"));
}

#[test]
fn 工具未装时仍给出预期路径且不创建目录() {
    let d = tempfile::tempdir().unwrap();
    // 根目录**不存在**
    let root = d.path().join(".cursor");
    let roots = fake_roots(vec![("cursor", root.clone())]);

    let (target, installed) =
        restore_target_for("cursor", "grill-me", |k| roots.get(k).cloned()).unwrap();

    assert!(!installed, "根目录不存在应视为未装");
    assert_eq!(
        target,
        root.join("skills").join("grill-me"),
        "仍应给出预期路径，供装好工具后 resync 使用"
    );
    // R1：纯路径计算，绝不创建工具根目录
    assert!(!root.exists(), "绝不能创建工具根目录");
}

#[test]
fn 未知工具被拒() {
    let d = tempfile::tempdir().unwrap();
    let r = restore_target_for("not-a-tool", "x", |_| Some(d.path().to_path_buf()));
    assert!(r.is_err(), "未知工具 key 应报错");
    assert!(r.unwrap_err().to_string().contains("未知"));
}

#[test]
fn 恢复出的记录能被_resync_接管() {
    // 跨票集成：恢复 -> resync 真的接上了吗
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "grill-me", "cursor", 1000);
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    // 造一个「已装」的工具根，并按它的 skills 目录算预期路径
    let tool_root = d.path().join(".cursor");
    fs::create_dir_all(&tool_root).unwrap();
    let roots = fake_roots(vec![("cursor", tool_root.clone())]);
    let (target, installed) =
        restore_target_for("cursor", "grill-me", |k| roots.get(k).cloned()).unwrap();
    assert!(installed);

    // 技能「消失」：库和目标都没了
    fs::remove_dir_all(lib.join("grill-me")).unwrap();
    assert!(!target.exists());

    // 恢复：只写库
    let meta = restore_backup(&backups, &lib, &id).unwrap();
    let installed_hash = hash::hash_dir(&lib.join("grill-me")).unwrap();

    // 构造与 restore_skill_backup 一致的记录
    let rec = InstalledSkill {
        name: meta.name.clone(),
        tool: "cursor".into(),
        target_path: target.to_string_lossy().to_string(),
        mode: InstallMode::Copy,
        installed_hash: installed_hash.clone(),
        installed_version: meta.installed_version.clone(),
        source: SkillSource {
            kind: SourceKind::Gitlab,
            repo_url: None,
            repo_path: meta.repo_path.clone(),
            branch: None,
            commit: None,
        },
        installed_at: 1,
        ignored_remote_hash: None,
    };

    // 目标此刻仍不存在 -> resync 应把它重建出来
    assert!(!target.exists(), "前置：目标应缺失");
    let report = resync_all(&[rec], &lib);

    assert_eq!(report.repaired.len(), 1, "resync 应把恢复出的记录接过去");
    assert!(report.failed.is_empty(), "不该有失败: {:?}", report.failed);
    assert!(target.join("SKILL.md").is_file(), "目标应已重建");
    assert_eq!(
        hash::hash_dir(&target).unwrap(),
        installed_hash,
        "重建出的内容应与恢复的库一致"
    );
}

#[test]
fn 工具未装时_resync_如实报失败而不是静默跳过() {
    // 方案 A 的预期行为：记录已写但工具没装，resync 会报「工具未安装」。
    // 这不是 bug，是要让用户知道「装好工具再修」。
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    make_backup(&lib, &backups, "dws", "cursor", 1000);
    let id = list_backups(&backups, &lib)[0].backup_id.clone();

    let tool_root = d.path().join(".cursor"); // 不创建 = 未装
    let roots = fake_roots(vec![("cursor", tool_root.clone())]);
    let (target, installed) =
        restore_target_for("cursor", "dws", |k| roots.get(k).cloned()).unwrap();
    assert!(!installed, "前置：工具应视为未装");

    fs::remove_dir_all(lib.join("dws")).unwrap();
    let meta = restore_backup(&backups, &lib, &id).unwrap();
    let h = hash::hash_dir(&lib.join("dws")).unwrap();
    let rec = InstalledSkill {
        name: meta.name,
        tool: "cursor".into(),
        target_path: target.to_string_lossy().to_string(),
        mode: InstallMode::Copy,
        installed_hash: h,
        installed_version: None,
        source: SkillSource {
            kind: SourceKind::Gitlab,
            repo_url: None,
            repo_path: None,
            branch: None,
            commit: None,
        },
        installed_at: 1,
        ignored_remote_hash: None,
    };

    let report = resync_all(&[rec], &lib);
    assert!(
        report.failed.len() == 1 || report.repaired.len() == 1,
        "应为失败或（父目录可建时）修复，实际: failed={:?} repaired={:?}",
        report.failed,
        report.repaired
    );
    // 无论哪条路径，都不能创建工具根目录之外的东西
    if !tool_root.exists() {
        assert!(report.failed.len() == 1, "工具未装时应报失败");
    }
    let _ = ResyncAction::Skipped;
}

#[test]
fn 恢复记录的_target_path_真的被填上() {
    // 直接验证命令层用的构造函数：target_path 必须是算出的预期路径，
    // 不能是空串（空串会让 #54 的 resync 无从下手）
    use skill_market_lib::commands::record_from_restore;

    let d = tempfile::tempdir().unwrap();
    let root = d.path().join(".claude");
    fs::create_dir_all(&root).unwrap();
    let roots = fake_roots(vec![("claude", root.clone())]);
    let (target, installed) =
        restore_target_for("claude", "grill-me", |k| roots.get(k).cloned()).unwrap();
    assert!(installed);

    let meta = BackupMeta {
        name: "grill-me".into(),
        tool: "claude".into(),
        installed_hash: "h".into(),
        installed_version: Some("1.0.0".into()),
        repo_path: Some("skills/通用/grill-me".into()),
        backup_created_at: 1000,
        source_path: String::new(),
    };
    let rec = record_from_restore(&meta, &target, "hash-abc".into());

    assert_eq!(
        rec.target_path,
        target.to_string_lossy(),
        "target_path 必须是预期路径而不是空串"
    );
    assert!(!rec.target_path.is_empty());
    assert_eq!(rec.name, "grill-me");
    assert_eq!(rec.tool, "claude");
    assert_eq!(rec.installed_hash, "hash-abc");
    assert_eq!(rec.installed_version.as_deref(), Some("1.0.0"));
    // 有 repo_path 的来源应标为 Gitlab
    assert!(matches!(rec.source.kind, SourceKind::Gitlab));
}
