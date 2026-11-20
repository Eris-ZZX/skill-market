//! 多工具安装逐条独立成败
//!
//! 核心风险是孤儿：一个工具失败时，已经落盘的其它工具必须已经被记录，
//! 否则磁盘上有目录、state.json 里没有，用户看到报错却发现技能「消失」。
//!
//! 全部用 tempdir + 路径注入（`install_each_tool` 的 `resolve_root` 与
//! `library_root` 都是参数），不依赖环境变量、不依赖 Tauri 运行时
//! （#30 的教训：`XDG_CONFIG_HOME` 在 Windows 上无效，`mock_app` 会让
//! 测试二进制 STATUS_ENTRYPOINT_NOT_FOUND）。

use skill_market_lib::install::{classify_install_failure, install_each_tool};
use skill_market_lib::state::{InstallMode, InstalledSkill, SkillSource, SourceKind};
use skill_market_lib::tools::ToolKind;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn source() -> SkillSource {
    SkillSource {
        kind: SourceKind::Gitlab,
        repo_url: Some("https://example.invalid/g/p".into()),
        repo_path: Some("skills/通用/demo".into()),
        branch: Some("master".into()),
        commit: Some("abc".into()),
    }
}

/// 造一个技能源目录
fn make_source(root: &Path, name: &str) -> PathBuf {
    let d = root.join("src").join(name);
    write(&d, "SKILL.md", &format!("---\nname: {name}\n---\nbody\n"));
    write(&d, "scripts/run.py", "print(1)\n");
    d
}

/// 给每个工具准备一个独立的临时技能目录，模拟「都装好了」
///
/// 返回 (map, 闭包所需的所有权数据)。这里用 HashMap 把 ToolKind 映射到路径，
/// 测试自己决定哪个工具的目录是坏的。
fn roots(
    base: &Path,
    tool_dirs: &[(ToolKind, &str)],
) -> HashMap<ToolKind, PathBuf> {
    let mut m = HashMap::new();
    for (t, dir) in tool_dirs {
        let p = base.join(dir).join("skills");
        fs::create_dir_all(&p).unwrap();
        m.insert(*t, p);
    }
    m
}

// ========== 失败分类 ==========

#[test]
fn 分类_占用与权限类可重试() {
    for detail in [
        "目录被占用",
        "os error 32 (file in use)",
        "os error 5: Access denied",
        "permission denied",
        "拒绝访问",
        "locked",
    ] {
        let f = classify_install_failure("claude", detail);
        assert!(f.retryable, "{detail:?} 应判为可重试");
        assert_eq!(f.tool, "claude");
        assert_eq!(f.detail, detail, "detail 应保留原始错误");
        assert!(!f.message.is_empty());
    }
}

#[test]
fn 分类_未安装与已存在不可重试() {
    for detail in [
        "Claude Code 未安装（/home/u/.claude 不存在）",
        "已存在同名技能 x，请先卸载或改名",
        "非法技能目录名（疑似路径穿越）: \"../x\"",
    ] {
        let f = classify_install_failure("claude", detail);
        assert!(!f.retryable, "{detail:?} 重试也没用，应判为不可重试");
    }
}

#[test]
fn 分类_未识别的一律不可重试() {
    let f = classify_install_failure("cursor", "某种没见过的错误");
    assert!(!f.retryable);
    assert!(!f.message.is_empty(), "应有可读的中文原因");
    assert_eq!(f.detail, "某种没见过的错误", "detail 保留原文");
}

// ========== 逐条成败（真实调用 install_each_tool） ==========

