import type { ReactNode } from "react";
import { MatrixCellState } from "../api";
// 仅本地静态资源，见下方 dangerouslySetInnerHTML 处的说明
import claudeSvg from "../assets/icons/claude.svg?raw";
import openaiSvg from "../assets/icons/openai.svg?raw";
import qwenSvg from "../assets/icons/qwen.svg?raw";
import cursorSvg from "../assets/icons/cursor.svg?raw";
import workbuddySvg from "../assets/icons/workbuddy.svg?raw";

/** 工具的首字母占位：映射里没有的键兜底，也用于 tooltip 之外的回退 */
const TOOL_GLYPH: Record<string, string> = {
  cursor: "C",
  claude: "CC",
  codex: "CX",
  agents: "AG",
  qwenwork: "Q",
  workbuddy: "W",
};

/**
 * 单色图标：填充为 currentColor，跟随界面文字色，浅色/深色主题都可见。
 *
 * 来源：claude / openai / qwen 取自 cc-switch（MIT），cursor 取自
 * simple-icons（CC0）。都是单个 path，渲染 14px 居中在 20px 圆形底上。
 */
const TOOL_ICON: Record<string, string> = {
  claude: claudeSvg,
  codex: openaiSvg,
  qwenwork: qwenSvg,
  cursor: cursorSvg,
};

/**
 * 彩色图标：保留自身配色，不单色化。
 *
 * workbuddy 是官方品牌形象（绿色渐变底 + 黄色光晕 + 有机形状），本身自带
 * 40×40 圆角色块。强行单色化会变成一个认不出的黑块，所以单独一档，
 * 渲染 20px 占满整个格子、不要额外底色与边框。
 */
const TOOL_ICON_COLOR: Record<string, string> = {
  workbuddy: workbuddySvg,
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
        ) : TOOL_ICON_COLOR[toolKey] ? (
          // 彩色图标自带圆角底色，占满 20px 即可，不要再套圆形底与边框
          <span
            className="matrix-glyph matrix-glyph-color"
            dangerouslySetInnerHTML={{ __html: TOOL_ICON_COLOR[toolKey] }}
          />
        ) : (
          <span className="matrix-glyph">{toolGlyph(toolKey)}</span>
        )}
        <span className="matrix-count">{count}</span>
      </button>
    </th>
  );
}
