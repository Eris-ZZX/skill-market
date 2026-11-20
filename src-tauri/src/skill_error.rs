//! 用户可见的错误码与中文文案
//!
//! 用户不该看到 `error sending request for url (...): tcp connect error ...
//! (os error 10061)`。这里把底层错误归类成稳定错误码，前端按 `code` 决定
//! 文案与动作按钮，`detail` 折叠展示供排查。
//!
//! 分类只影响「说什么、给什么按钮」，不影响任何判定逻辑，所以按特征字符串
//! 粗分即可。日志仍然记录原始错误，不受影响。

use serde::{Deserialize, Serialize};

/// 结构化错误：跨前后端的契约
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillError {
    /// 稳定标识，前端据此映射
    pub code: String,
    /// 中文，可直接展示
    pub message: String,
    /// 原始技术信息，可折叠
    pub detail: String,
    /// 建议动作：retry / open_settings / reinstall / contact_it
    pub action: Option<String>,
}

impl SkillError {
    /// 序列化成前端可直接 JSON.parse 的字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("ERROR:{}", self.code))
    }

    /// 从错误文本归类
    pub fn from_error_text(detail: &str) -> Self {
        classify(detail)
    }
}

/// 从底层错误文本推断错误码
///
/// 顺序有意为之：越具体的特征越靠前。例如 `HTTP 401 ... timeout` 应报认证
/// 而不是超时，因为认证失败才是用户要处理的那件事。
pub fn classify(detail: &str) -> SkillError {
    let raw = detail.trim();
    let lower = raw.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    let (code, message, action) = if has(&["401", "unauthorized", "令牌", "token invalid"]) {
        (
            "AUTH_INVALID",
            "访问令牌无效或已过期".to_string(),
            Some("open_settings"),
        )
    } else if has(&["403", "forbidden", "protected branch", "权限不足"]) {
        (
            "AUTH_FORBIDDEN",
            "权限不足，无法写入该分支".to_string(),
            Some("contact_it"),
        )
    } else if has(&["404", "not found", "notfound"]) {
        // 404 可能是仓库也可能是技能，看文本里有哪个词
        if has(&["技能", "skill"]) {
            (
                "SKILL_NOT_FOUND",
                "仓库里找不到这个技能（可能已被删除）".to_string(),
                Some("retry"),
            )
        } else {
            (
                "REPO_NOT_FOUND",
                "仓库不存在，请检查项目路径".to_string(),
                Some("open_settings"),
            )
        }
    } else if has(&["timeout", "timed out", "超时"]) {
        (
            "NETWORK_TIMEOUT",
            "连接超时，请稍后重试".to_string(),
            Some("retry"),
        )
    } else if has(&[
        "connect",
        "connection",
        "dns",
        "network",
        "unreachable",
        "tcp connect",
        "10061",
    ]) {
        (
            "NETWORK_UNREACHABLE",
            "无法连接到 GitLab，请检查网络".to_string(),
            Some("retry"),
        )
    } else if has(&["未安装"]) || (lower.contains("不存在") && has(&[".claude", ".cursor", ".codex", ".agents", ".qwenworkcn", ".workbuddy"]))
    {
        (
            "TOOL_NOT_INSTALLED",
            "目标工具未安装，请先安装它".to_string(),
            None,
        )
    } else if has(&["已存在同名", "目标已存在", "请先卸载"]) {
        (
            "TARGET_EXISTS",
            "该工具下已存在同名技能，请先卸载或改名".to_string(),
            None,
        )
    } else if has(&["被占用", "in use", "locked", "os error 32"]) {
        (
            "DIR_IN_USE",
            "目录被其他程序占用，请关闭后重试".to_string(),
            Some("retry"),
        )
    } else if has(&["denied", "permission", "拒绝访问", "os error 5", "没有权限"]) {
        (
            "PERMISSION_DENIED",
            "没有写入权限，请联系 IT".to_string(),
            Some("contact_it"),
        )
    } else if has(&["非法技能目录名", "非法技能名", "路径穿越"]) {
        (
            "INVALID_NAME",
            "技能名不合法（不能含路径分隔符或以点开头）".to_string(),
            None,
        )
    } else if has(&["损坏", "corrupt"]) {
        (
            "STATE_CORRUPT",
            "安装记录文件损坏，已备份原文件".to_string(),
            Some("open_settings"),
        )
    } else if has(&["本地有改动", "为避免覆盖"]) {
        (
            "LOCAL_MODIFIED_REFUSED",
            "本地有改动，为避免覆盖已拒绝本次操作".to_string(),
            None,
        )
    } else {
        (
            "UNKNOWN",
            "操作失败，请展开详情查看原因".to_string(),
            Some("retry"),
        )
    };

    SkillError {
        code: code.to_string(),
        message,
        detail: raw.to_string(),
        action: action.map(|a| a.to_string()),
    }
}

/// 把任意 `Result` 的错误转成可直接返回给前端的 JSON 字符串
pub fn error_json<E: std::fmt::Display>(e: E) -> String {
    classify(&e.to_string()).to_json()
}
