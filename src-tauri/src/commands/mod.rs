//! Tauri 命令层
//!
//! 前端只能通过这里的命令访问后端能力。
//! 所有命令返回 `Result<T, String>`，错误直接展示给用户。

use crate::state::{
    judge_record, ConflictChoice, InstallMode, InstalledSkill, SkillSource, SourceKind,
    UpdateStatus,
};
use crate::storage::{AppSettings, PersistedState};
use crate::tools::{self, ToolDetection, ToolKind};
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

/// 应用运行时状态
pub struct AppState {
    pub inner: Mutex<PersistedState>,
    /// 状态文件健康提示；None 表示正常
    pub health: Mutex<Option<String>>,
}

impl AppState {
    pub fn load() -> Self {
        let (state, health) = match crate::storage::load() {
            crate::storage::LoadOutcome::Ok(s) => (s, None),
            crate::storage::LoadOutcome::Recovered { state, from } => (
                state,
                Some(format!(
                    "安装记录文件已损坏，已备份并从上次备份恢复（{}）",
                    from.display()
                )),
            ),
            crate::storage::LoadOutcome::Corrupt { backup } => (
                PersistedState::default(),
                Some(format!(
                    "安装记录文件已损坏，已备份为 {}。当前列表来自磁盘扫描，安装记录需要重新纳入。",
                    backup.display()
                )),
            ),
        };
        Self {
            inner: Mutex::new(state),
            health: Mutex::new(health),
        }
    }
}

fn lock<'a>(s: &'a State<'_, AppState>) -> Result<std::sync::MutexGuard<'a, PersistedState>, String> {
    s.inner.lock().map_err(|_| "状态锁已损坏".to_string())
}

/// 状态文件的健康提示；None 表示一切正常
#[tauri::command]
pub fn state_health(state: State<'_, AppState>) -> Option<String> {
    state.health.lock().ok().and_then(|g| g.clone())
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

/// 「我的技能」里的一行（按技能名聚合）
///
/// 数据来自磁盘扫描，`state.json` 只用来补齐来源与安装时指纹。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MySkillItem {
    pub name: String,
    pub tools: Vec<String>,
    pub target_paths: Vec<String>,
    pub description: String,
    pub version: Option<String>,
    /// managed | external | local
    pub identity: String,
    /// 仅 managed 有值：其余身份没有可比的来源，判不了就不猜
    pub status: Option<UpdateStatus>,
    pub installed_version: Option<String>,
}

/// 身份判定
///
/// 优先级 managed > external > local。external 还需目标路径不在本应用
/// 数据目录下——指向自己 library 的重解析点是本客户端自己的 junction。
pub fn judge_identity(
    target_path: &str,
    tool: &str,
    is_link: bool,
    link_target: Option<&str>,
    installs: &[InstalledSkill],
    app_data: Option<&str>,
) -> &'static str {
    let managed = installs
        .iter()
        .any(|r| r.tool == tool && r.target_path == target_path);
    if managed {
        return "managed";
    }
    if is_link {
        // 只看到软链但读不到目标时，保守地当成外部管理：不该动它
        let outside = match (link_target, app_data) {
            (Some(t), Some(base)) => !t.starts_with(base),
            (None, _) => true,
            (Some(_), None) => true,
        };
        if outside {
            return "external";
        }
    }
    "local"
}

/// 扫描本机所有技能，按技能名聚合成「我的技能」列表
#[tauri::command]
pub fn scan_skills(state: State<'_, AppState>) -> Result<Vec<MySkillItem>, String> {
    let scanned = crate::scan::scan_all();
    let installs = lock(&state)?.installs.clone();
    let app_data = crate::storage::app_data_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    Ok(build_my_skills(&scanned, &installs, app_data.as_deref()))
}

