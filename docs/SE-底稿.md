# SE 底稿

> 只放会变的状态。稳定规则见 `CLAUDE.md`。
> 每次压缩或换会话前更新；剔除已被推翻的旧结论，不只追加。

最后更新：2026-11-19

## 仓库与会话

| 项 | 值 |
|---|---|
| 主仓库 | `Eris-ZZX/skill-market` |
| 事实主干 | `master` |
| 当前本地分支 | `master` |
| 最新提交 | `21ab680`（fix: WiX 缓存路径检测）|
| 远端状态 | 与本地一致 |
| 第二远端 | 无 |
| Cloud 会话 ID | **未建立**（尚未派工）|
| Cloud 默认模型 | 未配置 |

## 已完成（可实测验证）

| 能力 | 实测证据 |
|---|---|
| GitLab 索引拉取 + head 缓存 | 拉到 grill-me，缓存命中走 1 个请求 |
| 中文分类/标签解析 | `skills/通用/grill-me` 正确 |
| 单技能下载 | 绕开 GitLab 中文 path bug，本地切分 |
| 一装多工具 | WorkBuddy + 千问各一份，字节级一致 |
| 版本号记录 | `installedVersion: 1.0.0` |
| 复制模式部署 | 两处文件 SHA256 相同 |
| 直推发布（Maintainer Token）| commit `c16e9087` |
| 保护分支拦截（Developer）| 403 `not allowed to push into this branch` |
| MSI 打包 | 3.07 MB |
| 便携 zip | 2.81 MB |

## 分支位置

| 分支 | 状态 |
|---|---|
| `master` | 干净，与 origin 一致 |

无进行中的功能分支。

## 开放事项（18 个 Issue）

P0：#1 更新命令 · #2 冲突三选一 · #3 更新 e2e
P1：#4 未管理扫描 · #5 详情正文 · #6 发布前校验 · #7 junction 接通 · #8 首启引导
待决策：#13 WebView2 覆盖率 · #14 项目级安装 · #15 分发方式
P2：#9 CI · #10 错误文案 · #11 图标 · #12 版本号
待验证：#16 千问回写字段
不做：#17 排除清单
总览：#18

## 下一次开工的阻塞点

**#14 必须在 #1 之前定** —— 影响 `install_skill` 的目录设计与 `state.json` 结构，先做 #1 再改要动两次。

## 踩坑记录

### 构建环境（换机必看）

| 坑 | 解法 |
|---|---|
| crates.io 直连卡死 | `$CARGO_HOME/config.toml` 配 rsproxy 镜像 + `multiplexing = false` |
| `rust-toolchain.toml` 锁 1.95 解析成 gnu 变体，缺 dlltool | 改名 `.bak`，用本机 1.96 msvc |
| **`aws-lc-sys` 的 NASM 汇编被 DLP 实时加密** | 彻底移除 aws-lc，全链路改 `ring`。**必须 `default-features = false`**，否则仍被拉进来 |
| `reqwest` 用 `-no-provider` 后首次建连 panic | 启动时 + Client 构造时各装一次 `rustls::crypto::ring::default_provider()` |
| 产物路径找不到 | 本机设了 `CARGO_TARGET_DIR=C:\ck`（规避 260 字符路径限制）|
| 首次打 MSI 卡 13 分钟 | 从 GitHub 下载 WiX；缓存位置是 `%LOCALAPPDATA%\tauri\WixTools314` |
| dev 服务起不来、窗口 15x15 | vite 端口 3100 被残留进程占用，前端加载失败。用 `dev.cmd` 自动清理 |

### 运行时

| 现象 | 原因 |
|---|---|
| exe 被占用无法编译 | 先停 `skill-market` 进程再 `cargo build` |
| 窗口超出屏幕、三大键看不见 | 屏幕工作区仅 1280x680；`lib.rs` 有 `fit_window_to_screen` 兜底 |
| `tauri-plugin-window-state` 存了错误尺寸 | 已移除，改用主动校正 |

## 技术取舍（已定，不再讨论）

| 取舍 | 理由 |
|---|---|
| 不用 GitLab `archive?path=` | 实测中文子路径返回 404（编码后返回 200 但 0 字节）。改整仓下载 + 本地切分 |
| 默认分支 `master` | 实测内网 GitLab 项目如此 |
| 直推 master 而非 MR | 产品明确要求「软件内自助审批，不去 Lab 点 MR」。用快照 + 署名 + 二次确认补偿 |
| `webviewInstallMode: skip` | 产品确认「先假设有 WebView2」，MSI 3MB |
| 不恢复 `tauri-plugin-updater` | 强制引入 aws-lc-rs。自更新只能「提示 + 跳转下载」|
| 移除 `tauri-plugin-window-state` | 存了 15x15 的错误尺寸 |

## 凭据位置（不入库）

| 项 | 位置 |
|---|---|
| GitLab Maintainer Token | `%APPDATA%\skill-market\state.json`（`.gitignore` 已排除）|
| 内网仓库 | `http://gitlab.shokz.com.cn/gylzbz/skill` → 实际为 `gylzlb/skill`，分支 `master` |

⚠️ Token 已出现在聊天记录，正式分发前需轮换。

## 待办：下一次会话开场先做

1. 读本文件 + `CLAUDE.md`
2. 确认 #14 决策状态（项目级安装）
3. 若无决策，向产品问 #13 / #14 / #15 三项
4. #14 定了才能开工 #1
