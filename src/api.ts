import { invoke } from "@tauri-apps/api/core";

// ========== 类型（与后端 #[serde(rename_all = "camelCase")] 对应）==========

export interface ToolDetection {
  key: string;
  displayName: string;
  rootDir: string | null;
  installed: boolean;
  skillsDirExists: boolean;
  installedSkillCount: number;
}

export interface AppSettings {
  gitlabBaseUrl: string;
  gitlabProjectPath: string;
  gitlabToken: string | null;
  gitlabBranch: string;
  defaultInstallMode: "copy" | "junction" | "symlink" | "auto";
  /** 发布时的署名（姓名/工号）。内置 Token 下这是唯一的身份来源。 */
  publisherIdentity: string;
}

export interface RemoteSkill {
  name: string;
  repoPath: string;
  hash: string;
  version: string | null;
  description: string;
  tags: string[];
  category: string | null;
  lastCommitAt: string | null;
}

/** 安装方式：链接只存一份，复制各存一份 */
export type InstallMode = "copy" | "junction" | "symlink" | "auto";

export const INSTALL_MODE_LABEL: Record<InstallMode, string> = {
  junction: "链接",
  symlink: "软链",
  copy: "复制",
  auto: "自动",
};

/** 单工具安装失败 */
export interface InstallFailure {
  tool: string;
  message: string;
  detail: string;
  retryable: boolean;
}

/** 一次安装的逐条结果 */
export interface InstallOutcome {
  succeeded: InstalledSkill[];
  failed: InstallFailure[];
}

export interface InstalledSkill {
  name: string;
  tool: string;
  targetPath: string;
  mode: "copy" | "junction" | "symlink" | "auto";
  installedHash: string;
  installedVersion: string | null;
  source: {
    kind: "gitlab" | "local";
    repoUrl: string | null;
    repoPath: string | null;
    branch: string | null;
    commit: string | null;
  };
  installedAt: number;
}

export type UpdateStatus =
  | "uptodate"
  | "updateavailable"
  | "locallymodified"
  | "conflict"
  | "remotemissing"
  | "localmissing";

export interface UpdateCheckItem {
  name: string;
  tool: string;
  status: UpdateStatus;
  installedVersion: string | null;
  remoteVersion: string | null;
  /** 用户已对这个远端版本选了「保留我的」，界面不再计入待处理 */
  ignored: boolean;
}

/** 一次「检查更新」的完整结果 */
export interface UpdateCheckResult {
  items: UpdateCheckItem[];
  failures: RemoteFailure[];
}

/** 远端读取失败的一条记录 */
export interface RemoteFailure {
  /** network | auth | notfound | parse | config */
  kind: "network" | "auth" | "notfound" | "parse" | "config";
  /** 用户可读的中文原因 */
  message: string;
  /** 原始技术信息，供排查 */
  detail: string;
}

/** 后端结构化错误（skill_error::SkillError 的镜像） */
export interface SkillErrorPayload {
  code: string;
  message: string;
  detail: string;
  action: string | null;
}

/**
 * 解析后端返回的错误
 *
 * 四个高频命令现在返回结构化 JSON；其它命令仍可能返回纯文本。
 * 两种情况都要能显示，所以解析失败就退回纯文本形态。
 */
export function parseError(e: unknown): SkillErrorPayload {
  const raw = typeof e === "string" ? e : String(e ?? "");
  try {
    const v = JSON.parse(raw);
    if (
      v &&
      typeof v === "object" &&
      typeof v.code === "string" &&
      typeof v.message === "string"
    ) {
      return {
        code: v.code,
        message: v.message,
        detail: typeof v.detail === "string" ? v.detail : "",
        action: typeof v.action === "string" ? v.action : null,
      };
    }
  } catch {
    // 不是 JSON，按纯文本处理
  }
  return {
    code: "UNKNOWN",
    message: raw || "操作失败",
    detail: raw,
    action: null,
  };
}

/** 动作按钮的文案 */
export const ACTION_LABEL: Record<string, string> = {
  retry: "重试",
  open_settings: "去设置",
  reinstall: "卸载重装",
  contact_it: "联系 IT",
};

/** 重新同步的结果 */
export interface ResyncOutcome {
  repaired: string[];
  skipped: string[];
  failed: ResyncFailure[];
  skippedModified: ResyncFailure[];
}

export interface ResyncFailure {
  name: string;
  tool: string;
  message: string;
}

