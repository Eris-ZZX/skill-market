//! 发现结果的成功/失败区分
//!
//! 远端读不到时不能整体报错——那样界面上连上次的数据都看不到。
//! 失败信息放在 `failures` 里带回去，已缓存的结果照常返回。
//!
//! 这些是纯函数测试：不碰网络、不碰 Tauri（#30 的教训：mock_app 在
//! Windows 上会让测试二进制起不来）。

use skill_market_lib::commands::{
    assemble_update_result, classify_remote_failure, RemoteSkillView, UpdateCheckResult,
};
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind, UpdateStatus};

fn rec(name: &str, tool: &str, hash: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: tool.into(),
        target_path: format!("/p/{name}"),
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
        installed_at: 100,
        ignored_remote_hash: None,
    }
}

fn view(name: &str, repo_path: &str, hash: &str) -> RemoteSkillView {
    RemoteSkillView {
        name: name.into(),
        repo_path: repo_path.into(),
        hash: hash.into(),
        version: Some("1.0.0".into()),
        description: String::new(),
        tags: Vec::new(),
        category: None,
        last_commit_at: None,
    }
}

/// 固定返回「与安装时相同」的本地哈希，模拟本地没改过
fn unchanged(_: &InstalledSkill) -> Option<String> {
    Some("h1".to_string())
}

// ========== 错误分类 ==========

#[test]
fn 分类_认证失败() {
    for detail in [
        "取 head commit 失败（HTTP 401 Unauthorized）",
        "HTTP 403 Forbidden",
        "unauthorized",
        "访问令牌无效",
    ] {
        assert_eq!(
            classify_remote_failure(detail).kind,
            "auth",
            "{detail} 应归为 auth"
        );
    }
    let f = classify_remote_failure("HTTP 401 Unauthorized");
    assert!(!f.message.is_empty(), "应有中文提示");
    assert_eq!(f.detail, "HTTP 401 Unauthorized", "detail 应保留原始信息");
}

#[test]
fn 分类_仓库不存在() {
    for detail in ["HTTP 404", "404 Not Found", "仓库不存在"] {
        assert_eq!(
            classify_remote_failure(detail).kind,
            "notfound",
            "{detail} 应归为 notfound"
        );
    }
}

#[test]
fn 分类_网络问题() {
    for detail in [
        "request timed out",
        "connect error: connection refused",
        "dns error",
        "下载超时（60 秒）",
    ] {
        assert_eq!(
            classify_remote_failure(detail).kind,
            "network",
            "{detail} 应归为 network"
        );
    }
}

#[test]
fn 分类_其余归为解析失败() {
    for detail in ["解析出 0 个技能", "unexpected end of file", "某种没见过的错误"] {
        assert_eq!(
            classify_remote_failure(detail).kind,
            "parse",
            "{detail} 应归为 parse"
        );
    }
}

#[test]
fn 分类_按优先级判定_401_优先于超时字样() {
    // 同一句里既有 401 又有 timeout 时，认证问题更具体，优先报它
    let f = classify_remote_failure("HTTP 401 (previous attempt: timeout)");
    assert_eq!(f.kind, "auth");
}

// ========== 两条路径 ==========

#[test]
fn 成功时_failures_为空且用远端数据() {
    let snap = vec![rec("x", "claude", "h1")];
    let remote = vec![view("x", "skills/通用/x", "h2")];

    let r: UpdateCheckResult = assemble_update_result(&snap, Some(&remote), &[], None, unchanged);

    assert!(r.failures.is_empty(), "成功时不该有 failures");
    assert_eq!(r.items.len(), 1);
    assert_eq!(
        r.items[0].status,
        UpdateStatus::UpdateAvailable,
        "远端哈希变了应判为可更新"
    );
}

#[test]
fn 失败但有缓存时_items_非空且带一条_failure() {
    let snap = vec![rec("x", "claude", "h1")];
    // 缓存里是旧数据：哈希与安装时相同
    let cached = vec![view("x", "skills/通用/x", "h1")];
    let failure = classify_remote_failure("request timed out");

    let r = assemble_update_result(&snap, None, &cached, Some(failure), unchanged);

    assert_eq!(r.failures.len(), 1, "应带一条失败信息");
    assert_eq!(r.failures[0].kind, "network");
    assert_eq!(r.items.len(), 1, "缓存结果照常返回");
    assert_eq!(
        r.items[0].status,
        UpdateStatus::UpToDate,
        "基于缓存判定，本地与缓存一致"
    );
}

#[test]
fn 失败且无缓存时_items_为空但仍有一条_failure() {
    let snap = vec![rec("x", "claude", "h1")];
    let failure = classify_remote_failure("HTTP 404");

    let r = assemble_update_result(&snap, None, &[], Some(failure), unchanged);

    assert_eq!(r.failures.len(), 1, "应明确告知失败原因");
    assert_eq!(r.failures[0].kind, "notfound");
    // 无缓存时远端为空 → 每条记录都会命中「远端缺失」。
    // 这是既有语义（judge_record 的 RemoteMissing 分支），
    // 但配合 failures 提示，用户知道是「没读到」而不是「真删了」。
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].status, UpdateStatus::RemoteMissing);
}

#[test]
fn 失败时本地改动仍被如实识别() {
    // 关键：用缓存兜底不能把「本地改过」抹掉
    let snap = vec![rec("x", "claude", "h1")];
    let cached = vec![view("x", "skills/通用/x", "h1")];
    let failure = classify_remote_failure("request timed out");

    let r = assemble_update_result(
        &snap,
        None,
        &cached,
        Some(failure),
        |_| Some("h-locally-modified".to_string()),
    );

    assert_eq!(r.failures.len(), 1, "仍应带失败提示");
    assert_eq!(
        r.items[0].status,
        UpdateStatus::LocallyModified,
        "本地哈希不同应判为已修改，不能因为远端失败就忽略"
    );
}

#[test]
fn 本地导入的技能不参与远端判定() {
    let mut local = rec("y", "claude", "h1");
    local.source.repo_path = None;
    local.source.kind = SourceKind::Local;
    let snap = vec![local];
    let failure = classify_remote_failure("request timed out");

    let r = assemble_update_result(&snap, None, &[], Some(failure), unchanged);

    assert_eq!(
        r.items[0].status,
        UpdateStatus::UpToDate,
        "本地导入没有远端可比，不该被判成远端缺失"
    );
    assert_eq!(r.failures.len(), 1);
}

#[test]
fn 多工具同名技能都基于同一份数据判定() {
    let snap = vec![rec("x", "claude", "h1"), rec("x", "cursor", "h1")];
    let remote = vec![view("x", "skills/通用/x", "h2")];

    let r = assemble_update_result(&snap, Some(&remote), &[], None, unchanged);

    assert_eq!(r.items.len(), 2);
    assert!(r.items.iter().all(|i| i.status == UpdateStatus::UpdateAvailable));
}
