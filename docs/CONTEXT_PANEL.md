# Context panel (three-column layout)

WindTerm-style **third column** on the right: a **session context panel** for the focused pane.

Related: [ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、**[SFTP_POOL.md](./SFTP_POOL.md)**（SFTP 连接池 / 浏览与传输并行 / 资源回收，中文规格）、**[DOCKER_SESSION.md](./DOCKER_SESSION.md)**（Docker exec + Files / `docker cp`）。

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
| **Files** | Session file browser: **SSH** → SFTP；**Local** → 本机目录；**Docker**（本机阶段 2）→ 容器浏览 + `docker cp`（见 [DOCKER_SESSION.md](./DOCKER_SESSION.md)） |
| **Info** | Light session summary (profile, target, cwd, size) |

Transfer progress lives in a **footer under Files** (not a separate tab). SSH uploads/downloads only.

Both side panels hide completely when toggled off; width/visibility persist in `ui_state.json`. Default context panel: **off**.

## Files browser (primary)

Windows-style navigation:

| Action | Behavior |
|--------|----------|
| Double-click folder | Enter directory |
| Double-click file | **Local:** open with OS default app (non-blocking). **SSH:** confirm → download to temp → open when finished (Transfers footer + toast; UI stays responsive) |
| ← / Up | Parent directory |
| ⌂ Home | SSH: session home (`canonicalize(".")`). Local: terminal cwd (else user profile) |
| + | New folder |
| Toolbar Upload (SSH) | Pick local **file** → upload to current remote cwd (**no settings**). |
| Toolbar Upload folder (SSH) | Pick local **folder** → **settings** (remote dir / include·exclude / compress) → recursive upload |
| Drag-drop (SSH) | **Files only** → upload to cwd immediately. **Folders / mixed** → settings dialog |
| ↓ button (SSH) | **File** → download to last/Downloads folder (**no settings**). **Folder** → settings → transfer |

Settings dialog (folders / mixed only): local dest = Downloads; remote dest = current cwd; exclude presets (`node_modules`, `target`, `dist`, …) checked; include empty (= all). Paths support select / copy / paste; Tab cycles fields. **SSH double-click open** still skips settings (confirm → temp download → open).

Compress: upload zips locally then sends `.zip`; download tries remote `zip` → `tar.gz` → `tar` → `7z` → `gzip`, then saves the archive file into the chosen local folder (does not auto-extract).

File rows use **type-specific SVG icons** tinted with Seti colors (VS Code built-in Explorer palette): folders, git, Rust hexagon, Python, Markdown `M↓`, JSON braces, shell green, etc.

Path bar shows current cwd (`/home/user/...` or `C:\Users\...`).

### Explorer ops (SSH + Local)

Right-click an entry (or use prompts from the menu):

| Action | Notes |
|--------|--------|
| Open | Same as double-click (remote confirms download-then-open) |
| New folder | Name prompt |
| Rename | Name prompt |
| Permissions… | Octal mode (e.g. `755`). Local on Windows approximates via readonly bit |
| Delete… | Confirm; directories removed recursively |
| Reveal (Local) | File Explorer |
| Download (SSH file) | Straight to last/Downloads folder (no settings). Folders still open settings |

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

### Docker sessions（本机 + SSH 宿主）

入口：Docker 图标 → **本地 | SSH Profile** → 容器列表（不做侧栏 Docker 树）。本机：`docker_fs`；经 SSH：`docker_ssh`（列举 + 远端 `docker cp` 再 SFTP）。规格：[DOCKER_SESSION.md](./DOCKER_SESSION.md)。

## Info

Compact **Host** / **Container** view (no session summary). Loads on first open for the current pane; **↻** refreshes (no interval polling).

| Block | Content |
|-------|---------|
| Identity | Hostname (or container hostname) as title; OS/kernel and CPU/image as muted lines |
| Resources | Memory + up to 5 disks + up to 2 GPUs (hidden if undetected). Continuous bars; fill danger at ≥90%. **Hidden for Docker** |
| Listening | Open ports + process names (up to 24). Common ports (22/80/443/DB/dev…) ranked first. **Hidden for Docker** |
| Ports / Volumes | **Docker only:** published `host → container` ports; mounts as a compact table (Type / Mode / Source / Destination) from `docker inspect` |
| Footer | Host: `Load … · Up …`. Docker: container name / short id |

- **Local:** `sysinfo` disks/memory; GPU via `nvidia-smi` then Windows CIM / `lspci`; listening via PowerShell `Get-NetTCPConnection` (Windows) or `ss`/`netstat` (Unix) — all on a **background** thread
- **SSH:** probe + `df`; GPU via `nvidia-smi` then `lspci`; listening via remote `ss`/`netstat` (best-effort) — never blocks the UI thread
- **Docker** (local or over SSH): `docker inspect` → ports + mounts; see [DOCKER_SESSION.md](./DOCKER_SESSION.md)


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
| Docker FS | `src/session/docker_fs.rs`（本机）；`src/session/docker_ssh.rs`（经 SSH） |
| Host info | `src/session/host_info.rs`（Local sysinfo + SSH probe） |
| UI | `src/ui/context_panel.rs`、`src/ui/file_icon.rs`、`src/ui/transfer_settings.rs` |
| Transfer filter / archive | `src/session/transfer_filter.rs`、`src/session/transfer_archive.rs` |
| Open path | `src/platform.rs` (`open_path` / `open_path_detached`) |
| Pane handle | `PaneSession.ssh_sftp` |

## Stack

- `russh` session already used for the shell
- Extra channel + `request_subsystem("sftp")` + `russh-sftp::SftpSession`
- OS file drop → GPUI `ExternalPaths` on the Files list (SSH upload)