/** 一条备份 */
export interface SkillBackupEntry {
  backupId: string;
  name: string;
  tool: string;
  installedVersion: string | null;
  backupCreatedAt: number;
  sizeBytes: number;
  /** 库里当前没有同名技能时才可恢复 */
  isRestorable: boolean;
}

/** 恢复备份的结果 */
export interface RestoreResult {
  record: InstalledSkill;
  /** 目标工具根目录不存在——记录已写，但需先装该工具才能链过去 */
  pendingTool: boolean;
}

/** 冲突处理的三选一 */
export type ConflictChoice = "keepmine" | "useremote" | "keepboth";

export interface AppInfo {
  name: string;
  version: string;
  dataDir: string;
  stateFileExists: boolean;
  junctionSupported: boolean;
}

export interface SkillRevision {
  sha: string;
  shortSha: string;
  title: string;
  authorName: string;
  authorEmail: string | null;
  createdAt: string;
}

export interface PublishRequest {
  localDir: string;
  category: string | null;
  publisher: string;
  message: string | null;
}

export interface PublishResult {
  name: string;
  repoPath: string;
  commitSha: string;
  snapshotId: string | null;
  commitMessage: string;
}

export interface Snapshot {
  id: string;
  skillName: string;
  repoPath: string;
  commitSha: string;
  createdAt: number;
  dir: string;
}

/** 扫描到的一个技能（不是安装记录） */
export interface ScannedSkill {
  name: string;
  dirName: string;
  /** 首个落盘的工具（完整列表见 placements） */
  tool: string;
  /** 首个落盘的路径（完整列表见 placements） */
  targetPath: string;
  /** 在哪些工具的目录下发现 */
  foundIn: string[];
  /** 每个落盘的完整信息 */
  placements: ScannedSkillPlacement[];
  description: string;
  version: string | null;
  tags: string[];
  category: string | null;
  isLink: boolean;
  linkTarget: string | null;
}

/** 一个落盘位置 */
export interface ScannedSkillPlacement {
  tool: string;
  targetPath: string;
  isLink: boolean;
  linkTarget: string | null;
}

/** 「我的技能」的一行（按技能名聚合） */
export interface MySkillItem {
  name: string;
  tools: string[];
  targetPaths: string[];
  description: string;
  version: string | null;
  /** managed | external | local */
  identity: "managed" | "external" | "local";
  /** 仅 managed 有值 */
  status: UpdateStatus | null;
  installedVersion: string | null;
}

/** 一个技能在某个工具下的一次落盘 */
export interface SkillPlacement {
  tool: string;
  targetPath: string;
}

/**
 * 把 `tools` 与 `targetPaths` 按索引配成对
 *
 * 后端保证两者同长同序，但界面不应依赖 indexOf 反查——那在
 * 同名工具重复或数组错位时会静默取错路径。
 */
export function placements(item: MySkillItem): SkillPlacement[] {
  return item.tools.map((tool, i) => ({
    tool,
    targetPath: item.targetPaths[i] ?? "",
  }));
}

/** 矩阵里一个格子的状态 */
export interface MatrixCellState {
  tool: string;
  installed: boolean;
  /** 仅 installed 时有值 */
  targetPath: string | null;
  /** 仅 installed 时有意义 */
  updateStatus: UpdateStatus | null;
  /** 上次操作失败标记（前端内存态，不持久化） */
  failed: boolean;
}

/**
 * 把一个技能摊成「每工具一格」
 *
 * 列由调用方给定（只传本机已装的工具，避免 6 列过宽），
 * 没在该技能 `tools` 里的工具就是未装。
 */
export function matrixCells(
  item: MySkillItem,
  tools: string[],
  updates: UpdateCheckItem[],
  failedTools: string[] = [],
): MatrixCellState[] {
  return tools.map((tool) => {
    const idx = item.tools.indexOf(tool);
    const installed = idx >= 0;
    const update = installed
      ? updates.find((u) => u.name === item.name && u.tool === tool)
      : undefined;
    return {
      tool,
      installed,
      targetPath: installed ? (item.targetPaths[idx] ?? null) : null,
      updateStatus: update ? update.status : null,
      failed: failedTools.includes(tool),
    };
  });
}

/** 需要用户处理的更新状态 */
const PENDING_STATUSES: UpdateStatus[] = [
  "updateavailable",
  "conflict",
  "locallymodified",
  "localmissing",
];

/**
 * 待处理条数：侧边栏角标与页内文案共用这一份口径
 *
 * 口径 = 身份为 local 的技能数 + managed 且状态需要处理的技能数。
 * 已选「保留我的」的条目不再计入——用户已经表态过了。
 */
