//! Profiles sidebar — Zed project-panel inspired density (original styling).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::ProfileKind;
use crate::shared::theme;
use crate::ui::workspace_store::{Selection, WorkspaceStore};

const ICON_TREE: f32 = 14.0;
const ICON_HEADER: f32 = 13.0;
const ROW_H: f32 = 26.0;

pub struct Sidebar {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    /// Right-click context menu for a profile or group (Zed-style).
    context_menu: Option<ContextMenuState>,
    /// Keeps the caret blink loop alive while renaming.
    _caret_blink: Option<Task<()>>,
    _observe_store: Subscription,
}

#[derive(Clone, Copy)]
enum ContextTarget {
    Profile(Uuid),
    Group(Uuid),
}

struct ContextMenuState {
    target: ContextTarget,
    /// Window-space point from the right-click (for `anchored` positioning).
    position: Point<Pixels>,
}

/// Payload for sidebar drag-and-drop (profiles ↔ groups, group reorder).
#[derive(Clone, Debug)]
enum DraggedSidebarItem {
    Profile { id: Uuid, name: SharedString },
    Group { id: Uuid, name: SharedString },
}

struct DragPreview {
    label: SharedString,
}

impl DragPreview {
    fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::ACCENT)
            .shadow_md()
            .text_xs()
            .text_color(theme::TEXT)
            .child(self.label.clone())
    }
}

