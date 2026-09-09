# Loom

**Loom** is a desktop terminal client for local shells and SSH. Organize connections in a sidebar, work in tabs and splits in the center, and keep sessions tidy without scattering windows across the desktop.

Built with [GPUI](https://gpui.rs). **Windows** is the primary platform today.

---

## Who it’s for

- People who juggle many **local terminals** and **SSH** hosts and want them managed in one place  
- Anyone who prefers a dedicated connection list beside the terminal, instead of ad‑hoc windows  
- Users who need split panes, reconnect, remote files, and light session tooling around the shell  

---

## What you get

### Connections & workspace

- **Local shell**, **WSL** (`wsl.exe -d …` profiles), and **SSH** profiles, organized in groups  
- Open a session from the sidebar; create, rename, duplicate, and move profiles  
- Workspace state is saved so your usual tabs can come back next time  

### Tabs & splits

- Multiple tabs, each with its own session(s)  
- Split left / right / up / down within a tab—each pane is an independent session  
- SSH splits can reuse the in‑memory session password so you aren’t prompted again unnecessarily  

### SSH

- Password or private‑key auth; optional **Remember** stores the password in the OS keyring  
- Reconnect a whole tab or a single failed pane after disconnect  
- **Port forwarding** on the same connection  
- Optional right **context panel**: remote files (SFTP) and session info  

### Terminal

- Find in scrollback  
- Context menu: copy / paste / path helpers / find / split / close, and more  
- Font size, line numbers, and ANSI color presets in Settings  

---

## Layout

```
┌─ Sidebar ────────────┬─ Tabs ──────────────────────┐
│ Groups / profiles    │  [local] [bastion] [+]        │
│                      ├──────────────────────────────┤
│  Local / WSL / SSH … │  Terminal (optional splits)  │
│                      │                              │
└──────────────────────┴────────────┬─────────────────┘
                                    │ Context panel
                                    │  Files / Info …
```

---

## Run

Requires Rust (see `rust-toolchain.toml`).

```bash
cargo run --release
```

Data lives under the app data directory (on Windows: `%APPDATA%/Loom/`), including workspace, UI state, and settings. With Remember enabled, SSH passwords are kept in the OS credential store—not as plaintext in config files.

---

## Shortcuts (selected)

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | New ephemeral local tab |
| `Ctrl+W` | Close focused pane / tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+Shift+D` | Duplicate current tab |
| `F2` | Rename selected sidebar profile/group |
| `Ctrl+F` | Find in terminal |
| `Ctrl+,` | Settings |
| `Ctrl+S` | Save workspace now |

---

## License

MIT
