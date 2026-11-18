import { useEffect, useState } from "react";
import { api, RemoteSkill, SkillRevision, ToolDetection, toolLabel } from "../api";

interface Props {
  skill: RemoteSkill;
  tools: ToolDetection[];
  installedTools: string[];
  publisher: string;
  onClose: () => void;
  onInstall: () => void;
  onRolledBack: () => void;
}

export default function SkillDetail({
  skill,
  tools,
  installedTools,
  publisher,
  onClose,
  onInstall,
  onRolledBack,
}: Props) {
  const [history, setHistory] = useState<SkillRevision[]>([]);
  const [loadingHistory, setLoadingHistory] = useState(true);
  const [expanded, setExpanded] = useState(false);
  const [rollbackTarget, setRollbackTarget] = useState<SkillRevision | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const h = await api.skillHistory(skill.repoPath, 10);
        setHistory(h);
      } catch (e) {
        // 历史查不到不影响主流程（可能是没权限或网络问题）
        console.warn("查历史失败", e);
      } finally {
        setLoadingHistory(false);
      }
    })();
  }, [skill.repoPath]);

  const doRollback = async () => {
    if (!rollbackTarget) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.rollbackSkill({
        repoPath: skill.repoPath,
        targetSha: rollbackTarget.sha,
        publisher: publisher || "未署名",
        skillName: skill.name,
      });
      setDone(`已回滚到 ${rollbackTarget.shortSha}，新提交 ${r.commitSha.slice(0, 8)}`);
      setRollbackTarget(null);
      onRolledBack();
      const h = await api.skillHistory(skill.repoPath, 10);
      setHistory(h);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div>
            <h2 className="modal-title">{skill.name}</h2>
            <div className="page-sub">
              {skill.category ?? "未分类"}
              {skill.version && ` · v${skill.version}`}
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭">
            ✕
          </button>
        </div>

        <div className="modal-body">
          <div className="field">
            <label>描述</label>
            <div className="detail-text">{skill.description || "（无描述）"}</div>
          </div>

          {skill.tags.length > 0 && (
            <div className="field">
              <label>标签</label>
              <div className="card-meta">
                {skill.tags.map((t) => (
                  <span key={t} className="badge">
                    {t}
                  </span>
                ))}
              </div>
            </div>
          )}

          <div className="field">
            <label>仓库路径</label>
            <div className="mono">{skill.repoPath}</div>
          </div>

          <div className="field">
            <label>已安装到</label>
            {installedTools.length === 0 ? (
              <div className="page-sub">尚未安装</div>
            ) : (
              <div className="card-meta">
                {installedTools.map((k) => (
                  <span key={k} className="badge accent">
                    {toolLabel(k, tools)}
                  </span>
                ))}
              </div>
            )}
          </div>

          {/* 版本历史 */}
          <div className="field">
            <label
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
              }}
            >
              <span>版本历史</span>
              {history.length > 3 && (
                <button
                  className="ghost sm"
                  onClick={() => setExpanded((v) => !v)}
                >
                  {expanded ? "收起" : `展开全部 ${history.length} 条`}
                </button>
              )}
            </label>

            {loadingHistory ? (
              <div className="page-sub">
                <span className="spinner" />
                查询中…
              </div>
            ) : history.length === 0 ? (
              <div className="page-sub">暂无历史记录</div>
            ) : (
              <div className="history">
                {(expanded ? history : history.slice(0, 3)).map((h, i) => (
                  <div className="history-row" key={h.sha}>
                    <div className="history-main">
                      <span className="mono history-sha">{h.shortSha}</span>
                      <span className="history-title" title={h.title}>
                        {h.title}
                      </span>
                    </div>
                    <div className="history-meta">
                      {h.authorName} · {h.createdAt.slice(0, 10)}
                      {i > 0 && (
                        <button
                          className="ghost sm"
                          onClick={() => setRollbackTarget(h)}
                        >
                          回滚到此版本
                        </button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}

            <div className="hint">
              回滚会把该版本的内容重新提交到 master，对其他使用者来说等同于
              一次正常更新。
            </div>
          </div>

          {error && <div className="banner error">{error}</div>}
          {done && <div className="banner info">{done}</div>}
        </div>

        <div className="modal-foot">
          <button onClick={onClose}>关闭</button>
          <button className="primary" onClick={onInstall}>
            {installedTools.length > 0 ? "装到其他工具" : "安装"}
          </button>
        </div>
      </div>

      {rollbackTarget && (
        <div
          className="overlay"
          onClick={(e) => {
            e.stopPropagation();
            setRollbackTarget(null);
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <div>
                <h2 className="modal-title">回滚确认</h2>
              </div>
            </div>
            <div className="modal-body">
              <div className="detail-text">
                将把 <b>{skill.name}</b> 回滚到提交{" "}
                <span className="mono">{rollbackTarget.shortSha}</span>：
                <br />
                <br />
                <i>{rollbackTarget.title}</i>
                <br />
                <br />
                这会直接覆盖 master 上的当前版本，其他使用者下次检查更新时
                会收到这个旧版本。
                <br />
                <br />
                <span style={{ color: "var(--warn)" }}>
                  当前版本会被自动存为快照，如需恢复可以再回滚回来。
                </span>
              </div>
            </div>
            <div className="modal-foot">
              <button onClick={() => setRollbackTarget(null)} disabled={busy}>
                取消
              </button>
              <button className="primary" onClick={doRollback} disabled={busy}>
                {busy && <span className="spinner" />}
                确认回滚
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
