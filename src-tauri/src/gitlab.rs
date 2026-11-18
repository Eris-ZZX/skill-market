//! GitLab API 客户端
//!
//! 与 cc-switch 的根本差异：它只支持 GitHub，且下载地址硬编码为
//! `github.com/{owner}/{repo}/archive/refs/heads/{branch}.zip`。
//!
//! GitLab 的优势：`archive.zip?path=<subpath>` 可以**只下载单个技能目录**，
//! 不像 GitHub 必须下整个仓库。
//!
//! 安全要点（这是 SSRF 边界，改动必须带测试）：
//! - 项目路径必须整段 URL 编码（斜杠 → `%2F`）
//! - 出口断言：URL 拼好后再次校验 scheme / host / path
//! - 子路径禁止 `..`

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// GitLab 连接配置
#[derive(Debug, Clone)]
pub struct GitLabConfig {
    /// 实例地址，如 `https://gitlab.company.com`（不带末尾斜杠）
    pub base_url: String,
    /// 项目路径，如 `group/subgroup/skills-repo`
    pub project_path: String,
    /// 访问令牌（可选：仓库公开可读时不需要）
    pub token: Option<String>,
    /// 默认分支
    pub branch: String,
}

impl GitLabConfig {
    pub fn validate(&self) -> Result<()> {
        let url = url::Url::parse(&self.base_url)
            .with_context(|| format!("GitLab 地址无效: {}", self.base_url))?;
        if url.scheme() != "https" && url.scheme() != "http" {
            bail!("GitLab 地址必须是 http(s)");
        }
        if self.project_path.trim().is_empty() {
            bail!("项目路径不能为空");
        }
        if is_unsafe_path(&self.project_path) {
            bail!("项目路径含非法片段: {}", self.project_path);
        }
        Ok(())
    }

    /// 项目路径的 URL 编码形式（斜杠编成 %2F）
    ///
    /// GitLab 要求 `/projects/:id` 里的 id 是**整段路径编码**，
    /// 直接写 `group/sub/repo` 会被当成三段路径而 404。
    pub fn encoded_project(&self) -> String {
        self.project_path.replace('/', "%2F")
    }

    fn api_base(&self) -> String {
        format!("{}/api/v4", self.base_url.trim_end_matches('/'))
    }
}

    /// 仓库中的一个技能条目
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RemoteSkill {
        /// 技能名（frontmatter 的 name，回退到目录名）
        pub name: String,
        /// 仓库内子路径，如 `skills/销售/sales-report`
        pub repo_path: String,
        /// 该技能目录内容的哈希
        pub hash: String,
        /// 版本号（仅展示）
        pub version: Option<String>,
        pub description: String,
        pub tags: Vec<String>,
        pub category: Option<String>,
        /// 最近一次改动时间（ISO 串，展示用）
        pub last_commit_at: Option<String>,
    }

    /// 某个技能目录的历史版本
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SkillRevision {
        pub sha: String,
        pub short_sha: String,
        pub title: String,
        pub author_name: String,
        pub author_email: Option<String>,
        pub created_at: String,
    }

/// GitLab 客户端
pub struct GitLabClient {
    cfg: GitLabConfig,
    http: reqwest::Client,
}

