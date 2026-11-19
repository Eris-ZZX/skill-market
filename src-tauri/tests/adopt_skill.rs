//! 纳入管理：写出的记录不得关联任何远端
//!
//! 产品已确认方案 B：扫描到的技能来源未知，猜一个来源比留空更糟——
//! 猜错会让更新功能去覆盖一个其实来自别处的技能。

use skill_market_lib::commands::adopt_record;
use skill_market_lib::state::{InstallMode, SourceKind};

fn skill_dir(md: &str) -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let t = d.path().join("skills").join("my-skill");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("SKILL.md"), md).unwrap();
    let path = t.to_string_lossy().to_string();
    (d, path)
}

#[test]
fn 纳入管理后记录里没有任何远端字段() {
    let (_guard, path) = skill_dir(
        "---\nname: my-skill\ndescription: 本地技能\nversion: 2.1.0\n---\nbody\n",
    );

    let rec = adopt_record("my-skill".into(), "claude".into(), path.clone()).unwrap();

    assert_eq!(rec.name, "my-skill");
    assert_eq!(rec.tool, "claude");
    assert_eq!(rec.target_path, path);
    assert_eq!(rec.installed_version.as_deref(), Some("2.1.0"));
    assert_eq!(rec.installed_hash.len(), 64, "sha256 应为 64 个十六进制字符");
    assert!(matches!(rec.mode, InstallMode::Copy));

    // 关键断言：四个远端字段必须全部为空
    assert!(matches!(rec.source.kind, SourceKind::Local));
    assert!(rec.source.repo_url.is_none());
    assert!(rec.source.repo_path.is_none());
    assert!(rec.source.branch.is_none());
    assert!(rec.source.commit.is_none());
}

#[test]
fn 纳入管理对非技能目录报错() {
    let d = tempfile::tempdir().unwrap();
    let empty = d.path().join("not-a-skill");
    std::fs::create_dir_all(&empty).unwrap();
    let err = adopt_record(
        "x".into(),
        "claude".into(),
        empty.to_string_lossy().to_string(),
    )
    .unwrap_err();
    assert!(err.contains("SKILL.md"), "错误信息应说明缺 SKILL.md");
}

#[test]
fn 纳管时记录的指纹与之后比对一致() {
    // 纳管时写的 installed_hash 必须等于之后 local_hash 算出的值，
    // 否则纳入后立刻会显示「已修改」
    let (_guard, path) = skill_dir("---\nname: my-skill\n---\nbody\n");
    let rec = adopt_record("my-skill".into(), "claude".into(), path.clone()).unwrap();

    let tool = skill_market_lib::tools::ToolKind::Claude;
    let later = skill_market_lib::install::local_hash(tool, &path).unwrap();
    assert_eq!(rec.installed_hash, later);
}
