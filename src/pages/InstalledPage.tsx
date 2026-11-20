import { useEffect, useMemo, useState } from "react";
import {
  api,
  countPending,
  MySkillItem,
  placements,
  SkillPlacement,
  statusLabel,
  ToolDetection,
  UpdateCheckItem,
  toolLabel,
} from "../api";

interface Props {
  tools: ToolDetection[];
  updates: UpdateCheckItem[];
  onRefreshUpdates: (u: UpdateCheckItem[]) => void;
  /** 扫描结果由 App 持有，保证侧边栏角标与本页计数同源 */
  items: MySkillItem[];
  onItemsChange: (items: MySkillItem[]) => void;
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
  items,
  onItemsChange,
}: Props) {
  const [health, setHealth] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("bySkill");
  const [query, setQuery] = useState("");
  // 待卸载的技能；placements 决定弹层里给出几个工具的入口
  const [confirmUninstall, setConfirmUninstall] = useState<
    { name: string; placements: SkillPlacement[] } | null
  >(null);

  const load = async () => {
    setLoading(true);
    try {
      onItemsChange(await api.scanSkills());
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

  const [updating, setUpdating] = useState<string | null>(null);

  const update = async (name: string, tool: string) => {
    setUpdating(`${name}-${tool}`);
    setError(null);
    try {
      await api.updateSkill(name, tool);
      await load();
      // 更新后状态必须重算，否则旧蓝标会留在界面上
      const u = await api.checkUpdates().catch(() => []);
      onRefreshUpdates(u);
    } catch (e) {
      setError(String(e));
    } finally {
      setUpdating(null);
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

  // 按工具分组时直接携带落盘配对，避免后面用 indexOf 反查路径
  const byTool = useMemo(() => {
    const map = new Map<string, { item: MySkillItem; spot: SkillPlacement }[]>();
    filtered.forEach((i) => {
      placements(i).forEach((spot) => {
        const arr = map.get(spot.tool) ?? [];
        arr.push({ item: i, spot });
        map.set(spot.tool, arr);
      });
    });
    return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [filtered]);

  // 与侧边栏角标共用同一口径，两处数字在任何时刻都相等
  const pendingCount = countPending(items, updates);

  const statusBadge = (item: MySkillItem) => {
    if (item.identity === "external")
      return <span className="badge">{IDENTITY_LABEL.external}</span>;
    if (item.identity === "local")
      return <span className="badge">{IDENTITY_LABEL.local}</span>;

    // managed：取该技能任一工具下最需要关注的状态，未检查时如实说「未检查」
    const us = placements(item)
      .map((p) => updateOf(item.name, p.tool))
      .filter((x): x is UpdateCheckItem => x !== undefined);
    if (us.length === 0) return <span className="page-sub">未检查</span>;
    const u =
      us.find(
        (x) =>
          x.status === "conflict" ||
          x.status === "localmissing" ||
          x.status === "updateavailable" ||
          x.status === "locallymodified",
      ) ?? us[0];
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
                  const spots = placements(item);
                  // 「本地已有」才提供纳入入口；external 一律不动
                  const adoptable =
                    item.identity === "local" && spots.length === 1;
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
                          {spots.map((p) => (
                            <span key={p.tool} className="badge tool-chip">
                              {toolLabel(p.tool, tools)}
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
                              adopt(item.name, spots[0].tool, spots[0].targetPath)
                            }
                          >
                            纳入管理
                          </button>
                        ) : (
                          item.identity === "managed" && (
                            <div style={{ display: "flex", gap: 6 }}>
                              {/* 只有「可更新」才给入口；其余状态没有可执行的动作 */}
                              {updateOf(item.name, spots[0].tool)?.status ===
                                "updateavailable" && (
                                <button
                                  className="primary sm"
                                  disabled={updating === `${item.name}-${spots[0].tool}`}
                                  onClick={() => update(item.name, spots[0].tool)}
                                >
                                  {updating === `${item.name}-${spots[0].tool}` && (
                                    <span className="spinner" />
                                  )}
                                  更新
                                </button>
                              )}
                              <button
                                className="danger sm"
                                onClick={() =>
                                  setConfirmUninstall({
                                    name: item.name,
                                    placements: spots,
                                  })
                                }
                              >
                                卸载
                              </button>
                            </div>
                          )
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            byTool.map(([tool, rows]) => (
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
                  <span className="badge">{rows.length}</span>
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
                    {rows.map(({ item: it, spot }) => (
                      <tr key={`${it.name}-${spot.tool}`}>
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
                                adopt(it.name, spot.tool, spot.targetPath)
                              }
                            >
                              纳入管理
                            </button>
                          ) : it.identity === "managed" ? (
                            <button
                              className="danger sm"
                              onClick={() =>
                                setConfirmUninstall({
                                  name: it.name,
                                  placements: [spot],
                                })
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
                <b>{confirmUninstall.name}</b>
                {confirmUninstall.placements.length > 1
                  ? " 装在多个工具下，选择要从哪个工具移除："
                  : " 将从以下工具中移除："}
              </div>
              {/* 每个落盘一个入口，卸哪个由用户明确指定 */}
              {confirmUninstall.placements.map((p) => {
                const u = updateOf(confirmUninstall.name, p.tool);
                const risky =
                  u?.status === "locallymodified" || u?.status === "conflict";
                return (
                  <div key={p.tool} style={{ marginTop: 12 }}>
                    <div style={{ fontWeight: 500 }}>
                      {toolLabel(p.tool, tools)}
                      {risky && (
                        <span style={{ color: "var(--warn)", marginLeft: 8 }}>
                          本地有改动，移除后这些改动会丢失
                        </span>
                      )}
                    </div>
                    <div className="mono page-sub">{p.targetPath}</div>
                    <button
                      className="primary sm"
                      style={{
                        background: "var(--danger)",
                        borderColor: "var(--danger)",
                        marginTop: 6,
                      }}
                      onClick={() => uninstall(confirmUninstall.name, p.tool)}
                    >
                      从这个工具卸载
                    </button>
                  </div>
                );
              })}
            </div>
            <div className="modal-foot">
              <button onClick={() => setConfirmUninstall(null)}>取消</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
