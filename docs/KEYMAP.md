# Keymap（快捷键）

> **状态**：P0 — Action 稳定 + 默认绑定可发现；用户自定义覆盖尚未做。  
> 实现：`src/shared/actions.rs`（Action）+ `src/app.rs`（`cx.bind_keys`）+ `WorkspaceView` `key_context("Loom")`。

## 模型

| 层 | 职责 |
|---|---|
| **Action** | 稳定 ID（`NewLocalTab`、`ToggleSidebar`…）；菜单 / 快捷键 / 日后命令面板只认这个 |
| **KeyBinding** | Action → 按键；可一对多（同一 Action 多个键） |
| **Context** | 何时生效：`Loom`（主窗口）、`Renamable`（侧栏可改名）等 |

新增快捷键 = 新增或复用 Action + 在 `app.rs` 加默认绑定 + 更新本表。  
**禁止**用裸 `on_key_down` 散落业务快捷键。

## 硬规则

1. **终端优先**：不抢 shell 常用控制键（`Ctrl+C` / `Ctrl+D` / `Ctrl+Z` / `Ctrl+L` 等）。
2. App 快捷键放在 `Loom` context；表单 / Find / modal 用更窄 context 或本地处理。
3. 日后用户覆盖只存与默认的 **diff**（P1），不要整表快照锁死升级。

## 默认绑定（`Loom`）

| 按键 | Action | 行为 |
|------|--------|------|
| **Ctrl+N** | `NewLocalTab` | 新建临时 Local shell Tab（默认 shell） |
| Ctrl+T | `NewLocalTab` | 同上（兼容） |
| Ctrl+W | `CloseTab` | 关 focused pane；最后一格关 Tab |
| Ctrl+Tab / Ctrl+Shift+Tab | `NextTab` / `PrevTab` | 切 Tab |
| Ctrl+Shift+D | `DuplicateTab` | Duplicate 当前 Tab（Ephemeral） |
| Ctrl+S | `SaveWorkspace` | 落盘 |
| Ctrl+, | `ToggleSettings` | 设置 |
| Ctrl+B | `ToggleSidebar` | 左侧 Profiles |
| Ctrl+Shift+B | `ToggleContextPanel` | 右侧 Context / Files |
| Ctrl+E / Ctrl+Shift+I | `ExportWorkspace` / `ImportWorkspace` | 导入导出 |
| Ctrl+= / Ctrl+- / Ctrl+0 | `ZoomIn` / `ZoomOut` / `ZoomReset` | 字体 |
| Ctrl+Q | `QuitApp` | 退出 |
| Ctrl+K ←↑↓→ | `Split*` | 分屏 |
| Ctrl+\ | `SplitRight` | 向右分屏 |
| Ctrl+K Ctrl+←↑↓→ | `ActivatePane*` | 焦点移到邻格 |

| 按键 | Context | Action |
|------|---------|--------|
| F2 | `Renamable` | `RenameFocused` | 侧栏 Profile / Group 重命名 |

终端内另有 **Ctrl+F** Find（终端视图本地，不经 `Loom` Action 表）。

## 路线

| 期 | 内容 |
|----|------|
| **P0**（当前） | Action + 默认绑定 + 本文档 |
| **P1** | Settings 改键 / 清除 / 恢复默认；`settings` 只存覆盖 |
| **P2** | 命令面板按 Action 搜索；冲突检测；平台默认差异 |

## 相关代码

| 路径 | 职责 |
|------|------|
| `src/shared/actions.rs` | Action 定义 |
| `src/app.rs` | `bind_keys` 默认表 |
| `src/ui/workspace_view.rs` | `key_context("Loom")` + `on_action` |
| `src/ui/sidebar.rs` | `Renamable` + F2 |
