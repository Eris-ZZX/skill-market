//! 冲突处理：备份元数据、路径穿越防护、另存一份的命名、忽略标记
//!
//! 核心风险是「覆盖用户改动」与「从不可信 meta.json 拼路径」，两条都必须
//! 用测试钉住。

use skill_market_lib::install::{
    backup_library, read_backup_meta, restore_backup_as, unique_sibling_name, BackupMeta,
};
use skill_market_lib::state::{ConflictChoice, InstalledSkill, SkillSource, SourceKind};
use std::fs;
use std::path::Path;

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn skill(root: &Path, name: &str, body: &str) {
    write(
        &root.join(name),
        "SKILL.md",
        &format!("---\nname: {name}\n---\n{body}\n"),
    );
}

fn meta(name: &str) -> BackupMeta {
    BackupMeta {
        name: name.to_string(),
        tool: "claude".to_string(),
        installed_hash: "h1".to_string(),
        installed_version: Some("1.0.0".to_string()),
        repo_path: Some(format!("skills/通用/{name}")),
        backup_created_at: 1795000000,
        source_path: String::new(),
    }
}

#[test]
fn 备份写出可读的_meta_json() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    skill(&lib, "demo", "原始内容");

    let bak = backup_library(&lib, "demo", &backups, Some(&meta("demo"))).unwrap();

    // 内容与元数据都在
    assert!(bak.join("SKILL.md").is_file());
    let m = read_backup_meta(&bak).unwrap();
    assert_eq!(m.name, "demo");
    assert_eq!(m.tool, "claude");
    assert_eq!(m.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(m.repo_path.as_deref(), Some("skills/通用/demo"));
    assert!(
        m.source_path.contains("backups"),
        "source_path 应指向备份自身，实际: {}",
        m.source_path
    );
}

#[test]
fn 恶意_meta_里的路径穿越被拒绝() {
    // meta.json 是不可信输入：直接 join 会越过库目录写任意位置
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    fs::create_dir_all(&lib).unwrap();
    let bak = d.path().join("backups").join("20250101_000000_evil");
    write(&bak, "SKILL.md", "内容");

    for evil in ["../../etc", "/etc", "a/b", "a\\b", ".."] {
        let mut m = meta("x");
        m.name = evil.to_string();
        fs::write(bak.join("meta.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(
            read_backup_meta(&bak).is_err(),
            "恶意目录名 {evil:?} 必须被拒绝"
        );
    }
}

#[test]
fn 恢复备份到新名字且不带_meta_json() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    skill(&lib, "demo", "我改过的内容");
    let bak = backup_library(&lib, "demo", &backups, Some(&meta("demo"))).unwrap();

    let restored = restore_backup_as(&bak, &lib, "demo-local").unwrap();

    assert!(restored.join("SKILL.md").is_file());
    assert!(
        !restored.join("meta.json").exists(),
        "meta.json 是元数据，不该混进技能内容"
    );
    assert!(
        fs::read_to_string(restored.join("SKILL.md")).unwrap().contains("我改过的内容"),
        "另存的应是用户改动后的内容"
    );
}

#[test]
fn 恢复备份到已存在目标时拒绝覆盖() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    skill(&lib, "demo", "新版");
    let bak = backup_library(&lib, "demo", &backups, None).unwrap();

    // demo 已存在，恢复到同名必须失败而不是覆盖
    let r = restore_backup_as(&bak, &lib, "demo");
    assert!(r.is_err(), "目标已存在时必须拒绝，绝不静默覆盖");
    assert!(fs::read_to_string(lib.join("demo").join("SKILL.md"))
        .unwrap()
        .contains("新版"));
}

#[test]
fn 另存一份重名时依次加序号() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    fs::create_dir_all(&lib).unwrap();

    // demo-local 没被占
    assert_eq!(unique_sibling_name(&lib, "demo-local").unwrap(), "demo-local");

    // 占掉后应退到 -2
    skill(&lib, "demo-local", "x");
    assert_eq!(unique_sibling_name(&lib, "demo-local").unwrap(), "demo-local-2");

    // 再占掉 -2 应退到 -3
    skill(&lib, "demo-local-2", "x");
    assert_eq!(unique_sibling_name(&lib, "demo-local").unwrap(), "demo-local-3");
}

#[test]
fn 另存一份序号用尽时报错不覆盖() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    fs::create_dir_all(&lib).unwrap();
    skill(&lib, "demo-local", "x");
    for n in 2..=10 {
        skill(&lib, &format!("demo-local-{n}"), "x");
    }
    let r = unique_sibling_name(&lib, "demo-local");
    assert!(r.is_err(), "试到 -10 仍重名必须报错，不能覆盖任何一份");
}

#[test]
fn 备份保留数量上限会删最旧() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    skill(&lib, "demo", "v1");

    // 先塞满超过上限的历史备份，再正常备份一次触发清理
    for i in 0..25 {
        write(
            &backups.join(format!("202501{:02}_120000_demo", i + 1)),
            "SKILL.md",
            &format!("b{i}"),
        );
    }
    backup_library(&lib, "demo", &backups, None).unwrap();

    let mut names: Vec<String> = fs::read_dir(&backups)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names.len(),
        skill_market_lib::install::BACKUP_RETAIN_COUNT,
        "应恰好裁到上限，实际 {} 份",
        names.len()
    );
    // 时间戳前缀的字典序即时间序：留下的必须是最新的那一批
    assert!(
        !names.contains(&"20250101_120000_demo".to_string()),
        "最旧的那份应被删掉"
    );
}

