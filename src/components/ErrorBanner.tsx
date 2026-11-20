import { ACTION_LABEL, parseError, SkillErrorPayload } from "../api";

interface Props {
  /** 后端返回的原始错误（可能是结构化 JSON，也可能是纯文本） */
  error: unknown;
  /** 点动作按钮时触发；不给就不显示按钮 */
  onAction?: (action: string) => void;
}

/**
 * 统一的错误展示：中文标题 + 可折叠的技术详情 + 动作按钮
 *
 * 目标是让不懂技术的同事看懂「出了什么事、该怎么办」，
 * 而不是面对 `os error 10061`。
 */
export default function ErrorBanner({ error, onAction }: Props) {
  const e: SkillErrorPayload = parseError(error);
  const label = e.action ? ACTION_LABEL[e.action] : undefined;

  return (
    <div className="banner error">
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ fontWeight: 500 }}>{e.message}</span>
        {label && onAction && (
          <button
            className="ghost sm"
            style={{ marginLeft: "auto" }}
            onClick={() => onAction(e.action!)}
          >
            {label}
          </button>
        )}
      </div>
      {e.detail && e.detail !== e.message && (
        <details style={{ marginTop: 4 }}>
          <summary style={{ cursor: "pointer" }} className="page-sub">
            技术详情
          </summary>
          <div className="mono break page-sub">{e.detail}</div>
        </details>
      )}
    </div>
  );
}
