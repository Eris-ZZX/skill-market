//! 「我的技能」的身份判定与聚合
//!
//! 三种身份决定界面能对这条技能做什么：managed 可判更新，external 不许动，
//! local 可以「纳入管理」。判错会导致我们修改别人的软链目标。

use skill_market_lib::commands::{build_my_skills, judge_identity, MySkillItem};
use skill_market_lib::scan::ScannedSkill;
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};

const APP_DATA: &str = "/home/u/.config/skill-market";

fn rec(name: &str, tool: &str, target: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: target.into(),
        mode: InstallMode::Copy,
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
    }
}


/// 构造一个落盘位置
fn spot(tool: &str, path: &str, is_link: bool, link: Option<&str>) -> skill_market_lib::scan::SkillPlacement {
    skill_market_lib::scan::SkillPlacement {
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

#[test]
fn managed_优先于其它身份() {
    // 有记录且路径一致 → managed，哪怕它是软链
    let id = judge_identity(
        "/home/u/.claude/skills/x",
        "claude",
        true,
        Some("/somewhere/else/x"),
        &[rec("x", "claude", "/home/u/.claude/skills/x")],
        Some(APP_DATA),
    );
    assert_eq!(id, "managed");
}

#[test]
fn 记录存在但路径不一致不算_managed() {
    // 同名同工具但 target_path 不同：旧记录已失效，不能据此认定由本工具管理
    let id = judge_identity(
        "/home/u/.claude/skills/x",
        "claude",
        false,
        None,
        &[rec("x", "claude", "/home/u/.claude/skills/old-x")],
        Some(APP_DATA),
    );
    assert_eq!(id, "local");
}

#[test]
fn 指向应用外部的软链是_external() {
    let id = judge_identity(
        "/home/u/.claude/skills/x",
        "claude",
        true,
        Some("/home/u/.cc-switch/skills/x"),
        &[],
        Some(APP_DATA),
    );
    assert_eq!(id, "external");
}

#[test]
fn 指向应用自身数据目录的软链是本工具_junction() {
    // library 下的 junction 是本客户端自己建的，应算 local 而不是 external
    let id = judge_identity(
        "/home/u/.claude/skills/x",
        "claude",
        true,
        Some("/home/u/.config/skill-market/library/x"),
        &[],
        Some(APP_DATA),
    );
    assert_eq!(id, "local");
}

#[test]
fn 普通目录无记录是_local() {
    let id = judge_identity(
        "/home/u/.agents/skills/x",
        "agents",
        false,
        None,
        &[],
        Some(APP_DATA),
    );
    assert_eq!(id, "local");
}

#[test]
fn 软链但读不到目标是_external() {
    // 读不到链接目标时保守处理：不动它是安全的默认
    let id = judge_identity(
        "/home/u/.agents/skills/x",
        "agents",
        true,
        None,
        &[],
        Some(APP_DATA),
    );
    assert_eq!(id, "external");
}

#[test]
fn 按目录聚合后取最高优先级身份() {
    // scan_dirs 已按 dir_name 聚合：一个技能一条，placements 里是全部落盘。
    // 这里直接构造聚合后的形态，验证身份仍取最高优先级。
    let mut item = scanned("grill-me", "agents", "/a/grill-me", false, None);
    item.placements = vec![
        spot("agents", "/a/grill-me", false, None),
        // claude 那处是外部软链
        spot("claude", "/c/grill-me", true, Some("/x/grill-me")),
    ];
    item.found_in = vec!["agents".into(), "claude".into()];

    let got = build_my_skills(&[item], &[], Some(APP_DATA));
    assert_eq!(got.len(), 1, "聚合后一个技能一条");
    assert_eq!(got[0].tools.len(), 2);
    // /c 是非本应用的软链目标 → external 高于 /a 的 local
    assert_eq!(got[0].identity, "external", "external 高于 local");
}

#[test]
fn managed_聚合后带出已记录版本() {
    let mut r = rec("grill-me", "claude", "/c/grill-me");
    r.installed_version = Some("0.9.0".into());
    // 聚合后的形态：一条记录、两个落盘
    let mut item = scanned("grill-me", "claude", "/c/grill-me", false, None);
    item.placements = vec![
        spot("claude", "/c/grill-me", false, None),
        spot("agents", "/a/grill-me", false, None),
    ];
    item.found_in = vec!["claude".into(), "agents".into()];

    let got = build_my_skills(&[item], &[r], Some(APP_DATA));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].identity, "managed");
    assert_eq!(got[0].installed_version.as_deref(), Some("0.9.0"));
    // 扫描本身不做远端比对，状态留空由 check_updates 填——
    // 在这里编一个 UpToDate 等于撒谎
    assert!(
        got[0].status.is_none(),
        "扫描不产生状态，避免未检查就显示「最新」"
    );
}

#[test]
fn 非_managed_不给状态() {
    // 没有来源就判不了，不猜
    let list = vec![scanned("x", "cursor", "/c/x", false, None)];
    let got = build_my_skills(&list, &[], Some(APP_DATA));
    let item: &MySkillItem = &got[0];
    assert_eq!(item.identity, "local");
    assert!(item.status.is_none(), "非 managed 不应有状态");
}
