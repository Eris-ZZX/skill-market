//! 更新链路端到端（本地模拟，不依赖网络）
//!
//! `update_skill` 把「取远端内容」与「落盘」拆成两步：`download_skill_package`
//! 走网络，`deploy_downloaded` 只管落盘。本文件直接构造 `DownloadedSkill`
//! 指向本地目录，把后者（哈希重算、部署、回滚、mode 处理）跑通；
//! 需要验证命令层守卫时用 `tauri::test::mock_app` 构造 `State`；
//! 需要走完整下载流程时用一个本地 HTTP 服务器冒充 GitLab。
//!
//! `deploy_downloaded` 内部用真实的 `storage::library_dir()`，所以整个二进制
//! 通过 `XDG_CONFIG_HOME` 把应用数据目录重定向到临时目录，绝不碰
//! 用户真实的 `~/.config`。

use skill_market_lib::commands::market::{deploy_downloaded, DownloadedSkill};
use skill_market_lib::commands::{update_skill, AppState};
use skill_market_lib::hash;
use skill_market_lib::install::{
    backup_library, deploy_to_paths, is_link, junction_supported, restore_backup_as,
    symlink_supported,
};
use skill_market_lib::state::{InstalledSkill, InstallMode, SkillSource, SourceKind};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;
use tauri::Manager;

static INIT: Once = Once::new();

