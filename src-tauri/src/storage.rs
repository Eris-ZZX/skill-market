//! 本地状态持久化
//!
//! 用 JSON 而非 SQLite：记录规模是「每人几十个技能」，JSON 完全够用，
//! 且零额外依赖、可人工排查、可直接发给同事定位问题。
//!
//! 写入用「临时文件 + 原子替换」，避免写一半断电导致状态文件损坏。

use crate::state::InstalledSkill;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 应用配置（用户可编辑）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// GitLab 实例地址
    pub gitlab_base_url: String,
    /// 项目路径（如 `group/skills-repo`）
    pub gitlab_project_path: String,
    /// 访问令牌（可空）
    pub gitlab_token: Option<String>,
    /// 分支
    pub gitlab_branch: String,
    /// 默认安装方式
    pub default_install_mode: crate::state::InstallMode,
    /// 发布时的署名（姓名/工号）
    ///
    /// 内置 Token 意味着 GitLab 只能看到 bot 账号，
    /// 所以「谁改的」只能靠这个字段声明。
    #[serde(default)]
    pub publisher_identity: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            gitlab_base_url: String::new(),
            gitlab_project_path: String::new(),
            gitlab_token: None,
            // 内网 GitLab 项目默认分支普遍是 master（实测确认）
            gitlab_branch: "master".into(),
            default_install_mode: crate::state::InstallMode::Copy,
            publisher_identity: String::new(),
        }
    }
}

/// 持久化状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub settings: AppSettings,
    /// 已安装技能列表（一条 = 技能 × 工具）
    #[serde(default)]
    pub installs: Vec<InstalledSkill>,
}

fn default_schema_version() -> u32 {
    1
}

/// 应用数据目录：`%APPDATA%\skill-market`
pub fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .context("无法定位 AppData/Roaming")?;
    Ok(base.join("skill-market"))
}

pub fn state_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("state.json"))
}

/// 读取状态；文件不存在时返回默认值
pub fn load() -> Result<PersistedState> {
    let path = state_path()?;
    if !path.is_file() {
        return Ok(PersistedState {
            schema_version: default_schema_version(),
            ..Default::default()
        });
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("读取状态文件失败: {}", path.display()))?;
    let state: PersistedState = serde_json::from_str(&raw)
        .with_context(|| format!("状态文件格式损坏: {}", path.display()))?;
    Ok(state)
}

/// 原子写入状态
pub fn save(state: &PersistedState) -> Result<()> {
    let path = state_path()?;
    atomic_write_json(&path, state)
}

/// 原子写 JSON：先写临时文件，再替换目标
pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes())?;

    // Windows 上 rename 到已存在文件会失败，先尝试删除目标
    #[cfg(windows)]
    {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 技能库目录（junction 模式的源）
pub fn library_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("library"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_valid() {
        let s = PersistedState::default();
        assert_eq!(s.schema_version, 0); // Default derive 给 0，load 时才补 1
        assert!(s.installs.is_empty());
    }

    #[test]
    fn roundtrip_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let s = PersistedState {
            schema_version: 1,
            settings: AppSettings {
                gitlab_base_url: "https://gitlab.company.com".into(),
                gitlab_project_path: "group/skills".into(),
                ..Default::default()
            },
            installs: vec![],
        };
        atomic_write_json(&p, &s).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        let back: PersistedState = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.settings.gitlab_project_path, "group/skills");
    }
}
