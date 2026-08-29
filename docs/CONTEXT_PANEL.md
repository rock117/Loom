# Context panel (three-column layout)

WindTerm-style **third column** on the right: a **session context panel** for the focused pane.

Related: [ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、**[SFTP_POOL.md](./SFTP_POOL.md)**（SFTP 连接池 / 浏览与传输并行 / 资源回收，中文规格）。

## Layout

```
┌────────────┬──────────────────────────┬─────────────────┐
│  Profiles  │  Tabs + terminal panes   │  Context panel  │
│  Ctrl+B    │  (center — primary)      │  Ctrl+Shift+B   │
└────────────┴──────────────────────────┴─────────────────┘
```

Right panel sections:

| Tab | Role |
|-----|------|
| **Files** | Remote SFTP browser (SSH only): navigate like Explorer, upload/download |
| **Info** | Light session summary (profile, target, cwd, size) |

Transfer progress lives in a **footer under Files** (not a separate tab).

Both side panels hide completely when toggled off; width/visibility persist in `ui_state.json`. Default context panel: **off**.

## Files browser (primary)

Windows-style navigation:

| Action | Behavior |
|--------|----------|
| Double-click folder | Enter directory |
| Double-click file | Download (save dialog) |
| ← / Up | Parent directory |
| ⌂ Home | Session home (`canonicalize(".")`) |
| Toolbar Upload | Pick local **file** → upload into **current** remote dir |
| ↓ button | Download selected (file or recursive folder) |

Path bar shows current remote cwd (`/home/user/...`).

### Transfers footer

Transfers are scoped per SSH pane (switching tabs shows that session’s list only).  
Phases: **Queued** (waiting on the transfer lane) → **Scanning · N** → `m/n · size · rate · time` → **Done**.  
**×** / **Clear** cancel the SFTP job (not UI-only) so the transfer lane is freed for the next task.  
Right-click → Reveal in File Explorer / Remove.  
MVP: in-memory list for the panel lifetime; no separate Transfers tab.

Drag the sash between the file list and Transfers to change their height ratio
(`context_files_list_ratio` in UI state, default ~72% list).

### Local sessions

Files tab shows an empty state: SFTP is SSH-only. Use terminal context menu for local Reveal/Copy Path.

## Info

Read-only: profile name, Local/SSH, target, connection state, working directory (when known), terminal size.

## Non-goals (for now)

- Snippets as a default tab (optional later)
- Drag-drop into Explorer
- Full remote editor
- Jump hosts / second SSH connection solely for SFTP (use same session channel)

## Implementation map

| Piece | Location |
|-------|----------|
| Doc | `docs/CONTEXT_PANEL.md`、`docs/SFTP_POOL.md` |
| SFTP bridge | `src/session/sftp.rs` + `ssh.rs`（同 SSH；浏览/传输分车道池） |
| UI | `src/ui/context_panel.rs` |
| Pane handle | `PaneSession.ssh_sftp` |

## Stack

- `russh` session already used for the shell
- Extra channel + `request_subsystem("sftp")` + `russh-sftp::SftpSession`