/// 把扫描结果与安装记录合并成按名聚合的列表
pub fn build_my_skills(
    scanned: &[crate::scan::ScannedSkill],
    installs: &[InstalledSkill],
    app_data: Option<&str>,
) -> Vec<MySkillItem> {
    let mut order: Vec<String> = Vec::new();
    let mut by_name: std::collections::HashMap<String, MySkillItem> = Default::default();

    for s in scanned {
        let identity = judge_identity(
            &s.target_path,
            &s.tool,
            s.is_link,
            s.link_target.as_deref(),
            installs,
            app_data,
        );
        let rec = (identity == "managed")
            .then(|| {
                installs
                    .iter()
                    .find(|r| r.tool == s.tool && r.target_path == s.target_path)
            })
            .flatten();

        let entry = by_name.entry(s.name.clone()).or_insert_with(|| {
            order.push(s.name.clone());
            MySkillItem {
                name: s.name.clone(),
                tools: Vec::new(),
                target_paths: Vec::new(),
                description: s.description.clone(),
                version: s.version.clone(),
                identity: "local".into(),
                status: None,
                installed_version: None,
            }
        });

        entry.tools.push(s.tool.clone());
        entry.target_paths.push(s.target_path.clone());
        if entry.description.is_empty() {
            entry.description = s.description.clone();
        }
        if entry.version.is_none() {
            entry.version = s.version.clone();
        }

        // 聚合时取最高优先级：managed > external > local
        let rank = |id: &str| match id {
            "managed" => 2,
            "external" => 1,
            _ => 0,
        };
        if rank(identity) > rank(&entry.identity) {
            entry.identity = identity.to_string();
        }
        if let Some(r) = rec {
            entry.installed_version = r.installed_version.clone();
        }
    }

    order
        .into_iter()
        .filter_map(|n| by_name.remove(&n))
        .collect()
}

/// 构造一条「纳入管理」的安装记录
///
/// 只写本地来源，四个远端字段全为 None——猜来源比留空更糟：
/// 猜错会让后续的更新判定去覆盖一个其实来自别处的技能。
pub fn adopt_record(name: String, tool: String, target_path: String) -> Result<InstalledSkill, String> {
    let dir = std::path::Path::new(&target_path);
    if !dir.join("SKILL.md").is_file() {
        return Err(format!("{target_path} 下没有 SKILL.md"));
    }
    let installed_hash = crate::hash::hash_dir(dir).map_err(|e| e.to_string())?;
    let meta = crate::frontmatter::SkillMeta::from_dir(dir).ok();

    Ok(InstalledSkill {
        name,
        tool,
        target_path,
        mode: InstallMode::Copy,
        installed_hash,
        installed_version: meta.as_ref().and_then(|m| m.version.clone()),
        source: SkillSource {
            kind: SourceKind::Local,
            repo_url: None,
            repo_path: None,
            branch: None,
            commit: None,
        },
        installed_at: chrono::Utc::now().timestamp(),
        ignored_remote_hash: None,
    })
}

