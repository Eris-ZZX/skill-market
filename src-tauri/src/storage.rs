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

/// 读取状态的结果
///
/// 调用方必须区分「正常」与「坏了」——把损坏当成空状态会让下一次保存
/// 用空数据覆盖原文件，用户的安装记录就永久丢了。
pub enum LoadOutcome {
    Ok(PersistedState),
    /// 原文件损坏，内容已留档，从 state.json.bak 恢复成功
    Recovered { state: PersistedState, from: PathBuf },
    /// 原文件损坏且没有可用备份，内容已留档
    Corrupt { backup: PathBuf },
}

pub fn backup_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("state.json.bak"))
}

/// 读取状态
///
/// 文件不存在是首次启动，返回默认值；解析失败则留档并尝试从备份恢复。
pub fn load() -> LoadOutcome {
    let (Ok(path), Ok(bak)) = (state_path(), backup_path()) else {
        return LoadOutcome::Ok(default_state());
    };
    load_from(&path, &bak)
}

/// 从指定路径读取（测试用，避免碰真实 AppData）
pub fn load_from(path: &Path, bak: &Path) -> LoadOutcome {
    if !path.is_file() {
        return LoadOutcome::Ok(default_state());
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("读取状态文件失败: {e}");
            return LoadOutcome::Ok(default_state());
        }
    };
    if let Ok(state) = serde_json::from_str::<PersistedState>(&raw) {
        return LoadOutcome::Ok(state);
    }

    // 解析失败：先留档，再试备份
    let corrupt = path.with_file_name(format!(
        "{}.corrupt-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        chrono::Local::now().format("%Y%m%d%H%M%S")
    ));
    if let Err(e) = std::fs::rename(path, &corrupt) {
        log::error!("状态文件损坏，留档失败: {e}");
    } else {
        log::error!("状态文件损坏，已留档为 {}", corrupt.display());
    }

    if let Ok(raw) = std::fs::read_to_string(bak) {
        if let Ok(state) = serde_json::from_str::<PersistedState>(&raw) {
            log::warn!("已从备份恢复状态: {}", bak.display());
            return LoadOutcome::Recovered {
                state,
                from: bak.to_path_buf(),
            };
        }
    }
    LoadOutcome::Corrupt { backup: corrupt }
}

fn default_state() -> PersistedState {
    PersistedState {
        schema_version: default_schema_version(),
        ..Default::default()
    }
}

/// 原子写入状态
///
/// 写之前先把现有文件复制成 state.json.bak，这样即使下一次读到半截文件
/// 也还有上一份可恢复。
pub fn save(state: &PersistedState) -> Result<()> {
    let path = state_path()?;
    let bak = backup_path()?;
    save_to(&path, &bak, state)
}

/// 写到指定路径（测试用）
pub fn save_to(path: &Path, bak: &Path, state: &PersistedState) -> Result<()> {
    if path.is_file() {
        let _ = std::fs::copy(path, bak);
    }
    atomic_write_json(path, state)
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

    fn paths(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("state.json"), dir.join("state.json.bak"))
    }

    #[test]
    fn load_missing_file_returns_default() {
        let d = tempfile::tempdir().unwrap();
        let (p, bak) = paths(d.path());
        assert!(matches!(load_from(&p, &bak), LoadOutcome::Ok(_)));
    }

    #[test]
    fn load_valid_file_returns_ok() {
        let d = tempfile::tempdir().unwrap();
        let (p, bak) = paths(d.path());
        let s = PersistedState {
            schema_version: 1,
            ..Default::default()
        };
        atomic_write_json(&p, &s).unwrap();
        match load_from(&p, &bak) {
            LoadOutcome::Ok(got) => assert_eq!(got.schema_version, 1),
            _ => panic!("应判定为 Ok"),
        }
    }

    #[test]
    fn load_corrupt_recovers_from_backup() {
        let d = tempfile::tempdir().unwrap();
        let (p, bak) = paths(d.path());

        let good = PersistedState {
            schema_version: 1,
            settings: AppSettings {
                gitlab_project_path: "group/from-backup".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        std::fs::write(&bak, serde_json::to_string(&good).unwrap()).unwrap();
        std::fs::write(&p, "{ this is not json").unwrap();

        match load_from(&p, &bak) {
            LoadOutcome::Recovered { state, .. } => {
                assert_eq!(state.settings.gitlab_project_path, "group/from-backup");
            }
            _ => panic!("应判定为 Recovered"),
        }

        // 原损坏内容必须留档且内容完好，不能被删
        let corrupt: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(corrupt.len(), 1, "应留下一个 .corrupt-* 文件");
        let saved = std::fs::read_to_string(corrupt[0].path()).unwrap();
        assert_eq!(saved, "{ this is not json", "留档必须保留原始内容");
        assert!(!p.exists(), "损坏文件应已改名，不再是 state.json");
    }

    #[test]
    fn load_corrupt_without_backup_is_corrupt_and_keeps_content() {
        let d = tempfile::tempdir().unwrap();
        let (p, bak) = paths(d.path());
        std::fs::write(&p, "totally broken").unwrap();

        match load_from(&p, &bak) {
            LoadOutcome::Corrupt { backup } => {
                assert!(backup.exists());
                assert_eq!(std::fs::read_to_string(&backup).unwrap(), "totally broken");
            }
            _ => panic!("应判定为 Corrupt"),
        }
    }

    #[test]
    fn save_creates_bak_of_previous_content() {
        let d = tempfile::tempdir().unwrap();
        let (p, bak) = paths(d.path());

        let first = PersistedState {
            schema_version: 1,
            settings: AppSettings {
                gitlab_project_path: "group/first".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        save_to(&p, &bak, &first).unwrap();
        assert!(!bak.exists(), "首次保存时没有旧文件，不该产生 bak");

        let second = PersistedState {
            schema_version: 1,
            settings: AppSettings {
                gitlab_project_path: "group/second".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        save_to(&p, &bak, &second).unwrap();

        assert!(bak.is_file(), "第二次保存前应生成 state.json.bak");
        let saved: PersistedState =
            serde_json::from_str(&std::fs::read_to_string(&bak).unwrap()).unwrap();
        assert_eq!(
            saved.settings.gitlab_project_path, "group/first",
            "bak 必须是上一份内容"
        );
    }
}