/// 把应用数据目录指到临时位置，并返回 `library` 根
fn library_root() -> PathBuf {
    INIT.call_once(|| {
        let base = std::env::temp_dir().join(format!("skill-market-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // Linux 上 dirs::config_dir() 读 XDG_CONFIG_HOME
        std::env::set_var("XDG_CONFIG_HOME", &base);
    });
    let root = skill_market_lib::storage::library_dir().unwrap();
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

/// 在 `parent/<name>/` 造一个技能目录
///
/// 目录名必须与技能名一致：`deploy_to_paths` 用目录名作为库条目名，
/// 而 `deploy_downloaded` 用显式传入的 name——不一致就找不到库。
fn make_skill(parent: &Path, name: &str, version: &str) -> PathBuf {
    let d = parent.join(name);
    write(
        &d,
        "SKILL.md",
        &format!("---\nname: {name}\ndescription: d\nversion: {version}\n---\n{version}\n"),
    );
    write(&d, "scripts/run.py", &format!("print('{version}')\n"));
    d
}

/// 构造指向本地目录的「下载结果」，完全不走网络
fn fake_download(extracted: &Path, version: &str) -> DownloadedSkill {
    DownloadedSkill {
        dir: tempfile::tempdir().unwrap(),
        extracted: extracted.to_path_buf(),
        version: Some(version.to_string()),
        commit: "0".repeat(40),
    }
}

/// 链接类安装方式；环境都不支持时返回 None
fn link_mode() -> Option<InstallMode> {
    if junction_supported() {
        Some(InstallMode::Junction)
    } else if symlink_supported() {
        Some(InstallMode::Symlink)
    } else {
        None
    }
}

// ========== 落盘层（不走网络） ==========

#[test]
fn 场景1_装v1后部署v2_内容变新且哈希改变() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("tool").join("skills").join("e2e-s1");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s1", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let old_hash = hash::hash_dir(&lib.join("e2e-s1")).unwrap();
    assert!(read(&target.join("SKILL.md")).contains("1.0.0"));

    let v2 = make_skill(&work.path().join("v2"), "e2e-s1", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    let (new_hash, ver) =
        deploy_downloaded(&dl, "e2e-s1", &[(target.clone(), InstallMode::Copy)]).unwrap();

    assert!(read(&target.join("SKILL.md")).contains("2.0.0"), "副本应变新");
    assert!(
        read(&lib.join("e2e-s1").join("SKILL.md")).contains("2.0.0"),
        "库应变新"
    );
    assert_ne!(new_hash, old_hash, "内容变了哈希必须变");
    assert_eq!(
        new_hash,
        hash::hash_dir(&lib.join("e2e-s1")).unwrap(),
        "返回的哈希应等于新库哈希"
    );
    assert_eq!(ver.as_deref(), Some("2.0.0"));
}

#[test]
fn 场景5_两个工具混合模式更新后都变新() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let copy_target = work.path().join("a").join("skills").join("e2e-s5");
    let link_target = work.path().join("b").join("skills").join("e2e-s5");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s5", "1.0.0");
    deploy_to_paths(&v1, &lib, &[copy_target.clone()], InstallMode::Copy).unwrap();

    let Some(lm) = link_mode() else {
        eprintln!("跳过：当前环境既不能建 junction 也不能建软链");
        return;
    };
    deploy_to_paths(&v1, &lib, &[link_target.clone()], lm).unwrap();

    let v2 = make_skill(&work.path().join("v2"), "e2e-s5", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(
        &dl,
        "e2e-s5",
        &[
            (copy_target.clone(), InstallMode::Copy),
            (link_target.clone(), lm),
        ],
    )
    .unwrap();

    assert!(
        read(&copy_target.join("SKILL.md")).contains("2.0.0"),
        "复制副本应变新"
    );
    assert!(
        read(&link_target.join("SKILL.md")).contains("2.0.0"),
        "链接目标应跟随变新"
    );
}

#[test]
fn 场景6_第二个目标失败时第一个的原内容不消失() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let t1 = work.path().join("a").join("skills").join("e2e-s6");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s6", "1.0.0");
    deploy_to_paths(&v1, &lib, &[t1.clone()], InstallMode::Copy).unwrap();
    let before = read(&t1.join("SKILL.md"));
    assert!(before.contains("1.0.0"));

    // 让第二个目标在「准备阶段」失败（父路径被普通文件占住）。
    // 此时第一个目标还没开始替换，磁盘本就干净——这条验证「前置校验失败
    // 不会留下半套安装」。替换阶段的回滚由 install.rs 的 rollback_ 单测覆盖，
    // 那里直接构造「已替换」状态再调 rollback。
    let blocker = work.path().join("c").join("skills");
    fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    fs::write(&blocker, "not a dir").unwrap();

    let v2 = make_skill(&work.path().join("v2"), "e2e-s6", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    let r = deploy_downloaded(
        &dl,
        "e2e-s6",
        &[
            (t1.clone(), InstallMode::Copy),
            (blocker.join("e2e-s6"), InstallMode::Copy),
        ],
    );

    assert!(r.is_err(), "第二个目标失败应让整体失败");
    assert!(t1.join("SKILL.md").is_file(), "第一个目标不能被删空");
    assert_eq!(
        read(&t1.join("SKILL.md")),
        before,
        "第一个目标必须保持更新前的内容"
    );
}

#[test]
fn 场景7_备份能恢复出当时的内容且不带_meta_json() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join("e2e-s7");
    let backups = work.path().join("backups");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s7", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let v1_hash = hash::hash_dir(&lib.join("e2e-s7")).unwrap();

    // 更新到 v2
    let v2 = make_skill(&work.path().join("v2"), "e2e-s7", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(&dl, "e2e-s7", &[(target.clone(), InstallMode::Copy)]).unwrap();
    assert!(read(&lib.join("e2e-s7").join("SKILL.md")).contains("2.0.0"));

    // 单独造一份 v1 的库，备份它，验证能恢复回 v1
    let snap_lib = work.path().join("snap-lib");
    deploy_to_paths(&v1, &snap_lib, &[], InstallMode::Copy).ok();
    let bak = backup_library(&snap_lib, "e2e-s7", &backups, None).unwrap();

    let restored = restore_backup_as(&bak, &lib, "e2e-s7-restored").unwrap();

    // 内容（正文与脚本）必须与 v1 一致。注意不能直接比指纹：
    // restore_backup_as 会改写 frontmatter 的 name（#29 的实现，避免副本
    // 与正本在「我的技能」里按 name 聚合成一行），那一步本身就会改变指纹。
    assert_eq!(
        read(&restored.join("scripts/run.py")),
        read(&snap_lib.join("e2e-s7").join("scripts/run.py")),
        "脚本内容应与备份一致"
    );
    assert!(
        read(&restored.join("SKILL.md")).contains("version: 1.0.0"),
        "应是 v1 的内容而不是 v2"
    );
    assert!(
        read(&restored.join("SKILL.md")).contains("name: e2e-s7-restored"),
        "恢复出的副本应带上自己的名字"
    );
    assert_ne!(
        hash::hash_dir(&restored).unwrap(),
        hash::hash_dir(&lib.join("e2e-s7")).unwrap(),
        "恢复出的副本不该与当前 v2 相同"
    );
    let _ = v1_hash;
    assert!(
        !restored.join("meta.json").exists(),
        "meta.json 是元数据，不该混进技能内容"
    );
}

#[test]
fn 场景8_只改易变字段指纹不变() {
    // #19 的核心：enabled_at 这类工具回写的字段必须被剥离，
    // 否则跑一次技能就会被判成「本地已修改」。
    let work = tempfile::tempdir().unwrap();
    let a = work.path().join("a");
    let b = work.path().join("b");
    write(
        &a,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: d\nversion: 1.0.0\n---\nbody\n",
    );
    write(
        &b,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: d\nversion: 1.0.0\nenabled_at: 1787556778796\n---\nbody\n",
    );

    let ha = hash::hash_dir(&a).unwrap();
    assert_eq!(
        ha,
        hash::hash_dir(&b).unwrap(),
        "只加 enabled_at 不应改变指纹"
    );

    // 对照：真实内容改动必须改变指纹
    write(
        &b,
        "SKILL.md",
        "---\nname: e2e-s8\ndescription: 改过\nversion: 1.0.0\n---\nbody\n",
    );
    assert_ne!(ha, hash::hash_dir(&b).unwrap(), "真实内容改动必须改变指纹");
}

#[test]
fn 场景7补充_链接模式下更新不破坏链接() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join("e2e-s7b");

    let Some(mode) = link_mode() else {
        eprintln!("跳过：当前环境无法创建目录链接");
        return;
    };

    let v1 = make_skill(&work.path().join("v1"), "e2e-s7b", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], mode).unwrap();
    assert!(is_link(&target), "应为链接");

    let v2 = make_skill(&work.path().join("v2"), "e2e-s7b", "2.0.0");
    let dl = fake_download(&v2, "2.0.0");
    deploy_downloaded(&dl, "e2e-s7b", &[(target.clone(), mode)]).unwrap();

    assert!(is_link(&target), "更新不该把链接换成实体目录");
    assert!(read(&target.join("SKILL.md")).contains("2.0.0"));
}

// ========== 命令层守卫（需要 tauri::State） ==========

fn app_with_base(rec: InstalledSkill, base_url: &str) -> tauri::App<tauri::test::MockRuntime> {
    let app = tauri::test::mock_app();
    let st = AppState::load();
    {
        let mut g = st.inner.lock().unwrap();
        g.settings.gitlab_base_url = base_url.into();
        g.settings.gitlab_project_path = "g/p".into();
        g.installs.push(rec);
    }
    app.manage(st);
    app
}

fn app_with(rec: InstalledSkill) -> tauri::App<tauri::test::MockRuntime> {
    app_with_base(rec, "https://example.invalid")
}

fn remote_rec(name: &str, library_hash: &str, target_path: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.into(),
        tool: "claude".into(),
        target_path: target_path.into(),
        mode: InstallMode::Copy,
        installed_hash: library_hash.into(),
        installed_version: Some("1.0.0".into()),
        source: SkillSource {
            kind: SourceKind::Gitlab,
            repo_url: Some("https://example.invalid/g/p".into()),
            repo_path: Some(format!("skills/通用/{name}")),
            branch: Some("master".into()),
            commit: Some("abc".into()),
        },
        installed_at: 1700000000,
        ignored_remote_hash: None,
    }
}

