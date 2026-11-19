//! Tauri 命令层
//!
//! 前端只能通过这里的命令访问后端能力。
//! 所有命令返回 `Result<T, String>`，错误直接展示给用户。

use crate::state::{judge_record, InstalledSkill, UpdateStatus};
use crate::storage::{AppSettings, PersistedState};
use crate::tools::{self, ToolDetection, ToolKind};
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

/// 应用运行时状态
pub struct AppState {
    pub inner: Mutex<PersistedState>,
}

impl AppState {
    pub fn load() -> Self {
        let state = crate::storage::load().unwrap_or_default();
        Self {
            inner: Mutex::new(state),
        }
    }
}

fn lock<'a>(s: &'a State<'_, AppState>) -> Result<std::sync::MutexGuard<'a, PersistedState>, String> {
    s.inner.lock().map_err(|_| "状态锁已损坏".to_string())
}

// ========== 工具探测 ==========

/// 探测所有目标工具的安装状态（只读，不创建任何目录）
#[tauri::command]
pub fn detect_tools() -> Result<Vec<ToolDetection>, String> {
    Ok(tools::detect_all())
}

/// junction 是否可用（运行时探测）
#[tauri::command]
pub fn junction_available() -> bool {
    crate::install::junction_supported()
}

// ========== 设置 ==========

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(lock(&state)?.settings.clone())
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    let mut guard = lock(&state)?;
    guard.settings = settings;
    crate::storage::save(&guard).map_err(|e| e.to_string())
}

// ========== 技能市场 ==========

/// 刷新远端索引
///
/// 返回技能列表。实现见 `commands::market`（后续补）。
#[tauri::command]
pub async fn refresh_index(state: State<'_, AppState>) -> Result<Vec<RemoteSkillView>, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };
    crate::commands::market::fetch_index(cfg)
        .await
        .map_err(|e| e.to_string())
}

/// 读取已缓存的远端索引
#[tauri::command]
pub async fn list_remote_skills() -> Result<Vec<RemoteSkillView>, String> {
    crate::commands::market::load_cached_index().map_err(|e| e.to_string())
}

fn build_gitlab_config(s: &AppSettings) -> Result<crate::gitlab::GitLabConfig, String> {
    if s.gitlab_base_url.trim().is_empty() {
        return Err("尚未配置 GitLab 地址，请先在设置中填写".into());
    }
    if s.gitlab_project_path.trim().is_empty() {
        return Err("尚未配置项目路径，请先在设置中填写".into());
    }
    Ok(crate::gitlab::GitLabConfig {
        base_url: s.gitlab_base_url.clone(),
        project_path: s.gitlab_project_path.clone(),
        token: s.gitlab_token.clone(),
        branch: s.gitlab_branch.clone(),
    })
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSkillView {
    pub name: String,
    pub repo_path: String,
    pub hash: String,
    pub version: Option<String>,
    pub description: String,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub last_commit_at: Option<String>,
}

// ========== 安装管理 ==========

#[tauri::command]
pub fn list_installed(state: State<'_, AppState>) -> Result<Vec<InstalledSkill>, String> {
    Ok(lock(&state)?.installs.clone())
}

/// 安装一个技能
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn install_skill(
    state: State<'_, AppState>,
    name: String,
    repo_path: String,
    tools: Vec<String>,
    allow_create_skills_dir: bool,
) -> Result<Vec<InstalledSkill>, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };

    let installed = crate::commands::market::install_from_repo(
        cfg,
        &name,
        &repo_path,
        &tools,
        allow_create_skills_dir,
    )
    .await
    .map_err(|e| e.to_string())?;

    // 记录到状态
    {
        let mut guard = lock(&state)?;
        for rec in &installed {
            guard.installs.retain(|x| !(x.name == rec.name && x.tool == rec.tool));
            guard.installs.push(rec.clone());
        }
        crate::storage::save(&guard).map_err(|e| e.to_string())?;
    }

    Ok(installed)
}

#[tauri::command]
pub fn uninstall_skill(
    state: State<'_, AppState>,
    name: String,
    tool: String,
) -> Result<(), String> {
    let kind = ToolKind::from_key(&tool).ok_or_else(|| format!("未知工具: {tool}"))?;
    crate::install::uninstall(kind, &name).map_err(|e| e.to_string())?;

    let mut guard = lock(&state)?;
    guard.installs.retain(|x| !(x.name == name && x.tool == tool));
    crate::storage::save(&guard).map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新检查结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckItem {
    pub name: String,
    pub tool: String,
    pub status: UpdateStatus,
    pub installed_version: Option<String>,
    pub remote_version: Option<String>,
}

/// 在远端索引里找与某条安装记录对应的技能
///
/// 必须同时比对 `name` 与 `repo_path`：同名技能可能同时存在于
/// `skills/通用/x` 和 `skills/研发/x`，只按 name 匹配会把两者哈希串掉。
///
/// `repo_path` 为 None 表示本地导入，没有远端对应物。
pub fn find_remote<'a>(
    name: &str,
    repo_path: Option<&str>,
    remote: &'a [RemoteSkillView],
) -> Option<&'a RemoteSkillView> {
    let repo_path = repo_path?;
    remote
        .iter()
        .find(|r| r.name == name && r.repo_path == repo_path)
}