#[test]
fn 另存的副本要改掉_frontmatter_name() {
    // 「我的技能」按 name 聚合。不改 name 的话另存出来的副本会和正本
    // 合并成一行，用户看不到两份技能。
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    let backups = d.path().join("backups");
    skill(&lib, "demo", "我改过的内容");
    let bak = backup_library(&lib, "demo", &backups, Some(&meta("demo"))).unwrap();

    restore_backup_as(&bak, &lib, "demo-local").unwrap();

    let md = fs::read_to_string(lib.join("demo-local").join("SKILL.md")).unwrap();
    assert!(md.contains("name: demo-local"), "副本的 name 应被改写: {md}");
    assert!(md.contains("我改过的内容"), "内容不能被改坏");

    // 正本不受影响
    let orig = fs::read_to_string(lib.join("demo").join("SKILL.md")).unwrap();
    assert!(orig.contains("name: demo"), "正本的 name 不该动: {orig}");
}

#[test]
fn 另存副本只有单层_name_时也能处理() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    // 无 frontmatter 的技能：改名失败但内容必须保住
    write(&d.path().join("bk"), "SKILL.md", "# 没有 frontmatter\n");
    let bak = d.path().join("bk");
    let _ = restore_backup_as(&bak, &lib, "plain-copy").unwrap();
    assert!(
        fs::read_to_string(lib.join("plain-copy").join("SKILL.md"))
            .unwrap()
            .contains("没有 frontmatter"),
        "改名失败不能导致内容丢失"
    );
}

#[test]
fn 忽略标记只对同一个远端哈希有效() {
    // 语义：ignored 只在 remote_hash == 记录值时成立。远端再变即失效，
    // 不需要额外清理逻辑——这正是「按哈希相等比较」而不是「有没有标记」的好处。
    let same = ("abc".to_string(), Some("abc".to_string()));
    let changed = ("abc".to_string(), Some("def".to_string()));
    let no_remote = ("abc".to_string(), None);

    let ignored = |(seen, now): &(String, Option<String>)| match (Some(seen), now) {
        (Some(seen), Some(now)) => seen == now,
        _ => false,
    };
    assert!(ignored(&same), "远端没变时应保持忽略");
    assert!(!ignored(&changed), "远端再变时必须重新提示");
    assert!(!ignored(&no_remote), "远端取不到时不当作已忽略");
}

#[test]
fn 老_state_json_无忽略字段仍能反序列化() {
    // 老数据没有 ignoredRemoteHash，加了 serde(default) 才不会破
    let old = r#"{
        "name": "demo",
        "tool": "claude",
        "targetPath": "/home/u/.claude/skills/demo",
        "mode": "copy",
        "installedHash": "h1",
        "installedVersion": "1.0.0",
        "source": {"kind":"gitlab","repoUrl":null,"repoPath":"skills/x/demo","branch":null,"commit":null},
        "installedAt": 1700000000
    }"#;
    let rec: InstalledSkill = serde_json::from_str(old).unwrap();
    assert_eq!(rec.name, "demo");
    assert!(rec.ignored_remote_hash.is_none(), "老数据应落为 None");

    // 新数据能往返
    let mut fresh = rec.clone();
    fresh.ignored_remote_hash = Some("remote-abc".into());
    let s = serde_json::to_string(&fresh).unwrap();
    let back: InstalledSkill = serde_json::from_str(&s).unwrap();
    assert_eq!(back.ignored_remote_hash.as_deref(), Some("remote-abc"));
}

#[test]
fn 三个冲突选项都能序列化往返() {
    for c in [
        ConflictChoice::KeepMine,
        ConflictChoice::UseRemote,
        ConflictChoice::KeepBoth,
    ] {
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<ConflictChoice>(&s).unwrap(), c);
    }
    // 前端传的是小写字面量
    assert_eq!(
        serde_json::from_str::<ConflictChoice>("\"keepmine\"").unwrap(),
        ConflictChoice::KeepMine
    );
    assert_eq!(
        serde_json::from_str::<ConflictChoice>("\"useremote\"").unwrap(),
        ConflictChoice::UseRemote
    );
    assert_eq!(
        serde_json::from_str::<ConflictChoice>("\"keepboth\"").unwrap(),
        ConflictChoice::KeepBoth
    );
}

#[test]
fn 备份失败不破坏原库() {
    // 备份目录不可写时应当报错，但库里内容必须原样
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("library");
    skill(&lib, "demo", "原始");

    // 用一个普通文件冒充 backups 根，create_dir_all 必然失败
    let bad = d.path().join("backups");
    fs::write(&bad, "not a dir").unwrap();

    let r = backup_library(&lib, "demo", &bad, None);
    assert!(r.is_err(), "备份失败应如实报错");
    assert!(
        fs::read_to_string(lib.join("demo").join("SKILL.md"))
            .unwrap()
            .contains("原始"),
        "备份失败不能动到原库内容"
    );
}

#[test]
fn 本地导入技能的_source_无_repo_path_可用于判定不可更新() {
    // resolve_conflict 对这类记录直接报错，这里只验证数据形态可识别
    let rec = InstalledSkill {
        name: "local-import".into(),
        tool: "claude".into(),
        target_path: "/tmp/x".into(),
        mode: skill_market_lib::state::InstallMode::Copy,
        installed_hash: "h".into(),
        installed_version: None,
        source: SkillSource {
            kind: SourceKind::Local,
            repo_url: None,
            repo_path: None,
            branch: None,
            commit: None,
        },
        installed_at: 0,
        ignored_remote_hash: None,
    };
    assert!(rec.source.repo_path.is_none());
}
