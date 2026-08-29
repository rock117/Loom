# Context panel (three-column layout)

WindTerm-style **third column** on the right: a **session context panel** for the focused pane.

Related: [ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、**[SFTP_POOL.md](./SFTP_POOL.md)**（SFTP 连接池 / 浏览与传输并行 / 资源回收，中文规格）、**[DOCKER_SESSION.md](./DOCKER_SESSION.md)**（Docker exec + Files / `docker cp`，规格已定未实现）。

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
| **Files** | Session file browser: **SSH** → SFTP；**Local** → 本机目录；**Docker**（规划）→ 浏览 + `docker cp`（见 [DOCKER_SESSION.md](./DOCKER_SESSION.md)） |
| **Info** | Light session summary (profile, target, cwd, size) |

Transfer progress lives in a **footer under Files** (not a separate tab). SSH uploads/downloads only.

Both side panels hide completely when toggled off; width/visibility persist in `ui_state.json`. Default context panel: **off**.

## Files browser (primary)

Windows-style navigation:

| Action | Behavior |
|--------|----------|
| Double-click folder | Enter directory |
| Double-click file | **SSH:** download (save dialog). **Local:** Reveal in File Explorer |
| ← / Up | Parent directory |
| ⌂ Home | SSH: session home (`canonicalize(".")`). Local: terminal cwd (else user profile) |
| + | New folder |
| Toolbar Upload (SSH) | Pick local **file** → upload into **current** remote dir |
| Toolbar Upload folder (SSH) | Pick local **folder** → recursive upload (`upload_tree`) |
| Drag-drop (SSH) | Drop files/folders from OS onto the file list → upload into current remote dir |
| ↓ button (SSH) | Download selected (file or recursive folder) |

Path bar shows current cwd (`/home/user/...` or `C:\Users\...`).

### Explorer ops (SSH + Local)

Right-click an entry (or use prompts from the menu):

| Action | Notes |
|--------|--------|
| New folder | Name prompt |
| Rename | Name prompt |
| Permissions… | Octal mode (e.g. `755`). Local on Windows approximates via readonly bit |
| Delete… | Confirm; directories removed recursively |
| Reveal (Local) | File Explorer |
| Download (SSH file) | Same as toolbar download |

### Transfers footer

Transfers are scoped per SSH pane (switching tabs shows that session’s list only).  
Phases: **Queued** (waiting on the transfer lane) → **Scanning · N** → `m/n · size · rate · time` → **Done**.  
**×** / **Clear** cancel the SFTP job (not UI-only) so the transfer lane is freed for the next task.  
Right-click → Reveal in File Explorer / Remove.  
MVP: in-memory list for the panel lifetime; no separate Transfers tab.

Drag the sash between the file list and Transfers to change their height ratio
(`context_files_list_ratio` in UI state, default ~72% list).

### Local sessions

Files browses the local filesystem (terminal working directory as home when known).  
Transfers footer stays empty unless an SSH pane is focused. Use Reveal / Copy Path from the terminal context menu as needed.

### Docker sessions (planned)

Same Files UX as SSH; backend is container listing + `docker cp`, not SFTP. Spec: [DOCKER_SESSION.md](./DOCKER_SESSION.md).

## Info

Read-only: profile name, Local/SSH, target, connection state, working directory (when known), terminal size.

## Non-goals (for now)

- Snippets as a default tab (optional later)
- Full remote editor
- Jump hosts / second SSH connection solely for SFTP (use same session channel)
- Local multi-file copy/move as Transfers jobs

## Implementation map

| Piece | Location |
|-------|----------|
| Doc | `docs/CONTEXT_PANEL.md`、`docs/SFTP_POOL.md` |
| SFTP bridge | `src/session/sftp.rs` + `ssh.rs`（同 SSH；浏览/传输分车道池；mkdir/remove/rename/chmod） |
| Local FS | `src/session/local_fs.rs` |
| UI | `src/ui/context_panel.rs` |
| Pane handle | `PaneSession.ssh_sftp` |

## Stack

- `russh` session already used for the shell
- Extra channel + `request_subsystem("sftp")` + `russh-sftp::SftpSession`
- OS file drop → GPUI `ExternalPaths` on the Files list (SSH upload)
