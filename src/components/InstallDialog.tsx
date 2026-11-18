import { useState } from "react";
import { api, InstalledSkill, RemoteSkill, ToolDetection } from "../api";

interface Props {
  skill: RemoteSkill;
  tools: ToolDetection[];
  alreadyInstalled: string[];
  onClose: () => void;
  onDone: (recs: InstalledSkill[]) => void;
}

export default function InstallDialog({
  skill,
  tools,
  alreadyInstalled,
  onClose,
  onDone,
}: Props) {
  const available = tools.filter(
    (t) => t.installed && !alreadyInstalled.includes(t.key),
  );
  const [selected, setSelected] = useState<string[]>(
    available.length > 0 ? [available[0].key] : [],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [needCreateDir, setNeedCreateDir] = useState(false);

  const toggle = (key: string) => {
    setSelected((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key],
    );
  };

  const submit = async (allowCreate: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const recs = await api.installSkill({
        name: skill.name,
        repoPath: skill.repoPath,
        tools: selected,
        allowCreateSkillsDir: allowCreate,
      });
      onDone(recs);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("需要创建")) setNeedCreateDir(true);
      setError(msg);
    } finally {
      setBusy(false);
    }
  };

  const noToolInstalled = tools.every((t) => !t.installed);
  const selectable = tools.filter(
    (t) => t.installed && !alreadyInstalled.includes(t.key),
  );
  const allSelected =
    selectable.length > 0 &&
    selectable.every((t) => selected.includes(t.key));

  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div>
            <h2 className="modal-title">安装 {skill.name}</h2>
            <div className="page-sub">{skill.description || "（无描述）"}</div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭">
            ✕
          </button>
        </div>

        <div className="modal-body">
          {noToolInstalled ? (
            <div className="banner warn">
              没有检测到任何已安装的 AI 工具。请先安装至少一个支持的工具，
              再回来这里。
            </div>
          ) : (
            <div className="field">
              <label
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
              >
                <span>选择目标工具</span>
                {selectable.length > 1 && (
                  <button
                    className="ghost sm"
                    onClick={() =>
                      setSelected(allSelected ? [] : selectable.map((t) => t.key))
                    }
                  >
                    {allSelected ? "全不选" : "全选"}
                  </button>
                )}
              </label>

              <div className="tool-list">
                {tools.map((t) => {
                  const done = alreadyInstalled.includes(t.key);
                  const usable = t.installed && !done;
                  return (
                    <label
                      className="tool-row"
                      key={t.key}
                      style={{
                        opacity: usable ? 1 : 0.5,
                        cursor: usable ? "pointer" : "not-allowed",
                      }}
                    >
                      <input
                        type="checkbox"
                        checked={selected.includes(t.key)}
                        disabled={!usable}
                        onChange={() => toggle(t.key)}
                      />
                      <span className={`dot ${t.installed ? "on" : "off"}`} />
                      <span>{t.displayName}</span>
                      <span style={{ marginLeft: "auto" }}>
                        {!t.installed ? (
                          <span className="badge">未安装</span>
                        ) : done ? (
                          <span className="badge accent">已装</span>
                        ) : !t.skillsDirExists ? (
                          <span className="badge warn">缺 skills 目录</span>
                        ) : null}
                      </span>
                    </label>
                  );
                })}
              </div>

              {alreadyInstalled.length > 0 && (
                <div className="hint">
                  该技能已安装在：{alreadyInstalled.join("、")}
                </div>
              )}
            </div>
          )}

          {error && (
            <div className="banner error" style={{ marginBottom: 0 }}>
              {error}
              {needCreateDir && (
                <div style={{ marginTop: 8 }}>
                  <button
                    className="primary sm"
                    disabled={busy}
                    onClick={() => submit(true)}
                  >
                    确认创建 skills 目录并安装
                  </button>
                </div>
              )}
            </div>
          )}
        </div>

        <div className="modal-foot">
          <button onClick={onClose} disabled={busy}>
            取消
          </button>
          <button
            className="primary"
            disabled={busy || selected.length === 0 || noToolInstalled}
            onClick={() => submit(false)}
          >
            {busy && <span className="spinner" />}
            安装到 {selected.length} 个工具
          </button>
        </div>
      </div>
    </div>
  );
}
