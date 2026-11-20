import { useState } from "react";
import {
  api,
  INSTALL_MODE_LABEL,
  InstallMode,
  InstallOutcome,
  InstalledSkill,
  RemoteSkill,
  ToolDetection,
  toolLabel,
} from "../api";

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
  const [usedMode, setUsedMode] = useState<InstallMode | null>(null);

  const toggle = (key: string) => {
    setSelected((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key],
    );
  };

  const [outcome, setOutcome] = useState<InstallOutcome | null>(null);

  const submit = async (allowCreate: boolean, onlyTools?: string[]) => {
    setBusy(true);
    setError(null);
    setOutcome(null);
    const targets = onlyTools ?? selected;
    try {
      const result = await api.installSkill({
        name: skill.name,
        repoPath: skill.repoPath,
        tools: targets,
        allowCreateSkillsDir: allowCreate,
      });
      // 记录实际采用的方式：Auto 可能被解析成 junction 或 copy
      setUsedMode(result.succeeded[0]?.mode ?? null);
      setOutcome(result);
      // 成功的已经落盘并记录，先通知外层刷新
      if (result.succeeded.length > 0) onDone(result.succeeded);
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

              {usedMode && (
                <div className="hint">
                  本次使用方式：{INSTALL_MODE_LABEL[usedMode]}
                  {usedMode === "copy" && "（每个工具各存一份副本）"}
                  {(usedMode === "junction" || usedMode === "symlink") &&
                    "（各工具目录里是指向 library 的指针）"}
                </div>
              )}

              {/* 逐工具结果：一个工具失败不影响其它，失败的可单独重试 */}
              {outcome && (
                <div className="field" style={{ marginTop: 10 }}>
                  <label>安装结果</label>
                  <div className="tool-list">
                    {outcome.succeeded.map((r) => (
                      <div className="tool-row" key={`ok-${r.tool}`}>
                        <span className="badge accent">✓ 已安装</span>
                        <span>{toolLabel(r.tool, tools)}</span>
                      </div>
                    ))}
                    {outcome.failed.map((f) => (
                      <div className="tool-row" key={`fail-${f.tool}`}>
                        <span className="badge warn">✗ 失败</span>
                        <span>{toolLabel(f.tool, tools)}</span>
                        <span className="page-sub" style={{ marginLeft: 8 }}>
                          {f.message}
                        </span>
                        {f.retryable && (
                          <button
                            className="ghost sm"
                            style={{ marginLeft: "auto" }}
                            disabled={busy}
                            onClick={() => submit(false, [f.tool])}
                          >
                            重试
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
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
