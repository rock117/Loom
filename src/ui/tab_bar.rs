use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::ConnectionState;
use crate::shared::theme;
use crate::ui::pane_layout::SplitDirection;
use crate::ui::tab_manager::{TabManager, state_color};
use crate::ui::tooltip::Tooltip;

const ICON: f32 = 14.0;
const SPLIT_BTN: f32 = 26.0;

pub struct TabBar {
    pub tabs: Entity<TabManager>,
    /// Zed-style split popover (open under the columns control).
    split_menu_open: bool,
    /// Window-space point for the menu's top-right corner (same pattern as sidebar).
    split_menu_anchor: Option<Point<Pixels>>,
    _observe_tabs: Subscription,
}

impl TabBar {
    pub fn new(tabs: Entity<TabManager>, cx: &mut Context<Self>) -> Self {
        let _observe_tabs = cx.observe(&tabs, |_this, _tabs, cx| cx.notify());
        Self {
            tabs,
            split_menu_open: false,
            split_menu_anchor: None,
            _observe_tabs,
        }
    }

    pub fn close_split_menu(&mut self, cx: &mut Context<Self>) {
        if self.split_menu_open {
            self.split_menu_open = false;
            self.split_menu_anchor = None;
            cx.notify();
        }
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
            .w(px(168.0))
            .p(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::BORDER)
            .shadow_md()
            .text_xs()
            .text_color(theme::TEXT)
    }

    fn menu_item(
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

    fn split_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.menu_shell()
            .child(self.menu_item("split-right", "Split Right", SplitDirection::Right, cx))
            .child(self.menu_item("split-left", "Split Left", SplitDirection::Left, cx))
            .child(self.menu_item("split-up", "Split Up", SplitDirection::Up, cx))
            .child(self.menu_item("split-down", "Split Down", SplitDirection::Down, cx))
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    this.close_split_menu(cx);
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
                            if this.tabs.read(cx).active.is_none() {
                                return;
                            }
                            if this.split_menu_open {
                                this.close_split_menu(cx);
                            } else {
                                // Window coords (same as sidebar). Menu top-right sits on
                                // the click — which is on the columns icon.
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
                        this.tabs.update(cx, |m, cx| m.toggle_zoom_focused(cx));
                        cx.emit(TabBarEvent::Changed);
                        cx.stop_propagation();
                    })),
            )
            // Sidebar-style anchored menu in window space + Zed deferred paint priority.
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
    }
}

#[derive(Clone, Debug)]
pub enum TabBarEvent {
    NewTab,
    Changed,
    Split(SplitDirection),
}

impl EventEmitter<TabBarEvent> for TabBar {}
