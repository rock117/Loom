# Loom design decisions

Record **trade-offs and why**, so future reviews do not re-litigate from scratch.

- **Architecture** (`ARCHITECTURE.md`) = what the system is / should be  
- **This file** = options considered, choice, rationale, follow-ups  
- **Hard problems** (`HARD_PROBLEMS.md`) = non-obvious GPUI/platform pitfalls; starts with a standing **UI freeze checklist (GPUI + Loom)** plus dated incident write-ups   
- **Backlog** (`BACKLOG.md`) = low-priority icebox features (not decisions; do not implement unless explicitly ordered)  

Add a new section when a non-trivial option is chosen (stack, license boundary, terminal path, UX IA, etc.). Prefer facts over slogans. When a feature turns into a multi-hour positioning/focus/PTY fight, also add a dated entry under `HARD_PROBLEMS.md`.

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

**Decision:** Use **`russh`** on a dedicated Tokio thread; bridge stdin/stdout with flume. Persist passwords with **`keyring`** + feature **`windows-native`** (Windows Credential Manager). Host keys use trust-on-first-use in `known_hosts.json`. Without the platform keystore feature, keyring uses a non-persistent mock — passwords will not stick.

**Why:** Keeps MIT-friendly Rust stack, matches Local session I/O shape, avoids plaintext secrets on disk.

**Consequences / follow-ups:** Jump hosts / agent forwarding / passphrase UI later. Host-key change requires manual known_hosts edit for now.

---

## 2026-08-29 — Terminal find uses literal RegexSearch

**Status:** accepted  

**Context:** Ctrl+F over scrollback needs match + scroll; alacritty exposes `RegexSearch` / `search_next`.

**Decision:** Escape user input as a literal pattern; highlight via existing selection; overlay find bar on the terminal view (not a global Loom action yet).

**Why:** Matches common terminal UX; avoids building a separate highlighter; reuses selection paint.

**Consequences / follow-ups:** Regex mode / match count / highlight-all can come later if needed.

---

## 2026-08-29 — Pane splits follow Zed’s binary-tree model

**Status:** accepted  

**Context:** BACKLOG P4; user asked to match Zed’s split behavior rather than Windows Terminal–only Right/Down. Primary UX is a **tab-bar columns button + popover** (not keyboard-first).

