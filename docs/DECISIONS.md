# Loom design decisions

Record **trade-offs and why**, so future reviews do not re-litigate from scratch.

- **Architecture** (`ARCHITECTURE.md`) = what the system is / should be  
- **This file** = options considered, choice, rationale, follow-ups  
- **Backlog** (`BACKLOG.md`) = low-priority icebox features (not decisions; do not implement unless explicitly ordered)  

Add a new section when a non-trivial option is chosen (stack, license boundary, terminal path, UX IA, etc.). Prefer facts over slogans.

## Template

```markdown
## YYYY-MM-DD — Short title

**Status:** accepted | superseded | revisit later  
**Context:** What problem or constraint forced a choice  
**Options:**
- A — …
- B — …
**Decision:** …
**Why:** …
**Consequences / follow-ups:** …
```

---

## 2026-08-28 — Stay on GPUI (rollback Slint)

**Status:** accepted  

**Context:** A Slint + alacritty + fontdue experiment hit input/focus wiring gaps, PowerShell stall on unanswered CSI queries, and soft fonts on HiDPI. Zed’s success path is GPUI + grid paint via the text system.

**Options:**
- Continue Slint and harden custom paint/input  
- Return to GPUI and evolve the terminal in-tree  

**Decision:** Roll back to the GPUI tree; treat Slint as abandoned for this product phase.

**Why:** Match the proven UI stack for a GPUI-native terminal client; avoid fighting a second toolkit while the terminal core is still unfinished.

**Consequences / follow-ups:** Do not re-introduce a framework switch without a written revisit here. Invest in GPUI terminal feel and Windows ConPTY lifecycle instead.

---

## 2026-08-28 — Learn from Zed; do not copy GPL source

**Status:** accepted  

**Context:** Zed has a high-quality terminal (`crates/terminal`, `terminal_view`) on GPUI. Loom wants similar feel but remains MIT and may be sold as a product.

**Options:**
- Vendor / paste Zed terminal crates (GPL obligations)  
- Study architecture and behavior; reimplement under MIT  
- Depend on a third-party GPUI terminal only  

**Decision:** Reference Zed for structure and behavior only. No Zed (or other GPL) source in this repo. Prefer MIT + original or MIT-licensed adaptations.

**Why:** Commercial-friendly licensing; clear ownership of the core terminal path.

**Consequences / follow-ups:** Local Zed checkout is read-only inspiration. When porting *ideas* (e.g. `PtyWrite` writeback), implement in Loom’s own modules and cite behavior, not code.

---

## 2026-08-29 — Drop crates.io `gpui-terminal`; in-house alacritty + GPUI view

**Status:** accepted  

**Context:** `gpui-terminal` 0.1.0 already uses `alacritty_terminal` + GPUI paint (same class as Zed), but its `EventListener` **ignores `Event::PtyWrite`** with the incorrect comment that alacritty handles it internally. PowerShell (and similar) send CSI `6 n` (DSR); without writing the reply back to the PTY, the shell stalls with no prompt. The published crate cannot be patched in place without forking.

Zed does **not** use `gpui-terminal`; it owns `crates/terminal` and always `write_to_pty` on `PtyWrite` (and related reply events).

**Options:**
- Keep depending on `gpui-terminal` and live with the bug / wait upstream  
- Path/git fork of `gpui-terminal` with a one-line fix  
- In-tree MIT adaptation (`src/terminal/gpui_emu/`) + direct `alacritty_terminal`  

**Decision:** Remove the `gpui-terminal` dependency. Ship an in-house view under `src/terminal/gpui_emu/` (adapted from gpui-terminal MIT OR Apache-2.0; see `THIRD_PARTY.md`) and **synchronously write `PtyWrite` to the PTY**.

**Why:**
| | crates.io `gpui-terminal` | In-house |
|--|--|--|
| Fix `PtyWrite` | Hard without fork | Immediate |
| Match product lifecycle / focus / ConPTY | Limited | Full control |
| Maintenance | Upstream cadence | We own regressions and features |
| License story | External MIT crate | MIT tree + attribution |

**Consequences / follow-ups:**
- Still thin vs Zed: scrollback, selection, IME as needed  
- Reply-class events (`ColorRequest`, `TextAreaSizeRequest`, clipboard) are queued and drained **in order** after `process_bytes` (same channel as `PtyWrite`) so OSC/CSI replies do not reorder  
- Upstream `gpui-terminal` fixes will not flow in automatically  
- Revisit only if maintenance cost outweighs control (document here if reconsidered)

---

## 2026-08-29 — Product IA: profiles/groups, not a file tree

**Status:** accepted  

**Context:** Zed’s left side is a project file tree. Loom is a connection-oriented client (Postman-style).

**Options:**
- Mirror Zed workspace / project panel  
- Left nav = groups + named shell/SSH profiles; right = multi-tab sessions  

**Decision:** Keep Postman-style profiles/groups IA.

**Why:** Product identity is session management, not editing a codebase.

**Consequences / follow-ups:** Local shell remains the default path; SSH is implemented separately via `russh`.

---

## 2026-08-29 — SSH via russh + OS keyring passwords

**Status:** accepted  

**Context:** Loom needs interactive SSH sessions that reuse the existing GPUI terminal view. Passwords must be rememberable without writing secrets into `workspace.json`.

**Options:**
- Shell out to system `ssh`  
- Embed `russh` and bridge channel I/O to `TerminalView`  
- Store passwords in profile JSON vs OS credential store  

**Decision:** Use **`russh`** on a dedicated Tokio thread; bridge stdin/stdout with flume. Persist passwords with **`keyring`** (Windows Credential Manager). Host keys use trust-on-first-use in `known_hosts.json`.

**Why:** Keeps MIT-friendly Rust stack, matches Local session I/O shape, avoids plaintext secrets on disk.

**Consequences / follow-ups:** Jump hosts / agent forwarding / passphrase UI later. Host-key change requires manual known_hosts edit for now.

---

## How to use this file in review

1. Before changing terminal stack, UI toolkit, or license boundary, read matching sections here.  
2. If you overturn a decision, mark the old section **superseded** and add a new dated section that points at it.  
3. Link new ADRs from PR descriptions when the change is structural.