/// 把扫描到的「本地已有」技能纳入管理
#[tauri::command]
pub fn adopt_skill(
    state: State<'_, AppState>,
    name: String,
    tool: String,
    target_path: String,
) -> Result<InstalledSkill, String> {
    let rec = adopt_record(name, tool, target_path)?;

    let mut guard = lock(&state)?;
    if guard
        .installs
        .iter()
        .any(|r| r.name == rec.name && r.tool == rec.tool)
    {
        return Err(format!("{} 已存在于 {} 的安装记录中", rec.name, rec.tool));
    }
    guard.installs.push(rec.clone());
    crate::storage::save(&guard).map_err(|e| e.to_string())?;
    Ok(rec)
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
    mode: Option<InstallMode>,
) -> Result<Vec<InstalledSkill>, String> {
    let (cfg, chosen) = {
        let guard = lock(&state)?;
        (
            build_gitlab_config(&guard.settings)?,
            // 未显式指定时用设置里的默认方式——这个字段过去一直是死字段
            mode.unwrap_or(guard.settings.default_install_mode),
        )
    };

    let installed = crate::commands::market::install_from_repo(
        cfg,
        &name,
        &repo_path,
        &tools,
        chosen,
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

/// 更新前的校验（纯函数：不碰文件系统、不碰 Tauri）
///
/// 放在下载**之前**，让「本地改过」这类必然失败的请求立刻返回，
/// 不必等几十秒下载完才被拒。
///
/// `local_hash` 为 None 表示本地目录读不到——绝不能回退旧指纹，
/// 否则目录被删会被误判成「本地没改」，更新时覆盖掉用户的改动。
pub fn validate_update_preflight(
    name: &str,
    tool: &str,
    snapshot: &[InstalledSkill],
    local_hash: Option<&str>,
) -> Result<InstalledSkill, String> {
    let target_rec = snapshot
        .iter()
        .find(|r| r.tool == tool)
        .ok_or_else(|| format!("{name} 未安装在 {tool} 上"))?
        .clone();

    // 本地导入的技能没有远端可比
    if target_rec.source.repo_path.is_none() {
        return Err("本地导入的技能无法更新（没有远端来源）".to_string());
    }

    // 后面所有路径都用 name 拼，校验必须在此之前
    crate::install::require_valid_directory(name).map_err(|e| e.to_string())?;

    let Some(local_hash) = local_hash else {
        return Err("本地技能目录不存在，无法判断是否有改动".to_string());
    };
    if local_hash != target_rec.installed_hash {
        return Err("本地有改动，为避免覆盖你的修改已拒绝更新。请先处理本地改动（或卸载后重装）".into());
    }
    Ok(target_rec)
}

/// 下载完成后的校验（纯函数）
///
/// 下载耗时几十秒，期间用户可能已卸载或重装。必须在任何文件操作之前
/// 重新确认每条记录都还在且没被改动，否则会把已卸载的技能重新装回来。
pub fn verify_records_unchanged(
    snapshot: &[InstalledSkill],
    current: &[InstalledSkill],
) -> Result<(), String> {
    for snap in snapshot {
        let now = current
            .iter()
            .find(|r| r.name == snap.name && r.tool == snap.tool)
            .ok_or_else(|| format!("{} 已被卸载，更新取消", snap.name))?;
        if now.installed_hash != snap.installed_hash
            || now.installed_at != snap.installed_at
            || now.target_path != snap.target_path
        {
            return Err("下载期间安装记录已变化，请重试".into());
        }
    }
    Ok(())
}

/// 更新一个已安装技能到远端最新版
///
/// SSOT 下同名技能只存一份库内容，所以一次更新要同步该技能的所有工具记录。
#[tauri::command]
pub async fn update_skill(
    state: State<'_, AppState>,
    name: String,
    tool: String,
) -> Result<InstalledSkill, String> {
    // 1. 取快照：(cfg, 该技能所有工具记录)
    let (cfg, snapshot) = {
        let guard = lock(&state)?;
        let cfg = build_gitlab_config(&guard.settings)?;
        let recs: Vec<InstalledSkill> = guard
            .installs
            .iter()
            .filter(|r| r.name == name)
            .cloned()
            .collect();
        (cfg, recs)
    };

    // 2-4. 本地哈希与前置校验
    let library_root = crate::storage::library_dir().map_err(|e| e.to_string())?;
    let local_hash = crate::hash::hash_dir(&library_root.join(&name)).ok();
    let target_rec =
        validate_update_preflight(&name, &tool, &snapshot, local_hash.as_deref())?;
    let repo_path = target_rec
        .source
        .repo_path
        .clone()
        .ok_or_else(|| "本地导入的技能无法更新（没有远端来源）".to_string())?;

    // 5. 下载新版本到临时目录（客户端已有 60 秒传输层超时）
    let targets: Vec<(std::path::PathBuf, InstallMode)> = snapshot
        .iter()
        .map(|r| (std::path::PathBuf::from(&r.target_path), r.mode))
        .collect();

    let downloaded = crate::commands::market::download_skill_package(cfg, &repo_path)
        .await
        .map_err(|e| e.to_string())?;

    // 6.【必做】下载期间记录可能已变；在任何文件操作之前重新确认
    {
        let guard = lock(&state)?;
        verify_records_unchanged(&snapshot, &guard.installs)?;
    }

    // 7. 备份当前库内容；备份是兜底而非前置条件，失败只记日志
    if let Ok(app_data) = crate::storage::app_data_dir() {
        let meta = crate::install::BackupMeta {
            name: name.clone(),
            tool: tool.clone(),
            installed_hash: target_rec.installed_hash.clone(),
            installed_version: target_rec.installed_version.clone(),
            repo_path: Some(repo_path.clone()),
            backup_created_at: chrono::Utc::now().timestamp(),
            source_path: String::new(),
        };
        if let Err(e) = crate::install::backup_library(
            &library_root,
            &name,
            &crate::install::backups_root(&app_data),
            Some(&meta),
        ) {
            log::warn!("更新前备份 {name} 失败: {e}");
        }
    }

    // 8-9. 替换库并按记录各自的 mode 处理工具目录
    let (new_hash, new_version) =
        crate::commands::market::deploy_downloaded(&downloaded, &name, &targets)
            .map_err(|e| e.to_string())?;

    // 10. 同名技能可能装在多个工具，库只有一份，全部记录一起更新
    let mut updated = None;
    {
        let mut guard = lock(&state)?;
        let now = chrono::Utc::now().timestamp();
        for rec in guard.installs.iter_mut().filter(|r| r.name == name) {
            rec.installed_hash = new_hash.clone();
            rec.installed_version = new_version.clone();
            rec.installed_at = now;
            if rec.tool == tool {
                updated = Some(rec.clone());
            }
        }
        crate::storage::save(&guard).map_err(|e| e.to_string())?;
    }

    updated.ok_or_else(|| "更新后找不到记录".to_string())
}

/// 处理「本地改过」与「本地改过 + 远端也改」两种状态
///
/// 三个分支共用一套落盘动作，区别只在最后怎么摆放文件：
/// 保留我的＝什么都不做，用远端＝备份后覆盖，另存一份＝改名后装回官方版。
#[tauri::command]
pub async fn resolve_conflict(
    state: State<'_, AppState>,
    name: String,
    tool: String,
    choice: ConflictChoice,
) -> Result<InstalledSkill, String> {
    let (cfg, snapshot) = {
        let guard = lock(&state)?;
        let cfg = build_gitlab_config(&guard.settings)?;
        let recs: Vec<InstalledSkill> = guard
            .installs
            .iter()
            .filter(|r| r.name == name)
            .cloned()
            .collect();
        (cfg, recs)
    };

    let target_rec = snapshot
        .iter()
        .find(|r| r.tool == tool)
        .ok_or_else(|| format!("{name} 未安装在 {tool} 上"))?
        .clone();
    let repo_path = target_rec
        .source
        .repo_path
        .clone()
        .ok_or_else(|| "本地导入的技能没有远端版本可对比".to_string())?;

    crate::install::require_valid_directory(&name).map_err(|e| e.to_string())?;
    let library_root = crate::storage::library_dir().map_err(|e| e.to_string())?;
    let app_data = crate::storage::app_data_dir().map_err(|e| e.to_string())?;
    let backups = crate::install::backups_root(&app_data);

    // 保留我的：不动任何文件，只记下「忽略到哪个远端哈希」
    if choice == ConflictChoice::KeepMine {
        let remote_hash = fetch_remote_hash(&cfg, &repo_path).await?;
        let mut guard = lock(&state)?;
        let rec = guard
            .installs
            .iter_mut()
            .find(|r| r.name == name && r.tool == tool)
            .ok_or_else(|| format!("{name} 的安装记录已不存在"))?;
        rec.ignored_remote_hash = Some(remote_hash);
        let out = rec.clone();
        crate::storage::save(&guard).map_err(|e| e.to_string())?;
        return Ok(out);
    }

    // 后两个分支都要拿远端内容，先把包下下来
    let downloaded = crate::commands::market::download_skill_package(cfg, &repo_path)
        .await
        .map_err(|e| e.to_string())?;

    // 下载期间记录可能变化，与 update_skill 同样要在写盘前重新确认
    {
        let guard = lock(&state)?;
        let current = guard
            .installs
            .iter()
            .find(|r| r.name == name && r.tool == tool)
            .ok_or_else(|| format!("{name} 已被卸载，操作取消"))?;
        if current.target_path != target_rec.target_path
            || current.installed_hash != target_rec.installed_hash
        {
            return Err("下载期间安装记录已变化，请重试".into());
        }
    }

    // 两个分支都会先备份当前库——用户的改动不能无声消失
    let meta = crate::install::BackupMeta {
        name: name.clone(),
        tool: tool.clone(),
        installed_hash: target_rec.installed_hash.clone(),
        installed_version: target_rec.installed_version.clone(),
        repo_path: Some(repo_path.clone()),
        backup_created_at: chrono::Utc::now().timestamp(),
        source_path: String::new(),
    };
    let backup_dir = crate::install::backup_library(&library_root, &name, &backups, Some(&meta))
        .map_err(|e| format!("备份本地版本失败，已中止以避免丢失改动: {e}"))?;

    match choice {
        ConflictChoice::UseRemote => {
            // 备份已在上面落盘，这里直接覆盖
            apply_remote(&downloaded, &name, &snapshot)?;
            let hash = crate::hash::hash_dir(&library_root.join(&name))
                .map_err(|e| e.to_string())?;
            write_after_resolve(
                &state,
                &name,
                &hash,
                downloaded.version.clone(),
                &snapshot,
                false,
            )
            .map(|r| {
                log::info!("已用远端覆盖 {name}，原内容备份在 {}", backup_dir.display());
                r
            })
        }
        ConflictChoice::KeepBoth => {
            // 当前库改名保留，再放回官方版
            let local_name = crate::install::unique_sibling_name(&library_root, &format!("{name}-local"))
                .map_err(|e| e.to_string())?;
            crate::install::restore_backup_as(&backup_dir, &library_root, &local_name)
                .map_err(|e| e.to_string())?;
            apply_remote(&downloaded, &name, &snapshot)?;
            let hash = crate::hash::hash_dir(&library_root.join(&name))
                .map_err(|e| e.to_string())?;
            write_after_resolve(
                &state,
                &name,
                &hash,
                downloaded.version.clone(),
                &snapshot,
                false,
            )
            .map(|r| {
                log::info!("已保留你的改动为 {local_name}，并装回官方版 {name}");
                r
            })
        }
        ConflictChoice::KeepMine => unreachable!("已在上面提前返回"),
    }
}

/// 只取远端当前哈希，不下载内容
async fn fetch_remote_hash(
    cfg: &crate::gitlab::GitLabConfig,
    repo_path: &str,
) -> Result<String, String> {
    let index = crate::commands::market::fetch_index(cfg.clone())
        .await
        .map_err(|e| format!("无法获取远端索引：{e}"))?;
    index
        .iter()
        .find(|r| r.repo_path == repo_path)
        .map(|r| r.hash.clone())
        .ok_or_else(|| format!("远端已找不到 {repo_path}"))
}

/// 把远端内容写进库，并按记录各自的 mode 处理工具目录
fn apply_remote(
    downloaded: &crate::commands::market::DownloadedSkill,
    name: &str,
    snapshot: &[InstalledSkill],
) -> Result<(), String> {
    let targets: Vec<(std::path::PathBuf, InstallMode)> = snapshot
        .iter()
        .map(|r| (std::path::PathBuf::from(&r.target_path), r.mode))
        .collect();
    crate::commands::market::deploy_downloaded(downloaded, name, &targets)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 覆盖成功后刷新记录：哈希、版本，并清掉之前的忽略标记
fn write_after_resolve(
    state: &State<'_, AppState>,
    name: &str,
    new_hash: &str,
    new_version: Option<String>,
    snapshot: &[InstalledSkill],
    keep_ignored: bool,
) -> Result<InstalledSkill, String> {
    let mut guard = lock(state)?;
    let now = chrono::Utc::now().timestamp();
    let mut out = None;
    for rec in guard.installs.iter_mut().filter(|r| r.name == name) {
        rec.installed_hash = new_hash.to_string();
        rec.installed_version = new_version.clone();
        rec.installed_at = now;
        if !keep_ignored {
            // 内容已经与远端一致，忽略标记没有意义了
            rec.ignored_remote_hash = None;
        }
        if snapshot.iter().any(|s| s.tool == rec.tool) {
            out = Some(rec.clone());
        }
    }
    crate::storage::save(&guard).map_err(|e| e.to_string())?;
    out.ok_or_else(|| "处理后找不到记录".to_string())
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
    /// 用户已对这个远端哈希选了「保留我的」，界面不该再计入待处理
    pub ignored: bool,
}

/// 一次「检查更新」的完整结果
///
/// 远端读取失败时不能整体返回 Err——那样界面上连上次的数据都看不到。
/// 失败放进 `failures` 带回去，已缓存的结果照常返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub items: Vec<UpdateCheckItem>,
    pub failures: Vec<RemoteFailure>,
}

/// 远端读取失败的一条记录
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFailure {
    /// network | auth | notfound | parse | config
    pub kind: String,
    /// 用户可读的中文原因
    pub message: String,
    /// 原始技术信息，供排查
    pub detail: String,
}

/// 把底层错误归类
///
/// 分类只影响文案，不影响任何判定逻辑，所以按特征字符串粗分即可，
/// 不追求精确。认不出来就归为 parse——那至少说明是数据层面的问题。
pub fn classify_remote_failure(detail: &str) -> RemoteFailure {
    let lower = detail.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    let (kind, message) = if has(&[
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "token",
        "令牌",
        "权限",
    ]) {
        ("auth", "访问令牌无效或权限不足")
    } else if has(&["404", "not found", "notfound", "不存在"]) {
        ("notfound", "仓库不存在，请检查项目路径")
    } else if has(&[
        "timeout",
        "timed out",
        "超时",
        "connect",
        "connection",
        "dns",
        "network",
    ]) {
        ("network", "无法连接到 GitLab")
    } else {
        ("parse", "仓库数据解析失败")
    };

    RemoteFailure {
        kind: kind.to_string(),
        message: message.to_string(),
        detail: detail.to_string(),
    }
}

/// 组装一次检查结果
///
/// `remote` 为 None 表示远端读取失败；此时用 `cached` 兜底。
/// 纯函数：不碰网络、不碰 Tauri，方便直接测两条路径。
pub fn assemble_update_result(
    snapshot: &[InstalledSkill],
    remote: Option<&[RemoteSkillView]>,
    cached: &[RemoteSkillView],
    failure: Option<RemoteFailure>,
    local_hash_of: impl Fn(&InstalledSkill) -> Option<String>,
) -> UpdateCheckResult {
    // 有远端就用远端；失败时退回缓存（可能是空的）
    let source: &[RemoteSkillView] = remote.unwrap_or(cached);

    let mut items = Vec::with_capacity(snapshot.len());
    for rec in snapshot {
        let local = local_hash_of(rec);

        let hit = find_remote(&rec.name, rec.source.repo_path.as_deref(), source);
        let remote_hash = hit.map(|r| r.hash.clone());
        let remote_version = hit.and_then(|r| r.version.clone());

        // 本地导入的技能没有远端可比，不参与远端缺失判定
        let status = if rec.source.repo_path.is_none() {
            UpdateStatus::UpToDate
        } else {
            judge_record(&rec.installed_hash, local.as_deref(), remote_hash.as_deref())
        };

        // 忽略标记只抑制提示，不改状态：本地确实还和安装时不同。
        // 远端哈希再变时该标记自然失效（比较的是哈希相等，不是「有过标记」）。
        let ignored = match (&rec.ignored_remote_hash, &remote_hash) {
            (Some(seen), Some(now)) => seen == now,
            _ => false,
        };

        items.push(UpdateCheckItem {
            name: rec.name.clone(),
            tool: rec.tool.clone(),
            status,
            installed_version: rec.installed_version.clone(),
            remote_version,
            ignored,
        });
    }

    UpdateCheckResult {
        items,
        failures: failure.into_iter().collect(),
    }
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
pub async fn check_updates(state: State<'_, AppState>) -> Result<UpdateCheckResult, String> {
    let cfg = {
        let guard = lock(&state)?;
        // 配置缺失是配置问题，不是读取失败——照旧返回 Err 让用户去设置页
        build_gitlab_config(&guard.settings)?
    };

    // 远端读取失败不让整个操作失败：退回上次缓存，把失败原因带回去
    let (remote, failure) = match crate::commands::market::fetch_index(cfg).await {
        Ok(list) => (Some(list), None),
        Err(e) => {
            log::warn!("获取远端索引失败，将退回缓存: {e}");
            (None, Some(classify_remote_failure(&e.to_string())))
        }
    };

    let cached = if remote.is_none() {
        // 缓存读不出来也不能当作「远端没有技能」：那样会把每条记录判成
        // 「远端已删除」。这里退化成空切片 + 已有的 failure 提示。
        crate::commands::market::load_cached_index().unwrap_or_default()
    } else {
        Vec::new()
    };

    let snapshot = lock(&state)?.installs.clone();
    Ok(assemble_update_result(
        &snapshot,
        remote.as_deref(),
        &cached,
        failure,
        |rec| {
            // 读不到就如实传 None，绝不回退到安装时的旧指纹——
            // 否则目录被删或被占用会被误判成「本地没改」，更新时可能覆盖用户手改
            ToolKind::from_key(&rec.tool)
                .and_then(|k| crate::install::local_hash(k, &rec.target_path).ok())
        },
    ))
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