**Decision:** Each tab owns a binary `PaneLayout` tree. Actions mirror Zed: `SplitLeft/Right/Up/Down`, `ActivatePane*` by geometry. Each leaf is an independent session (same profile as the focused pane). Tab bar columns icon opens a four-way split menu; keybindings remain: `ctrl-k` + arrows to split, `ctrl-k ctrl-arrows` to activate, `ctrl-\` = SplitRight. `Ctrl+W` closes the focused pane (last pane closes the tab). Focus chrome only when `leaf_count() > 1`.

**Why:** Matches product UX the team already knows from Zed; GPUI already supports chord keybindings. Button + popover matches how users discover splits.

**Consequences / follow-ups:** Drag-rearrange / persist split trees later. `ctrl-k` chord may briefly compete with shell Ctrl+K (same as Zed terminal). Popover positioning is a GPUI hard problem — see `HARD_PROBLEMS.md` (2026-08-29 tab-bar split popover).

---

## 2026-08-29 — Three-column layout: right context panel

**Status:** accepted  

**Context:** Two-column Loom (Profiles | Terminal) works, but WindTerm-style clients get leverage from a **third column** for session-scoped tools. User asked to document and implement per value order: Snippets → Files/SFTP → Info.

**Options:**
- Stay two-column; put snippets in Settings / menus only  
- Overlay drawer from the right (temporary)  
- Persistent right **context panel** with show/hide like the left sidebar  

**Decision:** Add a toggleable right **Context panel** (`docs/CONTEXT_PANEL.md`). Center remains primary. Primary section = **Files (SFTP)** with Explorer-style navigation and a transfers footer; **Info** stays a thin summary. Snippets are deferred (not a default tab). Default visibility **off**.

**Why:** Clear IA (left = connections, right = tools for current session); SFTP is the high-value WindTerm-style differentiator for a Postman-style SSH client.

**Consequences / follow-ups:** Same russh session opens SFTP subsystem channel(s) on demand (`russh-sftp`). Browse/Transfer 分车道连接池、懒开与回收见 [SFTP_POOL.md](./SFTP_POOL.md)。Recursive folder download/upload in MVP; drag-drop and transfer persistence later.

---

## 2026-08-29 — Docker 会话与 SSH 同级（exec + docker cp Files）

**Status:** accepted（规格；实现未开始）  

**Context:** 进入容器内部是高频路径（`docker exec -it`），用户希望体验与 SSH 进服务器一致；并要求右侧 Files 可用，传输走 `docker cp`。

**Options:**
- A — 仅文档化 / 依赖用户手敲 exec，不做一等会话  
- B — 只做 exec Tab，Files 仍空状态  
- C — Docker = 一等会话：exec shell + Context Files（`docker cp`），IA 对齐 SSH  

**Decision:** 选 C。规格见 [DOCKER_SESSION.md](./DOCKER_SESSION.md)。不做 Docker Desktop 式管理器；K8s exec 另案。本机 MVP → 再远端（SSH 宿主机上的 Docker）。

**Why:** 与现有 Profile / Tab / Context 模型同构；Files 用 `docker cp` 补齐「进得去也能拷文件」，避免为容器再开一套 SFTP 假象。

**Consequences / follow-ups:** 浏览与 `docker cp` 分车道、可取消、按 pane 隔离 Transfers；实现须用户明确点名后按阶段 1→2 开工。

---

## 2026-08-30 — SSH 端口转发为增强能力（非核心）

**Status:** accepted（规格；实现未开始）  

**Context:** 讨论 Local / Remote / SOCKS UI。命令行等价于 `ssh -L/-R/-D`；可与 shell、SFTP 共用同一 SSH 连接，远端只需 `sshd`，无需 Loom 服务端。

**Options:**
- A — 不做，继续手敲 `ssh -L`  
- B — 做成核心卖点 / 独立隧道产品  
- C — 文档化为 **Profile/会话增强**；同连接 ForwardHub；分期 Local → Remote → SOCKS  

**Decision:** 选 C。规格见 [PORT_FORWARD.md](./PORT_FORWARD.md)。定位明确为**非核心**；默认本机 bind `127.0.0.1`。

**Why:** 覆盖连远端 DB/内网 HTTP 等高频排障，又不稀释「终端 + Files」主叙事；实现量相对可控，且不依赖自研服务端。

**Consequences / follow-ups:** Backlog **P1c**；用户点名后再按阶段 1 实现 Local。

---

## 2026-08-31 — Session / Profile / Group IA（Unix 根隐喻）

**Status:** accepted（实施中）  

**Context:** 侧栏强制「Profile 必须属于 Group」、Tab 与 Profile 强绑定，导致 Ctrl+T / Duplicate 语义拧巴。

**Decision:**
- 工作区根像 `/`：可挂 **Profile（文件）** 与 **Group（目录，可嵌套）**。  
- 侧栏 New* / Duplicate Profile → 只改收藏，**不**自动开 Session。  
- Ctrl+T / Tab Duplicate / Split → **Ephemeral** Session；重启不恢复。  
- 点侧栏打开 → Bound；Tab「Save to…」升级为 Profile。  
- 文案统一 **Duplicate**，靠 context 区分。  

规格：[SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md)。

**Why:** 对齐 Unix 文件哲学与 Postman「收藏 vs 运行中会话」；临时会话不污染侧栏。

**Consequences / follow-ups:** Group 跨层 DnD、删目录级联 UI 可后续增强。

---

## How to use this file in review

1. Before changing terminal stack, UI toolkit, or license boundary, read matching sections here.  
2. If you overturn a decision, mark the old section **superseded** and add a new dated section that points at it.  
3. Link new ADRs from PR descriptions when the change is structural.
