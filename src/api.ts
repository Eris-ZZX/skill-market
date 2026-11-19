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
      return { text: "本地缺失", color: "#d97706" };
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
