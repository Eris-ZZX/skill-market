import { useEffect, useMemo, useState } from "react";
import BackupDialog from "../components/BackupDialog";
import ErrorBanner from "../components/ErrorBanner";
import InstallDialog from "../components/InstallDialog";
import UpdateDialog from "../components/UpdateDialog";
import { MatrixCell, MatrixHeader } from "../components/MatrixCell";
import {
  api,
  ConflictChoice,
  countPending,
  MySkillItem,
  matrixCells,
  MatrixCellState,
  RemoteSkill,
  parseError,
  resolveBulkScope,
  applyFilters,
  EMPTY_FILTERS,
  FilterState,
  StatusFilter,
  IdentityFilter,
  pendingUpdates,
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
  // 矩阵里点未装格子时打开的安装弹窗
  const [installTarget, setInstallTarget] = useState<{
    skill: RemoteSkill;
    preselect: string[];
  } | null>(null);
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

  const [resyncing, setResyncing] = useState(false);
  const [resyncResult, setResyncResult] = useState<string | null>(null);
  const [showBackups, setShowBackups] = useState(false);
  const [showUpdates, setShowUpdates] = useState(false);

  /**
   * 检查并修复「记录还在但目录没了」的技能
   *
   * 被手改过的副本会被跳过——修的是「不见了」，不是「我想覆盖」。
   */
  const resync = async () => {
    setResyncing(true);
    setError(null);
    setResyncResult(null);
    try {
      const r = await api.resyncSkills();
      const lines: string[] = [];
      if (r.repaired.length > 0) lines.push(`✓ ${r.repaired.length} 个技能已修复`);
      if (r.skipped.length > 0) lines.push(`${r.skipped.length} 个无需操作`);
      if (r.skippedModified.length > 0) {
        const names = r.skippedModified
          .map((f) => `${f.name} (${toolLabel(f.tool, tools)})`)
          .join("、");
        lines.push(`⚠ ${r.skippedModified.length} 个已跳过（本地有改动）：${names}`);
      }
      if (r.failed.length > 0) {
        const names = r.failed
          .map((f) => `${f.name} (${toolLabel(f.tool, tools)}) — ${f.message}`)
          .join("；");
        lines.push(`⚠ ${r.failed.length} 个失败：${names}`);
      }
      setResyncResult(lines.length > 0 ? lines.join("　") : "所有技能状态正常");
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setResyncing(false);
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

  /** 点「检查更新」：先真实刷新一次，再打开弹窗——按钮上写检查，就得真检查 */
  const checkAndOpen = async () => {
    await check();
    setShowUpdates(true);
  };

  /**
   * 全部更新（只针对 updateavailable）
   *
   * 逐条独立：用循环而不是批量接口，一条失败不影响其余——与 #32 的
   * 批量安装同一原则。冲突与已修改不在此列，它们需要用户做选择。
   */
  const updateAll = async () => {
    const targets = pendingRows.filter((r) => r.status === "updateavailable");
    if (targets.length === 0) return;
    setLoading(true);
    setError(null);
    const failures: string[] = [];
    try {
      for (const t of targets) {
        try {
          await api.updateSkill(t.name, t.tool);
        } catch (e) {
          failures.push(`${t.name}（${parseError(e).message}）`);
        }
      }
      await load();
      await refreshUpdates();
      if (failures.length > 0) {
        setError(`全部更新完成，但 ${failures.length} 项失败：${failures.join("；")}`);
      }
    } finally {
      setLoading(false);
    }
  };

  const updateOf = (name: string, tool: string) =>
    updates.find((u) => u.name === name && u.tool === tool);

  // 筛选状态统一放在组件顶部：#56 的批量操作读的必须是这份筛选结果，
  // 页面里不能再另算一份近似版本
  const [filters, setFilters] = useState<FilterState>(EMPTY_FILTERS);

  const filtered = useMemo(
    () => applyFilters(items, updates, filters),
    [items, updates, filters],
  );

  // 与侧边栏角标共用同一口径，两处数字在任何时刻都相等
  const pendingCount = countPending(items, updates);

  // 弹窗用的列表：「一眼看全部」需要的数据。
  // 与矩阵里的就地「更新」是两条入口、同一个 update()。
  const pendingRows = useMemo(
    () => pendingUpdates(items, updates),
    [items, updates],
  );
  const hasConflict = pendingRows.some((r) => r.status === "conflict");

  // 只显示本机已装的工具列：6 列 × 36px 在 640px 窗口里会挤
  const matrixTools = useMemo(
    () => tools.filter((t) => t.installed).map((t) => t.key),
    [tools],
  );

  // 每列已装数量（格子里是实心的那些）
  const matrixCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const t of matrixTools) counts[t] = 0;
    for (const item of filtered) {
      for (const t of item.tools) {
        if (t in counts) counts[t] += 1;
      }
    }
    return counts;
  }, [filtered, matrixTools]);

  // 批量操作进行中，避免连点
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkResult, setBulkResult] = useState<string | null>(null);
  // 当前打开菜单的列（null = 没开）
  const [openColumn, setOpenColumn] = useState<string | null>(null);
  // 行级菜单打开的技能名
  const [rowMenu, setRowMenu] = useState<string | null>(null);

  /**
   * 批量执行：逐条独立
   *
   * 用 allSettled 而不是 all —— all 遇到第一个 reject 就中断，
   * 「批量装 5 个，第 3 个失败」会变成「后 2 个根本没尝试」。
   * 这里要的是「能成的都成，失败的单独报」。
   */
  const runBulk = async (
    names: string[],
    tool: string,
    wantInstalled: boolean,
  ) => {
    if (names.length === 0) return;
    setBulkBusy(true);
    setBulkResult(null);
    setError(null);
    try {
      // 索引只查一次：循环里重复读同一份磁盘缓存没有意义，
      // 缓存变大后会变成真实的性能问题
      const remote = wantInstalled ? await api.listRemoteSkills() : [];

      const results = await Promise.allSettled(
        names.map(async (name) => {
          if (!wantInstalled) {
            await api.uninstallSkill(name, tool);
            return name;
          }
          const hit = remote.find((r) => r.name === name);
          if (!hit) throw new Error(`仓库里找不到 ${name}`);
          await api.installSkill({
            name: hit.name,
            repoPath: hit.repoPath,
            tools: [tool],
            allowCreateSkillsDir: true,
          });
          return name;
        }),
      );

      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failures = results
        .map((r, i) => ({ r, name: names[i] }))
        .filter((x) => x.r.status === "rejected")
        .map((x) => {
          const reason = (x.r as PromiseRejectedResult).reason;
          const msg = parseError(reason).message;
          return `${x.name}（${msg}）`;
        });

      setBulkResult(
        failures.length === 0
          ? `批量完成：${ok} 个成功`
          : `批量完成：${ok} 个成功，${failures.length} 个失败 —— ${failures.join("；")}`,
      );
      await load();
    } finally {
      setBulkBusy(false);
    }
  };

  /** 矩阵格子点击：已装 → 卸载确认；未装 → 打开安装弹窗 */
  const onMatrixClick = (item: MySkillItem, cell: MatrixCellState) => {
    if (!cell.installed) {
      void openInstallFor(item, cell.tool);
      return;
    }
    // 外部管理的技能不是本工具装的，删了就是在动别人的东西
    if (item.externallyManaged) {
      setError(
        JSON.stringify({
          code: "EXTERNAL_MANAGED",
          message: "该技能由外部工具管理，请在原工具中操作",
          detail: cell.targetPath ?? "",
          action: null,
        }),
      );
      return;
    }
    setConfirmUninstall({ name: item.name, placements: [{ tool: cell.tool, targetPath: cell.targetPath ?? "" }] });
  };

  /** 未装格子 → 从缓存索引里找到这条远端技能，打开安装弹窗并预选该工具 */
  const openInstallFor = async (item: MySkillItem, tool: string) => {
    try {
      const remote = await api.listRemoteSkills();
      const hit = remote.find((r) => r.name === item.name);
      if (!hit) {
        setError(
          JSON.stringify({
            code: "SKILL_NOT_FOUND",
            message: "仓库里找不到这个技能（可能已被删除）",
            detail: `本地有 ${item.name}，但远端索引里没有同名技能`,
            action: null,
          }),
        );
        return;
      }
      setInstallTarget({ skill: hit, preselect: [tool] });
    } catch (e) {
      setError(String(e));
    }
  };

  /**
   * 更新状态标记
   *
   * `showExternalMark` 为 false 时，外部管理的技能不返回任何标记——矩阵的
   * 操作列已经显示「外部管理」，这里再显示一次就是重复。
   */
  const statusBadge = (item: MySkillItem, showExternalMark = true) => {
    // 外部管理的技能不参与更新判定（不是本客户端装的，改不改变无从比较）
    if (item.externallyManaged) {
      return showExternalMark ? (
        <span className="badge">外部管理</span>
      ) : null;
    }

    // 无来源时 check_updates 会返回 UpToDate，那是「没得比」而不是「最新」。
    // 显示「最新」等于骗用户，必须如实说无来源。
    if (!item.hasRemoteSource)
      return <span className="page-sub">无来源</span>;

    // 取该技能任一工具下最需要关注的状态，未检查时如实说「未检查」
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
        <button onClick={() => setShowBackups(true)} disabled={loading}>
          备份
        </button>
        <button onClick={resync} disabled={loading || resyncing}>
          {resyncing && <span className="spinner" />}
          检查并修复
        </button>
        <button onClick={checkAndOpen} disabled={loading}>
          {loading && <span className="spinner" />}
          检查更新
          {pendingRows.length > 0 && (
            <span
              className={`nav-badge ${hasConflict ? "warn" : ""}`}
              style={{ marginLeft: 4 }}
            >
              {pendingRows.length}
            </span>
          )}
        </button>
      </div>

      {resyncResult && <div className="banner info">{resyncResult}</div>}

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
              value={filters.query}
              onChange={(e) =>
                setFilters((f) => ({ ...f, query: e.target.value }))
              }
            />
            <select
              className="select-sm"
              value={filters.status}
              onChange={(e) =>
                setFilters((f) => ({
                  ...f,
                  status: e.target.value as StatusFilter,
                }))
              }
              title="按状态筛选"
            >
              <option value="all">状态：全部</option>
              <option value="updates">有更新</option>
              <option value="local-modified">本地已改</option>
              <option value="conflict">需处理</option>
              <option value="none">无问题</option>
            </select>
            <select
              className="select-sm"
              value={filters.tool}
              onChange={(e) =>
                setFilters((f) => ({ ...f, tool: e.target.value }))
              }
              title="按工具筛选"
            >
              <option value="all">工具：全部</option>
              {tools
                .filter((t) => t.installed)
                .map((t) => (
                  <option key={t.key} value={t.key}>
                    {t.displayName}
                  </option>
                ))}
            </select>
            <select
              className="select-sm"
              value={filters.identity}
              onChange={(e) =>
                setFilters((f) => ({
                  ...f,
                  identity: e.target.value as IdentityFilter,
                }))
              }
              title="按身份筛选"
            >
              <option value="all">来源：全部</option>
              <option value="external">只看外部管理</option>
            </select>
            <span className="page-sub" style={{ whiteSpace: "nowrap" }}>
              显示 {filtered.length} / {items.length}
            </span>
          </div>

          {openColumn && (
                <div className="matrix-popover">
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>
                    {toolLabel(openColumn, tools)}
                  </div>
                  {(() => {
                    const installed = resolveBulkScope(
                      filtered,
                      openColumn,
                      true,
                    ).alreadyDone.length;
                    const toInstall = filtered.length - installed;
                    return (
                      <>
                        <div className="page-sub" style={{ marginBottom: 6 }}>
                          {installed} / {filtered.length} 个技能已装
                          {filters.query && `（当前筛选：${filtered.length} 个）`}
                        </div>
                        <button
                          className="ghost sm"
                          disabled={bulkBusy || toInstall === 0}
                          onClick={() => {
                            const scope = resolveBulkScope(
                              filtered,
                              openColumn,
                              true,
                            );
                            setOpenColumn(null);
                            void runBulk(scope.targets, openColumn, true);
                          }}
                        >
                          装上其余 {toInstall} 个
                        </button>
                        <button
                          className="ghost sm"
                          disabled={bulkBusy || installed === 0}
                          onClick={() => {
                            const scope = resolveBulkScope(
                              filtered,
                              openColumn,
                              false,
                            );
                            setOpenColumn(null);
                            void runBulk(scope.targets, openColumn, false);
                          }}
                        >
                          全部卸载（{installed} 个）
                        </button>
                        <button
                          className="ghost sm"
                          onClick={() => {
                            // 只看这一列 = 把工具筛选设为该工具
                            setFilters((f) => ({ ...f, tool: openColumn }));
                            setOpenColumn(null);
                          }}
                        >
                          只看这一列
                        </button>
                      </>
                    );
                  })()}
                </div>
              )}
              {bulkResult && (
                <div className="banner info" style={{ marginBottom: 8 }}>
                  {bulkResult}
                </div>
              )}
              <table className="matrix">
              <thead>
                <tr>
                  <th className="matrix-name">技能</th>
                  {matrixTools.map((t) => (
                    <MatrixHeader
                      key={t}
                      toolKey={t}
                      label={toolLabel(t, tools)}
                      count={matrixCounts[t] ?? 0}
                      total={filtered.length}
                      active={openColumn === t}
                      onClick={() =>
                        setOpenColumn((prev) => (prev === t ? null : t))
                      }
                    />
                  ))}
                  <th className="matrix-identity">操作</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((item) => (
                  <tr key={`${item.name}-${item.targetPaths[0] ?? ""}`}>
                    <td className="matrix-name">
                      <div
                        style={{ display: "flex", alignItems: "center", gap: 8 }}
                      >
                        <span style={{ fontWeight: 500 }}>{item.name}</span>
                        {/* 外部管理标记在操作列，这里只显示更新状态 */}
                        {statusBadge(item, false)}
                        <span
                          className="page-sub"
                          style={{ marginLeft: "auto", whiteSpace: "nowrap" }}
                        >
                          {item.installedVersion ?? item.version ?? "—"}
                        </span>
                      </div>
                      {item.description && (
                        <div className="page-sub">{item.description}</div>
                      )}
                    </td>
                    {matrixCells(item, matrixTools, updates, []).map((cell) => (
                      <td key={cell.tool} style={{ textAlign: "center" }}>
                        <MatrixCell
                          cell={cell}
                          skillName={item.name}
                          toolLabel={toolLabel(cell.tool, tools)}
                          external={item.externallyManaged}
                          onClick={() => onMatrixClick(item, cell)}
                        />
                      </td>
                    ))}
                    <td className="matrix-identity">
                      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                        {/* 身份三分类已废弃；只剩「外部管理」这一个需要提示的标记 */}
                        {item.externallyManaged && (
                          <span className="badge">外部管理</span>
                        )}
                        {/* 状态标记在名称旁，操作入口在这里——看到「可更新」就要能点 */}
                        {(() => {
                          const first = item.tools[0];
                          if (!first) return null;
                          const u = updateOf(item.name, first);
                          if (!u) return null;
                          if (u.status === "updateavailable") {
                            return (
                              <button
                                className="primary sm"
                                disabled={updating === `${item.name}-${first}`}
                                onClick={() => update(item.name, first)}
                              >
                                {updating === `${item.name}-${first}` && (
                                  <span className="spinner" />
                                )}
                                更新
                              </button>
                            );
                          }
                          if (
                            (u.status === "conflict" ||
                              u.status === "locallymodified") &&
                            !u.ignored
                          ) {
                            return (
                              <button
                                className="primary sm"
                                onClick={() =>
                                  setResolving({
                                    name: item.name,
                                    tool: first,
                                    status: u.status,
                                  })
                                }
                              >
                                处理
                              </button>
                            );
                          }
                          return null;
                        })()}
                        <button
                          className="ghost sm"
                          title="批量操作"
                          disabled={bulkBusy}
                          onClick={() =>
                            setRowMenu((prev) =>
                              prev === item.name ? null : item.name,
                            )
                          }
                        >
                          ⋯
                        </button>
                      </div>
                      {rowMenu === item.name && (
                        <div className="matrix-menu">
                          <button
                            className="ghost sm"
                            disabled={bulkBusy || matrixTools.length === 0}
                            onClick={() => {
                              const names = matrixTools.filter(
                                (t) => !item.tools.includes(t),
                              );
                              setRowMenu(null);
                              // 行级「装到所有工具」= 逐个工具单独装，
                              // 这样某个工具失败不影响其余
                              void (async () => {
                                for (const t of names) {
                                  await runBulk([item.name], t, true);
                                  // 每轮都刷新，避免用旧 items 重复装
                                  await load();
                                }
                              })();
                            }}
                          >
                            装到所有工具
                          </button>
                          <button
                            className="ghost sm"
                            disabled={bulkBusy || item.tools.length === 0}
                            onClick={() => {
                              setRowMenu(null);
                              void (async () => {
                                for (const t of item.tools) {
                                  await runBulk([item.name], t, false);
                                  await load();
                                }
                              })();
                            }}
                          >
                            从所有工具卸载
                          </button>
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
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

      {showUpdates && (
        <UpdateDialog
          rows={pendingRows}
          tools={tools}
          updating={updating}
          onUpdate={(name, tool) => void update(name, tool)}
          onResolve={(name, tool, status) => {
            setShowUpdates(false);
            setResolving({ name, tool, status });
          }}
          onUpdateAll={() => void updateAll()}
          onClose={() => setShowUpdates(false)}
        />
      )}

      {showBackups && (
        <BackupDialog
          tools={tools}
          onClose={() => setShowBackups(false)}
          onChanged={() => void load()}
        />
      )}

      {installTarget && (
        <InstallDialog
          skill={installTarget.skill}
          tools={tools}
          alreadyInstalled={items
            .filter((i) => i.name === installTarget.skill.name)
            .flatMap((i) => i.tools)}
          preselect={installTarget.preselect}
          onClose={() => setInstallTarget(null)}
          onDone={() => {
            setInstallTarget(null);
            void load();
          }}
        />
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
