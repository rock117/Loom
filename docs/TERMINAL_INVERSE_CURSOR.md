# 终端反色与光标可见性（Agent CLI 等）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)、[TERMINAL_ANSI_PALETTE.md](./TERMINAL_ANSI_PALETTE.md)。

> **文档约定**：中文。  
> **状态**：已修复（`src/terminal/gpui_emu/render.rs`）。

---

## 现象

在 Loom 内运行 Cursor Agent CLI（或其它全屏 TUI）时：

- Ask / 输入区可以打字，但**看不见输入光标（caret）**；
- 同一命令在 Windows Terminal / 系统 cmd 里光标正常；
- 终端内容区底部的 `路径 · 分支` 一类文字多半是 **Agent 自己画在 PTY 里的 UI**，不是 Loom 应用状态栏（状态栏是更底下一行的 `Connected · …`）。

易被误读成「Agent 没接管成功 / 还停在 shell prompt」。实际上 Agent UI 往往已起来，缺的是 **自绘 caret 的渲染**。

---

## 原因

许多 TUI（含 Agent CLI）约定：

1. 发送 `\e[?25l`（DECTCEM）**隐藏宿主终端的原生光标**；
2. 在当前输入格上用 ANSI **反色（SGR 7 / `Flags::INVERSE`）** 自绘 caret（常落在空格格上，只靠背景色差可见）。

修复前 Loom 渲染有两处缺口：

| 缺口 | 后果 |
| --- | --- |
| 不处理 `Flags::INVERSE`（不对调 fg/bg） | 自绘 caret 几乎不可见 |
| 忽略 `SHOW_CURSOR` / `CursorShape::Hidden`，始终画方块光标 | 与 TUI 约定不符；输入区仍看不到「真正的」反色光标 |

社区对 Agent 在浅色主题下「光标看不见」亦有同类讨论（主题探测 / OSC 11 等）；本仓库本次修复针对的是 **宿主未实现反色 + 光标门控**，与 OSC 11 主题探测是不同层问题。

---

## 解决方案

代码：`src/terminal/gpui_emu/render.rs`。

1. **`resolve_cell_colors`**：若 cell 带 `Flags::INVERSE`，交换 fg/bg；`layout_row`（背景矩形）与字符绘制共用，空格 caret 靠背景色差显示。
2. **原生光标门控**：仅在 `TermMode::SHOW_CURSOR` 且形状不是 `Hidden` 时绘制；按 Block / Beam / Underline 区分样式（HollowBlock 暂按实心块）。
3. **顺带**：`Flags::HIDDEN`（SGR 8）的字符不再绘制，与常见终端一致。

单元测试：`inverse_swaps_fg_bg`、`layout_row_inverse_space_gets_swapped_background`。

---

## 影响面

| 场景 | 影响 |
| --- | --- |
| 普通 shell（未反色、未藏光标） | 与修复前基本一致 |
| vim / less / htop 等反色高亮 | 显示更正确（补齐本应有的 VT 行为） |
| 发 `?25l` 且自绘 UI 的 TUI | 宿主不再误画白块；自绘 caret 可见 |
| 选区 / Find / 链接 / IME / cwd | 不涉及（仅 paint） |

**边界：** 若程序藏了宿主光标却又不自绘 caret，现在会完全没有光标——符合 VT 约定；修复前是错误地帮它露白块。

仍未做（非本次范围）：DIM / 闪烁 / 删除线；Block 光标下反转格内字形；OSC 11 背景色查询。

---

## 验证建议

1. 普通 pwsh：光标、选区复制正常。
2. Agent Ask：输入区可见 caret。
3. 可选：`printf '\e[?25l'` 光标应消失；`printf '\e[?25h'` 应恢复。
4. 可选：vim 可视选区等高亮比修复前更正常。

若 Agent 主题整体偏暗/偏淡，可另试壳内 `$env:TERM_THEME='light'`（Agent 侧主题绕过，与 Loom 反色修复独立）。

---

## 规则（给后续改渲染的人）

- 画 cell 颜色时必须处理 **`Flags::INVERSE`**（背景与文字一致）。
- 画原生光标前必须看 **`SHOW_CURSOR` / `CursorShape::Hidden`**，不要无条件画方块。
- TUI 底部 cwd/分支条 ≠ Loom 状态栏；排障时先分清 PTY 内容与应用 chrome。
