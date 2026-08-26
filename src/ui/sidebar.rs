use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::shared::theme;
use crate::ui::widgets::IconButton;
use crate::ui::workspace_store::{Selection, WorkspaceStore};

pub struct Sidebar {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    search_focused: bool,
    _observe_store: Subscription,
}

impl Sidebar {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        Self {
            store,
            focus_handle: cx.focus_handle(),
            search_focused: false,
            _observe_store,
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_focused = true;
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn toolbar_button(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .bg(theme::PANEL_BG)
            .border_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT)
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
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

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .h_full()
            .w_full()
            .bg(theme::SIDEBAR_BG)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .gap_1()
                    .p_2()
                    .child(self.toolbar_button("btn-group", "+ Group", cx, |this, _, cx| {
                        this.store.update(cx, |s, cx| s.add_group(cx));
                    }))
                    .child(self.toolbar_button("btn-shell", "+ Shell", cx, |this, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.add_local_profile(cx);
                        });
                    }))
                    .child(
                        IconButton::new("btn-ssh-soon", "+ SSH (soon)", |_, _, _| {})
                            .muted(true),
                    ),
            )
            .child(
                div()
                    .id("sidebar-search")
                    .mx_2()
                    .mb_2()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(theme::PANEL_BG)
                    .border_1()
                    .border_color(if self.search_focused {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .text_color(theme::TEXT)
                    .text_sm()
                    .child(if search.is_empty() && !self.search_focused {
                        SharedString::from("Search profiles…")
                    } else {
                        SharedString::from(search.clone())
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.search_focused = true;
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
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .px_1()
                    .children(groups.into_iter().map(|(gid, gname, collapsed, profiles)| {
                        let selected_group = selection == Selection::Group(gid);
                        div()
                            .mb_1()
                            .child(
                                div()
                                    .id(SharedString::from(format!("group-{gid}")))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .when(selected_group, |d| d.bg(theme::ACCENT_SOFT))
                                    .hover(|s| s.bg(theme::HOVER))
                                    .cursor_pointer()
                                    .child(
                                        div()
                                            .text_color(theme::TEXT_MUTED)
                                            .text_xs()
                                            .child(if collapsed { "▸" } else { "▾" }),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme::TEXT)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(if renaming.is_some() && selected_group {
                                                format!("{}|", renaming.clone().unwrap_or_default())
                                            } else {
                                                gname.clone()
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _event: &ClickEvent, _, cx| {
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
                                    let summary = profile.kind.summary();
                                    div()
                                        .id(SharedString::from(format!("profile-{pid}")))
                                        .ml_3()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(4.0))
                                        .when(selected, |d| d.bg(theme::ACCENT_SOFT))
                                        .hover(|s| s.bg(theme::HOVER))
                                        .cursor_pointer()
                                        .child(
                                            div()
                                                .text_color(theme::TEXT)
                                                .text_sm()
                                                .child(label),
                                        )
                                        .child(
                                            div()
                                                .text_color(theme::TEXT_MUTED)
                                                .text_xs()
                                                .child(summary),
                                        )
                                        .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                            this.store.update(cx, |s, cx| s.select_profile(pid, cx));
                                            if event.click_count() >= 2 {
                                                cx.emit(SidebarEvent::OpenProfile(pid));
                                            }
                                        }))
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(move |this, _, _, cx| {
                                                this.store.update(cx, |s, cx| s.select_profile(pid, cx));
                                                cx.emit(SidebarEvent::ShowProfileMenu(pid));
                                            }),
                                        )
                                }))
                            })
                    })),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(self.toolbar_button("btn-open", "Open", cx, |this, _, cx| {
                        if let Selection::Profile(id) = this.store.read(cx).selection {
                            cx.emit(SidebarEvent::OpenProfile(id));
                        }
                    }))
                    .child(self.toolbar_button("btn-rename", "Rename", cx, |this, _, cx| {
                        this.store.update(cx, |s, cx| s.begin_rename(cx));
                    }))
                    .child(self.toolbar_button("btn-dup", "Dup", cx, |this, _, cx| {
                        if let Selection::Profile(id) = this.store.read(cx).selection {
                            this.store.update(cx, |s, cx| {
                                s.duplicate_profile(id, cx);
                            });
                        }
                    }))
                    .child(self.toolbar_button("btn-del", "Delete", cx, |this, _, cx| {
                        this.store.update(cx, |s, cx| s.delete_selection(cx));
                    }))
                    .child(self.toolbar_button("btn-settings", "Settings", cx, |_, _, cx| {
                        cx.emit(SidebarEvent::OpenSettings);
                    }))
                    .children(other_groups.into_iter().take(3).map(|(gid, name)| {
                        let label = format!("→ {name}");
                        div()
                            .id(SharedString::from(format!("move-{gid}")))
                            .px_2()
                            .py_1()
                            .rounded(px(4.0))
                            .bg(theme::PANEL_BG)
                            .border_1()
                            .border_color(theme::BORDER)
                            .text_color(theme::TEXT_MUTED)
                            .text_xs()
                            .cursor_pointer()
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Selection::Profile(pid) = this.store.read(cx).selection {
                                    this.store.update(cx, |s, cx| {
                                        s.move_profile_to_group(pid, gid, cx);
                                    });
                                }
                            }))
                    })),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let renaming = this.store.read(cx).rename_buffer.is_some();
                if !renaming {
                    return;
                }
                let key = &event.keystroke.key;
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
            }))
    }
}

#[derive(Clone, Debug)]
pub enum SidebarEvent {
    OpenProfile(Uuid),
    ShowProfileMenu(Uuid),
    OpenSettings,
}

impl EventEmitter<SidebarEvent> for Sidebar {}
