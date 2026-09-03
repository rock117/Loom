use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::ConnectionState;
use crate::shared::theme;
use crate::ui::pane_layout::SplitDirection;
use crate::ui::tab_manager::{TabManager, state_color};
use crate::ui::tooltip::Tooltip;
use crate::ui::workspace_store::WorkspaceStore;

const ICON: f32 = 14.0;
const SPLIT_BTN: f32 = 26.0;

struct TabContextMenu {
    tab_id: Uuid,
    position: Point<Pixels>,
}

pub struct TabBar {
    pub tabs: Entity<TabManager>,
    store: Entity<WorkspaceStore>,
    /// Zed-style split popover (open under the columns control).
    split_menu_open: bool,
    /// Window-space point for the menu's top-right corner (same pattern as sidebar).
    split_menu_anchor: Option<Point<Pixels>>,
    context_menu: Option<TabContextMenu>,
    _observe_tabs: Subscription,
    _observe_store: Subscription,
}

impl TabBar {
    pub fn new(
        tabs: Entity<TabManager>,
        store: Entity<WorkspaceStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _observe_tabs = cx.observe(&tabs, |_this, _tabs, cx| cx.notify());
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        Self {
            tabs,
            store,
            split_menu_open: false,
            split_menu_anchor: None,
            context_menu: None,
            _observe_tabs,
            _observe_store,
        }
    }