export function countPending(
  items: MySkillItem[],
  updates: UpdateCheckItem[],
): number {
  return items.filter((i) => {
    if (i.identity === "local") return true;
    if (i.identity !== "managed") return false;
    // 按 (技能名, 工具) 取状态：同名技能可能装在多个工具下，状态各不相同
    return placements(i).some((p) => {
      const u = updates.find((x) => x.name === i.name && x.tool === p.tool);
      return u ? !u.ignored && PENDING_STATUSES.includes(u.status) : false;
    });
  }).length;
}

/** 需要警示的条数（冲突 + 本地缺失） */
export function countAlerts(updates: UpdateCheckItem[]): number {
  return updates.filter(
    (u) => u.status === "conflict" || u.status === "localmissing",
  ).length;
}

// ========== API ==========

export const api = {
  detectTools: () => invoke<ToolDetection[]>("detect_tools"),
  junctionAvailable: () => invoke<boolean>("junction_available"),

  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) =>
    invoke<void>("save_settings", { settings }),

  refreshIndex: () => invoke<RemoteSkill[]>("refresh_index"),
  listRemoteSkills: () => invoke<RemoteSkill[]>("list_remote_skills"),

  listInstalled: () => invoke<InstalledSkill[]>("list_installed"),
  scanSkills: () => invoke<MySkillItem[]>("scan_skills"),
  adoptSkill: (args: { name: string; tool: string; targetPath: string }) =>
    invoke<InstalledSkill>("adopt_skill", args),
  stateHealth: () => invoke<string | null>("state_health"),
  installSkill: (args: {
    name: string;
    repoPath: string;
    tools: string[];
    allowCreateSkillsDir: boolean;
    mode?: InstallMode;
  }) => invoke<InstallOutcome>("install_skill", args),
  uninstallSkill: (name: string, tool: string) =>
    invoke<void>("uninstall_skill", { name, tool }),
  checkUpdates: () => invoke<UpdateCheckResult>("check_updates"),
  resyncSkills: () => invoke<ResyncOutcome>("resync_skills"),
  listSkillBackups: () => invoke<SkillBackupEntry[]>("list_skill_backups"),
  restoreSkillBackup: (backupId: string) =>
    invoke<RestoreResult>("restore_skill_backup", { backupId }),
  deleteSkillBackup: (backupId: string) =>
    invoke<void>("delete_skill_backup", { backupId }),
  updateSkill: (name: string, tool: string) =>
    invoke<InstalledSkill>("update_skill", { name, tool }),
  resolveConflict: (name: string, tool: string, choice: ConflictChoice) =>
    invoke<InstalledSkill>("resolve_conflict", { name, tool, choice }),

  // 发布 / 历史 / 回滚
  publishSkill: (req: PublishRequest) =>
    invoke<PublishResult>("publish_skill", { req }),
  skillHistory: (repoPath: string, limit?: number) =>
    invoke<SkillRevision[]>("skill_history", { repoPath, limit: limit ?? 20 }),
  listSnapshots: () => invoke<Snapshot[]>("list_snapshots"),
  rollbackSkill: (args: {
    repoPath: string;
    targetSha: string;
    publisher: string;
    skillName: string;
  }) => invoke<PublishResult>("rollback_skill", args),

  appInfo: () => invoke<AppInfo>("app_info"),
};

// ========== 展示辅助 ==========

export function statusLabel(s: UpdateStatus): { text: string; color: string } {
  switch (s) {
    case "uptodate":
      return { text: "最新", color: "#9aa4b2" };
    case "updateavailable":
      return { text: "可更新", color: "#2563eb" };
    case "locallymodified":
      return { text: "已修改", color: "#6b7280" };
    case "conflict":
      return { text: "需处理", color: "#d97706" };
    case "remotemissing":
      return { text: "远端已删除", color: "#9aa4b2" };
    case "localmissing":
      return { text: "本地缺失", color: "#dc2626" };
  }
}

export function toolLabel(key: string, tools: ToolDetection[]): string {
  return tools.find((t) => t.key === key)?.displayName ?? key;
}

export function formatTime(unixSec: number): string {
  if (!unixSec) return "—";
  const d = new Date(unixSec * 1000);
  const now = Date.now();
  const diff = now - d.getTime();
  const day = 86400_000;
  if (diff < day) return "今天";
  if (diff < day * 2) return "昨天";
  if (diff < day * 30) return `${Math.floor(diff / day)} 天前`;
  return d.toLocaleDateString("zh-CN");
}
