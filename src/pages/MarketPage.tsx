import { useEffect, useMemo, useState } from "react";
import {
  api,
  AppSettings,
  InstalledSkill,
  RemoteSkill,
  ToolDetection,
  toolLabel,
} from "../api";
import InstallDialog from "../components/InstallDialog";
import SkillDetail from "../components/SkillDetail";

interface Props {
  tools: ToolDetection[];
  settings: AppSettings;
  publisher: string;
}

type SortKey = "name" | "category";

export default function MarketPage({ tools, settings, publisher }: Props) {
  const [skills, setSkills] = useState<RemoteSkill[]>([]);
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string>("");
  const [sort, setSort] = useState<SortKey>("name");
  const [onlyInstalled, setOnlyInstalled] = useState(false);
  const [target, setTarget] = useState<RemoteSkill | null>(null);
  const [detail, setDetail] = useState<RemoteSkill | null>(null);

  const configured = !!(settings.gitlabBaseUrl && settings.gitlabProjectPath);

  const loadCache = async () => {
    try {
      const [cached, inst] = await Promise.all([
        api.listRemoteSkills(),
        api.listInstalled(),
      ]);
      setSkills(cached);
      setInstalled(inst);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void loadCache();
  }, []);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, inst] = await Promise.all([
        api.refreshIndex(),
        api.listInstalled(),
      ]);
      setSkills(list);
      setInstalled(inst);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const categories = useMemo(() => {
    const counts = new Map<string, number>();
    skills.forEach((s) => {
      if (s.category) counts.set(s.category, (counts.get(s.category) ?? 0) + 1);
    });
    return Array.from(counts.entries()).sort((a, b) => b[1] - a[1]);
  }, [skills]);

  const installedTools = (name: string) =>
    installed.filter((i) => i.name === name).map((i) => i.tool);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = skills.filter((s) => {
      if (category && s.category !== category) return false;
      if (onlyInstalled && installedTools(s.name).length === 0) return false;
      if (!q) return true;
      return (
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q))
      );
    });
    list = [...list].sort((a, b) => {
      if (sort === "category") {
        return (a.category ?? "\uffff").localeCompare(b.category ?? "\uffff");
      }
      return a.name.localeCompare(b.name);
    });
    return list;
  }, [skills, query, category, sort, onlyInstalled, installed]);

  const onInstalled = (recs: InstalledSkill[]) => {
    setInstalled((prev) => {
      const kept = prev.filter(
        (p) => !recs.some((r) => r.name === p.name && r.tool === p.tool),
      );
      return [...kept, ...recs];
    });
    setTarget(null);
  };

  if (!configured) {
    return (
      <div className="page">
        <div className="page-head">
          <h1 className="page-title">技能市场</h1>
        </div>
        <div className="empty">
          <div className="empty-title">还没有配置技能仓库</div>
          <div className="empty-hint">
            请先到「设置」填写内网 GitLab 地址和项目路径，
            <br />
            然后回到这里刷新技能列表。
          </div>
        </div>
      </div>
    );
  }

  const totalInstalledNames = new Set(installed.map((i) => i.name)).size;

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">技能市场</h1>
        <span className="page-sub">
          {skills.length > 0 && `${skills.length} 个技能`}
          {skills.length > 0 && totalInstalledNames > 0 && (
            <> · 已装 {totalInstalledNames}</>
          )}
        </span>
        <div className="spacer" />
        <button onClick={refresh} disabled={loading}>
          {loading && <span className="spinner" />}
          {loading ? "拉取中" : "刷新"}
        </button>
      </div>

      {error && <div className="banner error">{error}</div>}

      {skills.length > 0 && (
        <div className="toolbar">
          <input
            className="search"
            placeholder="搜索名称、描述、标签…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <select
            className="select-sm"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
          >
            <option value="">全部分类</option>
            {categories.map(([c, n]) => (
              <option key={c} value={c}>
                {c} ({n})
              </option>
            ))}
          </select>
          <select
            className="select-sm"
            value={sort}
            onChange={(e) => setSort(e.target.value as SortKey)}
          >
            <option value="name">按名称</option>
            <option value="category">按分类</option>
          </select>
          <label className="check">
            <input
              type="checkbox"
              checked={onlyInstalled}
              onChange={(e) => setOnlyInstalled(e.target.checked)}
            />
            只看已装
          </label>
        </div>
      )}

      {filtered.length === 0 ? (
        <div className="empty">
          <div className="empty-title">
            {skills.length === 0 ? "还没有拉到技能" : "没有匹配的技能"}
          </div>
          <div className="empty-hint">
            {skills.length === 0
              ? "点右上角「刷新」从仓库拉取技能列表"
              : "换个关键词或清空筛选条件试试"}
          </div>
        </div>
      ) : (
        <div className="grid">
          {filtered.map((s) => {
            const toolKeys = installedTools(s.name);
            const isInstalled = toolKeys.length > 0;
            return (
              <div className="card" key={s.name}>
                <div className="card-head">
                  <div
                    className="card-title clickable"
                    onClick={() => setDetail(s)}
                    title="查看详情"
                  >
                    {s.name}
                  </div>
                  {isInstalled && <span className="badge accent">已安装</span>}
                </div>

                <div className="card-desc">{s.description || "（无描述）"}</div>

                <div className="card-meta">
                  {s.version && <span className="badge">v{s.version}</span>}
                  {s.category && <span className="meta-cat">{s.category}</span>}
                  {s.tags.slice(0, 2).map((t) => (
                    <span key={t} className="badge">
                      {t}
                    </span>
                  ))}
                </div>

                {isInstalled && (
                  <div className="card-tools">
                    {toolKeys.map((k) => (
                      <span key={k} className="badge tool-chip">
                        {toolLabel(k, tools)}
                      </span>
                    ))}
                  </div>
                )}

                <div className="card-actions">
                  <button className="primary sm" onClick={() => setTarget(s)}>
                    {isInstalled ? "装到其他工具" : "安装"}
                  </button>
                  <button className="ghost sm" onClick={() => setDetail(s)}>
                    详情
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {target && (
        <InstallDialog
          skill={target}
          tools={tools}
          alreadyInstalled={installedTools(target.name)}
          onClose={() => setTarget(null)}
          onDone={onInstalled}
        />
      )}

      {detail && (
        <SkillDetail
          skill={detail}
          tools={tools}
          installedTools={installedTools(detail.name)}
          publisher={publisher}
          onClose={() => setDetail(null)}
          onInstall={() => {
            setDetail(null);
            setTarget(detail);
          }}
          onRolledBack={() => {
            void refresh();
          }}
        />
      )}
    </div>
  );
}