/// 检查所有已装技能的更新状态
///
/// 配置缺失和网络失败都返回 Err：把「连不上」伪装成「远端已删除」
/// 会把人引向保留本地版本，这比直接报错更危险。
#[tauri::command]
pub async fn check_updates(state: State<'_, AppState>) -> Result<Vec<UpdateCheckItem>, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };

    let remote = crate::commands::market::fetch_index(cfg)
        .await
        .map_err(|e| format!("无法获取远端技能索引：{e}"))?;

    let guard = lock(&state)?;
    let mut out = Vec::new();
    for rec in &guard.installs {
        // 读不到就如实传 None，绝不回退到安装时的旧指纹——
        // 否则目录被删或被占用会被误判成「本地没改」，更新时可能覆盖用户手改
        let local = ToolKind::from_key(&rec.tool)
            .and_then(|k| crate::install::local_hash(k, &rec.target_path).ok());

        let hit = find_remote(&rec.name, rec.source.repo_path.as_deref(), &remote);
        let remote_hash = hit.map(|r| r.hash.clone());
        let remote_version = hit.and_then(|r| r.version.clone());

        // 本地导入的技能没有远端可比，不参与远端缺失判定
        let status = if rec.source.repo_path.is_none() {
            UpdateStatus::UpToDate
        } else {
            judge_record(&rec.installed_hash, local.as_deref(), remote_hash.as_deref())
        };

        out.push(UpdateCheckItem {
            name: rec.name.clone(),
            tool: rec.tool.clone(),
            status,
            installed_version: rec.installed_version.clone(),
            remote_version,
        });
    }
    Ok(out)
}

// ========== 发布 / 历史 / 回滚 ==========

/// 发布一个本地技能到仓库（直推 master）
#[tauri::command]
pub async fn publish_skill(
    state: State<'_, AppState>,
    req: crate::publish::PublishRequest,
) -> Result<crate::publish::PublishResult, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };
    crate::publish::publish(cfg, req)
        .await
        .map_err(|e| e.to_string())
}

/// 查某个技能的历史版本
#[tauri::command]
pub async fn skill_history(
    state: State<'_, AppState>,
    repo_path: String,
    limit: Option<usize>,
) -> Result<Vec<crate::gitlab::SkillRevision>, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };
    let client = crate::gitlab::GitLabClient::new(cfg).map_err(|e| e.to_string())?;
    client
        .skill_history(&repo_path, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())
}

/// 发布前快照列表
#[tauri::command]
pub fn list_snapshots() -> Result<Vec<crate::publish::Snapshot>, String> {
    crate::publish::list_snapshots().map_err(|e| e.to_string())
}

/// 回滚：把某个历史版本重新发布到 master
///
/// 实现方式不是 git revert，而是「用旧版本内容覆盖当前版本」——
/// 对使用者来说效果相同，且不依赖 git 历史操作的复杂性。
#[tauri::command]
pub async fn rollback_skill(
    state: State<'_, AppState>,
    repo_path: String,
    target_sha: String,
    publisher: String,
    skill_name: String,
) -> Result<crate::publish::PublishResult, String> {
    let cfg = {
        let guard = lock(&state)?;
        build_gitlab_config(&guard.settings)?
    };

    let client = crate::gitlab::GitLabClient::new(cfg.clone()).map_err(|e| e.to_string())?;

    // 下载目标版本
    let zip_bytes = client
        .download_skill_at(&repo_path, &target_sha)
        .await
        .map_err(|e| e.to_string())?;

    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    crate::commands::market::extract_zip(&zip_bytes, tmp.path()).map_err(|e| e.to_string())?;
    let dir = crate::commands::market::find_skill_dir(tmp.path()).map_err(|e| e.to_string())?;

    let req = crate::publish::PublishRequest {
        local_dir: dir.to_string_lossy().to_string(),
        category: None,
        publisher,
        message: Some(format!(
            "回滚 {skill_name} 到 {}",
            &target_sha[..8.min(target_sha.len())]
        )),
    };

    crate::publish::publish(cfg, req)
        .await
        .map_err(|e| e.to_string())
}

// ========== 诊断 ==========

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub data_dir: String,
    pub state_file_exists: bool,
    pub junction_supported: bool,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    let data_dir = crate::storage::app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let state_file_exists = crate::storage::state_path()
        .map(|p| p.is_file())
        .unwrap_or(false);

    AppInfo {
        name: "Skill Market".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir,
        state_file_exists,
        junction_supported: crate::install::junction_supported(),
    }
}

pub mod market;
