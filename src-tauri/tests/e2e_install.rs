//! 端到端：从真实 GitLab 下载并安装到工具目录
//!
//! 用手工构造的假工具根目录，避免污染开发者真实的 `~/.cursor` 等目录。
//!
//! ```powershell
//! $env:SKILL_MARKET_TEST_BASE="https://gitlab.example.com"
//! $env:SKILL_MARKET_TEST_PROJECT="group/skills-repo"
//! $env:SKILL_MARKET_TEST_TOKEN="<your-token>"
//! cargo test --test e2e_install -- --ignored --nocapture
//! ```

use skill_market_lib::gitlab::{GitLabClient, GitLabConfig};
use skill_market_lib::install::local_hash;
use skill_market_lib::state::InstallMode;
use skill_market_lib::tools::{assert_writable, ToolKind};
use std::path::{Path, PathBuf};

/// 从环境变量构造配置；未设置时返回 None
fn cfg() -> Option<(GitLabConfig, String)> {
    let base = std::env::var("SKILL_MARKET_TEST_BASE").ok()?;
    let project = std::env::var("SKILL_MARKET_TEST_PROJECT").ok()?;
    let skill_path = std::env::var("SKILL_MARKET_TEST_SKILL_PATH")
        .unwrap_or_else(|_| "skills/通用/grill-me".into());
    Some((
        GitLabConfig {
            base_url: base,
            project_path: project,
            token: std::env::var("SKILL_MARKET_TEST_TOKEN").ok(),
            branch: std::env::var("SKILL_MARKET_TEST_BRANCH")
                .unwrap_or_else(|_| "master".into()),
        },
        skill_path,
    ))
}

/// 下载并解压技能，返回 (临时目录守卫, 技能目录)
async fn fetch_skill(cfg: GitLabConfig, repo_path: &str) -> (tempfile::TempDir, PathBuf) {
    let client = GitLabClient::new(cfg).expect("client");
    let sha = client.head_commit().await.expect("head");
    let bytes = client
        .download_skill_zip(repo_path, &sha)
        .await
        .expect("下载技能");

    let tmp = tempfile::tempdir().expect("tmpdir");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("open zip");
    for i in 0..archive.len() {
        let mut e = archive.by_index(i).expect("entry");
        let Some(p) = e.enclosed_name() else { continue };
        let out = tmp.path().join(p);
        if e.is_dir() {
            std::fs::create_dir_all(&out).ok();
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut f = std::fs::File::create(&out).expect("create");
            std::io::copy(&mut e, &mut f).expect("copy");
        }
    }

    let dir = find_skill_dir(tmp.path()).expect("找到技能目录");
    (tmp, dir)
}

fn find_skill_dir(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 5 {
            return None;
        }
        if dir.join("SKILL.md").is_file() {
            return Some(dir.to_path_buf());
        }
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            if e.path().is_dir() {
                if let Some(f) = walk(&e.path(), depth + 1) {
                    return Some(f);
                }
            }
        }
        None
    }
    walk(root, 0)
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn download_skill_has_expected_structure() {
    let Some((cfg, path)) = cfg() else {
        eprintln!("跳过：未设置 SKILL_MARKET_TEST_BASE / SKILL_MARKET_TEST_PROJECT");
        return;
    };
    let (_guard, dir) = fetch_skill(cfg, &path).await;

    println!("技能目录: {}", dir.display());
    let files = walkdir(&dir);
    for f in &files {
        println!("  {f}");
    }

    assert!(dir.join("SKILL.md").is_file(), "必须有 SKILL.md");
    let content = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
    assert!(content.contains("name:"), "frontmatter 应有 name");
}

fn walkdir(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn rec(base: &Path, cur: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(cur) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    rec(base, &p, out);
                } else {
                    out.push(
                        p.strip_prefix(base)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
    }
    rec(dir, dir, &mut out);
    out
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn install_to_fake_tool_creates_correct_files() {
    let Some((cfg, path)) = cfg() else {
        eprintln!("跳过：未设置环境变量");
        return;
    };
    let (_guard, skill_dir) = fetch_skill(cfg, &path).await;

    // 造假的工具根目录，模拟 ~/.cursor
    let fake_home = tempfile::tempdir().expect("fake home");
    let tool_root = fake_home.path().join(".cursor");
    std::fs::create_dir_all(tool_root.join("skills")).expect("skills 目录");

    let target = tool_root.join("skills").join("grill-me");

    // 1) 受保护目录断言
    assert!(
        assert_writable(ToolKind::Cursor, &tool_root.join("skills-cursor").join("x")).is_err(),
        "skills-cursor 应被拒绝"
    );
    assert!(assert_writable(ToolKind::Cursor, &target).is_ok());
    println!("受保护目录断言正确");

    // 2) 复制落盘
    copy_dir(&skill_dir, &target).expect("复制");
    assert!(target.join("SKILL.md").is_file());
    println!("安装后文件:");
    for f in walkdir(&target) {
        println!("  {f}");
    }

    // 3) 哈希可计算
    let h = local_hash(ToolKind::Cursor, &target.to_string_lossy()).expect("算哈希");
    println!("安装后哈希 = {h}");
    assert_eq!(h.len(), 64, "sha256 应为 64 个十六进制字符");
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[test]
fn install_mode_enum_serializes_as_lowercase() {
    let copy = serde_json::to_string(&InstallMode::Copy).unwrap();
    let junction = serde_json::to_string(&InstallMode::Junction).unwrap();
    println!("Copy={copy} Junction={junction}");
    assert_eq!(copy, "\"copy\"");
    assert_eq!(junction, "\"junction\"");
}
