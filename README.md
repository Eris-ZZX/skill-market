# Skill Market

公司内网的 AI 技能（Agent Skills）分发客户端。**内网 GitLab 是货架，客户端是橱窗和安装器。**

装哪个、升不升、别人改了你的技能要不要采纳 —— 全部由使用者自己决定。这是一个**自助工具**，不是管控工具。

---

## 它解决什么问题

Agent Skills（`SKILL.md`）已经成了 AI 编码工具的通用扩展格式，但各家的存放位置不同：

| 工具 | 技能目录 |
|---|---|
| Cursor | `~/.cursor/skills/` |
| Claude Code | `~/.claude/skills/` |
| Codex | `~/.codex/skills/` |
| ChatGPT / Agent Skills | `~/.agents/skills/` |
| 千问办公 | `~/.qwenworkcn/skills/` |
| WorkBuddy | `~/.workbuddy/skills/` |

团队共享技能时，每个人都要手动复制、手动同步、手动检查更新。这个工具把这件事变成：

```
浏览 → 勾选 → 选工具 → 安装 → 有新版时自己决定要不要升
```

---

## 核心设计

### 零服务端

不部署任何服务。所有状态在本地 `%APPDATA%\skill-market\` 里，数据源是内网 GitLab 的一个仓库。

### 更新判定不看版本号

主判据是**技能目录的内容哈希**，不是 `version` 字段：

- 内容变了 → 提示可更新
- 只改了 version 但内容没变 → 不提示
- **内容变了但忘了改 version → 仍然提示**，并标注「版本号未更新」

因为人一定会忘记改版本号。

哈希计算做了六件事，每件都是踩过的坑：

1. 剥离 frontmatter 里的易变字段（千问办公会写 `enabled_at`）
2. 排除 `__pycache__` / `*.pyc`（跑一次 Python 技能就会产生）
3. 排除隐藏文件
4. 换行符归一（Windows CRLF vs git LF）
5. 文件列表排序（文件系统遍历顺序不稳定）
6. 相对路径计入哈希（否则改名检测不出来）

### 四种状态

界面只说四句话：

| 状态 | 条件 | 表现 |
|---|---|---|
| 最新 | 本地 = 远端 | 无标记 |
| 可更新 | 本地没改，远端变了 | 蓝色「更新」 |
| 已修改 | 本地改过，远端没变 | 灰色「已修改」 |
| 冲突 | 两边都改了 | 黄色 + 三选一 |

**绝不静默覆盖用户改过的文件。**

### 只写已存在的工具目录

绝不创建工具根目录。`~/.cursor` 不存在就说明没装 Cursor，界面显示「未安装」且不可选。

### 复制而非软链

默认复制独立副本，每个工具一份。这样：
- 各工具互不影响
- 能区分「在 Cursor 里改的」和「在千问里改的」
- 不怕工具回写字段污染共享库

junction 作为高级选项，且必须运行时探测可用性（部分企业安全策略禁止重解析点）。

---

## 技术栈

| 层 | 选型 | 理由 |
|---|---|---|
| 桌面框架 | Tauri 2 | 包体小、原生性能 |
| 后端 | Rust | 需要处理文件哈希、zip、并发 |
| 前端 | React 18 + TypeScript | — |
| HTTP | reqwest + rustls(**ring**) | 见下方「关于 aws-lc-rs」 |

### 打包体积策略（实测数据）

本项目**假设目标机器已安装 WebView2 Runtime**，因此 `tauri.conf.json` 用：

```json
"webviewInstallMode": { "type": "skip" }
```

#### 实测体积

| 项 | 大小 |
|---|---|
| `skill-market.exe`（release，已 strip） | **10.31 MB** |
| zip 压缩后 | **4 MB** |
| 前端资源（编入 exe） | 178 KB |

对比参考项目 cc-switch：便携包 **19.53 MB**（同为 Tauri + 依赖系统 WebView2）。
本项目体积约为其一半，原因是 release profile 更激进：

```toml
[profile.release]
codegen-units = 1     # cc-switch 用 16（其注释承认这让体积 +17%，换编译快 64%）
lto = true            # cc-switch 用 "thin"
opt-level = "z"       # cc-switch 用 "s"
strip = true
panic = "abort"       # 省掉 unwind 表
```

代价是 release 构建慢（fat LTO，约 10-12 分钟），但发布频率低，可以接受。

#### 缺 WebView2 时怎么办

`skip` 模式的代价：机器没装 WebView2 时窗口会白屏。因此代码里加了启动检测：

- 读注册表 `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-...}\pv`
- 未检测到则弹中文提示，告诉用户去内网共享盘取安装包

WebView2 的实际覆盖情况：

| 系统 | 是否自带 |
|---|---|
| Windows 11 | ✅ 自带 |
| Win10 22H2 / 21H2 等消费版 | ✅ 微软已批量推送，绝大多数有 |
| Win10 LTSC 2019 / Server / 干净安装 | ⚠️ 可能没有 |

**上线前建议抽查 5-10 台真实办公机**：

```powershell
(Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -EA SilentlyContinue).pv
```

有输出即已安装。若发现缺失较多，改回 `{"type":"offlineInstaller","silent":true}` 重新打包即可（包体变为约 160MB）。

### 关于 aws-lc-rs

`rustls` 默认的加密后端是 `aws-lc-rs`，它包含 NASM 汇编文件。在某些企业 DLP 环境下，这些文件会被**实时加密拦截**，导致编译失败（错误表现为 `%TSD-Header%` 或 `label or instruction expected`）。

因此本项目强制使用 `ring` 后端：

```toml
hyper-rustls = { default-features = false, features = ["ring", ...] }
tokio-rustls = { default-features = false, features = ["ring", ...] }
rustls = { default-features = false, features = ["ring", ...] }
reqwest = { default-features = false, features = ["rustls-tls-native-roots-no-provider", ...] }
```

注意必须同时关掉 `default-features`，否则 `aws-lc-rs` 仍会被拉进来。

**并且**必须在任何 HTTP 请求前安装 provider，否则首次建连会 panic `No provider set`：

```rust
rustls::crypto::ring::default_provider().install_default();
```

---

## 快速开始

### 环境要求

- Rust 1.85+
- Node.js 22+
- pnpm 10+
- Windows 10+（macOS / Linux 未测试）

国内网络建议先配 crates 镜像（否则 `cargo fetch` 会卡死）：

```toml
# $CARGO_HOME/config.toml
[source.crates-io]
replace-with = 'rsproxy-sparse'

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[http]
multiplexing = false
```

### 构建

```bash
pnpm install
pnpm tauri dev      # 开发
pnpm tauri build    # 打包 MSI
```

> Windows 上首次启动若报「端口 3100 被占用」，用 `dev.cmd`（自动清理残留进程）。

### 配置

启动后在「设置」里填：

| 字段 | 说明 |
|---|---|
| GitLab 地址 | 实例根地址，不带 `/api/v4` |
| 项目路径 | 如 `group/subgroup/skills-repo` |
| 分支 | 通常 `master` |
| 访问令牌 | 公开可读时留空 |
| 发布人署名 | 姓名/工号，用于标识提交作者 |

---

## 仓库规范

技能仓库的结构：

```
skills/
  <分类>/              粗分组，如：通用 / 研发 / 销售
    <技能名>/          必须与 SKILL.md 的 name 一致
      SKILL.md         必需
      scripts/         可选
      references/      可选
