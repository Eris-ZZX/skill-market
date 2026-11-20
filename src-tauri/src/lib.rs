//! 内网 Skill 市场 — 后端库
//!
//! 从 cc-switch 提取的技能管理子系统，剥离了 provider 切换 / 代理 / MCP /
//! 会话管理等一切与技能无关的功能。
//!
//! 模块划分：
//! - `tools`       目标工具注册表（路径探测、安装目标准备）
//! - `hash`        内容哈希（更新检测 / 部署完整性）
//! - `frontmatter` SKILL.md 解析
//! - `state`       安装状态与四态判定
//! - `gitlab`      GitLab API 客户端（含 SSRF 断言）
//! - `storage`     本地状态持久化
//! - `install`     部署引擎（复制 / junction）
//! - `commands`    Tauri 命令层

/// 读取 WebView2 Runtime 版本号
///
/// 注册表位置（微软官方文档给的判定方式）：
/// - 机器级：HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv
/// - 用户级：HKCU\Software\Microsoft\EdgeUpdate\Clients\{同上}\pv
///
/// 两处都没有就是没装。返回 None 表示未安装。
#[cfg(windows)]
fn detect_webview2_version() -> Option<String> {
    use std::process::Command;

    const GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let paths = [
        format!(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{GUID}"),
        format!(r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{GUID}"),
        format!(r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{GUID}"),
    ];

    for key in paths {
        let out = Command::new("reg")
            .args(["query", &key, "/v", "pv"])
            .output()
            .ok()?;
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // 输出形如：    pv    REG_SZ    157.0.4341.46
        if let Some(line) = text.lines().find(|l| l.contains("REG_SZ")) {
            if let Some(v) = line.split_whitespace().last() {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_webview2_version() -> Option<String> {
    Some("non-windows".into())
}

/// 缺 WebView2 时给出可操作的中文提示，而不是静默白屏
#[cfg(windows)]
fn show_webview2_missing_dialog() {
    use std::process::Command;
    let msg = "Skill Market 需要 Microsoft Edge WebView2 运行时，但当前系统没有安装。\\n\\n\
               解决方法：\\n\
               1. 从内网共享盘获取 MicrosoftEdgeWebView2RuntimeInstallerX64.exe 并安装\\n\
               2. 或联系 IT 协助安装\\n\\n\
               安装完成后重新启动本程序。\\n\\n\
               说明：Windows 11 自带该组件；Windows 10 部分版本需要单独安装。";
    // 用 PowerShell 弹窗，避免在窗口创建失败时依赖 GUI 库
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &format!(
                "Add-Type -AssemblyName System.Windows.Forms; \
                 [System.Windows.Forms.MessageBox]::Show('{msg}', 'Skill Market 无法启动', \
                 [System.Windows.Forms.MessageBoxButtons]::OK, \
                 [System.Windows.Forms.MessageBoxIcon]::Warning)"
            ),
        ])
        .spawn();
}

#[cfg(not(windows))]
fn show_webview2_missing_dialog() {}

use tauri::Manager;

pub mod commands;
pub mod frontmatter;
pub mod gitlab;
pub mod hash;
pub mod install;
pub mod publish;
pub mod scan;
pub mod state;
pub mod storage;
pub mod tools;

/// 把窗口调整到当前显示器工作区内的可见位置
///
/// 处理三种情况：
/// 1. 窗口尺寸大于工作区 → 缩到工作区的 92%
/// 2. 窗口位置在屏幕外 → 移回居中
/// 3. 窗口过小（异常状态）→ 恢复到合理尺寸
fn fit_window_to_screen<R: tauri::Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> tauri::Result<()> {
    let Some(monitor) = win.current_monitor()?.or(win.primary_monitor()?) else {
        return Ok(());
    };

    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);
    let pos = monitor.position().to_logical::<f64>(scale);

    // 工作区预留一点边距（任务栏、Dock 等）
    let avail_w = (screen.width - 40.0).max(400.0);
    let avail_h = (screen.height - 80.0).max(300.0);

    let size = win.inner_size()?.to_logical::<f64>(scale);
    let loc = win.outer_position()?.to_logical::<f64>(scale);

    let min_w = 640.0_f64.min(avail_w);
    let min_h = 420.0_f64.min(avail_h);

    // 尺寸异常（过小或超出工作区）时重置
    let needs_resize = size.width < 320.0
        || size.height < 240.0
        || size.width > avail_w
        || size.height > avail_h;

    if needs_resize {
        let w = 760.0_f64.min(avail_w).max(min_w);
        let h = 480.0_f64.min(avail_h).max(min_h);
        win.set_size(tauri::LogicalSize::new(w, h))?;
        log::info!("窗口尺寸已校正为 {w}x{h}（工作区 {avail_w}x{avail_h}）");
    }

    // 位置校正：只要标题栏可能不可见就重新居中
    let new_size = win.inner_size()?.to_logical::<f64>(scale);
    let out_x = loc.x;
    let out_y = loc.y;
    let offscreen = out_x < pos.x - 8.0
        || out_y < pos.y - 8.0
        || out_x + new_size.width > pos.x + screen.width + 8.0
        || out_y + new_size.height > pos.y + screen.height + 8.0;

    if offscreen {
        let cx = pos.x + (screen.width - new_size.width) / 2.0;
        let cy = pos.y + (screen.height - new_size.height) / 2.0;
        win.set_position(tauri::LogicalPosition::new(
            cx.max(pos.x),
            cy.max(pos.y),
        ))?;
        log::info!("窗口已重新居中到 ({cx:.0}, {cy:.0})");
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 必须在任何 HTTP 请求前安装，否则 reqwest 首次建连会 panic
    gitlab::ensure_crypto_provider();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::load())
        .setup(|app| {
            // 日志：写到 %APPDATA%/skill-market/logs
            let log_dir = storage::app_data_dir()
                .map(|d| d.join("logs"))
                .unwrap_or_else(|_| std::env::temp_dir().join("skill-market-logs"));
            let _ = std::fs::create_dir_all(&log_dir);

            let target = tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::LogDir { file_name: None },
            );
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .targets([target, tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout)])
                    .level(log::LevelFilter::Info)
                    .build(),
            )?;

            log::info!("Skill Market 启动");

            // WebView2 检测
            //
            // tauri.conf 用的是 webviewInstallMode = "skip"（不打包运行时，
            // 依赖系统已装），好处是安装包只有 ~20MB。代价是：
            // 若目标机器没有 WebView2，主窗口会白屏且无任何提示。
            //
            // 实测：Win11 自带；Win10 22H2 多数已被微软推送；
            // Win10 LTSC 2019 / Server / 干净安装 可能没有。
            if let Some(ver) = detect_webview2_version() {
                log::info!("WebView2 Runtime: {ver}");
            } else {
                log::error!("未检测到 WebView2 Runtime，界面将无法渲染");
                show_webview2_missing_dialog();
            }

            // 窗口校正：确保窗口完整落在当前显示器的工作区内。
            //
            // 为什么需要：换显示器、改分辨率、或上次异常退出时，窗口可能
            // 被恢复到屏幕外，导致标题栏（含最小化/最大化/关闭三大键）不可见。
            // 这里主动量一次屏幕尺寸并夹取，比依赖持久化的窗口状态更可靠。
            if let Some(win) = app.get_webview_window("main") {
                if let Err(e) = fit_window_to_screen(&win) {
                    log::warn!("窗口校正失败: {e}");
                }
                // 显式显示 + 置前。某些情况下（如从托盘恢复、或窗口
                // 创建时焦点被其他程序抢走）窗口会停在隐藏状态。
                if !win.is_visible().unwrap_or(false) {
                    log::info!("窗口未显示，主动 show");
                    let _ = win.show();
                }
                let _ = win.set_focus();
            }

            // 首次启动：确保数据目录存在
            if let Ok(dir) = storage::app_data_dir() {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    log::error!("创建数据目录失败: {e}");
                } else {
                    log::info!("数据目录: {}", dir.display());
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 工具探测
            commands::detect_tools,
            // 设置
            commands::get_settings,
            commands::save_settings,
            // 技能市场
            commands::refresh_index,
            commands::list_remote_skills,
            // 安装管理
            commands::list_installed,
            commands::scan_skills,
            commands::adopt_skill,
            commands::state_health,
            commands::install_skill,
            commands::update_skill,
            commands::resolve_conflict,
            commands::uninstall_skill,
            commands::check_updates,
            // 发布 / 历史 / 回滚
            commands::publish_skill,            commands::skill_history,
            commands::list_snapshots,
            commands::rollback_skill,
            // 诊断
            commands::app_info,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
