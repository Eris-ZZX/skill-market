//! 「我的技能」的聚合与外部管理标记
//!
//! #66 起不再分「本工具管理 / 外部管理 / 本地已有」三类——所有扫到的技能
//! 一律视为已安装。保留下来唯一条语义是「外部管理」：cc-switch 用软链把
//! 同一份内容链进多个工具目录，本客户端一卸载就动了别人的东西。
//!
//! 另一个新增字段是 `has_remote_source`：无来源时 `check_updates` 返回
//! UpToDate，那是「没得比」而不是「最新」，界面必须据此区分。

use skill_market_lib::commands::{build_my_skills, is_externally_managed, MySkillItem};
use skill_market_lib::scan::{ScannedSkill, SkillPlacement};
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};

const APP_DATA: &str = "/home/u/.config/skill-market";

fn rec_with_source(
    name: &str,
    tool: &str,
    target: &str,
    repo_path: Option<&str>,
) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: target.into(),
        mode: InstallMode::Copy,
        installed_hash: "h".into(),
        installed_version: None,
        source: SkillSource {
            kind: if repo_path.is_some() {
                SourceKind::Gitlab
            } else {
                SourceKind::Local
            },
            repo_url: None,
            repo_path: repo_path.map(|s| s.to_string()),
            branch: None,
            commit: None,
        },
        installed_at: 0,
        ignored_remote_hash: None,
    }
}

fn rec(name: &str, tool: &str, target: &str) -> InstalledSkill {
    rec_with_source(name, tool, target, None)
}

fn spot(tool: &str, path: &str, is_link: bool, link: Option<&str>) -> SkillPlacement {
    SkillPlacement {
        tool: tool.into(),
        target_path: path.into(),
        is_link,
        link_target: link.map(|s| s.to_string()),
    }
}

fn scanned(name: &str, tool: &str, target: &str, is_link: bool, link: Option<&str>) -> ScannedSkill {
    ScannedSkill {
        name: name.into(),
        dir_name: name.into(),
        tool: tool.into(),
        target_path: target.into(),
        found_in: vec![tool.into()],
        placements: vec![spot(tool, target, is_link, link)],
        description: format!("{name} 的描述"),
        version: Some("1.0.0".into()),
        tags: Vec::new(),
        category: None,
        is_link,
        link_target: link.map(|s| s.to_string()),
    }
}

/// 一条技能装在两处的聚合形态
fn two_placements(name: &str, a: SkillPlacement, b: SkillPlacement) -> ScannedSkill {
    let mut s = scanned(
        name,
        &a.tool,
        &a.target_path,
        a.is_link,
        a.link_target.as_deref(),
    );
    s.placements = vec![a.clone(), b.clone()];
    s.found_in = vec![a.tool.clone(), b.tool.clone()];
    s
}

// ========== is_externally_managed 的分支 ==========

#[test]
fn 不是链接不算外部管理() {
    assert!(!is_externally_managed(false, None, Some(APP_DATA)));
    // 即使带了链接目标，只要 is_link 为 false 就不算
    assert!(!is_externally_managed(false, Some("/x"), Some(APP_DATA)));
}

#[test]
fn 指向应用自身数据目录的软链不算外部管理() {
    // library 下的 junction 是本客户端自己建的
    assert!(!is_externally_managed(
        true,
        Some("/home/u/.config/skill-market/library/x"),
        Some(APP_DATA),
    ));
}

#[test]
fn 指向别处的软链算外部管理() {
    assert!(is_externally_managed(
        true,
        Some("/home/u/.cc-switch/skills/x"),
        Some(APP_DATA),
    ));
}

#[test]
fn 软链但读不到目标时保守算外部管理() {
    // 看不见的东西不该动
    assert!(is_externally_managed(true, None, Some(APP_DATA)));
}

#[test]
fn 不知道应用目录在哪时保守算外部管理() {
    assert!(is_externally_managed(true, Some("/x"), None));
}

// ========== 聚合：外部管理标记 ==========

