# Loom backlog (icebox)

**Priority: lowest.** These are ideas only — **do not implement** unless the user **explicitly** asks for a named item (e.g. “做 SSH” / “实现主题切换”).  
Passing mentions, “继续 roadmap”、日常 bugfix、或 Agent 自作主张 **都不算** 授权。

Not a decision log. Technical choices go in [DECISIONS.md](./DECISIONS.md). Target architecture goes in [ARCHITECTURE.md](./ARCHITECTURE.md).

Active / near-term work stays in `ARCHITECTURE.md` → Implementation progress. This file is the long-horizon icebox.

---

## Product (connection client)

| ID | Idea | Notes |
|----|------|--------|
| P1 | **SSH / bastion** | **MVP shipped** (russh + keyring passwords + TOFU known_hosts). Still open: jump hosts, agent, key passphrase UI, nicer host-key change flow. |
| P2 | **Session templates** | One action opens a defined set of tabs (e.g. API + DB + logs). |
| P3 | **Profile env / startup** | Per-profile env vars, cwd, init command / script. |
| P4 | **Tab layouts / splits** | **MVP shipped** — tab-bar columns popover (Split Right/Left/Up/Down), Zed-style binary tree, sash resize, Ctrl+W closes focused pane; chords kept as secondary. Still open: drag-rearrange panes, persist layouts. Popover paint/anchor lesson: `HARD_PROBLEMS.md`. |
| P5 | **Command bookmarks / snippets** | Saved commands; insert into the active shell. |

## Terminal UX

| ID | Idea | Notes |
|----|------|--------|
| T1 | **Output search** | **Shipped** — Ctrl+F find bar over scrollback/grid (literal match, Enter/F3 next, Shift+Enter/Shift+F3 prev, Esc closes). |
| T2 | **Selection polish** | Double-click word, triple-click line, optional block select. |
| T2b | **Terminal context menu** | **Shipped** — right-click: Copy, Paste, Copy Path, Reveal in File Explorer, Find…, Select All, Close. Cwd tracks `cd` via OSC 7 / OSC 9;9 (local pwsh/cmd/bash inject OSC). |
| T3 | **Link detection** | Clickable paths / URLs in output. |
| T4 | **Notifications** | Long-running command done, bell, disconnect toasts. |
| T5 | **Output syntax color via external tools** | Prefer `bat` / highlighters that emit ANSI; do **not** build in-app language highlighters for PTY output. Optional: docs or settings hints. Low priority. |
| T6 | **Directory listing icons via external tools** | Prefer `eza` / `lsd` (+ Nerd Font); Loom only needs solid Unicode/Nerd Font rendering. Do **not** intercept `ls`. Low priority. |
| T7 | **Line number gutter** | **Shipped** — absolute scrollback line numbers (1 = oldest in buffer); Settings → Line numbers On/Off (default On). |

## Appearance

| ID | Idea | Notes |
|----|------|--------|
| A1 | **Theme system** | Built-in packs + user theme (e.g. JSON); wire UI + terminal palette. Prefer before plugins. |
| A2 | **Accessibility themes** | High-contrast / larger UI variants (can ship with A1). |

## Platform & shell

| ID | Idea | Notes |
|----|------|--------|
| S1 | **System tray / launch at login** | Background-friendly desktop behavior. |
| S2 | **Multi-window / multi-workspace** | Separate windows or workspace switcher. |
| S3 | **macOS / Linux polish** | Beyond stubs: fonts, default shells, packaging. |

## Extensibility

| ID | Idea | Notes |
|----|------|--------|
| E1 | **Plugin mechanism** | Do **after** core (SSH, scrollback search, themes) is stable. |

### Plugin scope (when E1 is ordered)

**Good candidates:** themes, status-bar fragments, snippets, custom profile kinds.  
**Avoid initially:** arbitrary PTY byte-stream hooks (security and stability risk).

---

## Explicitly out of this icebox (for now)

Workflow / collaboration ideas (shared history DB, session “recording for notes”, social share packs, etc.) are **not** tracked here unless product direction changes.

---

## How to promote an item

1. User names the ID or feature clearly.  
2. Optionally add a short section to `DECISIONS.md` if the approach has real trade-offs.  
3. Move or check off work under `ARCHITECTURE.md` progress / phases — not by silently starting icebox work.
