# Hard problems & lessons

Record **non-obvious GPUI / platform / terminal pitfalls** so the next pass does not rediscover them by trial and error.

- Prefer a short **symptom → failed attempts → what worked → rule** write-up for each incident.
- Keep the standing **UI freeze checklist** (below) up to date whenever a new freeze class appears — include **GPUI framework** patterns, not only Loom bugs.
- Link from the matching ADR in `DECISIONS.md` when the lesson drove a product decision.

---

## UI thread freezes — checklist (GPUI + Loom)

**Symptom class:** Window stops painting / accepting input (“卡住”), CPU may be idle (deadlock) or pegged (spin). Often misread as “slow feature” or “SSH hang”.

GPUI runs layout → prepaint → paint and input dispatch on the **window / UI thread**. Anything that **blocks** that thread, or **deadlocks** it against itself, freezes the whole app.

### A. GPUI framework / element pipeline

| Risk | Why it freezes | Do instead |
| --- | --- | --- |
| Heavy work in `Render::render`, `Element::request_layout`, `prepaint`, or `paint` | Frame cannot finish; input waits on the same thread | Keep frame callbacks O(visible work). Move I/O, crypto, DNS, large parses off-thread |
| `entity.update` / `cx.notify` that re-enters render while still inside paint/layout | Re-entrancy; easy to nest locks or infinite notify loops | Defer state changes with `cx.defer` / next frame; never “fix then notify mid-paint” unless proven safe |
| Blocking `.recv()`, `std::thread::park`, `sleep`, or `block_on(future)` inside click/key/`render` | UI thread waits forever or for a long time | `cx.spawn` + `recv_async` / channels; show Connecting UI immediately |
| Synchronous file / network / keyring / dialog on the UI thread | Looks like freeze until OS returns | Background thread or async task; only apply results via `entity.update` |
| Holding a lock across `cx.notify()`, `window.refresh()`, or painting children | Other code on the same thread needs the lock → self-deadlock | Lock → copy data → drop → then notify/paint |
| Nested `parking_lot::Mutex` / non-reentrant lock on the UI thread | Same thread locks twice → permanent hang (CPU idle) | One lock owner per call stack; pass `&T` you already hold; cache dims outside the lock |
| Infinite `notify` / layout thrash (size depends on content that changes every frame) | Spin: UI “alive” but unusable, high CPU | Stabilize layout inputs; avoid feedback loops in measure |
| Huge per-frame GPU/CPU work (shaping tens of thousands of glyphs naively) | Soft freeze / multi-second frames | Cull to viewport; batch; cache shaped lines |
| Waiting on a task that only runs on the UI executor while the UI thread is blocked | Classic async deadlock | Never block the UI executor waiting for itself |

**GPUI-oriented rules of thumb:**

1. **Frame path is sacred** — layout / prepaint / paint / `Render` must not block and must not take locks they might need again after calling into GPUI.
2. **Events are still UI thread** — `on_click` / `on_key_down` / mouse handlers share the same constraint as paint for blocking work.
3. **Prefer “snapshot then paint”** — clone the small data you need under a short lock; paint from the snapshot.
4. **Async for anything uncertain in latency** — SSH connect, disk, keyring, spawn, DNS: `thread::spawn` or `cx.spawn`, never inline on the frame path.
5. If it “only freezes when feature X is on” but X is cheap to draw, **suspect deadlock / re-entrancy**, not “X is too slow”.

### B. Loom-specific (terminal / PTY / workspace)

