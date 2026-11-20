import { useEffect, useState } from "react";
import { api, SkillBackupEntry, toolLabel, ToolDetection } from "../api";
import ErrorBanner from "./ErrorBanner";

interface Props {
  tools: ToolDetection[];
  onClose: () => void;
  /** 恢复或删除后通知外层刷新「我的技能」 */
  onChanged: () => void;
}

function fmtTime(unixSec: number): string {
  if (!unixSec) return "—";
  return new Date(unixSec * 1000).toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * 备份列表：恢复 / 删除
 *
 * 后端已有备份能力（#29），这里只是把它露出来——没有入口的备份等于没做。
 */
export default function BackupDialog({ tools, onClose, onChanged }: Props) {
  const [list, setList] = useState<SkillBackupEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<SkillBackupEntry | null>(
    null,
  );

  const load = async () => {
    setLoading(true);
    try {
      setList(await api.listSkillBackups());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const restore = async (b: SkillBackupEntry) => {
    setError(null);
    setNotice(null);
    try {
      const r = await api.restoreSkillBackup(b.backupId);
      onChanged();
      await load();
      const tool = toolLabel(b.tool, tools);
      setNotice(
        r.pendingTool
          ? `已恢复到本地库，但 ${tool} 未安装。装好之后再点「检查并修复」即可链过去。`
          : `已恢复到本地库。点「检查并修复」可把它推到 ${tool} 的技能目录。`,
      );
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (b: SkillBackupEntry) => {
    setError(null);
    try {
      await api.deleteSkillBackup(b.backupId);
      setConfirmDelete(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="overlay" onClick={onClose}>
      <div
        className="modal"
        style={{ width: 620 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <div>
            <h2 className="modal-title">备份（共 {list.length} 份）</h2>
            <div className="page-sub">
              更新与冲突处理前会自动创建。恢复只写回技能库，之后可用「检查并修复」推到各工具。
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭">
            ✕
          </button>
        </div>

        <div className="modal-body">
          {error && <ErrorBanner error={error} />}
          {notice && <div className="banner info">{notice}</div>}

          {loading ? (
            <div className="empty">
              <span className="spinner" />
              载入中…
            </div>
          ) : list.length === 0 ? (
            <div className="empty">
              <div className="empty-title">暂无备份</div>
              <div className="empty-hint">
                更新或冲突处理时会自动创建。
              </div>
            </div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>时间</th>
                  <th>技能</th>
                  <th>工具</th>
                  <th>版本</th>
                  <th>大小</th>
                  <th style={{ width: 130 }}></th>
                </tr>
              </thead>
              <tbody>
                {list.map((b) => (
                  <tr key={b.backupId}>
                    <td className="page-sub">{fmtTime(b.backupCreatedAt)}</td>
                    <td style={{ fontWeight: 500 }}>{b.name}</td>
                    <td className="page-sub">{toolLabel(b.tool, tools)}</td>
                    <td>{b.installedVersion ?? "—"}</td>
                    <td className="page-sub">{fmtSize(b.sizeBytes)}</td>
                    <td>
                      <div style={{ display: "flex", gap: 6 }}>
                        <button
                          className="primary sm"
                          disabled={!b.isRestorable}
                          title={
                            b.isRestorable
                              ? "恢复到技能库"
                              : "库里已有同名技能，请先卸载"
                          }
                          onClick={() => restore(b)}
                        >
                          恢复
                        </button>
                        <button
                          className="danger sm"
                          onClick={() => setConfirmDelete(b)}
                        >
                          删除
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <div className="modal-foot">
          <button onClick={onClose}>关闭</button>
        </div>
      </div>

      {confirmDelete && (
        <div
          className="overlay"
          onClick={(e) => {
            e.stopPropagation();
            setConfirmDelete(null);
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <div>
                <h2 className="modal-title">删除备份</h2>
              </div>
            </div>
            <div className="modal-body">
              <div className="detail-text">
                将删除 <b>{confirmDelete.name}</b> 在{" "}
                {fmtTime(confirmDelete.backupCreatedAt)} 的备份。
                <br />
                <br />
                <span style={{ color: "var(--warn)" }}>
                  删除后无法找回。技能库里的当前版本不受影响。
                </span>
              </div>
            </div>
            <div className="modal-foot">
              <button onClick={() => setConfirmDelete(null)}>取消</button>
              <button
                className="primary"
                style={{
                  background: "var(--danger)",
                  borderColor: "var(--danger)",
                }}
                onClick={() => remove(confirmDelete)}
              >
                确认删除
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
