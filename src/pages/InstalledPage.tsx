import { useEffect, useMemo, useState } from "react";
import ErrorBanner from "../components/ErrorBanner";
import {
  api,
  ConflictChoice,
  countPending,
  MySkillItem,
  placements,
  RemoteFailure,
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
  /**
   * 错误里的「去设置」按钮用
   *
   * 可选：App 目前没有把 tab 切换下沉给本页。没有传入时按钮不显示，
   * 只保留「重试」这类本页能自处理的动作。
   */
  onNavigateSettings?: () => void;
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
  onNavigateSettings,
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

  // 远端失败原因：有值时顶部显示提示，但列表照常显示（用缓存算出的状态）
  const [failures, setFailures] = useState<RemoteFailure[]>([]);

  /**
   * 统一处理 checkUpdates 的返回
   *
   * 后端现在不会因远端读取失败而整体报错，失败放进 failures 带回来。
   * 所以这里照常接收 items，同时记下 failures 供顶部提示使用。
   */
  const refreshUpdates = async () => {
    try {
      const r = await api.checkUpdates();
      onRefreshUpdates(r.items);
      setFailures(r.failures);
    } catch (e) {
      // 真正返回 Err 的只剩配置缺失这类问题，此时旧状态必须清掉
      onRefreshUpdates([]);
      setFailures([]);
      setError(String(e));
    }
  };

  const uninstall = async (name: string, tool: string) => {
    try {
      await api.uninstallSkill(name, tool);
      setConfirmUninstall(null);
      await load();
      await refreshUpdates();
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
  // 冲突/已修改的处理弹层：{技能名, 工具, 当前状态}
  const [resolving, setResolving] = useState<
    { name: string; tool: string; status: string } | null
  >(null);

  const update = async (name: string, tool: string) => {
    setUpdating(`${name}-${tool}`);
    setError(null);
    try {
      await api.updateSkill(name, tool);
      await load();
      // 更新后状态必须重算，否则旧蓝标会留在界面上
      await refreshUpdates();
    } catch (e) {
      setError(String(e));
    } finally {
      setUpdating(null);
    }
  };

  const resolve = async (
    name: string,
    tool: string,
    choice: ConflictChoice,
  ) => {
    setUpdating(`${name}-${tool}`);
    setError(null);
    try {
      await api.resolveConflict(name, tool, choice);
      setResolving(null);
      await load();
      await refreshUpdates();
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
      // 远端读不到时后端不再整体报错，而是带着 failures 返回缓存结果，
      // 所以这里不需要 catch 兜底清空——列表照常显示
      await refreshUpdates();
      await load();
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
      {failures.length > 0 && (
        <div className="banner warn">
          <div style={{ fontWeight: 500 }}>
            无法刷新远端数据，以下为上次缓存结果，可能不是最新
          </div>
          {failures.map((f, i) => (
            <div key={i} className="page-sub" style={{ marginTop: 4 }}>
              {f.message}
              {f.detail && (
                <details style={{ marginTop: 2 }}>
                  <summary style={{ cursor: "pointer" }}>技术详情</summary>
                  <div className="mono break">{f.detail}</div>
                </details>
              )}
            </div>
          ))}
        </div>
      )}
      {error && (
        <ErrorBanner
          error={error}
          onAction={(action) => {
            // 只有「去设置」有明确的落地页；其余动作交回用户自行处理
            if (action === "open_settings") onNavigateSettings?.();
            else if (action === "retry") void check();
          }}
        />
      )}

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
                              {/* 只有「可更新」才给直接更新入口 */}
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
                              {/* 冲突与已修改都需要用户选一个处理方式 */}
                              {(() => {
                                const u = updateOf(item.name, spots[0].tool);
                                if (!u) return null;
                                if (u.status !== "conflict" && u.status !== "locallymodified")
                                  return null;
                                if (u.ignored)
                                  return <span className="page-sub">已忽略此次更新</span>;
                                return (
                                  <button
                                    className="primary sm"
                                    onClick={() =>
                                      setResolving({
                                        name: item.name,
                                        tool: spots[0].tool,
                                        status: u.status,
                                      })
                                    }
                                  >
                                    处理
                                  </button>
                                );
                              })()}
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

      {resolving && (
        <div className="overlay" onClick={() => setResolving(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <div>
                <h2 className="modal-title">处理 {resolving.name}</h2>
                <div className="page-sub">
                  {resolving.status === "conflict"
                    ? "本地改过，远端也更新了"
                    : "本地改过，远端没有新版本"}
                </div>
              </div>
            </div>
            <div className="modal-body">
              <div className="detail-text">
                选择后会发生什么，逐条写在下面。所有会覆盖或移动你改动的方式，
                都会先把当前版本备份一份。
              </div>

              <div style={{ marginTop: 14 }}>
                <div style={{ fontWeight: 500 }}>保留我的</div>
                <div className="hint">
                  不动任何文件，只记下「这次更新我不需要」。远端下次再变会重新提示。
                </div>
                <button
                  className="primary sm"
                  style={{ marginTop: 6 }}
                  disabled={updating !== null}
                  onClick={() => resolve(resolving.name, resolving.tool, "keepmine")}
                >
                  保留我的
                </button>
              </div>

              <div style={{ marginTop: 14 }}>
                <div style={{ fontWeight: 500 }}>
                  {resolving.status === "conflict" ? "用远端版本" : "重置为仓库版"}
                </div>
                <div className="hint">
                  用仓库里的最新内容覆盖本地，你的改动会先备份。覆盖后本地与本仓库一致。
                </div>
                <button
                  className="danger sm"
                  style={{ marginTop: 6 }}
                  disabled={updating !== null}
                  onClick={() => resolve(resolving.name, resolving.tool, "useremote")}
                >
                  {resolving.status === "conflict" ? "用远端版本" : "重置为仓库版"}
                </button>
              </div>

              {resolving.status === "conflict" && (
                <div style={{ marginTop: 14 }}>
                  <div style={{ fontWeight: 500 }}>另存一份</div>
                  <div className="hint">
                    把你的改动另存为 <span className="mono">{resolving.name}-local</span>
                    （重名会自动加序号），同时装回官方版本。
                  </div>
                  <button
                    className="primary sm"
                    style={{ marginTop: 6 }}
                    disabled={updating !== null}
                    onClick={() => resolve(resolving.name, resolving.tool, "keepboth")}
                  >
                    另存一份
                  </button>
                </div>
              )}
            </div>
            <div className="modal-foot">
              <button onClick={() => setResolving(null)} disabled={updating !== null}>
                取消
              </button>
            </div>
          </div>
        </div>
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