impl GitLabClient {
    pub fn new(cfg: GitLabConfig) -> Result<Self> {
        cfg.validate()?;
        // reqwest 用的是 rustls-tls-native-roots-no-provider，
        // 必须显式安装 ring 作为加密 provider（不能依赖 aws-lc-rs：
        // 后者含 NASM 汇编，在内网 DLP 环境下会被加密拦截）。
        ensure_crypto_provider();
        let http = reqwest::Client::builder()
            .user_agent("skill-market/0.1")
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { cfg, http })
    }

    fn req(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let mut b = self.http.request(method, url);
        if let Some(t) = &self.cfg.token {
            // GitLab 支持 PRIVATE-TOKEN 头
            b = b.header("PRIVATE-TOKEN", t);
        }
        b
    }

    /// 取仓库默认分支的最新 commit SHA
    pub async fn head_commit(&self) -> Result<String> {
        let url = format!(
            "{}/projects/{}/repository/commits?ref_name={}&per_page=1",
            self.cfg.api_base(),
            self.cfg.encoded_project(),
            encode_query(&self.cfg.branch),
        );
        let resp = self.req(reqwest::Method::GET, &url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("取 head commit 失败（HTTP {status}）: {}", truncate(&body, 300));
        }
        #[derive(Deserialize)]
        struct Commit {
            id: String,
        }
        let list: Vec<Commit> = resp.json().await?;
        list.into_iter()
            .next()
            .map(|c| c.id)
            .ok_or_else(|| anyhow::anyhow!("仓库为空或分支 {} 不存在", self.cfg.branch))
    }

    /// 下载单个技能目录的 zip
    ///
    /// ⚠️ **重要：不直接使用 GitLab 的 `archive.zip?path=` 参数。**
    ///
    /// 实测发现（GitLab 自建实例，Master 分支）：
    /// - `path=README.md` → 200，2571 字节 ✅
    /// - `path=skills` → 200，5025 字节 ✅
    /// - `path=skills/通用` → **404** ❌
    /// - `path=skills/通用/grill-me` → **404** ❌
    /// - 即使正确 URL 编码 `path=skills/%E9%80%9A%E7%94%A8/...` → 200 但**内容 0 字节**
    ///
    /// 也就是说 GitLab 的 path 过滤对含中文的路径有缺陷。
    /// 而整仓下载完全正常（中文目录都在）。
    ///
    /// 因此改为：下载整仓归档，在内存里只取出目标子目录的文件重新打包。
    /// 代价是多下载整仓，收益是中文路径能正常工作 —— 中文分类是本项目的
    /// 硬需求，不能妥协。
    pub async fn download_skill_zip(&self, repo_path: &str, sha: &str) -> Result<Vec<u8>> {
        assert_safe_subpath(repo_path)?;

        let full = self.download_repo_zip(sha).await?;
        extract_subdir_as_zip(&full, repo_path)
    }

    /// 下载整个仓库的 zip（用于一次性建立索引）
    pub async fn download_repo_zip(&self, sha: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/projects/{}/repository/archive.zip?sha={}",
            self.cfg.api_base(),
            self.cfg.encoded_project(),
            encode_query(sha),
        );
        self.assert_archive_url(&url, "")?;

        let resp = self.req(reqwest::Method::GET, &url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("下载仓库归档失败（HTTP {status}）: {}", truncate(&body, 300));
        }

        const MAX_BYTES: u64 = 256 * 1024 * 1024; // 256 MB
        let mut out = Vec::new();
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await? {
            if out.len() as u64 + chunk.len() as u64 > MAX_BYTES {
                bail!("仓库归档超过 256MB 上限");
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// 查询某个技能目录的提交历史
    ///
    /// 用 `path=` 参数过滤，只返回影响该目录的提交。
    /// 这是「版本历史」和「回滚」的基础。
    pub async fn skill_history(&self, repo_path: &str, limit: usize) -> Result<Vec<SkillRevision>> {
        let url = format!(
            "{}/projects/{}/repository/commits?ref_name={}&path={}&per_page={}",
            self.cfg.api_base(),
            self.cfg.encoded_project(),
            encode_query(&self.cfg.branch),
            encode_query(repo_path),
            limit.clamp(1, 100),
        );
        let resp = self.req(reqwest::Method::GET, &url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("取技能历史失败（HTTP {status}）: {}", truncate(&body, 300));
        }

        #[derive(Deserialize)]
        struct Commit {
            id: String,
            short_id: String,
            title: String,
            author_name: String,
            author_email: Option<String>,
            created_at: String,
        }
        let list: Vec<Commit> = resp.json().await?;
        Ok(list
            .into_iter()
            .map(|c| SkillRevision {
                sha: c.id,
                short_sha: c.short_id,
                title: c.title,
                author_name: c.author_name,
                author_email: c.author_email,
                created_at: c.created_at,
            })
            .collect())
    }

    /// 按指定 commit 下载单个技能目录（回滚用）
    ///
    /// 与 `download_skill_zip` 相同，只是 sha 由调用方指定而非 head。
    pub async fn download_skill_at(&self, repo_path: &str, sha: &str) -> Result<Vec<u8>> {
        self.download_skill_zip(repo_path, sha).await
    }

    /// 下载整个仓库某个 commit 的归档（用于建立历史版本的索引）
    pub async fn download_repo_at(&self, sha: &str) -> Result<Vec<u8>> {
        self.download_repo_zip(sha).await
    }

    /// 一次提交多个文件到 master（发布用）
    ///
    /// 用 Commits API 的 `actions` 数组，单次请求提交所有文件，
    /// 避免多次往返产生中间状态。
    ///
    /// 注意：GitLab 18.4.6 起该端点**只从 JSON body 读参数**，
    /// 放 query string 会返回 400。
    pub async fn commit_files(
        &self,
        repo_path: &str,
        files: &[(String, Vec<u8>)],
        message: &str,
        publisher: &str,
    ) -> Result<String> {
        assert_safe_subpath(repo_path)?;
        if files.is_empty() {
            bail!("没有要提交的文件");
        }

        // 先查该路径下已存在的文件，决定每个 action 是 create 还是 update
        let existing = self.list_dir_files(repo_path).await.unwrap_or_default();

        #[derive(Serialize)]
        struct Action<'a> {
            action: &'a str,
            file_path: String,
            content: String,
            encoding: &'a str,
        }

        let actions: Vec<Action> = files
            .iter()
            .map(|(rel, bytes)| {
                let full = format!("{repo_path}/{rel}");
                let is_update = existing.contains(&full);
                Action {
                    action: if is_update { "update" } else { "create" },
                    file_path: full,
                    // 二进制安全：统一走 base64
                    content: base64_encode(bytes),
                    encoding: "base64",
                }
            })
            .collect();

        let body = serde_json::json!({
            "branch": self.cfg.branch,
            "commit_message": message,
            "actions": actions,
        });

        let url = format!(
            "{}/projects/{}/repository/commits",
            self.cfg.api_base(),
            self.cfg.encoded_project()
        );
        let resp = self
            .req(reqwest::Method::POST, &url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "提交失败（HTTP {status}）: {}\n提示：若提示 protected branch，\
                 说明 bot 权限不足，需要 Maintainer 级 Token",
                truncate(&text, 400)
            );
        }

        #[derive(Deserialize)]
        struct CommitResp {
            id: String,
        }
        let parsed: CommitResp =
            serde_json::from_str(&text).context("解析提交响应失败")?;
        log::info!(
            "已提交 {} 个文件到 {}@{}（发布人 {publisher}）",
            files.len(),
            repo_path,
            &parsed.id[..8.min(parsed.id.len())]
        );
        Ok(parsed.id)
    }

    /// 列出某个目录下已存在的文件（递归）
    async fn list_dir_files(&self, repo_path: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/projects/{}/repository/tree?ref={}&path={}&recursive=true&per_page=100",
            self.cfg.api_base(),
            self.cfg.encoded_project(),
            encode_query(&self.cfg.branch),
            encode_query(repo_path),
        );
        let resp = self.req(reqwest::Method::GET, &url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        #[derive(Deserialize)]
        struct Entry {
            path: String,
            #[serde(rename = "type")]
            kind: String,
        }
        let list: Vec<Entry> = resp.json().await.unwrap_or_default();
        Ok(list
            .into_iter()
            .filter(|e| e.kind == "blob")
            .map(|e| e.path)
            .collect())
    }

    /// 出口断言：确认 URL 确实指向配置的 GitLab 实例的 archive 端点
    ///
    /// 纵深防御：即便上面的字符校验有遗漏，这里也能拦住落点被改写的请求。
    fn assert_archive_url(&self, url: &str, expected_subpath: &str) -> Result<()> {
        let parsed = url::Url::parse(url).context("归档 URL 解析失败")?;
        let base = url::Url::parse(&self.cfg.base_url)?;

        if parsed.scheme() != base.scheme() {
            bail!("归档 URL scheme 与配置不一致");
        }
        if parsed.host_str() != base.host_str() {
            bail!("归档 URL host 与配置不一致");
        }
        if parsed.port_or_known_default() != base.port_or_known_default() {
            bail!("归档 URL 端口与配置不一致");
        }

        let expected_prefix = format!(
            "/api/v4/projects/{}/repository/archive",
            self.cfg.encoded_project()
        );
        if !parsed.path().starts_with(&expected_prefix) {
            bail!("归档 URL 路径不在预期端点下");
        }

        // sha 与 path 之外的 query 参数一律拒绝
        for (k, _) in parsed.query_pairs() {
            if k != "sha" && k != "path" {
                bail!("归档 URL 含意外参数: {k}");
            }
        }

        if !expected_subpath.is_empty() {
            let actual = parsed
                .query_pairs()
                .find(|(k, _)| k == "path")
                .map(|(_, v)| v.to_string());
            if actual.as_deref() != Some(expected_subpath) {
                bail!("归档 URL 的 path 参数与请求不一致");
            }
        }
        Ok(())
    }
}

