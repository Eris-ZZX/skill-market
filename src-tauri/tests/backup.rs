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
