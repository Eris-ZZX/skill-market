# 图标来源与许可

## 单色图标（跟随主题色）

`claude.svg`、`openai.svg`、`qwen.svg` 取自 [cc-switch](https://github.com/farion1231/cc-switch)
的 `src/icons/extracted/`，MIT 许可。原文件带品牌色填充（Claude 橙色、Qwen 紫色渐变），
为了跟随界面文字色、适配深色主题，本项目把填充统一改为 `currentColor`，
Qwen 的渐变 `defs` 一并删除。图形路径未做任何改动。

`cursor.svg` 取自 [simple-icons](https://github.com/simple-icons/simple-icons)
（CC0 1.0），454 字节，单个 path，`viewBox="0 0 24 24"`，填充改为 `currentColor`。

这四个都渲染成 14×14，居中在 20×20 的圆形底色里。

## 彩色图标（保留原色）

`workbuddy.svg` 是 WorkBuddy 官方 logo（`https://www.workbuddy.cn/` 页面引用的
`assets/logo.svg`），4142 字节，`viewBox="0 0 40 40"`。

它自带圆角矩形底、绿色渐变与黄色光晕，是完整品牌形象。**不做单色化** ——
实测单色化后在 14px 下会变成一个认不出的黑块，失去可辨识度。因此单独一档，
渲染成 20×20 占满格子，不叠 `.matrix-glyph` 的圆形底色与边框。

> 注意：该 svg 含 `feGaussianBlur` 光晕，会绘制到 viewBox 之外。
> `.matrix-glyph-color` 用 `overflow: hidden` + 父级 `border-radius: 50%`
> 把光晕裁在 20×20 圆内，实测边界外 2px 处无外溢。

## 导入方式

用 Vite 的 `?raw` 导入为字符串，由 `MatrixCell.tsx` 注入 `dangerouslySetInnerHTML`。
仅限本目录下的打包期静态资源，不涉及任何远端或用户内容。