/// 安装 rustls 的 ring crypto provider（幂等）
///
/// `reqwest` 用 `rustls-tls-native-roots-no-provider` feature 时不带默认
/// provider，不显式装的话首次建连会 panic `No provider set`。
pub fn ensure_crypto_provider() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // 已装过（例如别处先装了）会返回 Err，忽略即可
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// 路径片段安全检查：禁止 `..`、绝对路径、空段
fn is_unsafe_path(p: &str) -> bool {
    p.split('/').any(|seg| seg == ".." || seg == "." || seg.is_empty())
        || p.starts_with('/')
        || p.contains('\\')
        || p.contains('\0')
}

/// 子路径断言（用于 archive 的 path 参数）
pub fn assert_safe_subpath(p: &str) -> Result<()> {
    if p.is_empty() {
        bail!("子路径不能为空");
    }
    if is_unsafe_path(p) {
        bail!("子路径含非法片段: {p}");
    }
    Ok(())
}

fn encode_query(s: &str) -> String {
    // 简化实现：只编码会破坏 query 的字符
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('#', "%23")
        .replace('+', "%2B")
        .replace(' ', "%20")
}

/// 从整仓归档里取出指定子目录，重新打包成新的 zip
///
/// 归档结构为 `<repo>-<sha前8>-<sha>/skills/通用/grill-me/...`，
/// 需要先剥掉顶层目录，再匹配目标子路径。
///
/// 输出的 zip 保持「带一层根目录」的形态，与直接下载子目录的行为一致，
/// 这样上层 `find_skill_dir` 不需要区分两种来源。
pub fn extract_subdir_as_zip(full_zip: &[u8], repo_path: &str) -> Result<Vec<u8>> {
    use std::io::Write;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(full_zip))
        .context("打开仓库归档失败")?;

    // 先把所有条目读进内存（归档总量已在上层限制）
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        std::io::copy(&mut entry, &mut buf)?;
        entries.push((name, buf));
    }

    // 剥掉顶层 `<repo>-<sha>/` 前缀
    let stripped: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .filter_map(|(name, data)| {
            name.split_once('/')
                .map(|(_, rest)| (rest.to_string(), data))
        })
        .collect();

    if stripped.is_empty() {
        bail!("归档为空或结构异常");
    }

    // 筛选目标子目录
    let prefix = format!("{}/", repo_path.trim_end_matches('/'));
    let matched: Vec<&(String, Vec<u8>)> = stripped
        .iter()
        .filter(|(n, _)| n.starts_with(&prefix))
        .collect();

    if matched.is_empty() {
        bail!(
            "仓库里找不到 {}（归档中共 {} 个文件）",
            repo_path,
            stripped.len()
        );
    }

    // 重新打包：保留一层根目录，与 GitLab 原生子目录归档形态一致
    let root = "skill/";
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        for (name, data) in matched {
            let rel = name.strip_prefix(&prefix).unwrap_or(name);
            // 跳过隐藏文件与字节码（与哈希/发布的排除规则一致）
            if rel.split('/').any(|s| s.starts_with('.') || s == "__pycache__") {
                continue;
            }
            if rel.ends_with(".pyc") {
                continue;
            }
            let entry_name = format!("{root}{rel}");
            w.start_file(entry_name, opts)?;
            w.write_all(data)?;
        }
        w.finish()?;
    }

    Ok(out)
}

