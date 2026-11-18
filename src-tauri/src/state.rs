//! 安装状态与更新判定
//!
//! 界面只说四种状态，不多说：
//! - 最新：无标记
//! - 可更新：蓝色「更新」按钮
//! - 已修改：灰色「已修改」标记
//! - 冲突：黄色 + 三选一

use serde::{Deserialize, Serialize};

/// 一个技能在一个工具里的安装记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    /// 技能标识（frontmatter 的 name）
    pub name: String,
    /// 安装到哪个工具
    pub tool: String,
    /// 目标目录绝对路径
    pub target_path: String,
    /// 安装方式
    pub mode: InstallMode,
    /// 安装时的内容哈希（用来判断本地是否被改过）
    pub installed_hash: String,
    /// 安装时的版本号（仅展示）
    pub installed_version: Option<String>,
    /// 来源信息
    pub source: SkillSource,
    /// 安装时间（Unix 秒）
    pub installed_at: i64,
}

/// 安装方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// 复制独立副本（默认）
    Copy,
    /// 目录 junction 指向统一库（高级选项，需运行时探测可用性）
    Junction,
}

/// 技能的来源
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    /// 来源类型
    pub kind: SourceKind,
    /// 仓库地址（GitLab）
    pub repo_url: Option<String>,
    /// 仓库内路径，如 `skills/销售/sales-report`
    pub repo_path: Option<String>,
    /// 分支
    pub branch: Option<String>,
    /// 下载时的 commit SHA
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// 来自内网 GitLab
    Gitlab,
    /// 用户手工导入的本地技能
    Local,
}

/// 更新状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateStatus {
    /// 本地与远端一致
    UpToDate,
    /// 本地未改，远端有新版本
    UpdateAvailable,
    /// 本地改过，远端没变
    LocallyModified,
    /// 本地改过，远端也变了
    Conflict,
    /// 远端技能已删除
    RemoteMissing,
}

impl UpdateStatus {
    /// 从三个哈希判定状态
    ///
    /// - `installed_hash`：安装时记录的
    /// - `local_hash`：当前本地目录算出来的
    /// - `remote_hash`：远端技能算出来的；远端不存在时传 None
    pub fn judge(
        installed_hash: &str,
        local_hash: &str,
        remote_hash: Option<&str>,
    ) -> UpdateStatus {
        let local_changed = local_hash != installed_hash;

        let Some(remote_hash) = remote_hash else {
            return if local_changed {
                // 远端没了、本地又改过 —— 仍归为「已修改」，让用户保留自己的版本
                UpdateStatus::LocallyModified
            } else {
                UpdateStatus::RemoteMissing
            };
        };

        let remote_changed = remote_hash != installed_hash;

        match (local_changed, remote_changed) {
            (false, false) => UpdateStatus::UpToDate,
            (false, true) => UpdateStatus::UpdateAvailable,
            (true, false) => UpdateStatus::LocallyModified,
            (true, true) => UpdateStatus::Conflict,
        }
    }

    /// 是否需要在界面上提示
    pub fn needs_attention(&self) -> bool {
        !matches!(self, UpdateStatus::UpToDate)
    }
}

/// 冲突时用户的选择
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictChoice {
    /// 保留本地改动，忽略此次更新
    KeepMine,
    /// 用远端覆盖本地（先备份）
    UseRemote,
    /// 本地改名保留，同时装新版
    KeepBoth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_states() {
        // 本地没改，远端没变
        assert_eq!(
            UpdateStatus::judge("h1", "h1", Some("h1")),
            UpdateStatus::UpToDate
        );
        // 本地没改，远端变了
        assert_eq!(
            UpdateStatus::judge("h1", "h1", Some("h2")),
            UpdateStatus::UpdateAvailable
        );
        // 本地改了，远端没变
        assert_eq!(
            UpdateStatus::judge("h1", "h1local", Some("h1")),
            UpdateStatus::LocallyModified
        );
        // 两边都变了
        assert_eq!(
            UpdateStatus::judge("h1", "h1local", Some("h2")),
            UpdateStatus::Conflict
        );
    }

    #[test]
    fn remote_missing_with_local_edits_keeps_local() {
        assert_eq!(
            UpdateStatus::judge("h1", "h1local", None),
            UpdateStatus::LocallyModified
        );
    }

    #[test]
    fn remote_missing_without_edits_is_missing() {
        assert_eq!(
            UpdateStatus::judge("h1", "h1", None),
            UpdateStatus::RemoteMissing
        );
    }

    #[test]
    fn up_to_date_needs_no_attention() {
        assert!(!UpdateStatus::UpToDate.needs_attention());
        assert!(UpdateStatus::UpdateAvailable.needs_attention());
        assert!(UpdateStatus::Conflict.needs_attention());
    }
}
