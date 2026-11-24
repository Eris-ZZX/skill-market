# 图标来源与许可

## 第三方 SVG

`claude.svg`、`openai.svg`、`qwen.svg` 取自 [cc-switch](https://github.com/farion1231/cc-switch)
的 `src/icons/extracted/`，MIT 许可。原文件带品牌色填充（Claude 橙色、Qwen 紫色渐变），
为了跟随界面文字色、适配深色主题，本项目把填充统一改为 `currentColor`，
Qwen 的渐变 `defs` 一并删除。图形路径未做任何改动。

## Cursor / WorkBuddy

这两个工具没有可用的开源图标，用品牌色圆形底 + 白色首字母表示：

| 工具 | 字母 | 品牌色 |
|---|---|---|
| Cursor | C | `#26251E` |
| WorkBuddy | W | `#0EC8A9` |

## 导入方式

用 Vite 的 `?raw` 导入为字符串，由 `MatrixCell.tsx` 注入 `dangerouslySetInnerHTML`。
仅限本目录下的打包期静态资源，不涉及任何远端或用户内容。
