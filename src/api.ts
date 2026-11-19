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
  defaultInstallMode: "copy" | "junction";
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

export interface InstalledSkill {
  name: string;
  tool: string;
  targetPath: string;
  mode: "copy" | "junction";
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
}

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
  tool: string;
  targetPath: string;
  description: string;
  version: string | null;
  tags: string[];
  category: string | null;
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
 * 两处各写一份过滤条件必然分叉（#21 已统一过一次，又分叉过一次）。
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
      return u ? PENDING_STATUSES.includes(u.status) : false;
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
  }) => invoke<InstalledSkill[]>("install_skill", args),
  uninstallSkill: (name: string, tool: string) =>
    invoke<void>("uninstall_skill", { name, tool }),
  checkUpdates: () => invoke<UpdateCheckItem[]>("check_updates"),

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