    pub fn close_split_menu(&mut self, cx: &mut Context<Self>) {
        if self.split_menu_open {
            self.split_menu_open = false;
            self.split_menu_anchor = None;
            cx.notify();
        }
    }

    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.is_some() {
            self.context_menu = None;
            cx.notify();
        }
    }

    /// Dismiss split popover and tab context menu (e.g. click outside).
    pub fn close_all_menus(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.split_menu_open {
            self.split_menu_open = false;
            self.split_menu_anchor = None;
            changed = true;
        }
        if self.context_menu.is_some() {
            self.context_menu = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// True when the split popover or tab context menu is open.
    pub fn has_open_menu(&self) -> bool {
        self.context_menu.is_some() || self.split_menu_open
    }

    fn svg_icon(path: &'static str, size: f32, color: Hsla) -> impl IntoElement {
        svg()
            .path(path)
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
    }

    fn menu_shell(&self) -> Div {
        div()
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(180.0))
            .p(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::BORDER)
            .shadow_md()
            .text_xs()
            .text_color(theme::TEXT)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    fn menu_item(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let label = label.into();
        div()
            .id(id.into())
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .when(enabled, |d| {
                d.text_color(theme::TEXT)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
            })
            .when(!enabled, |d| d.text_color(theme::TEXT_DISABLED))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                if !enabled {
                    return;
                }
                on_click(this, window, cx);
                this.context_menu = None;
                this.close_split_menu(cx);
                cx.notify();
                cx.stop_propagation();
            }))
    }

    fn menu_divider(&self) -> impl IntoElement {
        div()
            .w_full()
            .h(px(1.0))
            .my(px(theme::SPACE_1))
            .bg(theme::BORDER_SUBTLE)
    }

    fn split_menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.split_menu_open = false;
                this.split_menu_anchor = None;
                cx.emit(TabBarEvent::Split(direction));
                cx.stop_propagation();
                cx.notify();
            }))
    }

    fn tab_split_menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        tab_id: Uuid,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.context_menu = None;
                this.close_split_menu(cx);
                this.tabs.update(cx, |m, cx| m.select_tab(tab_id, window, cx));
                cx.emit(TabBarEvent::Split(direction));
                cx.stop_propagation();
                cx.notify();
            }))
    }

    fn split_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.menu_shell()
            .child(self.split_menu_item("split-right", "Split Right", SplitDirection::Right, cx))
            .child(self.split_menu_item("split-left", "Split Left", SplitDirection::Left, cx))
            .child(self.split_menu_item("split-up", "Split Up", SplitDirection::Up, cx))
            .child(self.split_menu_item("split-down", "Split Down", SplitDirection::Down, cx))
    }

    fn tab_context_menu(&self, tab_id: Uuid, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let tab_index = manager.tabs.iter().position(|t| t.id == tab_id);
        let tab_count = manager.tabs.len();
        let (has_left, has_right, has_others) = match tab_index {
            Some(i) => (i > 0, i + 1 < tab_count, tab_count > 1),
            None => (false, false, false),
        };
        let profile_id = manager.profile_id_for_tab(tab_id);
        let current_group = profile_id.and_then(|pid| {
            self.store
                .read(cx)
                .workspace
                .group_id_for_profile(pid)
        });
        let other_groups: Vec<(Uuid, String)> = self
            .store
            .read(cx)
            .workspace
            .walk_groups()
            .into_iter()
            .filter(|(id, _, _, _)| Some(*id) != current_group)
            .map(|(id, name, _, _)| (id, name))
            .collect();

        let mut menu = self
            .menu_shell()
            .child(self.menu_item("tab-ctx-close", "Close", true, cx, move |this, _, cx| {
                this.tabs.update(cx, |m, cx| m.close_tab(tab_id, cx));
                cx.emit(TabBarEvent::Changed);
            }))
            .child(self.menu_item(
                "tab-ctx-close-others",
                "Close Others",
                has_others,
                cx,
                move |this, _, cx| {
                    this.tabs
                        .update(cx, |m, cx| m.close_other_tabs(tab_id, cx));
                    cx.emit(TabBarEvent::Changed);
                },
            ))
            .child(self.menu_item(
                "tab-ctx-close-left",
                "Close to the Left",
                has_left,
                cx,
                move |this, _, cx| {
                    this.tabs
                        .update(cx, |m, cx| m.close_tabs_to_left(tab_id, cx));
                    cx.emit(TabBarEvent::Changed);
                },
            ))
            .child(self.menu_item(
                "tab-ctx-close-right",
                "Close to the Right",
                has_right,
                cx,
                move |this, _, cx| {
                    this.tabs
                        .update(cx, |m, cx| m.close_tabs_to_right(tab_id, cx));
                    cx.emit(TabBarEvent::Changed);
                },
            ))
            .child(self.menu_item(
                "tab-ctx-close-all",
                "Close All",
                tab_count > 0,
                cx,
                |this, _, cx| {
                    this.tabs.update(cx, |m, cx| m.close_all_tabs(cx));
                    cx.emit(TabBarEvent::Changed);
                },
            ))
            .child(self.menu_divider())
            .child(self.tab_split_menu_item(
                "tab-ctx-split-left",
                "Split Left",
                tab_id,
                SplitDirection::Left,
                cx,
            ))
            .child(self.tab_split_menu_item(
                "tab-ctx-split-right",
                "Split Right",
                tab_id,
                SplitDirection::Right,
                cx,
            ))
            .child(self.tab_split_menu_item(
                "tab-ctx-split-up",
                "Split Up",
                tab_id,
                SplitDirection::Up,
                cx,
            ))
            .child(self.tab_split_menu_item(
                "tab-ctx-split-down",
                "Split Down",
                tab_id,
                SplitDirection::Down,
                cx,
            ))
            .child(self.menu_divider())
            .child(self.menu_item(
                "tab-ctx-dup",
                "Duplicate",
                true,
                cx,
                move |_, _, cx| {
                    cx.emit(TabBarEvent::DuplicateProfile(Uuid::nil()));
                },
            ));

        if profile_id.is_none() {
            menu = menu.child(self.menu_divider()).child(self.menu_item(
                "tab-ctx-save-root",
                "Save to / (root)",
                true,
                cx,
                move |_, _, cx| {
                    cx.emit(TabBarEvent::SaveTab {
                        tab_id,
                        group_id: None,
                    });
                },
            ));
            for (gid, name, depth, _) in self.store.read(cx).workspace.walk_groups_all() {
                let indent = "  ".repeat(depth as usize);
                let label: SharedString = format!("{indent}Save to {name}").into();
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("tab-ctx-save-{gid}")))
                        .w_full()
                        .px(px(theme::SPACE_2))
                        .py(px(theme::SPACE_1))
                        .rounded(px(theme::RADIUS_SM))
                        .text_color(theme::TEXT)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::HOVER))
                        .child(label)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(TabBarEvent::SaveTab {
                                tab_id,
                                group_id: Some(gid),
                            });
                            cx.stop_propagation();
                        })),
                );
            }
        }

        if let Some(pid) = profile_id {
            menu = menu.child(self.menu_divider()).child(self.menu_item(
                "tab-ctx-save-root-move",
                "Move Profile → / (root)",
                current_group.is_some(),
                cx,
                move |this, _, cx| {
                    this.store.update(cx, |s, cx| {
                        s.move_profile_to_root(pid, cx);
                    });
                    this.context_menu = None;
                    cx.notify();
                },
            ));
            if !other_groups.is_empty() {
                for (gid, name) in other_groups {
                    let label: SharedString = format!("Move Profile → {name}").into();
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("tab-ctx-move-{gid}")))
                            .w_full()
                            .px(px(theme::SPACE_2))
                            .py(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS_SM))
                            .text_color(theme::TEXT)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::HOVER))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.move_profile_to_group(pid, gid, cx);
                                });
                                this.context_menu = None;
                                cx.notify();
                                cx.stop_propagation();
                            })),
                    );
                }
            }
        }

        menu
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let active = manager.active;
        let has_session = active.is_some();
        let can_zoom = manager.can_zoom_active();
        let is_zoomed = manager.active_zoomed();
        let items: Vec<(Uuid, String, ConnectionState, bool)> = manager
            .tabs
            .iter()
            .map(|t| (t.id, t.title.clone(), t.display_state(), Some(t.id) == active))
            .collect();
        let menu_open = self.split_menu_open;
        let menu_anchor = self.split_menu_anchor;
        let context_menu = self.context_menu.as_ref().map(|m| (m.tab_id, m.position));

        div()
            .relative()
            .flex()
            .items_center()
            .h(px(theme::TAB_BAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .bg(theme::PANEL_BG)
            .border_b_1()
            .border_color(theme::BORDER)
            .px(px(theme::SPACE_1))
            .gap(px(theme::SPACE_1))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_context_menu(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    this.close_split_menu(cx);
                    this.close_context_menu(cx);
                    cx.stop_propagation();
                }
            }))
            .children(items.into_iter().map(|(id, title, state, is_active)| {
                div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1))
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .rounded(px(theme::RADIUS_SM))
                    .when(is_active, |d| d.bg(theme::TAB_ACTIVE))
                    .hover(|s| s.bg(theme::HOVER))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.close_split_menu(cx);
                            this.context_menu = Some(TabContextMenu {
                                tab_id: id,
                                position: event.position,
                            });
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-select-{id}")))
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .cursor_pointer()
                            .child(div().size(px(8.0)).rounded_full().bg(state_color(state)))
                            .child(div().text_sm().text_color(theme::TEXT).child(title))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_split_menu(cx);
                                this.close_context_menu(cx);
                                this.tabs.update(cx, |m, cx| m.select_tab(id, window, cx));
                                cx.emit(TabBarEvent::Changed);
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-close-{id}")))
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .px(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS_SM))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::BORDER).text_color(theme::TEXT))
                            .tooltip(|_, cx| Tooltip::with_key("Close Tab", "Ctrl+W", cx))
                            .child("×")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.close_split_menu(cx);
                                    this.close_context_menu(cx);
                                    this.tabs.update(cx, |m, cx| m.close_tab(id, cx));
                                    cx.emit(TabBarEvent::Changed);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                            })),
                    )
            }))
            .child(
                div()
                    .id("tab-new")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(SPLIT_BTN))
                    .rounded(px(theme::RADIUS_SM))
                    .text_color(theme::TEXT_MUTED)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
                    .tooltip(|_, cx| Tooltip::with_key("New Tab", "Ctrl+T", cx))
                    .child(Self::svg_icon("icons/ui/plus.svg", ICON, theme::TEXT_MUTED))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_split_menu(cx);
                        this.close_context_menu(cx);
                        cx.emit(TabBarEvent::NewTab);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("tab-split")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(SPLIT_BTN))
                    .rounded(px(theme::RADIUS_SM))
                    .cursor_pointer()
                    .when(menu_open, |d| d.bg(theme::HOVER))
                    .when(has_session, |d| {
                        d.hover(|s| s.bg(theme::HOVER)).child(Self::svg_icon(
                            "icons/ui/columns.svg",
                            ICON,
                            if menu_open {
                                theme::TEXT
                            } else {
                                theme::TEXT_MUTED
                            },
                        ))
                    })
                    .when(!has_session, |d| {
                        d.child(Self::svg_icon(
                            "icons/ui/columns.svg",
                            ICON,
                            theme::TEXT_DISABLED,
                        ))
                    })
                    .tooltip(|_, cx| Tooltip::text("Split Pane", cx))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.close_context_menu(cx);
                            if this.tabs.read(cx).active.is_none() {
                                return;
                            }
                            if this.split_menu_open {
                                this.close_split_menu(cx);
                            } else {
                                this.split_menu_anchor = Some(point(
                                    event.position.x,
                                    px(theme::TAB_BAR_HEIGHT + 2.0),
                                ));
                                this.split_menu_open = true;
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.stop_propagation();
                    })),
            )
            .child(
                div()
                    .id("tab-zoom")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(SPLIT_BTN))
                    .rounded(px(theme::RADIUS_SM))
                    .cursor_pointer()
                    .when(is_zoomed, |d| d.bg(theme::HOVER))
                    .when(can_zoom, |d| {
                        d.hover(|s| s.bg(theme::HOVER)).child(Self::svg_icon(
                            if is_zoomed {
                                "icons/ui/minimize-2.svg"
                            } else {
                                "icons/ui/maximize-2.svg"
                            },
                            ICON,
                            if is_zoomed {
                                theme::TEXT
                            } else {
                                theme::TEXT_MUTED
                            },
                        ))
                    })
                    .when(!can_zoom, |d| {
                        d.child(Self::svg_icon(
                            "icons/ui/maximize-2.svg",
                            ICON,
                            theme::TEXT_DISABLED,
                        ))
                    })
                    .tooltip(move |_, cx| {
                        if is_zoomed {
                            Tooltip::text("Restore Pane Size", cx)
                        } else {
                            Tooltip::text("Maximize Pane", cx)
                        }
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.tabs.read(cx).can_zoom_active() {
                            return;
                        }
                        this.close_split_menu(cx);
                        this.close_context_menu(cx);
                        this.tabs.update(cx, |m, cx| m.toggle_zoom_focused(cx));
                        cx.emit(TabBarEvent::Changed);
                        cx.stop_propagation();
                    })),
            )
            .when_some(menu_anchor.filter(|_| menu_open), |d, anchor| {
                d.child(
                    deferred(
                        anchored()
                            .position(anchor)
                            .anchor(Corner::TopRight)
                            .snap_to_window_with_margin(Edges {
                                top: px(4.0),
                                right: px(4.0),
                                bottom: px(4.0),
                                left: px(4.0),
                            })
                            .child(div().occlude().child(self.split_popover(cx))),
                    )
                    .with_priority(1),
                )
            })
            .when_some(context_menu, |d, (tab_id, position)| {
                d.child(
                    deferred(
                        anchored()
                            .position(position)
                            .anchor(Corner::TopLeft)
                            .snap_to_window_with_margin(Edges {
                                top: px(4.0),
                                right: px(4.0),
                                bottom: px(4.0),
                                left: px(4.0),
                            })
                            .child(self.tab_context_menu(tab_id, cx)),
                    )
                    .with_priority(1),
                )
            })
    }
}

#[derive(Clone, Debug)]
pub enum TabBarEvent {
    NewTab,
    Changed,
    Split(SplitDirection),
    /// Duplicate focused tab as ephemeral session (id unused).
    DuplicateProfile(Uuid),
    /// Save ephemeral tab to workspace root (`group_id: None`) or a group.
    SaveTab {
        tab_id: Uuid,
        group_id: Option<Uuid>,
    },
}

impl EventEmitter<TabBarEvent> for TabBar {}
