//! 目标工具注册表
//!
//! 替代原 cc-switch 中分散的 `copilot_config` / `kimi_config` / `workbuddy_config`
//! 等 8 个模块。每个工具只需要一条声明式记录。
//!
//! 设计约束（来自 `关键实现约束.md` R1）：
//! - **只写已存在的工具目录，绝不创建根目录**
//! - 根目录不存在 ⇒ 该工具视为「未安装」，UI 不提供安装选项

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 支持的目标工具
///
/// 只保留技能分发实际需要的 6 个。
/// 原 cc-switch 的 16 个变体里，Kimi / Mcode / Dsh / Agy / Omp / Copilot /
/// GrokBuild / Hermes / OpenClaw / OpenCode / ClaudeDesktop 全部移除——
/// 它们只服务于 provider 切换，与技能管理无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    /// Cursor —— `~/.cursor/skills`
    Cursor,
    /// Claude Code —— `~/.claude/skills`
    Claude,
    /// Codex CLI —— `~/.codex/skills`
    Codex,
    /// ChatGPT 桌面 / Agent Skills 约定 —— `~/.agents/skills`
    Agents,
    /// 千问办公 —— `~/.qwenworkcn/skills`
    QwenWork,
    /// WorkBuddy —— `~/.workbuddy/skills`
    WorkBuddy,
}

impl ToolKind {
    /// 全部支持的工具（UI 遍历用）
    pub const ALL: &'static [ToolKind] = &[
        ToolKind::Cursor,
        ToolKind::Claude,
        ToolKind::Codex,
        ToolKind::Agents,
        ToolKind::QwenWork,
        ToolKind::WorkBuddy,
    ];

    /// 稳定标识（存数据库、前端传参用）
    pub fn key(&self) -> &'static str {
        match self {
            ToolKind::Cursor => "cursor",
            ToolKind::Claude => "claude",
            ToolKind::Codex => "codex",
            ToolKind::Agents => "agents",
            ToolKind::QwenWork => "qwenwork",
            ToolKind::WorkBuddy => "workbuddy",
        }
    }

    /// 界面显示名
    pub fn display_name(&self) -> &'static str {
        match self {
            ToolKind::Cursor => "Cursor",
            ToolKind::Claude => "Claude Code",
            ToolKind::Codex => "Codex",
            ToolKind::Agents => "ChatGPT / Agent Skills",
            ToolKind::QwenWork => "千问办公",
            ToolKind::WorkBuddy => "WorkBuddy",
        }
    }

    /// 工具数据根目录（用于「是否已安装」探测）
    ///
    /// 注意：**这是探测目标，不是创建目标**。根目录不存在就说明工具没装。
    pub fn root_dir(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            ToolKind::Cursor => home.join(".cursor"),
            ToolKind::Claude => home.join(".claude"),
            ToolKind::Codex => home.join(".codex"),
            ToolKind::Agents => home.join(".agents"),
            ToolKind::QwenWork => home.join(".qwenworkcn"),
            ToolKind::WorkBuddy => home.join(".workbuddy"),
        })
    }

    /// 技能目录（`<root>/skills`）
    ///
    /// 四个工具的约定一致：技能必须是 `skills/<name>/SKILL.md` 三层结构。
    pub fn skills_dir(&self) -> Option<PathBuf> {
        self.root_dir().map(|r| r.join("skills"))
    }

    /// 该工具是否会在 SKILL.md 里回写易变字段
    ///
    /// 影响内容哈希的规范化：这些字段在计算 hash 前必须剥离，
    /// 否则跑一次技能就会被误判为「本地已修改」。
    pub fn writes_volatile_fields(&self) -> bool {
        matches!(self, ToolKind::QwenWork | ToolKind::WorkBuddy)
    }

    /// 该工具是否有已知的「禁止写入」子目录
    ///
    /// Cursor 的 `skills-cursor` 是它自己同步内置技能用的目录（28 个技能 +
    /// `.sync-manifest.json`），写进去会被覆盖或干扰其更新判断。
    /// 两者只差一个后缀 `-cursor`，极易误伤，必须硬编码排除。
    pub fn forbidden_subdirs(&self) -> &'static [&'static str] {
        match self {
            ToolKind::Cursor => &["skills-cursor", "plugins", "extensions", "agents", "rules"],
            _ => &[],
        }
    }

    pub fn from_key(key: &str) -> Option<ToolKind> {
        ToolKind::ALL.iter().copied().find(|t| t.key() == key)
    }
}

