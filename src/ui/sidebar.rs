//! Profiles sidebar — Zed project-panel inspired density (original styling).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::shared::theme;
use crate::ui::workspace_store::{Selection, WorkspaceStore};

pub struct Sidebar {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    search_focused: bool,
    /// Right-click context menu for a profile (Zed-style; actions not always-visible).
    context_menu: Option<ContextMenuState>,
    _observe_store: Subscription,
}

struct ContextMenuState {
    profile_id: Uuid,
    /// Window-space point from the right-click (for `anchored` positioning).
    position: Point<Pixels>,
}

impl Sidebar {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        Self {
            store,
            focus_handle: cx.focus_handle(),
            search_focused: false,
            context_menu: None,
            _observe_store,
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_focused = true;
        self.context_menu = None;
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn ghost_icon(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        muted: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px(px(theme::SPACE_1 + 2.0))
            .py(px(2.0))
            .rounded(px(theme::RADIUS_SM))
            .text_xs()
            .font_family("Segoe UI")
            .text_color(if muted {
                theme::TEXT_DISABLED
            } else {
                theme::TEXT_MUTED
            })
            .cursor_pointer()
            .hover(|s| {
                s.bg(theme::HOVER).text_color(if muted {
                    theme::TEXT_MUTED
                } else {
                    theme::TEXT
                })
            })
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.context_menu = None;
                on_click(this, window, cx);
            }))
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
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let search = store.search.clone();
        let renaming = store.rename_buffer.clone();
        let selection = store.selection;
        let groups = store.filtered_groups();
        let other_groups: Vec<(Uuid, String)> = store
            .workspace
            .groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        let context_menu = self
            .context_menu
            .as_ref()
            .map(|m| (m.profile_id, m.position));

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
                            .child(self.ghost_icon("btn-group", "+", false, cx, |this, _, cx| {
                                this.store.update(cx, |s, cx| s.add_group(cx));
                            }))
                            .child(self.ghost_icon("btn-shell", ">_", false, cx, |this, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.add_local_profile(cx);
                                });
                            }))
                            .child(self.ghost_icon("btn-ssh", "SSH", true, cx, |_, _, _| {}))
                            .child(self.ghost_icon("btn-settings", "⚙", false, cx, |_, _, cx| {
                                cx.emit(SidebarEvent::OpenSettings);
                            })),
                    ),
            )
            // Filter — understated, no heavy chrome
            .child(
                div()
                    .id("sidebar-search")
                    .mx(px(theme::SPACE_2))
                    .mb(px(theme::SPACE_1))
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .rounded(px(theme::RADIUS_SM))
                    .bg(if self.search_focused {
                        theme::ELEVATED
                    } else {
                        theme::SIDEBAR_BG
                    })
                    .border_1()
                    .border_color(if self.search_focused {
                        theme::BORDER
                    } else {
                        theme::BORDER_SUBTLE
                    })
                    .text_xs()
                    .text_color(if search.is_empty() && !self.search_focused {
                        theme::TEXT_DISABLED
                    } else {
                        theme::TEXT
                    })
                    .child(if search.is_empty() && !self.search_focused {
                        SharedString::from("Filter…")
                    } else {
                        SharedString::from(search.clone())
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.search_focused = true;
                        this.context_menu = None;
                        this.focus_handle.focus(window);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if !this.search_focused {
                            return;
                        }
                        let key = &event.keystroke.key;
                        if key == "backspace" {
                            this.store.update(cx, |s, cx| {
                                let mut q = s.search.clone();
                                q.pop();
                                s.set_search(q, cx);
                            });
                            cx.stop_propagation();
                        } else if key == "escape" {
                            this.search_focused = false;
                            this.store.update(cx, |s, cx| s.set_search(String::new(), cx));
                            cx.notify();
                            cx.stop_propagation();
                        } else if event.keystroke.modifiers.control {
                            // let global shortcuts through
                        } else if let Some(ch) = key.chars().next() {
                            if key.len() == 1 && !ch.is_control() {
                                this.store.update(cx, |s, cx| {
                                    let mut q = s.search.clone();
                                    q.push(ch);
                                    s.set_search(q, cx);
                                });
                                cx.stop_propagation();
                            }
                        }
                    })),
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
                        div()
                            .mb(px(2.0))
                            .child(
                                div()
                                    .id(SharedString::from(format!("group-{gid}")))
                                    .flex()
                                    .items_center()
                                    .gap(px(theme::SPACE_1))
                                    .h(px(24.0))
                                    .px(px(theme::SPACE_1))
                                    .rounded(px(theme::RADIUS_SM))
                                    .when(selected_group, |d| d.bg(theme::SELECTION))
                                    .hover(|s| s.bg(theme::HOVER))
                                    .cursor_pointer()
                                    .child(
                                        div()
                                            .w(px(12.0))
                                            .text_xs()
                                            .text_color(theme::TEXT_MUTED)
                                            .child(if collapsed { "▸" } else { "▾" }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::TEXT)
                                            .child(if renaming.is_some() && selected_group {
                                                format!("{}|", renaming.clone().unwrap_or_default())
                                            } else {
                                                gname.clone()
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
                                        this.context_menu = None;
                                        this.store.update(cx, |s, cx| {
                                            s.select_group(gid, cx);
                                            s.toggle_group(gid, cx);
                                        });
                                    })),
                            )
                            .when(!collapsed, |d| {
                                d.children(profiles.into_iter().map(|profile| {
                                    let pid = profile.id;
                                    let selected = selection == Selection::Profile(pid);
                                    let label = if renaming.is_some() && selected {
                                        format!("{}|", renaming.clone().unwrap_or_default())
                                    } else {
                                        profile.name.clone()
                                    };
                                    div()
                                        .id(SharedString::from(format!("profile-{pid}")))
                                        .flex()
                                        .items_center()
                                        .gap(px(theme::SPACE_1))
                                        .h(px(24.0))
                                        .pl(px(theme::SPACE_4))
                                        .pr(px(theme::SPACE_1))
                                        .rounded(px(theme::RADIUS_SM))
                                        .when(selected, |d| d.bg(theme::SELECTION))
                                        .hover(|s| s.bg(theme::HOVER))
                                        .cursor_pointer()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme::TEXT_MUTED)
                                                .child("›"),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_xs()
                                                .text_color(theme::TEXT)
                                                .child(label),
                                        )
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
                                                        profile_id: pid,
                                                        position: event.position,
                                                    });
                                                    cx.notify();
                                                    cx.stop_propagation();
                                                },
                                            ),
                                        )
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
                    .child("↵ open · F2 rename · Del · right-click"),
            )
            // Context menu — anchored to right-click point (window coords)
            .when_some(context_menu, |this, (pid, position)| {
                let moves = other_groups.clone();
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
                        .child(
                            div()
                                .min_w(px(160.0))
                                .p(px(theme::SPACE_1))
                                .rounded(px(theme::RADIUS))
                                .bg(theme::ELEVATED)
                                .border_1()
                                .border_color(theme::BORDER)
                                .shadow_md()
                                .occlude()
                                .child(self.menu_item(
                                    "ctx-open",
                                    "Open",
                                    false,
                                    cx,
                                    move |_, _, cx| {
                                        cx.emit(SidebarEvent::OpenProfile(pid));
                                    },
                                ))
                                .child(self.menu_item(
                                    "ctx-rename",
                                    "Rename",
                                    false,
                                    cx,
                                    |this, _, cx| {
                                        this.store.update(cx, |s, cx| s.begin_rename(cx));
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
                                .child(
                                    div()
                                        .h(px(1.0))
                                        .my(px(theme::SPACE_1))
                                        .bg(theme::BORDER_SUBTLE),
                                )
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
                                )),
                        ),
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
                let key = &event.keystroke.key;
                let renaming = this.store.read(cx).rename_buffer.is_some();

                if key == "escape" {
                    if this.context_menu.is_some() {
                        this.context_menu = None;
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    if this.search_focused {
                        this.search_focused = false;
                        this.store.update(cx, |s, cx| s.set_search(String::new(), cx));
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                }

                if renaming {
                    if key == "enter" {
                        this.store.update(cx, |s, cx| s.commit_rename(cx));
                        cx.stop_propagation();
                    } else if key == "escape" {
                        this.store.update(cx, |s, cx| s.cancel_rename(cx));
                        cx.stop_propagation();
                    } else if key == "backspace" {
                        this.store.update(cx, |s, cx| s.pop_rename_char(cx));
                        cx.stop_propagation();
                    } else if let Some(ch) = key.chars().next() {
                        if key.len() == 1 {
                            this.store.update(cx, |s, cx| s.push_rename_char(ch, cx));
                            cx.stop_propagation();
                        }
                    }
                    return;
                }

                if this.search_focused {
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
}

impl EventEmitter<SidebarEvent> for Sidebar {}