#[tokio::test]
async fn 场景2_本地改过时更新被拒绝且文件未变() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join("e2e-s2");

    let v1 = make_skill(&work.path().join("v1"), "e2e-s2", "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let installed_hash = hash::hash_dir(&lib.join("e2e-s2")).unwrap();

    // 用户手改库里的内容
    fs::write(
        lib.join("e2e-s2").join("SKILL.md"),
        "---\nname: e2e-s2\ndescription: 我改了\nversion: 1.0.0\n---\n手改\n",
    )
    .unwrap();
    let modified = read(&lib.join("e2e-s2").join("SKILL.md"));

    let app = app_with(remote_rec(
        "e2e-s2",
        &installed_hash,
        &target.to_string_lossy(),
    ));
    let st = app.state::<AppState>();

    let err = update_skill(st, "e2e-s2".into(), "claude".into())
        .await
        .expect_err("本地改过必须被拒绝");

    assert!(err.contains("本地"), "错误信息应说明是本地改动: {err}");
    assert_eq!(
        read(&lib.join("e2e-s2").join("SKILL.md")),
        modified,
        "被拒绝时不得改动文件"
    );
}

#[tokio::test]
async fn 场景3_库目录不存在时报错且不回退旧指纹() {
    let lib = library_root();
    let work = tempfile::tempdir().unwrap();

    let app = app_with(remote_rec(
        "e2e-s3-gone",
        "some-old-hash",
        &work.path().join("t").join("e2e-s3-gone").to_string_lossy(),
    ));
    let st = app.state::<AppState>();

    assert!(
        !lib.join("e2e-s3-gone").exists(),
        "前置条件：库目录必须确实不存在"
    );

    let err = update_skill(st, "e2e-s3-gone".into(), "claude".into())
        .await
        .expect_err("目录不在必须报错");

    assert!(
        err.contains("本地技能目录不存在"),
        "错误信息应明确指出目录缺失（而不是静默用旧指纹）: {err}"
    );
}

