import type { ReactNode } from "react";
import { MatrixCellState } from "../api";
// 仅本地静态资源，见下方 dangerouslySetInnerHTML 处的说明
import claudeSvg from "../assets/icons/claude.svg?raw";
import openaiSvg from "../assets/icons/openai.svg?raw";
import qwenSvg from "../assets/icons/qwen.svg?raw";

/** 工具的首字母占位。有真图标的工具仍保留字母，供 tooltip 与回退使用。 */
const TOOL_GLYPH: Record<string, string> = {
  cursor: "C",
  claude: "CC",
  codex: "CX",
  agents: "AG",
  qwenwork: "Q",
  workbuddy: "W",
};

/**
 * 有开源单色图形的工具（来自 cc-switch，MIT）。
 * 其余工具没有可用的开源图标，用品牌色 + 字母占位。
 */
const TOOL_ICON: Record<string, string> = {
  claude: claudeSvg,
  codex: openaiSvg,
  qwenwork: qwenSvg,
};

/** 无图标工具的品牌色。取官方公开品牌色，白字保证对比度。 */
const TOOL_BRAND: Record<string, string> = {
  cursor: "#26251E",
  workbuddy: "#0EC8A9",
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
  onClick: (e: React.MouseEvent<HTMLButtonElement>) => void;
  active: boolean;
  /**
   * 展开时的浮层内容。
   *
   * 由调用方（InstalledPage）构造——那里才有 filtered / runBulk 等状态，
   * 这里只负责挂到列头单元格内部。绝对定位相对 `th` 计算，所以不会
   * 占据表格流内空间，展开时下面的行不会跳。
   */
  popover?: ReactNode;
}

export function MatrixHeader({
  toolKey,
  label,
  count,
  total,
  onClick,
  active,
  popover,
}: HeaderProps) {
  return (
    <th className="matrix-col">
      {popover}
      <button
        type="button"
        className={`matrix-header-btn ${active ? "active" : ""}`}
        data-floating-trigger=""
        title={`${label}：${count} / ${total} 个技能已装，点击查看批量操作`}
        aria-label={`${label} 批量操作`}
        onClick={onClick}
      >
        {TOOL_ICON[toolKey] ? (
          // dangerouslySetInnerHTML 只用于打包进 bundle 的本地静态 SVG
          // （src/assets/icons/*.svg，经 Vite ?raw 内联为字符串）。
          // 绝不用于任何远端/用户内容——那会变成 XSS 入口。
          <span
            className="matrix-glyph has-icon"
            dangerouslySetInnerHTML={{ __html: TOOL_ICON[toolKey] }}
          />
        ) : (
          <span
            className={`matrix-glyph ${TOOL_BRAND[toolKey] ? "brand" : ""}`}
            style={TOOL_BRAND[toolKey] ? { background: TOOL_BRAND[toolKey] } : undefined}
          >
            {toolGlyph(toolKey)}
          </span>
        )}
        <span className="matrix-count">{count}</span>
      </button>
    </th>
  );
}