/// 最小 base64 编码（避免为这一个用途引入额外依赖）
/// GitLab Commits API 用 `encoding: base64` 可以安全传输任意二进制，
/// 避免 text 模式下的编码问题。
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GitLabConfig {
        GitLabConfig {
            base_url: "https://gitlab.company.com".into(),
            project_path: "group/sub/skills".into(),
            token: None,
            branch: "main".into(),
        }
    }

    #[test]
    fn project_path_is_fully_encoded() {
        assert_eq!(cfg().encoded_project(), "group%2Fsub%2Fskills");
    }

    #[test]
    fn rejects_path_traversal_in_project_path() {
        let mut c = cfg();
        c.project_path = "group/../../etc".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        let mut c = cfg();
        c.base_url = "file:///etc/passwd".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_unsafe_subpath() {
        assert!(assert_safe_subpath("skills/../../etc").is_err());
        assert!(assert_safe_subpath("/absolute").is_err());
        assert!(assert_safe_subpath("skills/./x").is_err());
        assert!(assert_safe_subpath("skills//x").is_err());
        assert!(assert_safe_subpath("").is_err());
        assert!(assert_safe_subpath("skills/销售/sales-report").is_ok());
    }

    #[test]
    fn archive_url_assertion_accepts_correct_url() {
        let client = GitLabClient::new(cfg()).unwrap();
        let url = "https://gitlab.company.com/api/v4/projects/group%2Fsub%2Fskills/repository/archive.zip?sha=abc&path=skills%2Fx";
        // sha/path 参数会被 decode，这里只验证端点与 host
        assert!(client
            .assert_archive_url(url, "skills/x")
            .is_ok());
    }

    #[test]
    fn archive_url_assertion_rejects_wrong_host() {
        let client = GitLabClient::new(cfg()).unwrap();
        let url = "https://evil.com/api/v4/projects/group%2Fsub%2Fskills/repository/archive.zip?sha=abc";
        assert!(client.assert_archive_url(url, "").is_err());
    }

    #[test]
    fn archive_url_assertion_rejects_extra_params() {
        let client = GitLabClient::new(cfg()).unwrap();
        let url = "https://gitlab.company.com/api/v4/projects/group%2Fsub%2Fskills/repository/archive.zip?sha=abc&redirect=https://evil.com";
        assert!(client.assert_archive_url(url, "").is_err());
    }

    #[test]
    fn archive_url_assertion_rejects_wrong_endpoint() {
        let client = GitLabClient::new(cfg()).unwrap();
        let url = "https://gitlab.company.com/api/v4/projects/group%2Fsub%2Fskills/repository/files/secret/raw?sha=abc";
        assert!(client.assert_archive_url(url, "").is_err());
    }
}