#[test]
fn 四工具装三成一败() {
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo");
    let lib = d.path().join("library");

    let tools = [
        ToolKind::Claude,
        ToolKind::Codex,
        ToolKind::Agents,
        ToolKind::QwenWork,
    ];
    let mut map = roots(
        d.path(),
        &[
            (ToolKind::Claude, "claude"),
            (ToolKind::Codex, "codex"),
            (ToolKind::Agents, "agents"),
            (ToolKind::QwenWork, "qwen"),
        ],
    );
    // 把千问的技能目录换成一个普通文件 → 它的目标路径非法，必然失败
    let bad = d.path().join("qwen").join("skills");
    fs::remove_dir_all(&bad).unwrap();
    fs::write(&bad, "not a dir").unwrap();
    map.insert(ToolKind::QwenWork, bad);

    let recorded: RefCell<Vec<InstalledSkill>> = RefCell::new(Vec::new());
    let out = install_each_tool(
        "demo",
        &src,
        InstallMode::Copy,
        &tools,
        source(),
        Some("1.0.0".into()),
        false,
        &lib,
        |t| {
            map.get(&t)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("没有为 {:?} 准备路径", t))
        },
        |rec| {
            recorded.borrow_mut().push(rec.clone());
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(out.succeeded.len(), 3, "三个应成功");
    assert_eq!(out.failed.len(), 1, "千问应失败");
    assert_eq!(out.failed[0].tool, "qwenwork");

    // 成功的三个都落了盘
    for t in ["claude", "codex", "agents"] {
        let p = d.path().join(t).join("skills").join("demo").join("SKILL.md");
        assert!(p.is_file(), "{t} 的技能目录应存在: {}", p.display());
    }

    // 防孤儿：成功的那三条必须已经被上报（调用方据此写 state.json）
    let recs = recorded.borrow();
    assert_eq!(recs.len(), 3, "每个成功的都应立即上报一次");
    assert!(
        !recs.iter().any(|r| r.tool == "qwenwork"),
        "失败的不该被上报为成功"
    );
}

#[test]
fn 失败的那个没进_state_记录() {
    // 承接上一条：failed 里的工具绝不能出现在上报里，
    // 否则 state.json 会记下一个磁盘上没有的技能
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo2");
    let lib = d.path().join("library");

    let mut map = roots(d.path(), &[(ToolKind::Claude, "claude")]);
    // 目标位置被同名普通文件占住 → 部署失败
    let claude_root = map.remove(&ToolKind::Claude).unwrap();
    fs::write(claude_root.join("demo2"), "occupied").unwrap();

    let recorded: RefCell<Vec<InstalledSkill>> = RefCell::new(Vec::new());
    let out = install_each_tool(
        "demo2",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude],
        source(),
        None,
        false,
        &lib,
        |_| Ok(claude_root.clone()),
        |rec| {
            recorded.borrow_mut().push(rec.clone());
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(out.succeeded.len(), 0, "唯一的目标失败，succeeded 应为空");
    assert_eq!(out.failed.len(), 1);
    assert!(
        recorded.borrow().is_empty(),
        "失败的工具不得被上报为已安装（防孤儿）"
    );
}

#[test]
fn 库写入失败时全部失败且不崩溃() {
    let d = tempfile::tempdir().unwrap();
    // 来源目录没有 SKILL.md → 库准备阶段就失败
    let src = d.path().join("src").join("noskill");
    fs::create_dir_all(&src).unwrap();
    let lib = d.path().join("library");

    let r = install_each_tool(
        "noskill",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude, ToolKind::Codex],
        source(),
        None,
        false,
        &lib,
        |_| Ok(d.path().join("unused")),
        |_| Ok(()),
    );
    assert!(r.is_err(), "库写不进去时应整体失败");
}

#[test]
fn 全部成功时_failed_为空() {
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo3");
    let lib = d.path().join("library");
    let map = roots(
        d.path(),
        &[(ToolKind::Claude, "claude"), (ToolKind::Codex, "codex")],
    );

    let out = install_each_tool(
        "demo3",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude, ToolKind::Codex],
        source(),
        None,
        false,
        &lib,
        |t| Ok(map[&t].clone()),
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(out.succeeded.len(), 2);
    assert!(out.failed.is_empty(), "全成功时不该有失败项");
}

#[test]
fn 全部失败时_succeeded_为空且不_panic() {
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo4");
    let lib = d.path().join("library");

    // 两个工具的目标父目录都是普通文件
    let b1 = d.path().join("bad1");
    let b2 = d.path().join("bad2");
    fs::write(&b1, "x").unwrap();
    fs::write(&b2, "x").unwrap();
    let mut map = HashMap::new();
    map.insert(ToolKind::Claude, b1);
    map.insert(ToolKind::Codex, b2);

    let out = install_each_tool(
        "demo4",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude, ToolKind::Codex],
        source(),
        None,
        false,
        &lib,
        |t| Ok(map[&t].clone()),
        |_| Ok(()),
    )
    .unwrap();

    assert!(out.succeeded.is_empty(), "全失败时 succeeded 应为空");
    assert_eq!(out.failed.len(), 2, "两条失败都要被单独记录");
}

#[test]
fn 记录回调失败的工具不进_succeeded() {
    // 安装成功但落盘失败：这条必须进 failed，
    // 否则界面说成功、state.json 里没有，就是孤儿
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo5");
    let lib = d.path().join("library");
    let map = roots(d.path(), &[(ToolKind::Claude, "claude")]);

    let out = install_each_tool(
        "demo5",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude],
        source(),
        None,
        false,
        &lib,
        |t| Ok(map[&t].clone()),
        |_| Err(anyhow::anyhow!("磁盘已满")),
    )
    .unwrap();

    assert!(
        out.succeeded.is_empty(),
        "记录失败的工具不该算成功"
    );
    assert_eq!(out.failed.len(), 1);
    assert!(
        out.failed[0].detail.contains("记录失败"),
        "错误应说明是记录环节失败: {}",
        out.failed[0].detail
    );
}

#[test]
fn 非法技能名在动任何文件之前被拒() {
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "evil");
    let lib = d.path().join("library");
    let map = roots(d.path(), &[(ToolKind::Claude, "claude")]);

    let r = install_each_tool(
        "../evil",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude],
        source(),
        None,
        false,
        &lib,
        |t| Ok(map[&t].clone()),
        |_| Ok(()),
    );
    assert!(r.is_err(), "路径穿越式名字必须被拒");
}

#[test]
fn 没有目标工具时直接报错() {
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo6");
    let lib = d.path().join("library");

    let r = install_each_tool(
        "demo6",
        &src,
        InstallMode::Copy,
        &[],
        source(),
        None,
        false,
        &lib,
        |_| Ok(d.path().join("unused")),
        |_| Ok(()),
    );
    assert!(r.is_err());
}

#[test]
fn 一个工具路径解析失败不影响其它() {
    // resolve_root 本身报错（等价于「该工具未安装」）时，
    // 应只让这个工具失败，其余照常安装
    let d = tempfile::tempdir().unwrap();
    let src = make_source(d.path(), "demo7");
    let lib = d.path().join("library");
    let map = roots(d.path(), &[(ToolKind::Claude, "claude")]);

    let out = install_each_tool(
        "demo7",
        &src,
        InstallMode::Copy,
        &[ToolKind::Claude, ToolKind::Codex],
        source(),
        None,
        false,
        &lib,
        |t| match map.get(&t) {
            Some(p) => Ok(p.clone()),
            None => Err(anyhow::anyhow!("Codex 未安装")),
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(out.succeeded.len(), 1, "Claude 应装成功");
    assert_eq!(out.failed.len(), 1, "Codex 应单独失败");
    assert_eq!(out.failed[0].tool, "codex");
    assert!(!out.failed[0].retryable, "未安装重试也没用");
}
