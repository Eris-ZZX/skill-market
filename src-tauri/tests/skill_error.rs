//! 错误码与中文文案映射
//!
//! 目的是让不懂技术的同事看懂「出了什么事、该怎么办」，而不是
//! `os error 10061`。分类只影响文案与按钮，不影响任何判定逻辑。

use skill_market_lib::skill_error::{classify, error_json, SkillError};

#[test]
fn 网络_连接失败() {
    let e = classify(
        "error sending request for url (http://gitlab.example.com/api/v4/...): \
         error trying to connect: tcp connect error: 由于目标计算机积极拒绝，无法连接。 (os error 10061)",
    );
    assert_eq!(e.code, "NETWORK_UNREACHABLE");
    assert!(e.message.contains("无法连接"), "应有中文说明: {}", e.message);
    assert_eq!(e.action.as_deref(), Some("retry"));
    // detail 必须保留原文，供排查
    assert!(e.detail.contains("os error 10061"), "detail 不能丢");
}

#[test]
fn 网络_超时() {
    for detail in [
        "request timed out",
        "下载超时（60 秒），请检查网络",
        "operation timed out",
    ] {
        let e = classify(detail);
        assert_eq!(e.code, "NETWORK_TIMEOUT", "{detail}");
        assert_eq!(e.action.as_deref(), Some("retry"));
    }
}

#[test]
fn 认证_401与403区分() {
    let a = classify("取 head commit 失败（HTTP 401 Unauthorized）: {\"message\":\"401\"}");
    assert_eq!(a.code, "AUTH_INVALID");
    assert_eq!(a.action.as_deref(), Some("open_settings"));

    let f = classify("提交失败（HTTP 403）: You are not allowed to push to protected branch");
    assert_eq!(f.code, "AUTH_FORBIDDEN");
    assert_eq!(f.action.as_deref(), Some("contact_it"));
}

#[test]
fn 仓库不存在() {
    let e = classify("下载仓库归档失败（HTTP 404）: {\"message\":\"404 Project Not Found\"}");
    assert_eq!(e.code, "REPO_NOT_FOUND");
    assert_eq!(e.action.as_deref(), Some("open_settings"));
}

#[test]
fn 技能不存在与仓库不存在区分() {
    // 同样 404，但文本里提到「技能」时应报 SKILL_NOT_FOUND
    let e = classify("404 仓库里找不到技能 grill-me");
    assert_eq!(e.code, "SKILL_NOT_FOUND");
    assert_eq!(e.action.as_deref(), Some("retry"));
}

#[test]
fn 工具未安装() {
    let e = classify("千问办公 未安装（/home/u/.qwenworkcn 不存在）");
    assert_eq!(e.code, "TOOL_NOT_INSTALLED");
    assert!(e.action.is_none(), "未安装没有可点的动作");
    assert!(e.message.contains("未安装"));
}

#[test]
fn 目标已存在() {
    for detail in [
        "Claude Code 下已存在同名技能 grill-me，请先卸载或改名",
        "目标已存在，不能覆盖: /p/x",
    ] {
        let e = classify(detail);
        assert_eq!(e.code, "TARGET_EXISTS", "{detail}");
        assert!(e.action.is_none());
    }
}

#[test]
fn 目录被占用() {
    for detail in [
        "复制 /a/b 失败: os error 32 (file in use)",
        "目录被占用",
        "locked by another process",
    ] {
        let e = classify(detail);
        assert_eq!(e.code, "DIR_IN_USE", "{detail}");
        assert_eq!(e.action.as_deref(), Some("retry"));
    }
}

#[test]
fn 权限不足() {
    for detail in [
        "os error 5: Access denied",
        "permission denied (os error 13)",
        "拒绝访问",
    ] {
        let e = classify(detail);
        assert_eq!(e.code, "PERMISSION_DENIED", "{detail}");
        assert_eq!(e.action.as_deref(), Some("contact_it"));
    }
}

#[test]
fn 技能名非法() {
    let e = classify("非法技能目录名（疑似路径穿越）: \"../evil\"");
    assert_eq!(e.code, "INVALID_NAME");
    assert!(e.action.is_none());
}

#[test]
fn 状态文件损坏() {
    let e = classify("状态文件格式损坏: /p/state.json");
    assert_eq!(e.code, "STATE_CORRUPT");
    assert_eq!(e.action.as_deref(), Some("open_settings"));
}

#[test]
fn 本地有改动被拒() {
    let e = classify("本地有改动，为避免覆盖你的修改已拒绝更新");
    assert_eq!(e.code, "LOCAL_MODIFIED_REFUSED");
    assert!(e.action.is_none(), "这不是错误，不该给「重试」");
}

#[test]
fn 未匹配的错误退化为通用文案且不丢_detail() {
    let raw = "某种从未见过的内部错误 xyz-123";
    let e = classify(raw);
    assert_eq!(e.code, "UNKNOWN");
    assert!(!e.message.is_empty(), "仍要有可读文案");
    assert_eq!(e.detail, raw, "原始信息必须一字不差地保留");
}

#[test]
fn 空字符串也不崩溃() {
    let e = classify("");
    assert_eq!(e.code, "UNKNOWN");
    assert_eq!(e.detail, "");
}

#[test]
fn 序列化后可反序列化() {
    // 前后端契约：前端 parseError 依赖这些字段名（camelCase）
    let e = classify("HTTP 401 Unauthorized");
    let json = e.to_json();
    let back: SkillError = serde_json::from_str(&json).expect("应能反序列化");
    assert_eq!(back, e);

    // 字段名必须是 camelCase，且 action 序列化为 null 而不是缺失
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["code"], "AUTH_INVALID");
    assert!(v.get("message").is_some());
    assert!(v.get("detail").is_some());
    assert!(v.get("action").is_some(), "action 字段必须存在（可为 null）");

    let none_action = classify("千问办公 未安装");
    let v2: serde_json::Value = serde_json::from_str(&none_action.to_json()).unwrap();
    assert!(v2["action"].is_null(), "无动作时应序列化为 null");
}

#[test]
fn error_json_产出可解析的_json() {
    // 命令层用 error_json 把任何 Display 错误转成前端可直接 JSON.parse 的串
    let s = error_json("HTTP 404 not found");
    let v: serde_json::Value = serde_json::from_str(&s).expect("必须是合法 JSON");
    assert_eq!(v["code"], "REPO_NOT_FOUND");

    // 带引号和换行的原始错误也不能破坏 JSON
    let nasty = "解析失败: {\"a\": \"b\"}\n第二行";
    let s2 = error_json(nasty);
    let v2: serde_json::Value = serde_json::from_str(&s2).expect("含引号换行也要是合法 JSON");
    assert_eq!(v2["detail"], nasty);
}

#[test]
fn 优先级_认证优先于超时() {
    // 两者都出现时，认证才是用户需要处理的那件事
    let e = classify("HTTP 401 (previous attempt: timeout)");
    assert_eq!(e.code, "AUTH_INVALID");
}
