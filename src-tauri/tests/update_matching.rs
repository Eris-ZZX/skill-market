//! 远端匹配：同名技能跨分类不能互相错配
//!
//! 远端索引按 `repo_path` 平铺，同一个 name 可以出现在多个分类下
//! （`skills/通用/x` 与 `skills/研发/x`）。匹配必须同时比对 name 与 repo_path，
//! 只按 name 匹配会让两个分类的技能互相串掉哈希。

use skill_market_lib::commands::{find_remote, RemoteSkillView};

fn view(name: &str, repo_path: &str, hash: &str) -> RemoteSkillView {
    RemoteSkillView {
        name: name.to_string(),
        repo_path: repo_path.to_string(),
        hash: hash.to_string(),
        version: None,
        description: String::new(),
        tags: Vec::new(),
        category: None,
        last_commit_at: None,
    }
}

#[test]
fn 同名不同路径各自匹配不串() {
    let remote = vec![
        view("grill-me", "skills/通用/grill-me", "hash-tongyong"),
        view("grill-me", "skills/研发/grill-me", "hash-yanfa"),
    ];

    let a = find_remote("grill-me", Some("skills/通用/grill-me"), &remote).unwrap();
    assert_eq!(a.hash, "hash-tongyong");
    assert_eq!(a.repo_path, "skills/通用/grill-me");

    let b = find_remote("grill-me", Some("skills/研发/grill-me"), &remote).unwrap();
    assert_eq!(b.hash, "hash-yanfa");
    assert_eq!(b.repo_path, "skills/研发/grill-me");
}

#[test]
fn 本地导入无路径返回_none() {
    let remote = vec![view("grill-me", "skills/通用/grill-me", "h")];
    // repo_path 为 None 表示本地导入，没有远端对应物
    assert!(find_remote("grill-me", None, &remote).is_none());
}

#[test]
fn 远端无同名返回_none() {
    let remote = vec![view("other", "skills/通用/other", "h")];
    assert!(find_remote("grill-me", Some("skills/通用/grill-me"), &remote).is_none());
}

#[test]
fn 名称与路径都匹配返回正确那条() {
    let remote = vec![
        view("grill-me", "skills/通用/grill-me", "h1"),
        view("grill-me", "skills/研发/grill-me", "h2"),
        view("sales-report", "skills/通用/sales-report", "h3"),
    ];
    let hit = find_remote("sales-report", Some("skills/通用/sales-report"), &remote).unwrap();
    assert_eq!(hit.hash, "h3");
}

#[test]
fn 同名同路径但远端只有一条时也能命中() {
    let remote = vec![view("x", "skills/通用/x", "only")];
    let hit = find_remote("x", Some("skills/通用/x"), &remote).unwrap();
    assert_eq!(hit.hash, "only");
}