```

`SKILL.md` 格式：

```markdown
---
name: sales-report
description: >
  生成销售周报。当用户提到销售周报、业绩报表时使用。
version: 1.0.0
metadata:
  skillmarket:
    category: 销售
    tags: [销售, 报表]
    authors: [zhangx]
---

# 技能正文
```

**扩展字段必须放在 `metadata.skillmarket` 下** —— 这是 Agent Skills 生态的惯例，各工具会忽略自己不认识的前缀；放顶层会被严格校验的工具拒绝。

---

## GitLab 权限配置

### 推荐配置

| 项 | 值 | 理由 |
|---|---|---|
| Token 类型 | **Project Access Token** | 只限单个项目，比 PAT 安全 |
| 角色 | **Maintainer** | 需要直推 master |
| Scopes | `api` | 读写仓库 |
| 过期 | 定期轮换 | — |

### 关于「不要 MR、直推 master」

本项目采用**直推**而非 MR 流程。理由：内网小团队、技能是 Markdown 文档、没有专职审核人，「每次人工点 MR」的仪式感大于实际价值。

用三道保险补偿取消的人工门禁：

1. **发布前快照**：自动把当前 master 版本存到本地（每技能保留 10 份）
2. **署名留痕**：commit message 格式 `[发布人: 姓名/工号] 说明`
3. **二次确认**：界面上必须勾选「我确认这个版本的改动是正确的」

**如果你需要人工审核**，把 `master` 设为保护分支（Allowed to push: No one），然后改用 MR 流程 —— 代码里 `publish.rs` 的结构支持这种切换。

### ⚠️ 内置 Token 的风险

Token 编译进 exe 只是混淆，**反编译就能取出**。这是内置 Token 的固有代价。

因此：
- 仓库内容必须是**全员可见级别**，不放任何敏感资料
- 出问题时吊销 Token，但已发出的客户端会全部失效，需要推新版
- 建议设过期时间，配合定期发版

---

## 已知限制

| 限制 | 说明 |
|---|---|
| GitLab `archive?path=` 中文路径失效 | 自建 GitLab 实测对含中文的子路径返回 404。本项目改为「下整仓 + 本地切分」，多下流量但能正常工作 |
| junction 模式 | 代码已实现，但需运行时探测；未做完整验证 |
| 版本历史 / 回滚 | 已实现，未做端到端验证 |
| macOS / Linux | 未测试 |

---

## 项目结构

```
skill-market/
├── src/                       前端
│   ├── App.tsx                外壳与导航
│   ├── api.ts                 类型与 Tauri 命令封装
│   ├── pages/                 四个页面
│   └── components/            卡片、弹窗
└── src-tauri/
    ├── src/
    │   ├── tools.rs           工具路径表 + R1/R4 约束
    │   ├── hash.rs            内容哈希（双模式）
    │   ├── frontmatter.rs     SKILL.md 解析
    │   ├── state.rs           四态判定
    │   ├── gitlab.rs          GitLab API + SSRF 断言
    │   ├── install.rs         部署引擎（copy / junction）
    │   ├── publish.rs         发布 + 快照 + 回滚
    │   ├── storage.rs         状态持久化
    │   └── commands/          Tauri 命令层
    └── tests/                 端到端测试
```

### 端到端测试

依赖真实 GitLab，默认 `#[ignore]`：

```powershell
$env:SKILL_MARKET_TEST_BASE="https://gitlab.example.com"
$env:SKILL_MARKET_TEST_PROJECT="group/skills-repo"
$env:SKILL_MARKET_TEST_TOKEN="<token>"
cargo test -- --ignored --nocapture
```

---

## 设计文档

仓库根目录（未包含在本项目内）有完整设计文档，本项目是其裁剪实现。

---

## 许可

MIT
