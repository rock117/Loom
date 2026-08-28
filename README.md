# Loom

A desktop terminal client built with [GPUI](https://gpui.rs): local shell and SSH only, with a Postman-style layout.

## Features

- **Left nav**: groups and named local shell profiles
- **Right pane**: multi-tab sessions; each tab can be renamed
- **Profiles**: create, rename, duplicate, move between groups, delete
- **Groups**: create, rename, delete; workspace auto-saves to disk
- **Sessions**: open from a profile, duplicate tab, reconnect when disconnected
- **Local shell** (priority): via `portable-pty` + `gpui-terminal` (Windows: pwsh / PowerShell / cmd)
- **SSH** (later): planned via `russh`; not in the current implementation focus

## Layout

```
+-- Sidebar ------------------+-- Tabs --------------------+
| Search / New Group|Shell|SSH | [pwsh] [bastion] [+]       |
| v Local                     +----------------------------+
|   PowerShell                | Terminal                   |
| v Production                |                            |
|   bastion                   |                            |
+-----------------------------+----------------------------+
```

## Requirements

- Rust stable with edition 2024 (see `rust-toolchain.toml`)
- Windows 10+ (primary), or macOS / Linux

## Build & run

```bash
cargo run --release
```

Debug:

```bash
cargo run
```

## Data locations

On Windows (under `%APPDATA%/Loom/`):

| File | Purpose |
|------|---------|
| `workspace.json` | Groups and profiles |
| `ui_state.json` | Sidebar width, open tabs, window bounds |
| `settings.json` | Default shell, font |
| `known_hosts.json` | SSH host key fingerprints |

Passwords and key passphrases are kept in memory only and are never written to disk.

## Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | New local tab (default profile) |
| `Ctrl+W` | Close current tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+Shift+D` | Duplicate current tab |
| `F2` | Rename focused profile or tab |
| `Ctrl+S` | Save workspace now |
| `Ctrl+,` | Open Settings |
| `Ctrl+E` | Export workspace |
| `Ctrl+Shift+I` | Import workspace |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Font size |
| `Ctrl+F` | Focus sidebar search |

## Project layout

```
src/
  app.rs                 # Application entry / key bindings / quit
  platform.rs + platform/# OS paths, default shell, fonts (Windows-first)
  model.rs + model/       # Workspace, profiles, persistence
  session.rs + session/  # Local PTY + teardown helpers
  terminal.rs + terminal/# Scaffold toward in-house GPUI terminal
  shared.rs + shared/    # Theme, actions, paths
  ui.rs + ui/            # Sidebar, tabs, settings, widgets
```

GPUI code follows the skills under `~/.agents/skills` (entities observe, Stateful `.id()` + click handlers, no `mod.rs`, SharedString, explicit error logging).

Target architecture (Zed-inspired ideas, Windows-first, MIT): see **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**.

## Roadmap

Execution order: **scaffold → layout/local profiles → local PTY → local polish → SSH (deferred)**.

Longer-term direction (custom GPUI terminal element, platform layer, Zed-like feel) is described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

v1 also excludes serial/Telnet, SFTP UI, port forwarding, cloud sync, and plugins.

## License

MIT
