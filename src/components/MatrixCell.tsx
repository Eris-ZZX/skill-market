import { MatrixCellState } from "../api";

/** 工具的首字母占位。真图标是 #40 的范围，这里只保证列头能认。 */
const TOOL_GLYPH: Record<string, string> = {
  cursor: "C",
  claude: "CC",
  codex: "CX",
  agents: "AG",
  qwenwork: "Q",
  workbuddy: "W",
};

export function toolGlyph(key: string): string {
  return TOOL_GLYPH[key] ?? key.slice(0, 2).toUpperCase();
}

interface CellProps {
  cell: MatrixCellState;
  /** 用于 tooltip 与无障碍文案 */
  skillName: string;
  toolLabel: string;
  /** external 身份的技能不能从本工具卸载 */
  external: boolean;
  onClick: () => void;
}

/**
 * 一个格子：装 / 未装 / 上次失败
 *
 * 注意这是「装/卸」语义，不是 cc-switch 的「启用/停用」——
 * 停用需要 state.json 记录状态，不在本票范围。
 */
export function MatrixCell({
  cell,
  skillName,
  toolLabel,
  external,
  onClick,
}: CellProps) {
  const state = cell.failed ? "fail" : cell.installed ? "on" : "off";
  const hint = cell.failed
    ? "上次操作失败，点击重试"
    : cell.installed
      ? external
        ? "由外部工具管理，请在原工具中操作"
        : "已安装，点击卸载"
      : "未安装，点击安装";

  return (
    <button
      type="button"
      className={`matrix-cell matrix-${state}`}
      title={`${skillName} → ${toolLabel}：${hint}`}
      aria-label={`${skillName} 在 ${toolLabel}：${hint}`}
      aria-pressed={cell.installed}
      onClick={onClick}
    >
      <span className="matrix-box">
        {state === "fail" ? "!" : state === "on" ? "✓" : ""}
      </span>
    </button>
  );
}

interface HeaderProps {
  toolKey: string;
  label: string;
  /** 当前筛选范围内该工具下已装的技能数 */
  count: number;
  /** 当前筛选范围内的技能总数 */
  total: number;
  /** 展开菜单 */
  onClick: () => void;
  active: boolean;
}

export function MatrixHeader({
  toolKey,
  label,
  count,
  total,
  onClick,
  active,
}: HeaderProps) {
  return (
    <th className="matrix-col">
      <button
        type="button"
        className={`matrix-header-btn ${active ? "active" : ""}`}
        title={`${label}：${count} / ${total} 个技能已装，点击查看批量操作`}
        aria-label={`${label} 批量操作`}
        onClick={onClick}
      >
        <span className="matrix-glyph">{toolGlyph(toolKey)}</span>
        <span className="matrix-count">{count}</span>
      </button>
    </th>
  );
}
