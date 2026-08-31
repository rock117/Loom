# Loom architecture

This document captures the target architecture for Loom: learn from [Zed](https://github.com/zed-industries/zed)’s **ideas and structure**, implement our own code under **MIT**, ship **Windows first**, and keep clean abstractions for **macOS / Linux** later.

It is a design guide, not a license to copy Zed source. Do not paste Zed crates or GPL-covered files into this tree.

For **why we chose X over Y**, see [DECISIONS.md](./DECISIONS.md).  
For **non-obvious GPUI/platform pitfalls** (including UI-thread freeze / deadlock checklist), see [HARD_PROBLEMS.md](./HARD_PROBLEMS.md).  
For **three-column context panel** (Files/SFTP + Info), see [CONTEXT_PANEL.md](./CONTEXT_PANEL.md).  
For **SFTP 连接池 / 浏览与传输并行 / 资源回收**（中文规格）, see [SFTP_POOL.md](./SFTP_POOL.md).  
For **Docker 会话（exec + Files / docker cp）**（中文规格，尚未实现）, see [DOCKER_SESSION.md](./DOCKER_SESSION.md).  
For **SSH 端口转发（Local / Remote / SOCKS）**（中文规格，尚未实现；非核心增强）, see [PORT_FORWARD.md](./PORT_FORWARD.md).  
For **主题系统（可切换 Theme / CSD / 终端 palette）**（中文规格，尚未实现）, see [THEME.md](./THEME.md).  
For **壳内 `loom …` 元命令 / 批处理 / 指令组合**（中文需求草案，尚未实现）, see [LOOM_CLI.md](./LOOM_CLI.md).  
For **插件系统（扩展点 / Lua 沙箱 / 隔离与诊断）**（中文规格，尚未实现）, see [PLUGINS.md](./PLUGINS.md).  
For **SSH/PTY 断线检测与手动重连**（已实现，中文说明）, see [SESSION_RECONNECT.md](./SESSION_RECONNECT.md).  
For **Local shell 代理（env 注入 / 系统代理侦测）**, see [LOCAL_PROXY.md](./LOCAL_PROXY.md).  
For **Session / Profile / Group IA（根级 Profile、嵌套 Group、临时 Tab）**, see [SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md).  
For **Local shell 选型 / pwsh 慢 / cmd 快 / 启动性能**, see [LOCAL_SHELL.md](./LOCAL_SHELL.md).  
For **low-priority icebox features** (do not build unless explicitly ordered), see [BACKLOG.md](./BACKLOG.md).

## Goals

| Goal | Meaning |
|------|---------|
| UI quality | Visual rhythm close to Zed (spacing, contrast, typography, restrained chrome) |
| Terminal feel | Snappy input/output like Zed’s terminal (GPU text, not a soft CPU bitmap) |
| Product shape | Postman-style client: left **profiles/groups**, center **multi-tab shells**, optional right **context panel** |
| Commercial-friendly | Keep **MIT**; study Zed, do not vendor its GPL code |
| Portability | Windows implementation first; `platform` traits ready for macOS / Linux |

Non-goals for this phase: becoming a mini-IDE, embedding Zed’s `project_panel` / `workspace` stack, switching UI frameworks again.

## What to learn from Zed (mapping)

| Zed concept | Loom counterpart |
|-------------|------------------|
| `terminal` — PTY + VT grid | `src/terminal/` — `alacritty_terminal` + `portable-pty` |
| `terminal_view` — GPUI element paints cells | `TerminalView` + `TerminalElement` via GPUI `text_system` |
| `workspace` + panels | Lightweight `WorkspaceView`: sidebar + tabs + terminal + context panel |
| `ui` + `theme` | Own `theme` tokens + small widgets (list row, button) |
| Platform crates (`gpui_windows`, …) | `src/platform/` — traits + `windows` impl, macOS/Linux stubs |

Key terminal idea (same class as Zed, not the same files):

```
PTY bytes → VT / grid (alacritty_terminal)
         → TerminalElement paint (GPUI shape_line / glyphs)
         → focus + keystrokes → PTY write
```

Answer terminal queries (e.g. CSI `6 n` / `Event::PtyWrite`) or shells such as PowerShell will stall without a prompt.

## Module layout (target)

```
src/
  main.rs
  app.rs                      # App lifecycle, global keybindings, quit
  platform/
    mod.rs                    # Traits: paths, default shell, font families, …
    windows.rs                # Current focus
    macos.rs                  # Stub / cfg
    linux.rs                  # Stub / cfg
  model/                      # workspace.json, profiles, settings (keep)
  terminal/
    mod.rs
    session.rs                # Spawn, resize, read/write, teardown
    grid.rs                   # Thin wrap over alacritty Term
    input.rs                  # Keystroke → bytes (Zed-like try_keystroke idea)
  ui/
    theme.rs                  # Zed-inspired palette, type scale, spacing
    workspace_view.rs
    sidebar.rs                # Profiles/groups (not a file tree)
    tab_bar.rs
    tab_manager.rs
    terminal_view.rs          # Entity, focus, lifecycle
    terminal_element.rs       # Paint grid with GPUI text system
    settings.rs
    widgets.rs
  session/                    # May merge into terminal/ over time
  shared/                     # Actions, paths helpers
```

Prefer flat `foo.rs` modules (no unnecessary `mod.rs` nests) where it matches existing Loom/GPUI skills; `platform/` and `terminal/` directories are fine when they grow.

## Platform abstraction (minimum)

Implement fully on Windows; other OSes compile with stubs or minimal defaults.

| API | Windows | Later |
|-----|---------|--------|
| `config_dir()` | `%APPDATA%/Loom` | `~/Library/Application Support/Loom`, `~/.config/loom` |
| `default_shell()` | pwsh → powershell → cmd | `$SHELL`, `/bin/zsh`, etc. |
| `monospace_font_family()` | Cascadia Mono / Consolas | Menlo / DejaVu Sans Mono |
| PTY | `portable-pty` (already cross-platform) | same crate |

Keep OS-specific `#cfg` inside `platform/*`. UI and terminal grid code should not sprinkle Win32 calls.

## UI / product rules

- **Left nav** = connection profiles and groups (Loom’s product). Do not adopt Zed’s disk worktree panel as the primary nav.
- Optionally later: a separate lightweight folder browser; not required for v1 polish.
- Visual system: dark layered surfaces, soft selection fills, hairline dividers, consistent type scale — inspired by Zed screenshots/behavior, expressed as our tokens in `theme.rs`.

## Terminal rendering rules

- Prefer **GPUI text system** painting of the cell grid (Zed-style path).
- Avoid long-term reliance on CPU bitmap → `Image` stretch (blurry on HiDPI).
- In-house terminal (`terminal/gpui_emu`) paints via GPUI `text_system`; always handle `Event::PtyWrite`.
- Multi-tab: each tab owns a session; closing a tab must not block the UI thread (careful kill / drop order for ConPTY on Windows).
- App shortcuts (`Ctrl+T` / `Ctrl+W` / `Ctrl+Q`, …) must not steal shell chords such as `Ctrl+C`.

## Implementation phases

### Phase 1 — Stabilize on current GPUI shell

- Fix focus, typing, tab close, window quit, shortcuts.
- Keep Postman layout and `model` persistence.
- Read Zed `terminal_view` design notes for input/focus ideas only.

### Phase 2 — Theme pass

- Introduce explicit theme tokens aligned with Zed-like density/contrast.
- Restyle sidebar, tabs, empty states, settings chrome.

### Phase 3 — In-house terminal pipeline

- [x] `terminal/gpui_emu` (alacritty grid + GPUI paint) with `PtyWrite` writeback.
- [x] Tabs wired to in-house `TerminalView`; `gpui-terminal` dependency removed.
- [x] Ordered reply writebacks: `PtyWrite`, `ColorRequest`, `TextAreaSizeRequest`, clipboard load/store; Ctrl+V / Shift+Insert paste.
- Polish: scrollback, selection, IME.

### Phase 4 — Windows polish

- Resize, scrollback feel, IME/paste paths, reconnect, status line.
- ConPTY teardown and process exit hygiene.

### Phase 5 — macOS / Linux

- Fill `platform/macos.rs` and `platform/linux.rs`.
- Same UI/terminal code paths; fix font and shell defaults only.

## Success criteria (Windows)

- New shell shows a real prompt and correct cwd; typing works after open/click.
- Terminal glyphs look sharp at common DPI scales (100% / 125% / 150%).
- Heavy output (e.g. large `ls` / `Get-ChildItem`) stays usable without multi-second UI freezes.
- Multiple tabs open/close reliably; closing the window exits the process cleanly.
- `platform` stubs exist so non-Windows cfg does not require deleting the abstraction.

## Relationship to the current tree

Keep:

- `model/` persistence (`workspace.json`, `ui_state.json`, `settings.json`)
- Postman-style IA (groups, profiles, multi-tab)
- crates.io **GPUI** (unless a future decision explicitly vendors Zed’s GPUI)

Evolve:

- `ui/terminal_pane.rs` + in-house `terminal/gpui_emu` (further `TerminalElement` polish)
- `shared/theme.rs` → richer tokens
- Add `platform/` and first-class `terminal/` modules

## License

Loom remains **MIT**. Contributors must not copy GPL-covered Zed (or other) source into this repository. Architecture and UX inspiration are welcome; implementations must be original.

## References (read-only)

Local Zed checkout (example): `C:\rock\coding\code\opensource\rust\zed`

Useful areas to **read for ideas** (do not copy):

- `crates/terminal_view/README.md` — input paths, builder/subscribe pattern
- `crates/terminal_view/src/terminal_element.rs` — grid paint approach
- `crates/terminal/` — PTY / event-loop boundaries
- Zed UI theme usage — visual density and contrast (observe behavior, reimplement tokens)

---

When implementing, prefer small vertical slices (theme → one working terminal tab → multi-tab) over a big-bang rewrite.

## Implementation progress

Started on the GPUI tree (post Slint rollback):

- [x] `platform/` — Windows + macOS/Linux stubs (`config_dir`, `default_shell`, `monospace_font_family`)
- [x] Theme tokens expanded in `shared/theme.rs` (Zed-inspired, original values)
- [x] PTY spawn keeps child killer; default cwd; background teardown on tab close / `TabManager` drop
- [x] App quit: `Ctrl+Q`, `on_window_closed` → `cx.quit()` (Ctrl+C left for the shell)
- [x] Root `WorkspaceView` no longer `track_focus` (avoids stealing terminal focus)
- [x] In-house `terminal/gpui_emu` (adapted MIT gpui-terminal + `PtyWrite` fix); drop crates.io `gpui-terminal`
- [x] Theme spacing/radius tokens applied to sidebar, tab bar, terminal status chrome
- [x] Sidebar restyle (Zed project-panel density: compact tree, ghost actions, context menu)
- [x] Terminal reply events + paste (`ColorRequest`, `TextAreaSizeRequest`, clipboard, Ctrl+V)
- [x] Mouse selection + auto-copy on release; Ctrl+C copies when selected; right-click paste
- [x] Scrollback wheel + overlay scrollbar; Ctrl+F find over scrollback/grid
- [x] Optional line-number gutter (Settings toggle; 1 = oldest scrollback line)
- [x] Zed-style pane splits (binary tree, sash resize, Ctrl+W closes focused pane)
- [x] Context panel (right column): SFTP Files browser + Info — see [CONTEXT_PANEL.md](./CONTEXT_PANEL.md)
- [x] SSH/PTY disconnect → Failed + status-bar Reconnect (manual) — see [SESSION_RECONNECT.md](./SESSION_RECONNECT.md)
- [x] Local shell proxy (Off/Auto/Manual env inject) — see [LOCAL_PROXY.md](./LOCAL_PROXY.md)
- [x] Session/Profile/Group IA (root profiles, nested groups, ephemeral tabs) — see [SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md)
- [ ] macOS / Linux runtime validation
