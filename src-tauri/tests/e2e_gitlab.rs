//! 端到端验证：对接真实 GitLab 实例
//!
//! **默认被 ignore**，因为它依赖内网环境。手动运行：
//!
//! ```powershell
//! $env:SKILL_MARKET_TEST_BASE="https://gitlab.example.com"
//! $env:SKILL_MARKET_TEST_PROJECT="group/skills-repo"
//! $env:SKILL_MARKET_TEST_BRANCH="master"
//! $env:SKILL_MARKET_TEST_TOKEN="<your-token>"
//! cargo test --test e2e_gitlab -- --ignored --nocapture
//! ```
//!
//! 所有连接参数都从环境变量读取，仓库里不出现任何真实内网信息。

use skill_market_lib::gitlab::{GitLabClient, GitLabConfig};

struct TestEnv {
    base: String,
    project: String,
    branch: String,
    token: Option<String>,
    /// 用于测试「单技能下载」的技能在仓库内的路径
    skill_path: String,
}

fn env() -> Option<TestEnv> {
    let base = std::env::var("SKILL_MARKET_TEST_BASE").ok()?;
    let project = std::env::var("SKILL_MARKET_TEST_PROJECT").ok()?;
    Some(TestEnv {
        base,
        project,
        branch: std::env::var("SKILL_MARKET_TEST_BRANCH")
            .unwrap_or_else(|_| "master".into()),
        token: std::env::var("SKILL_MARKET_TEST_TOKEN").ok(),
        skill_path: std::env::var("SKILL_MARKET_TEST_SKILL_PATH")
            .unwrap_or_else(|_| "skills/通用/grill-me".into()),
    })
}

fn cfg() -> Option<GitLabConfig> {
    let e = env()?;
    Some(GitLabConfig {
        base_url: e.base,
        project_path: e.project,
        token: e.token,
        branch: e.branch,
    })
}

macro_rules! require_env {
    () => {
        match env() {
            Some(e) => e,
            None => {
                eprintln!("跳过：未设置 SKILL_MARKET_TEST_BASE / SKILL_MARKET_TEST_PROJECT");
                return;
            }
        }
    };
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn head_commit_is_reachable() {
    require_env!();
    let client = GitLabClient::new(cfg().unwrap()).expect("client");
    let sha = client.head_commit().await.expect("取 head commit");
    println!("head commit = {sha}");
    assert_eq!(sha.len(), 40, "GitLab commit SHA 应为 40 字符");
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn repo_archive_downloads_and_parses() {
    require_env!();
    let client = GitLabClient::new(cfg().unwrap()).expect("client");
    let sha = client.head_commit().await.expect("head");
    let bytes = client.download_repo_zip(&sha).await.expect("下载整仓");
    println!("归档大小 = {} 字节", bytes.len());
    assert!(bytes.len() > 100, "归档不应为空");
    assert_eq!(&bytes[0..2], b"PK", "应为 zip 格式");

    let archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("打开 zip");
    println!("归档内共 {} 个条目", archive.len());

    // GitLab 归档应带一层 `<repo>-<sha8>-<sha>/` 前缀
    let mut archive = archive;
    let first = archive.by_index(0).expect("entry").name().to_string();
    println!("根前缀形态 = {first}");
    assert!(first.contains('/'), "归档应有顶层目录前缀");
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn single_skill_download_works_with_chinese_path() {
    let e = require_env!();
    let client = GitLabClient::new(cfg().unwrap()).expect("client");
    let sha = client.head_commit().await.expect("head");

    // 注意：GitLab 的 archive?path= 对中文路径有缺陷（实测 404），
    // 客户端内部改为「下整仓 + 本地切分」，所以这里能成功。
    let bytes = client
        .download_skill_zip(&e.skill_path, &sha)
        .await
        .expect("下载单技能");

    println!("单技能归档 = {} 字节", bytes.len());
    assert!(bytes.len() > 100);
    assert_eq!(&bytes[0..2], b"PK");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("zip");
    let mut found = false;
    for i in 0..archive.len() {
        let name = archive.by_index(i).expect("entry").name().to_string();
        if name.ends_with("SKILL.md") {
            found = true;
            println!("找到 SKILL.md: {name}");
        }
    }
    assert!(found, "单技能归档里应含 SKILL.md");
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn path_traversal_is_rejected_before_network() {
    require_env!();
    let client = GitLabClient::new(cfg().unwrap()).expect("client");
    let sha = client.head_commit().await.expect("head");
    let err = client
        .download_skill_zip("skills/../../../etc/passwd", &sha)
        .await
        .expect_err("应拒绝路径穿越");
    println!("正确拒绝: {err}");
}

#[tokio::test]
#[ignore = "依赖真实 GitLab，需设置环境变量后手动运行"]
async fn skill_history_is_queryable() {
    let e = require_env!();
    let client = GitLabClient::new(cfg().unwrap()).expect("client");
    let history = client
        .skill_history(&e.skill_path, 5)
        .await
        .expect("查历史");
    println!("历史提交数 = {}", history.len());
    for h in &history {
        println!("  {} {} {}", h.short_sha, h.author_name, h.title);
    }
}
