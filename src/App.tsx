import { useEffect, useState } from "react";
import { api, AppSettings, ToolDetection, UpdateCheckItem } from "./api";
import MarketPage from "./pages/MarketPage";
import InstalledPage from "./pages/InstalledPage";
import SettingsPage from "./pages/SettingsPage";
import PublishPage from "./pages/PublishPage";

type Tab = "market" | "installed" | "publish" | "settings";

export default function App() {
  const [tab, setTab] = useState<Tab>("market");
  const [tools, setTools] = useState<ToolDetection[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [updates, setUpdates] = useState<UpdateCheckItem[]>([]);
  const [ready, setReady] = useState(false);

  // 首次加载：探测工具 + 读设置
  useEffect(() => {
    (async () => {
      try {
        const [t, s] = await Promise.all([api.detectTools(), api.getSettings()]);
        setTools(t);
        setSettings(s);
      } catch (e) {
        console.error("初始化失败", e);
      } finally {
        setReady(true);
      }
    })();
  }, []);

  // 打开应用时静默检查一次更新（只显示红点，不弹窗）
  useEffect(() => {
    if (!ready) return;
    const hasConfig =
      settings?.gitlabBaseUrl && settings?.gitlabProjectPath;
    if (!hasConfig) return;

    api.checkUpdates().then(setUpdates).catch(() => {
      /* 离线或未配置时静默失败 */
    });
  }, [ready, settings?.gitlabBaseUrl, settings?.gitlabProjectPath]);

  const pendingCount = updates.filter(
    (u) =>
      u.status === "updateavailable" ||
      u.status === "conflict" ||
      u.status === "locallymodified" ||
      u.status === "localmissing",
  ).length;
  // 本地缺失和冲突一样是「要人看一眼」的异常，一起触发警示色
  const alertCount = updates.filter(
    (u) => u.status === "conflict" || u.status === "localmissing",
  ).length;

  if (!ready || !settings) {
    return (
      <div className="empty">
        <span className="spinner" />
        正在启动…
      </div>
    );
  }

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">S</span>
          Skill Market
        </div>
        <nav className="nav">
          <button
            className={`nav-item ${tab === "market" ? "active" : ""}`}
            onClick={() => setTab("market")}
          >
            技能市场
          </button>
          <button
            className={`nav-item ${tab === "installed" ? "active" : ""}`}
            onClick={() => setTab("installed")}
          >
            我的技能
            {pendingCount > 0 && (
              <span className={`nav-badge ${alertCount > 0 ? "warn" : ""}`}>
                {pendingCount}
              </span>
            )}
          </button>
          <button
            className={`nav-item ${tab === "publish" ? "active" : ""}`}
            onClick={() => setTab("publish")}
          >
            发布技能
          </button>
          <button
            className={`nav-item ${tab === "settings" ? "active" : ""}`}
            onClick={() => setTab("settings")}
          >
            设置
          </button>
        </nav>
        <div className="sidebar-foot">
          {tools.filter((t) => t.installed).length} / {tools.length} 个工具已安装
        </div>
      </aside>

      <main className="main">
        {tab === "market" && (
          <MarketPage
            tools={tools}
            settings={settings}
            publisher={settings.publisherIdentity}
          />
        )}
        {tab === "installed" && (
          <InstalledPage
            tools={tools}
            updates={updates}
            onRefreshUpdates={setUpdates}
          />
        )}
        {tab === "publish" && (
          <PublishPage
            settings={settings}
            onPublished={() => {
              /* 发布后索引会变，下次进市场页刷新即可 */
            }}
          />
        )}
        {tab === "settings" && (
          <SettingsPage
            tools={tools}
            settings={settings}
            onSaved={setSettings}
            onToolsChanged={setTools}
          />
        )}
      </main>
    </div>
  );
}