#[tokio::test]
async fn 场景3补充_未安装的工具上报明确错误() {
    let work = tempfile::tempdir().unwrap();
    let app = app_with(remote_rec(
        "e2e-s3b",
        "h",
        &work.path().join("t").to_string_lossy(),
    ));
    let st = app.state::<AppState>();
    let err = update_skill(st, "e2e-s3b".into(), "cursor".into())
        .await
        .expect_err("没装在这个工具上应报错");
    assert!(err.contains("未安装"), "应说明未安装在该工具: {err}");
}

// ========== 场景 4：下载期间被卸载，不得复活 ==========

/// 构造 GitLab 整仓归档形态的 zip：顶层 `<repo>-<sha>/` 下放技能
fn repo_zip(sha: &str, skill_name: &str, version: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let root = format!("repo-master-{sha}/skills/通用/{skill_name}/");
        w.start_file(format!("{root}SKILL.md"), opts).unwrap();
        w.write_all(
            format!("---\nname: {skill_name}\ndescription: d\nversion: {version}\n---\n{version}\n")
                .as_bytes(),
        )
        .unwrap();
        w.start_file(format!("{root}scripts/run.py"), opts).unwrap();
        w.write_all(format!("print('{version}')\n").as_bytes())
            .unwrap();
        w.finish().unwrap();
    }
    out
}

/// 极简 GitLab mock：只实现 head_commit 与 archive.zip
fn spawn_mock_gitlab(sha: String, zip: Vec<u8>, delay_ms: u64) -> String {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let sha = sha.clone();
            let zip = zip.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    return;
                }
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.trim().is_empty() => break,
                        Ok(_) => {}
                    }
                }

                if request_line.contains("repository/commits") {
                    let json = format!(
                        r#"[{{"id":"{sha}","short_id":"deadbeef","title":"t","author_name":"a","author_email":null,"created_at":"2025-01-01T00:00:00Z"}}]"#
                    );
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        json.len(),
                        json
                    );
                } else if request_line.contains("archive.zip") {
                    // 故意慢一点，给测试留出「下载期间卸载」的窗口
                    if delay_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        zip.len()
                    );
                    let _ = stream.write_all(&zip);
                } else {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
                let _ = stream.flush();
            });
        }
    });

    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn 场景4_下载期间被卸载时不得复活() {
    const NAME: &str = "e2e-s4";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    let lib = library_root();
    let work = tempfile::tempdir().unwrap();
    let target = work.path().join("t").join("skills").join(NAME);

    let v1 = make_skill(&work.path().join("v1"), NAME, "1.0.0");
    deploy_to_paths(&v1, &lib, &[target.clone()], InstallMode::Copy).unwrap();
    let installed_hash = hash::hash_dir(&lib.join(NAME)).unwrap();

    // mock GitLab，archive 延迟 800ms
    let base = spawn_mock_gitlab(SHA.to_string(), repo_zip(SHA, NAME, "2.0.0"), 800);

    let mut rec = remote_rec(NAME, &installed_hash, &target.to_string_lossy());
    rec.source.repo_url = Some(format!("{base}/g/p"));
    let app = app_with_base(rec, &base);
    let st = app.state::<AppState>();

    // 并发：等 300ms（此时已在下载中），删掉记录模拟用户卸载
    let handle = app.handle().clone();
    let uninstaller = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let st2 = handle.state::<AppState>();
        let mut g = st2.inner.lock().unwrap();
        g.installs.retain(|r| r.name != NAME);
        g.installs.len()
    });

    let err = update_skill(st, NAME.into(), "claude".into())
        .await
        .expect_err("下载期间被卸载，更新必须拒绝");
    let remaining = uninstaller.await.unwrap();

    assert!(err.contains("已被卸载"), "错误信息应说明技能已被卸载: {err}");
    assert_eq!(remaining, 0, "记录应确实已被删除");

    // 最关键：光看报错不够，要证明技能没被复活
    let st_after = app.state::<AppState>();
    let g = st_after.inner.lock().unwrap();
    assert!(
        !g.installs.iter().any(|r| r.name == NAME),
        "已卸载的技能不得被下载结果重新写回记录"
    );
    assert!(
        read(&lib.join(NAME).join("SKILL.md")).contains("1.0.0"),
        "库内容不该被替换成远端版本"
    );
}
