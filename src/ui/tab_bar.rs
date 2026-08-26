use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::ConnectionState;
use crate::shared::theme;
use crate::ui::tab_manager::{TabManager, state_color};

pub struct TabBar {
    pub tabs: Entity<TabManager>,
    _observe_tabs: Subscription,
}

impl TabBar {
    pub fn new(tabs: Entity<TabManager>, cx: &mut Context<Self>) -> Self {
        let _observe_tabs = cx.observe(&tabs, |_this, _tabs, cx| cx.notify());
        Self {
            tabs,
            _observe_tabs,
        }
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let active = manager.active;
        let items: Vec<(Uuid, String, ConnectionState, bool)> = manager
            .tabs
            .iter()
            .map(|t| (t.id, t.title.clone(), t.state, Some(t.id) == active))
            .collect();

        div()
            .flex()
            .items_center()
            .h(px(36.0))
            .w_full()
            .bg(theme::PANEL_BG)
            .border_b_1()
            .border_color(theme::BORDER)
            .px_1()
            .gap_1()
            .children(items.into_iter().map(|(id, title, state, is_active)| {
                div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .when(is_active, |d| d.bg(theme::TAB_ACTIVE))
                    .hover(|s| s.bg(theme::HOVER))
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-select-{id}")))
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .child(div().size(px(8.0)).rounded_full().bg(state_color(state)))
                            .child(div().text_sm().text_color(theme::TEXT).child(title))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.tabs.update(cx, |m, cx| m.select_tab(id, window, cx));
                                cx.emit(TabBarEvent::Changed);
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-close-{id}")))
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .px_1()
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::BORDER).text_color(theme::TEXT))
                            .child("×")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.tabs.update(cx, |m, cx| m.close_tab(id, cx));
                                    cx.emit(TabBarEvent::Changed);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| {
                                // Swallow click so nothing else treats this as a tab select.
                                cx.stop_propagation();
                            })),
                    )
            }))
            .child(
                div()
                    .id("tab-new")
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .text_color(theme::TEXT_MUTED)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
                    .child("+")
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(TabBarEvent::NewTab);
                    })),
            )
    }
}

#[derive(Clone, Debug)]
pub enum TabBarEvent {
    NewTab,
    Changed,
}

impl EventEmitter<TabBarEvent> for TabBar {}
