# cc-switch 参考评估

> 调研日期：2026-11-13
> 参考版本：`86c749b`（2026-11-13，主线最新）
> 本地路径：`_ref-cc-switch/`（`git clone --depth 1`，105 MB，可随时删除）

---

## 一句话结论

**cc-switch 的 skill 子系统可以直接作为改造基础。** 它的核心架构（SSOT 单一事实源 + 多应用分发 + 内容哈希更新检测 + 数据库记录安装状态）与我们的设计**完全一致**，而且已经踩平了大量我们还没遇到的坑。

**而且这台机器上它已经在真实使用中**：40 个技能（28 个来自 Git 仓库、12 个本地），分发到 Claude Code 和 Codex。这不是纸上评估。

**但它有三个硬伤必须改**：只支持 GitHub、不支持 Cursor / 千问办公、用 Tauri（内网 WebView2 风险）。

---

## 1. 基本事实

| 项 | 值 |
|---|---|
| 仓库 | [farion1231/cc-switch](https://github.com/farion1231/cc-switch) |
| Stars | 154,114 |
| 许可证 | **MIT**（可闭源分发、可商用） |
| 技术栈 | Tauri 2 + Rust + React 19 + TypeScript + shadcn/Radix |
| 代码量 | Rust 387 文件 / 13.6 MB；TS 521 文件 / 5.8 MB |
| 定位 | 多 AI CLI 的统一配置管理（provider 切换 + MCP + Skills + Prompts） |
| 更新频率 | 极高频（今日仍有提交） |

### skill 子系统的代码分布

| 文件 | 行数 | 作用 |
|---|---|---|
| `src-tauri/src/services/skill.rs` | **7,937** | 核心：发现、安装、同步、哈希、更新、备份、迁移 |
| `src-tauri/src/commands/skill.rs` | 418 | Tauri command 薄封装 |
| `src-tauri/src/database/dao/skills.rs` | 388 | SQLite DAO |
| `src-tauri/src/workbuddy_config.rs` | 519 | WorkBuddy 专用配置 |
| `src/components/skills/UnifiedSkillsPanel.tsx` | ~78 KB | 主界面（矩阵表） |
| `src/components/skills/SkillsPage.tsx` | ~39 KB | 列表页 |
| `tests/skill_sync.rs` | 14 KB | 同步集成测试 |
| `docs/user-manual/{zh,en,ja}/3-extensions/3.3-skills.md` | ~11-14 KB | 用户手册 |

**注意 `skill.rs` 有 7,937 行**。这不是缺点，是财富——里面绝大部分是已修复的边界问题。

---

## 2. 它的架构（与我们的设计对照）

### 2.1 SSOT + 分发模型 —— 完全一致 ✅

```
~/.cc-switch/skills/          ← SSOT，唯一事实源
        ↓ symlink / copy
~/.claude/skills/     ~/.codex/skills/     ~/.gemini/skills/ ...
```

源码注释原文：

```rust
//! v3.10.0+ 统一管理架构：
//! - SSOT（单一事实源）：`~/.cc-switch/skills/`
//! - 安装时下载到 SSOT，按需同步到各应用目录
//! - 数据库存储安装记录和启用状态
```

对照我们的方案 §7.1（`%APPDATA%\SkillMarket\` + 分发到四个工具目录）——**同构**。

### 2.2 内容哈希更新检测 —— 思路一致，但他们更细 ✅

我们的方案 §7.2 主张"规范化内容 hash"。他们的实现：

```rust
fn compute_dir_hash_with(dir: &Path, skip_bytecode: bool) -> Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    Self::collect_files_for_hash(dir, dir, skip_bytecode, &mut files)?;
    files.sort();                          // ← 排序保证稳定

    let mut hasher = Sha256::new();
    for file_path in &files {
        let relative = file_path.strip_prefix(dir).unwrap_or(file_path);
        let rel_str = relative.to_string_lossy().replace('\\', "/");  // ← 路径归一
        hasher.update(rel_str.as_bytes());
        hasher.update(b"\0");
        let content = fs::read(file_path)?;
        hasher.update(&content);
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}
```

**他们踩过、我们还没踩的坑（全部有源码注释佐证）**：

| 坑 | 他们的处理 | 对应 issue |
|---|---|---|
| Python 运行时生成 `__pycache__` / `*.pyc`，跑一遍技能就被判"有更新" | 哈希时跳过这两类 | #7772 |
| 旧版算法把字节码算进哈希，升级后老数据全成了"有更新" | 保留 `compute_legacy_dir_hash`，专门识别并静默迁移旧哈希 | — |
| 隐藏文件（`.` 开头）不该计入 | `name.starts_with('.')` 直接跳过 | — |
| 路径分隔符 `\` vs `/` 导致跨平台不一致 | `.replace('\\', "/")` | — |
| 文件顺序不稳定导致假更新 | `files.sort()` | — |
| 部署校验需要更严格的比较（区分"用户改过"）| 另有一个 `compute_pi_deployment_hash`，**连隐藏文件、空目录、文件类型都算** | — |

> **重要发现**：他们区分了**两种哈希**。一种是"更新检测哈希"（宽松，忽略隐藏文件和字节码），一种是"部署完整性哈希"（严格，含空目录和符号链接目标）。我们的方案只设计了前者。**后者是我们漏掉的**——判断"用户在工具目录里改过文件没有"时，用宽松哈希会漏掉"用户删了个空目录"这类改动。

### 2.3 安装状态记录 ✅

```rust
pub struct InstalledSkill {
    pub id: String,            // "owner/repo:directory" 或 "local:directory"
    pub name: String,
    pub description: Option<String>,
    pub directory: String,     // SSOT 里的子目录名
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub readme_url: Option<String>,
    pub apps: SkillApps,       // ← 每个应用的启用状态
    pub installed_at: i64,
    pub content_hash: Option<String>,   // ← 装的时候记录
    pub updated_at: i64,
}
```

`SkillApps` 是固定字段的结构体（`claude` / `codex` / `gemini` / `workbuddy` / … 各一个 bool），存 SQLite。

**对照我们的 `state.json`**：我们的设计是"一条记录 = 一个技能装在一个工具里"。他们是"一条记录 = 一个技能，内含各应用开关"。**他们的更好**，因为：
- "同一技能装到 3 个工具"天然是一个概念，不是三条独立记录
- 内容哈希只需存一份（我们的设计要在三条记录里各存一份，容易不一致）
- 矩阵表 UI 直接映射这个结构（行=技能，列=应用）

**建议修改我们的 `state.json` 结构**，采用他们的模型。

### 2.4 分发方式：symlink 优先，失败回退 copy ✅

```rust
pub enum SyncMethod {
    Auto,      // 默认：优先 symlink，失败回退 copy
    Symlink,
    Copy,
}
```

对照我们方案 §6.3 选了"默认 copy，junction 作为高级选项"。

**他们的 `Auto` 更聪明**：先试 symlink，失败自动降级 copy。这正好解决了我们的顾虑（不能假设管理员权限 / 开发者模式）。

**但要注意**：他们主要面向开发者（Claude Code / Codex 用户），这类人机器上开发者模式多半开着。我们的用户是普通办公同事，symlink 大概率失败，实际会全部落到 copy。所以 **`Auto` 是对的，但我们预期结果是 copy**。

### 2.5 每应用路径映射 —— 结构好抄，但内容要改 ⚠️

```rust
pub fn get_app_skills_dir(app: &AppType) -> Result<PathBuf> {
    // 1. 先看用户是否在 settings.json 里配了 override 目录
    // 2. 否则回退到标准路径
}
```

**支持的应用（16 个）**：
`Claude` / `ClaudeDesktop` / `Codex` / `Gemini` / `GrokBuild` / `OpenCode` / `OpenClaw` / `Hermes` / `Pi` / `Mcode` / `Kimi` / `Dsh` / `Agy` / `Omp` / `Copilot` / **`WorkBuddy`**

**关键差异**：

| 我们要的 | cc-switch 现状 | 工作量 |
|---|---|---|
| Cursor | ❌ **完全不支持** | 需新增（`~/.cursor/skills`） |
| ChatGPT | ⚠️ 支持 `Codex`，但路径是 `~/.codex/skills` | **与我们调研的 `~/.agents/skills` 不同**，需确认 |
| 千问办公 | ❌ **完全不支持** | 需新增（`~/.qwenworkcn/skills`） |
| WorkBuddy | ✅ **原生支持** | 直接可用 |

> **ChatGPT 路径分歧要查清**：cc-switch 用 `~/.codex/skills`，官方文档说 Codex/ChatGPT 读 `~/.agents/skills`。可能是版本演进（`~/.agents` 是较新的跨工具约定），也可能两个都读。**M0 必须实测确认**。cc-switch 里 `get_agents_skills_dir()` 的存在说明它也认 `.agents`。

**好消息**：`override` 机制让路径可配置，这正好是我们方案 §2.2（坑 1）要求的"路径表可配置"。

### 2.6 界面 —— 已验证的产品形态 ✅

从用户手册看，他们的 skills 页面是**矩阵表**：

- **每行一个技能，每列一个应用**，格子里的开关 = "在此应用启用"
- 顶部两个视图：**已安装** / **发现**
- 筛选：全部 / 可更新 / 未在任何应用启用 / 在某应用启用 / 按来源仓库
- 搜索：名称、目录、描述、仓库
- 批量：多选后批量启用到某应用 / 停用 / 更新 / 卸载
- 点列头可整列批量开关
- `⋯` 菜单：检查更新 / 从备份恢复 / 仓库管理 / 存储与同步

**这个信息架构比我们的"市场页 + 我的技能页"更成熟**。特别是"矩阵表"这个设计——它一眼就能看出"哪个技能在哪些工具里启用了"，比我们设想的分开两个页面强。

**但**：它是给"管理已安装技能"设计的，**"市场浏览"的体验较弱**（Discover 只是一个 tab）。我们要的"市场感"（卡片、分类、预览）还是得自己做。

---

## 3. 三个硬伤

### 硬伤 1：只支持 GitHub，没有 GitLab 🔴

这是**最致命**的一条。全代码库搜索 `gitlab` 的结果是**零**。

下载实现写死了 GitHub 归档 URL：

```rust
let url = format!(
    "https://github.com/{}/{}/archive/refs/heads/{}.zip",
    repo.owner, repo.name, branch
);
Self::assert_github_archive_url(&url, &repo.owner, &repo.name)?;  // ← 出口断言
```

而且有**安全出口校验**（防 SSRF / 防恶意 branch 重定向）：

```rust
if parsed.host_str() != Some("github.com") { ... 拒绝 ... }
```

**改造点**：这层需要重写为"Provider 抽象"：

```rust
trait GitProvider {
    fn archive_url(&self, repo: &RepoRef, branch: &str) -> String;
    fn api_base(&self) -> &str;
    fn parse_tree_url(&self, url: &str) -> Option<...>;
}
```

GitHub / GitLab / 任意 Git 三个实现。**这层改造量不大（约 200-300 行），但必须做对**，因为出口断言是安全边界，改错了会引入 SSRF。

**参考**：我们方案 §4.2 已经确认 GitLab 的 `archive.zip?path=` 能只下子目录，比他们的"整仓下载"更省。

### 硬伤 2：没有 Cursor 和千问办公 🟡

`AppType` 枚举里没有这两个。新增一个应用需要改：

| 位置 | 改什么 |
|---|---|
| `AppType` enum + `as_str()` | 加两个变体 |
| `get_app_skills_dir()` | 加两个路径分支 |
| `SkillApps` 结构体 | 加两个 bool 字段 |
| 数据库 schema + migration | 加两列 |
| 前端 `SKILLS_APP_IDS` / `appConfig` | 加两个 id |
| 图标资源 | 加两个 svg |
| i18n 文案 | 加标签 |

**工作量：约 1-1.5 人日**（主要是数据库迁移和前端铺开）。不算难，但散布在十几个文件里。

### 硬伤 3：Tauri 2 → 内网 WebView2 风险 🟡

它用 Tauri 2，MSI 打包，**没有配置 `webviewInstallMode`**，即用 Tauri 默认的 `downloadBootstrapper` 模式。

它自己的 WiX 模板里（`src-tauri/wix/per-user-main.wxs` 第 376-410 行）**三种模式都写了**：

```xml
{{#if install_webview}}              ← 默认：检测注册表，缺失则...
  {{#if download_bootstrapper}}      ← 联网下载 bootstrapper（连微软 CDN）
  {{#if webview2_bootstrapper_path}} ← 内嵌 2MB bootstrapper（仍要联网）
  {{#if webview2_installer_path}}    ← 内嵌离线安装包（~150MB，可气隙）
```

**默认走的是第一个分支——需要联网下载**。内网环境这条路走不通。

**但这同时是好消息**：它已经把"内嵌离线安装包"的代码写好了，只要换个配置。

> **不过**：这跟我们的技术选型结论冲突。我们在方案 §3.4 定了 WPF + .NET FW 4.7.2，理由是零依赖。**如果 fork cc-switch，就等于接受了 WebView2 + 150MB 兜底 + Tauri/Rust 技能栈要求。**

---

## 4. 除上述外，他们踩过的坑（可直接吸收）

这些是 7,937 行里最有价值的部分，全是真实 issue 换来的：

### 4.1 安全

| 机制 | 说明 |
|---|---|
| **压缩炸弹防护** | `MAX_ARCHIVE_ENTRIES = 30_000`、`MAX_ARCHIVE_TOTAL_BYTES = 512MB`、`DIRECTORY_BUDGET_COST`（空目录按一块磁盘计费）、`FILE_ENTRY_BUDGET_COST` |
| **下载体积上限** | 逐块读并计数，不信 `Content-Length`（可撒谎）；写入匿名临时文件而非内存 |
| **symlink 目标长度上限** | 4 KiB，因为 `zip 2.4.2` 的 `make_reader` 不按声明大小截断读取 |
| **路径穿越防护** | `restore_from_backup_rejects_traversal_directory_in_metadata` 有专门测试 |
| **GitHub URL 出口断言** | URL 拼好后再次确认 host 和路径，防 branch 注入 |
| **仓库坐标校验** | 任何会拼进 URL 的 owner/repo/branch 都先过 `validate_repo_ref` |

> **我们方案的缺口**：完全没考虑压缩炸弹和路径穿越。虽然"内网自产自用"风险较低，但如果支持"任意人上传"，就必须有这些防护。

### 4.2 健壮性

| 坑 | 处理 |
|---|---|
| 下载失败后临时目录残留，反复触发可填满磁盘 | 用 `TempDir` 守卫，成功才 `keep()`，失败自动清理 |
| 分支不存在（404）vs 网络错误不分 | 只有 404 才换下一个分支（main → master），其他错误立即返回，避免掩盖真实原因 |
| 多个候选分支各自失败导致残留叠加 | 每轮失败都清空临时目录再试 |
| 共享 HTTP 客户端没设 UA，被 GitHub 限流 429 | 显式设 `SKILL_DOWNLOAD_USER_AGENT` |
| 结构化错误缺 owner/name，前端占位符原样显示 | `with_repo_context()` 补全 |
| 下载超时 | 60 秒 `timeout` 包裹 |

### 4.3 状态一致性

| 坑 | 处理 |
|---|---|
| 数据库说装了，但 SSOT 目录不存在 | `local_hash_for_update_check` 注释明确："必须先确认 SSOT 目录存在，再信任数据库缓存的 content_hash" |
| 两处存储根目录重叠导致互删 | `ensure_distinct_skill_roots` + `paths_overlap`（含符号链接解析、悬空链接的父目录解析） |
| 并发安装/同步竞争 | `skill_state_lock()` 全局 RwLock，明确锁顺序："全局同步锁 → 此锁 → 数据库 mutex" |
| 异步期间持有非 Send 的 guard | 注释明确："install/update 只有在下载完成后才取写锁，所以不会有 guard 跨 `.await`" |

### 4.4 用户体验

| 特性 | 说明 |
|---|---|
| **未管理技能扫描** | `scan_unmanaged()` —— 发现工具目录里不是本应用装的技能，列出来但不接管 |
| **备份与恢复** | 每次更新前备份，保留 20 份（`SKILL_BACKUP_RETAIN_COUNT`） |
| **存储位置迁移** | `migrate_storage()` —— SSOT 可以在 `~/.cc-switch` 和其他位置之间搬 |
| **本地技能元数据刷新** | 用户手改了 SKILL.md，列表里的描述会自动更新 |
| **ZIP 导入** | 支持从 zip 包安装，目录名冲突时跳过并列明 |
| **逐仓库报错** | `SkillRepoFailure` —— 某个仓库读不到不静默吞掉，明确告诉用户 |

---

## 5. 三条路线对比

| | **A. Fork cc-switch 改造** | **B. 自研 WPF（原方案）** | **C. 抄架构自研** |
|---|---|---|---|
| 起点 | 154k star 的成熟产品 | 空项目 | 空项目 |
| 已有能力 | SSOT、分发、哈希、备份、迁移、安全防护全有 | 无 | 无 |
| GitLab 支持 | 需重写 provider 层（200-300 行） | 从零写（约 400 行） | 从零写 |
| Cursor + 千问 | 需新增 2 个应用（1-1.5 人日） | 从零写 | 从零写 |
| 技术栈 | Tauri + Rust + React（需会 Rust） | C# + WPF（需会 C#） | 同 B |
| 包体 | 6-10 MB + WebView2 风险 | 1-3 MB 零依赖 | 同 B |
| 内网适配 | 需处理 WebView2 离线包（150MB 兜底） | 天然满足 | 同 B |
| 现有 UI | 矩阵表（偏管理，市场感弱） | 需从零设计 | 需从零设计 |
| 维护负担 | **重**（上游日更，diff 会越来越大） | 轻（自己的代码） | 轻 |
| 首版工期 | **4-6 人日**（改造 + 内网适配） | 10-13 人日 | 10-13 人日 |
| 长期风险 | 上游大改时合并冲突 | 无 | 无 |
| 学习价值 | 可读 7,937 行的实战经验 | 无 | 可选择性参考 |

### 我的判断

**推荐 A，但要设一个止损条件。**

理由：
1. **它的 skill 架构就是对的**，我们重新设计一遍不会更好，只会更晚
2. `skill.rs` 里那些安全防护和边界处理，自己写要踩一遍所有坑才发现
3. 4-6 人日 vs 10-13 人日，差一倍
4. MIT 许可，闭源分发没问题

**止损条件**（任一触发就转 B）：
- 团队里没人会 Rust，且短期内学不会 → 改 bug 会很痛苦
- 内网 WebView2 抽查发现大量缺失，且 IT 不允许装 → 强行上 Tauri 是给自己挖坑
- 上游出现破坏性重构（比如彻底改掉 SSOT 模型）→ diff 无法维护

**折中方案 C 也值得考虑**：不 fork，但**把 `skill.rs` 的算法逻辑抄过来**（哈希算法、压缩炸弹防护、状态机），用 C# 重写。这样既有"踩过的坑"的经验，又保住 WPF 的零依赖优势。工期约 8-10 人日。

---

## 6. 如果选 A，具体改造清单

### 第一步：验证环境（半天）

- [ ] 本机能 `pnpm install` + `pnpm tauri dev` 跑起来
- [ ] 确认 Rust 工具链版本要求（`rust-toolchain.toml`）
- [ ] 确认 Node 版本要求（`.node-version`）

### 第二步：GitLab provider（1-1.5 人日）

- [ ] 抽出 `GitProvider` trait
- [ ] 实现 `GitHubProvider`（迁移现有逻辑）
- [ ] 实现 `GitLabProvider`：
  - 归档 URL：`{base}/api/v4/projects/{urlencoded_path}/repository/archive.zip?sha={ref}`（需带 `PRIVATE-TOKEN`）
  - **单技能下载**：`&path={subpath}`（比 GitHub 更省）
  - 分支/标签列表：`/repository/branches`、`/repository/tags`
- [ ] 出口断言改为按 provider 校验（**这是安全边界，要写单测**）
- [ ] 配置界面加"仓库类型"选项

### 第三步：新增两个应用（1-1.5 人日）

- [ ] `AppType` 加 `Cursor`、`QwenWork`
- [ ] 路径映射：`~/.cursor/skills`、`~/.qwenworkcn/skills`
- [ ] `SkillApps` 加两个字段 + SQLite migration
- [ ] 前端 `SKILLS_APP_IDS`、图标、i18n

### 第四步：确认 ChatGPT 路径（半天）

- [ ] 实测 `~/.codex/skills` vs `~/.agents/skills` 到底哪个生效
- [ ] 若两个都读，考虑都写入或取其一并在文档说明

### 第五步：内网适配（1 人日）

- [ ] WebView2：改成 `webviewInstallMode: { type: "offlineInstaller" }`，把 ~150MB 离线包打进 MSI（或放共享盘）
- [ ] 关掉所有 GitHub 相关功能（skills.sh 市场、GitHub 登录、自更新检查指向 GitHub Releases）
- [ ] 自更新改为查内网 GitLab Releases
- [ ] 代理/自签证书处理

### 第六步：市场感 UI（1-2 人日）

- [ ] 现有 Discover tab 偏功能，改成卡片网格 + 分类侧栏
- [ ] 详情页：渲染 SKILL.md + 版本历史 + 安装按钮组

**合计：5-7 人日**（比前面的 4-6 保守一些，因为内网适配的不确定性）

---

## 7. 实测实证（本机数据，非文档推断）

> 这一节全部是**读取本机真实状态**得到的数据，不是看文档推测的。本机已安装并使用 cc-switch。

### 7.1 本机 cc-switch 数据

```
~/.cc-switch/
├── cc-switch.db          1.9 MB（SQLite）
├── settings.json
├── skills/               ← SSOT，40 个技能
├── skill-backups/        ← 更新前备份
└── logs/
```

`settings.json` 关键配置：

```json
{
  "skillSyncMethod": "auto",              // symlink 优先，失败回退 copy
  "skillStorageLocation": "cc_switch",    // SSOT 在 ~/.cc-switch/skills
  "language": "zh"
}
```

### 7.2 数据库结构（已验证）

```sql
CREATE TABLE skills (
    id TEXT PRIMARY KEY,          -- "owner/repo:directory" 或 "local:directory"
    name TEXT NOT NULL,
    description TEXT,
    directory TEXT NOT NULL,
    repo_owner TEXT, repo_name TEXT, repo_branch TEXT,
    readme_url TEXT,
    enabled_claude BOOLEAN NOT NULL DEFAULT 0,
    enabled_codex BOOLEAN NOT NULL DEFAULT 0,
    enabled_gemini BOOLEAN NOT NULL DEFAULT 0,
    enabled_opencode BOOLEAN NOT NULL DEFAULT 0,
    enabled_hermes BOOLEAN NOT NULL DEFAULT 0,
    installed_at INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT,            -- ← 装的时候记录，用于更新检测
    updated_at INTEGER NOT NULL DEFAULT 0,
    enabled_grokbuild BOOLEAN, enabled_mcode BOOLEAN,
    enabled_agy BOOLEAN, enabled_kimi BOOLEAN
);
```

**注意**：`enabled_*` 是**一列一个应用**，靠 migration 逐步 `ALTER TABLE ADD COLUMN` 加进来的（后 4 个字段在 `CREATE TABLE` 之外，说明是后加的）。

**这对我们很重要**：新增 Cursor 和千问办公，就是再加两个 `enabled_cursor` / `enabled_qwenwork` 列。**migration 模式已经验证可行**。

### 7.3 技能来源分布（关键数据）

| 来源 | 数量 | 说明 |
|---|---|---|
| **Git 仓库** | **28** | 主要来自 `larksuite/cli`（飞书技能）和 `Eris-ZZX/grok-bot-shared-skills` |
| 本地 | 12 | `local:*` 前缀，手工导入 |

**这直接证明**：cc-switch 的"从 Git 仓库拉技能 → 装到本地 SSOT → 分发到各应用"这条链路，在真实使用中是**跑通的**，不是理论设计。

本机配置的仓库（`skill_repos` 表）：

| owner/name | branch | enabled |
|---|---|---|
| `anthropics/skills` | main | 0 |
| `ComposioHQ/awesome-claude-skills` | master | 0 |
| `cexll/myclaude` | master | 0 |
| `JimLiu/baoyu-skills` | main | 0 |
| `larksuite/cli` | HEAD | 0 |
| **`Eris-ZZX/grok-bot-shared-skills`** | main | **1** |

> 注：`enabled=0` 不代表没装过——28 个 git 技能已经在库里了，只是这些仓库当前被禁用（不再刷新）。

### 7.4 分发实际落盘情况（重要）

| 目录 | 技能数 | 其中符号链接 | cc-switch 管理？ |
|---|---|---|---|
| `~/.cc-switch/skills`（SSOT） | 40 | — | 是，事实源 |
| `~/.claude/skills` | 26 | **24** | 是（8 个链到 cc-switch，16 个链到 .agents） |
| `~/.codex/skills` | 7 | 2 | 是（2 个链到 cc-switch，5 个是真实目录） |
| `~/.agents/skills` | **44** | **0** | **否**，不在 cc-switch 管理范围 |
| `~/.cursor/skills` | 1 | 0 | **否** |
| `~/.qwenworkcn/skills` | 13 | 0 | **否** |
| `~/.workbuddy/skills` | 2 | 0 | **否** |

**数据库里的启用统计**：`claude=12, codex=7`，其余应用全 0。

### 7.5 从实测得到的五条硬结论

#### 结论 1：`symlink` 在本机可用，但靠的是开发者模式

`.claude/skills` 里 24 个是符号链接，`.codex/skills` 里 2 个。说明这台机器 `Auto` 模式**成功降到了 symlink**（开发者模式已开）。

**但这不能外推到同事机器**。普通办公机没开开发者模式时，`Auto` 会回退到 copy——这正好是我们要的默认行为，但**必须自己实测确认回退逻辑真的工作**，不能假设。

#### 结论 2：`.agents/skills` 是一套完全独立的野库，cc-switch 管不到

这是**最重要的发现**，推翻了我之前"`.agents` 和 `.codex` 是同一个目录"的推测：

- `~/.agents/skills` 有 44 个技能，**0 个符号链接**，全是真实目录
- `~/.cc-switch/skills` 有 40 个，与之**交集 28 个**
- 只在 `.agents` 里的 16 个，全是 `dingtalk-*`
- `.claude/skills` 里 16 个 `dingtalk-*` 链接，**指向 `.agents/skills`**，不是指向 cc-switch

**含义**：
1. `.agents/skills` 和 `.codex/skills` 是**两个不同的物理目录**，互不链接
2. 这台机器上已经存在**两套并存的管理体系**（cc-switch 管 40 个 + 手工/其他方式管 44 个）
3. 同一批技能有重复：28 个同时存在于两边

**对产品设计的直接影响**：**"未管理技能扫描"不是可选功能，是必需的**。用户机器上一定会有本工具管不到的技能，客户端必须能看见它们、并且明确区分"我管的"和"不是我管的"。

cc-switch 已经有 `scan_unmanaged()` 和 `UnmanagedSkill` 结构体处理这个——**直接用**。

#### 结论 3：ChatGPT / Codex 的路径存在真实歧义

| 来源 | 说法 |
|---|---|
| 官方文档（Codex） | 读 `$HOME/.agents/skills` |
| cc-switch 代码 | 写 `~/.codex/skills` |
| **本机实测** | 两个目录都存在、都有内容、**互不链接** |

**无法从现有证据判定哪个是"正确"的**。两种可能：
- Codex 同时读两处（`.agents` 用于新约定，`.codex` 用于旧版本兼容）
- Codex 版本迭代中迁移过，两个目录是不同时期留下的

**必须做的验证**：在 `~/.agents/skills` 里放一个新技能，重启 Codex，看能不能 `$技能名` 调用。这是唯一可靠的判定方法。

**在此之前的安全做法**：**两个目录都写**。代价是多占一份磁盘，但保证在任何 Codex 版本上都能生效。

#### 结论 4：Cursor 的目录也不是单一答案

| 路径 | 本机状态 |
|---|---|
| `~/.cursor/skills/` | 存在，1 个技能（`se-delivery-workflow`） |
| `~/.cursor/rules/` | 存在，**空** |
| `~/.cursor/skills-cursor/` | 存在，28 个（Cursor 官方内置技能，带 `.sync-manifest.json`） |

说明：
- `rules/` 为空 → Cursor 现在用的是 **skills 机制**，不是老的 rules 机制
- `skills-cursor/` 是 Cursor 自己同步内置技能用的，**我们绝对不能写进去**（会被覆盖或被同步逻辑干扰）
- 用户自装技能放在 `~/.cursor/skills/` —— 这是我们的目标目录

**风险**：`skills-cursor/` 和 `skills/` 只差一个后缀，配置路径时极易写错。**必须在代码里硬编码排除 `skills-cursor`**。

#### 结论 5：千问办公确实会污染 SKILL.md（已验证一个）

实测扫描 40 个技能目录的 frontmatter，发现 `abnormal-hours-approver` **连 frontmatter 都没有**（`no frontmatter`）。

这说明技能文件的格式在真实使用中比预想的更乱。cc-switch 的解析逻辑对此是宽容的（`read_skill_name_desc` 有 fallback）。

**对我们的启示**：
- 规范化哈希的"剥离易变字段"逻辑要处理**没有 frontmatter** 的情况（不能崩）
- 不能假设所有技能都符合规范，解析必须容错

> 之前我基于 `dws` 技能的 `enabled_at` 字段判断千问会回写。这次扫描没在 cc-switch 的 40 个技能里复现该字段——因为那些是 cc-switch 自己的副本，不是千问目录里的。**该结论仍需在 `.qwenworkcn/skills` 上单独验证**。

---

## 8. 必须自己验证的事（按优先级）

| # | 事项 | 方法 | 影响 |
|---|---|---|---|
| 1 | **Codex 到底读 `.agents` 还是 `.codex`** | 往 `.agents/skills` 放个新技能，重启 Codex 试 `$技能名` | 决定写一个还是写两个目录 |
| 2 | **千问办公是否回写 SKILL.md** | 对比 `.qwenworkcn/skills/<x>/SKILL.md` 安装前后 diff | 决定哈希黑名单是否必要 |
| 3 | **Cursor 是否真读 `~/.cursor/skills`** | 放个技能进去，重启 Cursor 看是否出现在技能列表 | 路径正确性 |
| 4 | **`Auto` 模式的 copy 回退是否真工作** | 关闭开发者模式后测一次 | 同事机器的默认行为 |
| 5 | **内网 WebView2 覆盖率** | 抽查 ≥10 台真实办公机 | 决定 A 是否可行 |

---

## 9. 目录说明

`_ref-cc-switch/` 是只读参考副本，`--depth 1` 克隆，**未做任何修改**。

- 建议保留到方案定稿，之后可删除（105 MB）
- 若要长期参考，可改为 `git remote add` 跟踪，避免占工作区
- 核心文件位置：
  - `src-tauri/src/services/skill.rs` —— 全部逻辑（7,937 行）
  - `src-tauri/src/app_config.rs:321` —— `InstalledSkill` 结构
  - `src-tauri/src/app_config.rs:498` —— `AppType` 枚举
  - `src/components/skills/` —— 前端
  - `docs/user-manual/zh/3-extensions/3.3-skills.md` —— 产品说明
