import { PendingRow, ToolDetection, toolLabel } from "../api";

interface Props {
  rows: PendingRow[];
  tools: ToolDetection[];
  /** 正在更新中的 `${name}-${tool}`，用于禁用按钮与显示 spinner */
  updating: string | null;
  onUpdate: (name: string, tool: string) => void;
  onResolve: (name: string, tool: string, status: string) => void;
  onUpdateAll: () => void;
  onClose: () => void;
}

const STATUS_TEXT: Record<PendingRow["status"], string> = {
  updateavailable: "可更新",
  conflict: "需处理",
  locallymodified: "已修改",
};

/**
 * 「检查更新」结果弹窗
 *
 * 矩阵行里也有「更新」按钮——那是就地操作；这个弹窗解决的是「一眼看全部」。
 * 两条路径都调同一个 update()，不是两套逻辑。
 */
export default function UpdateDialog({
  rows,
  tools,
  updating,
  onUpdate,
  onResolve,
  onUpdateAll,
  onClose,
}: Props) {
  const updatable = rows.filter((r) => r.status === "updateavailable");

  return (
    <div className="overlay" onClick={onClose}>
      <div
        className="modal"
        style={{ width: 640 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <div>
            <h2 className="modal-title">检查更新</h2>
            <div className="page-sub">
              {rows.length === 0
                ? "所有技能都是最新"
                : `${rows.length} 项需要关注（${updatable.length} 项可直接更新）`}
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭">
            ✕
          </button>
        </div>

        <div className="modal-body">
          {rows.length === 0 ? (
            <div className="empty">
              <div className="empty-title">所有技能都是最新</div>
              <div className="empty-hint">
                没有可更新、需处理或本地已修改的技能。
              </div>
            </div>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>技能</th>
                  <th>工具</th>
                  <th>版本</th>
                  <th>状态</th>
                  <th style={{ width: 90 }}></th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => {
                  const busy = updating === `${r.name}-${r.tool}`;
                  return (
                    <tr key={`${r.name}-${r.tool}`}>
                      <td style={{ fontWeight: 500 }}>{r.name}</td>
                      <td className="page-sub">{toolLabel(r.tool, tools)}</td>
                      <td className="page-sub" style={{ whiteSpace: "nowrap" }}>
                        {r.installedVersion ?? "—"}
                        {r.remoteVersion && r.remoteVersion !== r.installedVersion
                          ? ` → ${r.remoteVersion}`
                          : ""}
                      </td>
                      <td>
                        <span
                          className={
                            r.status === "updateavailable"
                              ? "badge accent"
                              : "badge warn"
                          }
                        >
                          {STATUS_TEXT[r.status]}
                        </span>
                      </td>
                      <td>
                        {r.status === "updateavailable" ? (
                          <button
                            className="primary sm"
                            disabled={busy}
                            onClick={() => onUpdate(r.name, r.tool)}
                          >
                            {busy && <span className="spinner" />}
                            更新
                          </button>
                        ) : r.status === "conflict" ? (
                          <button
                            className="primary sm"
                            onClick={() =>
                              onResolve(r.name, r.tool, r.status)
                            }
                          >
                            处理
                          </button>
                        ) : (
                          // 已修改没有可自动执行的动作用——重置会丢改动，
                          // 得让用户在冲突弹窗里自己选
                          <button
                            className="ghost sm"
                            onClick={() =>
                              onResolve(r.name, r.tool, r.status)
                            }
                          >
                            查看
                          </button>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>

        <div className="modal-foot">
          <button onClick={onClose}>关闭</button>
          {updatable.length > 0 && (
            <button className="primary" onClick={onUpdateAll}>
              全部更新（{updatable.length}）
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