| Risk | Why it freezes | Do instead |
| --- | --- | --- |
| Canvas paint holds `term_arc.lock()`, then calls `TerminalState::with_term` / `with_term_mut` | Nested lock on same `parking_lot::Mutex` → hard deadlock | While paint owns `&Term`, use that reference only; for layout math use `cols`/`rows`/`scrollback` caches |
| Long critical section under term lock (shape all cells, do I/O, call into GPUI) | Blocks PTY reader side effects and other UI that needs the term | Minimize lock scope; paint from grid snapshot where possible |
| `stdin_writer.lock()` held across notify/paint | Same class of nested / ordered lock issues | Write bytes, drop guard, then notify |
| SSH `connect_blocking` (or any russh handshake) on the UI thread | Network RTT freezes UI | Keep pattern in `tab_manager`: worker thread + `flume` + `cx.spawn` await |
| `persist_now()` / large JSON write on every keystroke on UI thread | Stutter under load | Debounce; or write on a background task (follow-up if it becomes hot) |
| PTY spawn + waiting for first output synchronously in open handler | Shell startup blocks open | Spawn async; show tab in Connecting; attach terminal when ready |
| Lock order inversion (e.g. store lock then term lock vs reverse) | Cross-thread or same-thread deadlock | Document order: prefer **no nested locks**; if needed, fixed global order |

**Loom rules of thumb:**

1. **`term_arc` / `with_term*`:** at most one owner on a call stack. Paint already locking ⇒ no `with_term*`.
2. **SSH / PTY / disk:** never on the frame or click stack without an async boundary (existing SSH connect path is the template).
3. New terminal chrome (gutter, find, minimap): compute layout from **cached metrics**, draw from **already-locked** `&Term` or a snapshot.

### C. How to diagnose quickly

1. **Idle CPU + frozen UI** → almost always **deadlock** or **blocking wait** (lock, `recv`, OS dialog).
2. **High CPU + frozen/janky UI** → **spin** (notify loop) or **too much per-frame work**.
3. Bisect: disable the last feature (line numbers, find, overlay). If hang vanishes, inspect that feature’s **paint/event path for locks and blocking I/O**, not its “algorithm cost” first.
4. On Windows, break in the debugger when hung: if the UI thread sits inside `Mutex::lock` / `recv`, you found it.
5. Add a dated incident section below when a new freeze class is confirmed.

### D. Safe patterns (copy these)

```text
# SSH / slow work
thread::spawn { blocking_work → channel.send(result) }
cx.spawn { result = channel.recv_async().await; entity.update(|…| apply) }

# Terminal paint
let snap = { let t = term.lock(); copy_what_you_need(&t) }; // or use &Term only inside the lock
// drop lock before entity.update / with_term / nested GPUI that might paint again

# Gutter / chrome width
use config.scrollback + state.rows()   // no term lock
```

---

## Incident log

### 2026-08-29 — Tab-bar split popover (GPUI paint order + anchor)

**Symptom:** Columns icon on the tab bar should open a Zed-like menu (Split Right / Left / Up / Down) **directly under the icon**. Early builds: menu invisible, under the terminal, opening then vanishing, or floating far to the left of the icon.

**Why it is hard (GPUI):**

1. **Paint / hit order** — Tab bar is laid out *before* the terminal sibling. A normal absolute child of the tab bar (or of the icon button) paints **under** the terminal, so the menu looks “gone”.
2. **`deferred` vs layout origin** — Zed’s `PopoverMenu` uses `deferred(anchored(...)).with_priority(1)` so the menu paints after the rest of the frame. Nesting that under the button with `AnchoredPositionMode::Local` + a pixel offset still used the wrong origin once deferred re-prepainted, so the menu sat tens of pixels left of the control.
3. **Dismiss races** — A full-window transparent dismiss layer on the same click that opens the menu toggles open→closed in one gesture.

**Failed approaches (do not repeat without a new reason):**

| Approach | Result |
| --- | --- |
| Absolute popover as flex/absolute child of the icon | Wrong place and/or under terminal |
| Parent (`WorkspaceView`) overlay slot toggled by flag | Easy to forget parent `notify`; still fought stacking |
| `anchored` + `Local` + offset under the button + `deferred` | Visible, but horizontally detached from the icon |
| Huge dismiss hit target on open | Menu never stays open |

**What worked:**

Same pattern as the **sidebar context menu** + Zed’s deferred priority:

1. On `MouseDown` on the columns control, store **`event.position` (window coordinates)**.
2. Render `deferred(anchored().position(anchor).anchor(Corner::TopRight).child(menu)).with_priority(1)` on the tab bar root.
3. No full-window dismiss on the opening click; close via Esc, item click, or toggle the icon again.

**Rule of thumb:**

- Need “above everything” → `deferred(...).with_priority(1)` (Zed `PopoverMenu`).
- Need “next to this control” → **window-space** `anchored().position(...)` from a mouse event or measured trigger bounds — not `Local` offsets on a deferred child of the trigger.
- Mirror existing in-tree proof: `src/ui/sidebar.rs` context menu.

**Code:** `src/ui/tab_bar.rs` (split popover). Model: `src/ui/pane_layout.rs`, `src/ui/tab_manager.rs`, `src/ui/terminal_pane.rs`.

---

### 2026-08-29 — Line-number gutter froze new shell / SSH (term mutex deadlock)

**Symptom:** After enabling the scrollback line-number gutter (default on), opening a local shell or finishing SSH connect hung the UI (looked like a freeze / “卡住”).

**Cause:** Not CPU cost. Fits checklist **A (nested non-reentrant lock)** + **B (term_arc held in paint, then `with_term`)**. The canvas paint path already held `state_arc.lock()` on the alacritty `Term`, then called `content_padding()` → `line_number_gutter_width()` → `with_term()` on the same `parking_lot::Mutex` → deadlock.

**What worked:** Compute gutter width from `config.scrollback + state.rows()` (no term lock). While the paint lock is held, only read the already-locked `&Term` (for drawing numbers) — never call `with_term` / `with_term_mut`.

**See also:** [UI thread freezes — checklist](#ui-thread-freezes--checklist-gpui--loom) sections A–D.

**Code:** `src/terminal/gpui_emu/view/mod.rs` (`line_number_gutter_width`, canvas paint).

---

### 2026-08-30 — 终端 IME（中文）与接入后回车失效

**现象：**（1）中文输入法组字确认后，汉字进不了 PTY。（2）接上 GPUI `InputHandler` 后，中英文按 Enter 都没反应。

**原因归类：** 输入要拆成两条管道。可打印 / CJK 走 `Window::handle_input`（IME / `WM_CHAR`）；Enter 必须走 KeyDown——Windows 的 `WM_CHAR` 会丢掉控制字符（`\r`）。用「凡有 `key_char` 就交给 IME」会误伤 Enter。

**有效做法：** 双管道——`keystroke_to_bytes` 只返回 Escape/控制序列；`EntityInputHandler` 提交可打印与 IME 文本；仅在 KeyDown 真正写入序列时 `stop_propagation`。

**完整过程（失败尝试 + 规则）：** [TERMINAL_IME.md](./TERMINAL_IME.md)。

**代码：** `src/terminal/gpui_emu/view/mod.rs`、`src/terminal/gpui_emu/input.rs`。

---

### 2026-08-30 — SSH 断线后假活输入、只能重开 tab

**现象：** 连接已断，prompt 仍在，按键无效果；Files 常 `SFTP unavailable`；状态栏不出 Reconnect，只能重新打开 profile。

**原因归类：** Exit/写失败未反映到 `ConnectionState`；`write_to_pty` 静默吞 BrokenPipe；Reconnect UI 依赖 Failed 态；密码回调曾走「新开 tab」。

**有效做法：** `SessionEnded` → pane Failed + 回收 SFTP；`session_alive` 挡写入 + 横幅；状态栏手动 Reconnect（密码走 `pending_reconnect_tab`）；Files 用 `bound_sftp_alive` 重绑。不做自动重连（主路径）。

**完整说明：** [SESSION_RECONNECT.md](./SESSION_RECONNECT.md)。

**代码：** `src/terminal/gpui_emu/view/mod.rs`、`src/ui/tab_manager.rs`、`src/ui/workspace_view.rs`、`src/ui/context_panel.rs`。

---