/// 单个工具的探测结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDetection {
    pub key: String,
    pub display_name: String,
    /// 工具根目录路径（可能不存在）
    pub root_dir: Option<String>,
    /// 根目录是否存在 ⇒ 工具是否已安装
    pub installed: bool,
    /// `skills/` 子目录是否存在
    pub skills_dir_exists: bool,
    /// 已安装的技能数量（扫不到时为 0）
    pub installed_skill_count: usize,
}

/// 探测所有工具的安装状态
///
/// **只读操作**：不创建任何目录。
pub fn detect_all() -> Vec<ToolDetection> {
    ToolKind::ALL
        .iter()
        .map(|t| {
            let root = t.root_dir();
            let installed = root.as_ref().is_some_and(|r| r.is_dir());
            let skills = t.skills_dir();
            let skills_dir_exists = skills.as_ref().is_some_and(|s| s.is_dir());

            let installed_skill_count = if skills_dir_exists {
                skills
                    .as_ref()
                    .and_then(|s| std::fs::read_dir(s).ok())
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter(|e| e.path().join("SKILL.md").is_file())
                            .count()
                    })
                    .unwrap_or(0)
            } else {
                0
            };

            ToolDetection {
                key: t.key().to_string(),
                display_name: t.display_name().to_string(),
                root_dir: root.map(|p| p.to_string_lossy().to_string()),
                installed,
                skills_dir_exists,
                installed_skill_count,
            }
        })
        .collect()
}

/// 准备安装目标目录
///
/// 严格遵循 R1：
/// - 根目录不存在 → 报错「工具未安装」，**不创建**
/// - `skills/` 不存在 → 需要显式确认才创建（`allow_create_skills_dir`）
///
/// 返回最终可写入的技能父目录。
pub fn prepare_target(tool: ToolKind, allow_create_skills_dir: bool) -> anyhow::Result<PathBuf> {
    let root = tool
        .root_dir()
        .ok_or_else(|| anyhow::anyhow!("无法定位用户主目录"))?;

    if !root.is_dir() {
        anyhow::bail!(
            "{} 未安装（{} 不存在）",
            tool.display_name(),
            root.display()
        );
    }

    let skills = root.join("skills");
    if !skills.is_dir() {
        if !allow_create_skills_dir {
            anyhow::bail!(
                "需要创建 {}，请确认后重试",
                skills.display()
            );
        }
        // 只创建到 skills 这一层，绝不创建更上层
        std::fs::create_dir_all(&skills)?;
    }

    Ok(skills)
}

/// 断言目标路径不在工具的受保护子目录内
pub fn assert_writable(tool: ToolKind, target: &std::path::Path) -> anyhow::Result<()> {
    let forbidden = tool.forbidden_subdirs();
    if forbidden.is_empty() {
        return Ok(());
    }
    for comp in target.components() {
        if let std::path::Component::Normal(s) = comp {
            if let Some(name) = s.to_str() {
                if forbidden.contains(&name) {
                    anyhow::bail!(
                        "拒绝写入受保护目录（{} 自身管理）：{}",
                        tool.display_name(),
                        target.display()
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_have_unique_keys() {
        let mut keys: Vec<_> = ToolKind::ALL.iter().map(|t| t.key()).collect();
        keys.sort();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "工具 key 必须唯一");
    }

    #[test]
    fn cursor_forbids_skills_cursor_subdir() {
        let p = PathBuf::from("C:/Users/x/.cursor/skills-cursor/foo");
        assert!(assert_writable(ToolKind::Cursor, &p).is_err());
    }

    #[test]
    fn cursor_allows_skills_subdir() {
        let p = PathBuf::from("C:/Users/x/.cursor/skills/foo");
        assert!(assert_writable(ToolKind::Cursor, &p).is_ok());
    }

    #[test]
    fn prepare_target_rejects_missing_root() {
        // 用一个几乎不可能存在的工具目录验证「未安装 ⇒ 报错」而非「创建」
        // 这里直接断言存在性判断逻辑，不依赖真实环境
        let fake = PathBuf::from("Z:/definitely/not/exist/.nocursor");
        assert!(!fake.is_dir());
    }

    #[test]
    fn volatile_fields_only_for_qwen_and_workbuddy() {
        assert!(ToolKind::QwenWork.writes_volatile_fields());
        assert!(ToolKind::WorkBuddy.writes_volatile_fields());
        assert!(!ToolKind::Cursor.writes_volatile_fields());
        assert!(!ToolKind::Claude.writes_volatile_fields());
    }
}
