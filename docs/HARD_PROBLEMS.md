# Hard problems & lessons

Record **non-obvious GPUI / platform / terminal pitfalls** so the next pass does not rediscover them by trial and error.

- Prefer a short **symptom → failed attempts → what worked → rule** write-up.
- Link from the matching ADR in `DECISIONS.md` when the lesson drove a product decision.
- Add a new dated section whenever a feature burns more than a quick iteration on positioning, focus, paint order, or PTY lifecycle.

---

## 2026-08-29 — Tab-bar split popover (GPUI paint order + anchor)

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