impl Sidebar {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        Self {
            store,
            focus_handle: cx.focus_handle(),
            context_menu: None,
            _caret_blink: None,
            _observe_store,
        }
    }

    fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        self._caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.store.read(cx).rename.is_some() {
                            this.store.update(cx, |s, cx| s.toggle_rename_caret(cx));
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    pub fn begin_rename(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.store.update(cx, |s, cx| s.begin_rename(cx));
        if self.store.read(cx).rename.is_some() {
            self.start_caret_blink(cx);
        }
        cx.notify();
    }

    fn svg_icon(path: &'static str, size: f32, color: Hsla) -> impl IntoElement {
        svg()
            .path(path)
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
    }

    fn ghost_svg(
        &self,
        id: impl Into<ElementId>,
        icon: &'static str,
        tip: &'static str,
        muted: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        use crate::ui::tooltip::Tooltip;

        let color = if muted {
            theme::TEXT_DISABLED
        } else {
            theme::TEXT_MUTED
        };
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.0))
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .tooltip(move |_, cx| Tooltip::text(tip, cx))
            .child(Self::svg_icon(icon, ICON_HEADER, color))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.context_menu = None;
                on_click(this, window, cx);
            }))
    }

    fn profile_icon(kind: &ProfileKind) -> (&'static str, Hsla) {
        match kind {
            ProfileKind::Local { .. } => ("icons/ui/terminal.svg", theme::ICON_LOCAL),
            ProfileKind::Ssh { .. } => ("icons/ui/remote.svg", theme::ICON_REMOTE),
        }
    }

    fn menu_item(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        danger: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .text_sm()
            .text_color(if danger { theme::DANGER } else { theme::TEXT })
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                on_click(this, window, cx);
                this.context_menu = None;
                cx.notify();
            }))
    }

    fn menu_shell(&self) -> Div {
        div()
            .min_w(px(160.0))
            .p(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::BORDER)
            .shadow_md()
            .occlude()
    }

    fn menu_divider(&self) -> impl IntoElement {
        div()
            .h(px(1.0))
            .my(px(theme::SPACE_1))
            .bg(theme::BORDER_SUBTLE)
    }

    fn profile_context_menu(
        &self,
        pid: Uuid,
        moves: Vec<(Uuid, String)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_ssh = self
            .store
            .read(cx)
            .workspace
            .find_profile(pid)
            .map(|p| matches!(p.kind, ProfileKind::Ssh { .. }))
            .unwrap_or(false);

        self.menu_shell()
            .child(self.menu_item(
                "ctx-open",
                "Open",
                false,
                cx,
                move |_, _, cx| {
                    cx.emit(SidebarEvent::OpenProfile(pid));
                },
            ))
            .when(is_ssh, |d| {
                d.child(self.menu_item(
                    "ctx-edit-ssh",
                    "Edit SSH…",
                    false,
                    cx,
                    move |_, _, cx| {
                        cx.emit(SidebarEvent::EditSshProfile(pid));
                    },
                ))
            })
            .child(self.menu_item(
                "ctx-rename",
                "Rename",
                false,
                cx,
                |this, window, cx| {
                    this.begin_rename(cx);
                    this.focus_handle.focus(window);
                },
            ))
            .child(self.menu_item(
                "ctx-dup",
                "Duplicate",
                false,
                cx,
                move |this, _, cx| {
                    this.store.update(cx, |s, cx| {
                        s.duplicate_profile(pid, cx);
                    });
                },
            ))
            .children(moves.into_iter().take(4).map(|(gid, name)| {
                let label: SharedString = format!("Move → {name}").into();
                div()
                    .id(SharedString::from(format!("ctx-move-{gid}")))
                    .w_full()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .rounded(px(theme::RADIUS_SM))
                    .text_sm()
                    .text_color(theme::TEXT_MUTED)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.move_profile_to_group(pid, gid, cx);
                        });
                        this.context_menu = None;
                        cx.notify();
                    }))
            }))
            .child(self.menu_divider())
            .child(self.menu_item(
                "ctx-del",
                "Delete",
                true,
                cx,
                |this, _, cx| {
                    this.store.update(cx, |s, cx| s.delete_selection(cx));
                },
            ))
            .child(self.menu_item(
                "ctx-dismiss",
                "Close",
                false,
                cx,
                |_, _, _| {},
            ))
    }

    fn group_context_menu(&self, gid: Uuid, cx: &mut Context<Self>) -> impl IntoElement {
        self.menu_shell()
            .child(self.menu_item(
                "ctx-g-rename",
                "Rename",
                false,
                cx,
                |this, window, cx| {
                    this.begin_rename(cx);
                    this.focus_handle.focus(window);
                },
            ))
            .child(self.menu_item(
                "ctx-g-toggle",
                "Expand / Collapse",
                false,
                cx,
                move |this, _, cx| {
                    this.store.update(cx, |s, cx| s.toggle_group(gid, cx));
                },
            ))
            .child(self.menu_item(
                "ctx-g-shell",
                "New Shell",
                false,
                cx,
                move |this, _, cx| {
                    this.store.update(cx, |s, cx| {
                        s.select_group(gid, cx);
                        s.add_local_profile(cx);
                    });
                },
            ))
            .child(self.menu_item(
                "ctx-g-ssh",
                "New SSH…",
                false,
                cx,
                move |this, _, cx| {
                    this.store.update(cx, |s, cx| s.select_group(gid, cx));
                    cx.emit(SidebarEvent::OpenSshForm);
                },
            ))
            .child(self.menu_divider())
            .child(self.menu_item(
                "ctx-g-del",
                "Delete Group",
                true,
                cx,
                |this, _, cx| {
                    this.store.update(cx, |s, cx| s.delete_selection(cx));
                },
            ))
            .child(self.menu_item(
                "ctx-g-dismiss",
                "Close",
                false,
                cx,
                |_, _, _| {},
            ))
    }
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let renaming = store.rename.clone();
        let selection = store.selection;
        let groups = store.sidebar_groups();
        let other_groups: Vec<(Uuid, String)> = store
            .workspace
            .groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        let context_menu = self
            .context_menu
            .as_ref()
            .map(|m| (m.target, m.position));

        div()
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .w_full()
            .bg(theme::SIDEBAR_BG)
            .border_r_1()
            .border_color(theme::BORDER_SUBTLE)
            .text_sm()
            // Header: title + compact actions (Zed panel header density)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(32.0))
                    .px(px(theme::SPACE_2))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::TEXT_MUTED)
                            .child("Profiles"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .child(self.ghost_svg(
                                "btn-group",
                                "icons/ui/folder.svg",
                                "New Group",
                                false,
                                cx,
                                |this, _, cx| {
                                    this.store.update(cx, |s, cx| s.add_group(cx));
                                },
                            ))
                            .child(self.ghost_svg(
                                "btn-shell",
                                "icons/ui/terminal.svg",
                                "New Local Shell",
                                false,
                                cx,
                                |this, _, cx| {
                                    this.store.update(cx, |s, cx| {
                                        s.add_local_profile(cx);
                                    });
                                },
                            ))
                            .child(self.ghost_svg(
                                "btn-ssh",
                                "icons/ui/remote.svg",
                                "New SSH Profile",
                                false,
                                cx,
                                |_, _, cx| {
                                    cx.emit(SidebarEvent::OpenSshForm);
                                },
                            ))
                            .child(self.ghost_svg(
                                "btn-settings",
                                "icons/ui/settings.svg",
                                "Settings",
                                false,
                                cx,
                                |_, _, cx| {
                                    cx.emit(SidebarEvent::OpenSettings);
                                },
                            )),
                    ),
            )
            // Tree
            .child(
                div()
                    .id("sidebar-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(theme::SPACE_1))
                    .pb(px(theme::SPACE_2))
                    .children(groups.into_iter().map(|(gid, gname, collapsed, profiles)| {
                        let selected_group = selection == Selection::Group(gid);
                        let folder_icon = if collapsed {
                            "icons/ui/folder.svg"
                        } else {
                            "icons/ui/folder-open.svg"
                        };
                        let chevron = if collapsed {
                            "icons/ui/chevron-right.svg"
                        } else {
                            "icons/ui/chevron-down.svg"
                        };
                        div()
                            .mb(px(2.0))
                            .child(
                                div()
                                    .id(SharedString::from(format!("group-{gid}")))
                                    .flex()
                                    .items_center()
                                    .gap(px(theme::SPACE_1))
                                    .h(px(ROW_H))
                                    .px(px(theme::SPACE_1))
                                    .rounded(px(theme::RADIUS_SM))
                                    .when(selected_group, |d| d.bg(theme::SELECTION))
                                    .hover(|s| s.bg(theme::HOVER))
                                    .drag_over::<DraggedSidebarItem>(|style, _, _, _| {
                                        style.bg(theme::ACCENT.opacity(0.25))
                                    })
                                    .cursor_pointer()
                                    .child(Self::svg_icon(
                                        chevron,
                                        12.0,
                                        theme::TEXT_DISABLED,
                                    ))
                                    .child(Self::svg_icon(
                                        folder_icon,
                                        ICON_TREE,
                                        theme::ICON_GROUP,
                                    ))
                                    .child({
                                        if let Some(edit) =
                                            renaming.as_ref().filter(|_| selected_group)
                                        {
                                            edit.into_element()
                                        } else {
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme::TEXT)
                                                .child(gname.clone())
                                                .into_any_element()
                                        }
                                    })
                                    .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                                        this.context_menu = None;
                                        if this.store.read(cx).rename.is_some() {
                                            return;
                                        }
                                        this.store.update(cx, |s, cx| {
                                            s.select_group(gid, cx);
                                            s.toggle_group(gid, cx);
                                        });
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                            this.store.update(cx, |s, cx| {
                                                s.select_group(gid, cx);
                                            });
                                            this.context_menu = Some(ContextMenuState {
                                                target: ContextTarget::Group(gid),
                                                position: event.position,
                                            });
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .on_drag(
                                        DraggedSidebarItem::Group {
                                            id: gid,
                                            name: gname.clone().into(),
                                        },
                                        |item, _offset, _, cx| {
                                            let label = match item {
                                                DraggedSidebarItem::Group { name, .. } => {
                                                    name.clone()
                                                }
                                                DraggedSidebarItem::Profile { name, .. } => {
                                                    name.clone()
                                                }
                                            };
                                            cx.new(|_| DragPreview::new(label))
                                        },
                                    )
                                    .can_drop(|dragged: &dyn std::any::Any, _, _| {
                                        dragged.is::<DraggedSidebarItem>()
                                    })
                                    .on_drop(cx.listener(
                                        move |this, item: &DraggedSidebarItem, _, cx| {
                                            this.context_menu = None;
                                            match item {
                                                DraggedSidebarItem::Profile { id, .. } => {
                                                    this.store.update(cx, |s, cx| {
                                                        s.move_profile_to_group(*id, gid, cx);
                                                    });
                                                }
                                                DraggedSidebarItem::Group { id, .. } => {
                                                    if *id != gid {
                                                        this.store.update(cx, |s, cx| {
                                                            s.reorder_group_before(*id, gid, cx);
                                                        });
                                                    }
                                                }
                                            }
                                        },
                                    )),
                            )
                            .when(!collapsed, |d| {
                                d.children(profiles.into_iter().map(|profile| {
                                    let pid = profile.id;
                                    let selected = selection == Selection::Profile(pid);
                                    let (icon_path, icon_color) =
                                        Self::profile_icon(&profile.kind);
                                    let drag_name: SharedString = profile.name.clone().into();
                                    let name_el: AnyElement =
                                        if let Some(edit) = renaming.as_ref().filter(|_| selected) {
                                            edit.into_element()
                                        } else {
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_xs()
                                                .text_color(theme::TEXT)
                                                .child(profile.name.clone())
                                                .into_any_element()
                                        };
                                    div()
                                        .id(SharedString::from(format!("profile-{pid}")))
                                        .flex()
                                        .items_center()
                                        .gap(px(theme::SPACE_1))
                                        .h(px(ROW_H))
                                        .pl(px(theme::SPACE_4 + 4.0))
                                        .pr(px(theme::SPACE_1))
                                        .rounded(px(theme::RADIUS_SM))
                                        .when(selected, |d| d.bg(theme::SELECTION))
                                        .hover(|s| s.bg(theme::HOVER))
                                        .drag_over::<DraggedSidebarItem>(|style, _, _, _| {
                                            style.bg(theme::ACCENT.opacity(0.25))
                                        })
                                        .cursor_pointer()
                                        .child(Self::svg_icon(icon_path, ICON_TREE, icon_color))
                                        .child(name_el)
                                        .on_click(cx.listener(
                                            move |this, event: &ClickEvent, _window, cx| {
                                                this.context_menu = None;
                                                this.store.update(cx, |s, cx| {
                                                    s.select_profile(pid, cx)
                                                });
                                                if event.click_count() >= 2 {
                                                    cx.emit(SidebarEvent::OpenProfile(pid));
                                                }
                                            },
                                        ))
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(
                                                move |this, event: &MouseDownEvent, _, cx| {
                                                    this.store.update(cx, |s, cx| {
                                                        s.select_profile(pid, cx)
                                                    });
                                                    this.context_menu = Some(ContextMenuState {
                                                        target: ContextTarget::Profile(pid),
                                                        position: event.position,
                                                    });
                                                    cx.notify();
                                                    cx.stop_propagation();
                                                },
                                            ),
                                        )
                                        .on_drag(
                                            DraggedSidebarItem::Profile {
                                                id: pid,
                                                name: drag_name,
                                            },
                                            |item, _offset, _, cx| {
                                                let label = match item {
                                                    DraggedSidebarItem::Profile { name, .. } => {
                                                        name.clone()
                                                    }
                                                    DraggedSidebarItem::Group { name, .. } => {
                                                        name.clone()
                                                    }
                                                };
                                                cx.new(|_| DragPreview::new(label))
                                            },
                                        )
                                        .can_drop(|dragged: &dyn std::any::Any, _, _| {
                                            matches!(
                                                dragged.downcast_ref::<DraggedSidebarItem>(),
                                                Some(DraggedSidebarItem::Profile { .. })
                                            )
                                        })
                                        .on_drop(cx.listener(
                                            move |this, item: &DraggedSidebarItem, _, cx| {
                                                this.context_menu = None;
                                                if let DraggedSidebarItem::Profile { id, .. } = item
                                                {
                                                    if *id != pid {
                                                        this.store.update(cx, |s, cx| {
                                                            s.move_profile_before(*id, pid, cx);
                                                        });
                                                    }
                                                }
                                            },
                                        ))
                                }))
                            })
                    })),
            )
            // Hint footer — shortcuts instead of button wall
            .child(
                div()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .border_t_1()
                    .border_color(theme::BORDER_SUBTLE)
                    .text_xs()
                    .text_color(theme::TEXT_DISABLED)
                    .child("↵ open · drag to move · F2 · Del · right-click"),
            )
            // Context menu — anchored to right-click point (window coords)
            .when_some(context_menu, |this, (target, position)| {
                let moves = other_groups.clone();
                let menu: AnyElement = match target {
                    ContextTarget::Profile(pid) => {
                        self.profile_context_menu(pid, moves, cx).into_any_element()
                    }
                    ContextTarget::Group(gid) => {
                        self.group_context_menu(gid, cx).into_any_element()
                    }
                };
                this.child(
                    anchored()
                        .position(position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(Edges {
                            top: px(4.0),
                            right: px(4.0),
                            bottom: px(4.0),
                            left: px(4.0),
                        })
                        .child(menu),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    // Clicking empty chrome dismisses the menu (items stop propagation via their handlers).
                    if this.context_menu.is_some() {
                        // Don't clear here — child clicks fire first; Esc / menu actions clear.
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let key = event.keystroke.key.as_str();
                let mods = &event.keystroke.modifiers;
                let renaming = this.store.read(cx).rename.is_some();

                if key == "escape" {
                    if this.context_menu.is_some() {
                        this.context_menu = None;
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                }

                if renaming {
                    let shift = mods.shift;
                    let chord = mods.control || mods.platform;

                    if key == "enter" {
                        this.store.update(cx, |s, cx| s.commit_rename(cx));
                        cx.stop_propagation();
                        return;
                    }
                    if key == "escape" {
                        this.store.update(cx, |s, cx| s.cancel_rename(cx));
                        cx.stop_propagation();
                        return;
                    }
                    if chord && key.eq_ignore_ascii_case("a") {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.select_all());
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if chord && key.eq_ignore_ascii_case("c") {
                        this.store.update(cx, |s, cx| {
                            if let Some(edit) = &s.rename {
                                let text = if edit.has_selection() {
                                    edit.selected_text()
                                } else {
                                    edit.text.clone()
                                };
                                if !text.is_empty() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }
                            }
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if chord && key.eq_ignore_ascii_case("x") {
                        this.store.update(cx, |s, cx| {
                            if let Some(edit) = &s.rename {
                                let text = if edit.has_selection() {
                                    edit.selected_text()
                                } else {
                                    edit.text.clone()
                                };
                                if !text.is_empty() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }
                            }
                            s.with_rename(cx, |e| {
                                if e.has_selection() {
                                    e.delete_selection();
                                } else {
                                    e.text.clear();
                                    e.cursor = 0;
                                    e.anchor = 0;
                                }
                            });
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if (chord && key.eq_ignore_ascii_case("v"))
                        || (mods.shift && key.eq_ignore_ascii_case("insert"))
                    {
                        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                            let cleaned = text.replace('\r', "").replace('\n', "");
                            if !cleaned.is_empty() {
                                this.store.update(cx, |s, cx| {
                                    s.with_rename(cx, |e| e.insert(&cleaned));
                                });
                            }
                        }
                        cx.stop_propagation();
                        return;
                    }
                    if key == "backspace" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.backspace());
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if key == "delete" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.delete_forward());
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if key == "left" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.move_left(shift));
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if key == "right" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.move_right(shift));
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if key == "home" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.move_home(shift));
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if key == "end" {
                        this.store.update(cx, |s, cx| {
                            s.with_rename(cx, |e| e.move_end(shift));
                        });
                        cx.stop_propagation();
                        return;
                    }
                    if !chord
                        && !mods.alt
                        && let Some(typed) = event.keystroke.key_char.as_deref()
                    {
                        let mut cleaned = String::new();
                        for ch in typed.chars() {
                            if !ch.is_control() {
                                cleaned.push(ch);
                            }
                        }
                        if !cleaned.is_empty() {
                            this.store.update(cx, |s, cx| {
                                s.with_rename(cx, |e| e.insert(&cleaned));
                            });
                            cx.stop_propagation();
                        }
                    }
                    return;
                }

                // Delete selection (profile/group) when not renaming.
                if key == "delete" || key == "backspace" {
                    this.store.update(cx, |s, cx| s.delete_selection(cx));
                    this.context_menu = None;
                    cx.stop_propagation();
                } else if key == "enter" {
                    if let Selection::Profile(id) = this.store.read(cx).selection {
                        cx.emit(SidebarEvent::OpenProfile(id));
                        cx.stop_propagation();
                    }
                }
            }))
    }
}

#[derive(Clone, Debug)]
pub enum SidebarEvent {
    OpenProfile(Uuid),
    #[allow(dead_code)]
    ShowProfileMenu(Uuid),
    OpenSettings,
    OpenSshForm,
    EditSshProfile(Uuid),
}

impl EventEmitter<SidebarEvent> for Sidebar {}
