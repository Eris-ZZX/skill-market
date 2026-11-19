import { useEffect, useMemo, useState } from "react";
import {
  api,
  MySkillItem,
  statusLabel,
  ToolDetection,
  UpdateCheckItem,
  toolLabel,
} from "../api";

interface Props {
  tools: ToolDetection[];
  updates: UpdateCheckItem[];
  onRefreshUpdates: (u: UpdateCheckItem[]) => void;
}

type View = "bySkill" | "byTool";

const IDENTITY_LABEL: Record<string, string> = {
  managed: "本工具管理",
  external: "外部管理",
  local: "本地已有",
};

export default function InstalledPage({
  tools,
  updates,
  onRefreshUpdates,
}: Props) {
  // 数据源是磁盘扫描，state.json 只补充来源信息
  const [items, setItems] = useState<MySkillItem[]>([]);
  const [health, setHealth] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("bySkill");
  const [query, setQuery] = useState("");
  const [confirmUninstall, setConfirmUninstall] = useState<
    { name: string; tool: string } | null
  >(null);

  const load = async () => {
    setLoading(true);
    try {
      setItems(await api.scanSkills());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    api.stateHealth().then(setHealth).catch(() => setHealth(null));
  }, []);

  const uninstall = async (name: string, tool: string) => {
    try {
      await api.uninstallSkill(name, tool);
      setConfirmUninstall(null);
      await load();
      const u = await api.checkUpdates().catch(() => []);
      onRefreshUpdates(u);
    } catch (e) {
      setError(String(e));
    }
  };

  const adopt = async (name: string, tool: string, targetPath: string) => {
    try {
      await api.adoptSkill({ name, tool, targetPath });
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  const check = async () => {
    setLoading(true);
    setError(null);
    try {
      const u = await api.checkUpdates();
      onRefreshUpdates(u);
      await load();
    } catch (e) {
      // 上一次的蓝标必须清掉：留着旧结果等于告诉用户「刚检查过」，
      // 而实际上这一轮什么都没查成
      onRefreshUpdates([]);
      setError(`无法连接远端仓库，当前状态未能刷新：${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  const updateOf = (name: string, tool: string) =>
    updates.find((u) => u.name === name && u.tool === tool);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (i) =>
        i.name.toLowerCase().includes(q) ||
        i.description.toLowerCase().includes(q) ||
        i.tools.some((t) => toolLabel(t, tools).toLowerCase().includes(q)),
    );
  }, [items, query, tools]);

  const byTool = useMemo(() => {
    const map = new Map<string, MySkillItem[]>();
    filtered.forEach((i) => {
      i.tools.forEach((t) => {
        const arr = map.get(t) ?? [];
        arr.push(i);
        map.set(t, arr);
      });
    });
    return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [filtered]);

  // 顶部计数沿用「需要人处理」的口径
  const pendingCount = items.filter(
    (i) => i.identity === "local" || i.status === "updateavailable" || i.status === "conflict",
  ).length;

  const statusBadge = (item: MySkillItem) => {
    if (item.identity === "external")
      return <span className="badge">{IDENTITY_LABEL.external}</span>;
    if (item.identity === "local")
      return <span className="badge">{IDENTITY_LABEL.local}</span>;

    // managed：状态来自 check_updates，未检查时如实说「未检查」
    const u = item.tools
      .map((t) => updateOf(item.name, t))
      .find((x) => x !== undefined);
    if (!u) return <span className="page-sub">未检查</span>;
    if (u.status === "uptodate") return <span className="page-sub">最新</span>;
    const st = statusLabel(u.status);
    // 需处理和本地缺失都是「要人看一眼」的异常，用现有警示色；
    // 可更新才是蓝色推进色
    const warnish = u.status === "conflict" || u.status === "localmissing";
    const cls = warnish ? "badge warn" : "badge accent";
    return <span className={cls}>{st.text}</span>;
  };

  if (loading && items.length === 0) {
    return (
      <div className="page">
        <div className="empty">
          <span className="spinner" />
          载入中…
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">我的技能</h1>
        <span className="page-sub">
          {items.length} 个技能
          {pendingCount > 0 && ` · ${pendingCount} 项待处理`}
        </span>
        <div className="spacer" />
        <button onClick={check} disabled={loading}>
          {loading && <span className="spinner" />}
          检查更新
        </button>
      </div>

      {health && <div className="banner warn">{health}</div>}
      {error && <div className="banner error">{error}</div>}

      {items.length === 0 ? (
        <div className="empty">
          <div className="empty-title">本机还没有任何技能</div>
          <div className="empty-hint">
            到「技能市场」挑一个技能，选择目标工具后安装。
          </div>
        </div>
      ) : (
        <>
          <div className="toolbar">
            <input
              className="search"
              placeholder="搜索技能名、描述、工具…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <div style={{ display: "flex", gap: 0 }}>
              <button
                className={view === "bySkill" ? "primary sm" : "sm"}
                style={{ borderRadius: "6px 0 0 6px" }}
                onClick={() => setView("bySkill")}
              >
                按技能
              </button>
              <button
                className={view === "byTool" ? "primary sm" : "sm"}
                style={{ borderRadius: "0 6px 6px 0", marginLeft: -1 }}
                onClick={() => setView("byTool")}
              >
                按工具
              </button>
            </div>
          </div>

          {view === "bySkill" ? (
            <table className="table">
              <thead>
                <tr>
                  <th>技能</th>
                  <th>已装工具</th>
                  <th>身份</th>
                  <th>版本</th>
                  <th style={{ width: 150 }}></th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((item) => {
                  // 「本地已有」才提供纳入入口；external 一律不动
                  const adoptable =
                    item.identity === "local" && item.tools.length === 1;
                  return (
                    <tr key={item.name}>
                      <td style={{ fontWeight: 500 }}>
                        {item.name}
                        {item.description && (
                          <div className="page-sub">{item.description}</div>
                        )}
                      </td>
                      <td>
                        <div className="card-meta">
                          {item.tools.map((t) => (
                            <span key={t} className="badge tool-chip">
                              {toolLabel(t, tools)}
                            </span>
                          ))}
                        </div>
                      </td>
                      <td>{statusBadge(item)}</td>
                      <td>{item.installedVersion ?? item.version ?? "—"}</td>
                      <td>
                        {adoptable ? (
                          <button
                            className="primary sm"
                            onClick={() =>
                              adopt(item.name, item.tools[0], item.targetPaths[0])
                            }
                          >
                            纳入管理
                          </button>
                        ) : (
                          item.identity === "managed" &&
                          item.tools.length === 1 && (
                            <button
                              className="danger sm"
                              onClick={() =>
                                setConfirmUninstall({
                                  name: item.name,
                                  tool: item.tools[0],
                                })
                              }
                            >
                              卸载
                            </button>
                          )
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            byTool.map(([tool, recs]) => (
              <div key={tool} style={{ marginBottom: 20 }}>
                <div
                  style={{
                    fontSize: 13,
                    fontWeight: 600,
                    marginBottom: 8,
                    display: "flex",
                    alignItems: "center",
                    gap: 6,
                  }}
                >
                  {toolLabel(tool, tools)}
                  <span className="badge">{recs.length}</span>
                </div>
                <table className="table">
                  <thead>
                    <tr>
                      <th>技能</th>
                      <th>版本</th>
                      <th>身份</th>
                      <th style={{ width: 150 }}></th>
                    </tr>
                  </thead>
                  <tbody>
                    {recs.map((it) => (
                      <tr key={it.name}>
                        <td>
                          <div style={{ fontWeight: 500 }}>{it.name}</div>
                          {it.description && (
                            <div className="page-sub">{it.description}</div>
                          )}
                        </td>
                        <td>{it.installedVersion ?? it.version ?? "—"}</td>
                        <td>{statusBadge(it)}</td>
                        <td>
                          {it.identity === "local" ? (
                            <button
                              className="primary sm"
                              onClick={() =>
                                adopt(
                                  it.name,
                                  tool,
                                  it.targetPaths[it.tools.indexOf(tool)] ??
                                    it.targetPaths[0],
                                )
                              }
                            >
                              纳入管理
                            </button>
                          ) : it.identity === "managed" ? (
                            <button
                              className="danger sm"
                              onClick={() =>
                                setConfirmUninstall({ name: it.name, tool })
                              }
                            >
                              卸载
                            </button>
                          ) : null}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ))
          )}
        </>
      )}

      {confirmUninstall && (
        <div className="overlay" onClick={() => setConfirmUninstall(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <div>
                <h2 className="modal-title">卸载技能</h2>
              </div>
            </div>
            <div className="modal-body">
              <div className="detail-text">
                将从 <b>{toolLabel(confirmUninstall.tool, tools)}</b> 中移除
                <b> {confirmUninstall.name}</b>。
                <br />
                <br />
                目标目录里的文件会被删除。
                {(() => {
                  const u = updateOf(confirmUninstall.name, confirmUninstall.tool);
                  return u?.status === "locallymodified" ||
                    u?.status === "conflict" ? (
                    <>
                      <br />
                      <br />
                      <span style={{ color: "var(--warn)" }}>
                        注意：该技能本地有改动，卸载后这些改动会丢失。
                      </span>
                    </>
                  ) : null;
                })()}
              </div>
            </div>
            <div className="modal-foot">
              <button onClick={() => setConfirmUninstall(null)}>取消</button>
              <button
                className="primary"
                style={{ background: "var(--danger)", borderColor: "var(--danger)" }}
                onClick={() =>
                  uninstall(confirmUninstall.name, confirmUninstall.tool)
                }
              >
                确认卸载
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