#[test]
fn 多工具混合时任一外部管理即标记为外部管理() {
    let item = two_placements(
        "grill-me",
        // 普通目录
        spot("agents", "/a/grill-me", false, None),
        // 外部软链
        spot("claude", "/c/grill-me", true, Some("/x/grill-me")),
    );
    let got = build_my_skills(&[item], &[], Some(APP_DATA));
    assert_eq!(got.len(), 1);
    assert!(
        got[0].externally_managed,
        "任一落盘是外部软链就该标记"
    );
}

#[test]
fn 全普通目录不算外部管理() {
    let item = two_placements(
        "x",
        spot("agents", "/a/x", false, None),
        spot("claude", "/c/x", false, None),
    );
    let got = build_my_skills(&[item], &[], Some(APP_DATA));
    assert!(!got[0].externally_managed);
}

#[test]
fn 本客户端有记录的软链不算外部管理() {
    // 即使是软链，只要本客户端记录过这个落点，就归自己管——
    // 那是用户自己选的部署方式，不该被当成「别人的」拒绝卸载
    let item = scanned(
        "x",
        "claude",
        "/c/x",
        true,
        Some("/mnt/shared/skills/x"),
    );
    let r = rec("x", "claude", "/c/x");
    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert!(
        !got[0].externally_managed,
        "有记录的软链归本客户端管，可卸载"
    );
}

// ========== 聚合：has_remote_source ==========

#[test]
fn 无_repo_path_时没有远端来源() {
    let item = scanned("x", "claude", "/c/x", false, None);
    let r = rec("x", "claude", "/c/x"); // repo_path 为 None
    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert!(!got[0].has_remote_source);
}

#[test]
fn 有_repo_path_时有远端来源() {
    let item = scanned("x", "claude", "/c/x", false, None);
    let r = rec_with_source("x", "claude", "/c/x", Some("skills/通用/x"));
    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert!(got[0].has_remote_source);
}

#[test]
fn 完全没有记录时没有远端来源() {
    let item = scanned("x", "claude", "/c/x", false, None);
    let got = build_my_skills(&[item], &[], Some(APP_DATA));
    assert!(!got[0].has_remote_source);
}

#[test]
fn 多工具任一有来源即算有来源() {
    let item = two_placements(
        "x",
        spot("claude", "/c/x", false, None),
        spot("agents", "/a/x", false, None),
    );
    // 只有 agents 那处有来源
    let r = rec_with_source("x", "agents", "/a/x", Some("skills/通用/x"));
    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert!(got[0].has_remote_source, "任一落盘有来源就算有来源");
}

// ========== identity 字段恒为 installed ==========

#[test]
fn identity_恒为_installed() {
    for (label, item) in [
        ("普通目录", scanned("a", "claude", "/c/a", false, None)),
        (
            "外部软链",
            scanned("b", "claude", "/c/b", true, Some("/x/b")),
        ),
    ] {
        let got = build_my_skills(&[item], &[], Some(APP_DATA));
        assert_eq!(got[0].identity, "installed", "{label} 的 identity 应统一");
    }
}

// ========== 其它既有性质不受影响 ==========

#[test]
fn 聚合后带出已记录版本() {
    let mut r = rec("grill-me", "claude", "/c/grill-me");
    r.installed_version = Some("0.9.0".into());
    let item = two_placements(
        "grill-me",
        spot("claude", "/c/grill-me", false, None),
        spot("agents", "/a/grill-me", false, None),
    );
    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].installed_version.as_deref(), Some("0.9.0"));
    // 扫描本身不做远端比对，状态留空由 check_updates 填
    assert!(
        got[0].status.is_none(),
        "扫描不产生状态，避免未检查就显示「最新」"
    );
}

#[test]
fn 扫描结果保持一个技能一条() {
    let list = vec![
        scanned("a", "cursor", "/c/a", false, None),
        scanned("b", "cursor", "/c/b", false, None),
    ];
    let got = build_my_skills(&list, &[], Some(APP_DATA));
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|i: &MySkillItem| i.identity == "installed"));
}
