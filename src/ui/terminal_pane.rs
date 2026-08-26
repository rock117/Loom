use gpui::prelude::*;
use gpui::*;

use crate::model::ConnectionState;
use crate::shared::theme;
use crate::ui::tab_manager::TabManager;

pub struct TerminalPane {
    pub tabs: Entity<TabManager>,
    _observe_tabs: Subscription,
}

impl TerminalPane {
    pub fn new(tabs: Entity<TabManager>, cx: &mut Context<Self>) -> Self {
        let _observe_tabs = cx.observe(&tabs, |_this, _tabs, cx| cx.notify());
        Self {
            tabs,
            _observe_tabs,
        }
    }
}

impl Render for TerminalPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let active_id = manager.active;

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::BG)
            .child(match active_id.and_then(|id| manager.tabs.iter().find(|t| t.id == id)) {
                None => div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(theme::TEXT)
                            .child("No session open"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::TEXT_MUTED)
                            .child("Open a profile from the left, or press Ctrl+T"),
                    )
                    .into_any_element(),
                Some(tab) => {
                    let state = tab.state;
                    let msg = tab.status_message.clone();
                    let tab_id = tab.id;
                    let term = tab.terminal.clone();

                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .size_full()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_3()
                                .py_1()
                                .bg(theme::PANEL_BG)
                                .border_b_1()
                                .border_color(theme::BORDER)
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(format!("{} · {msg}", state.label())),
                                )
                                .when(
                                    matches!(
                                        state,
                                        ConnectionState::Disconnected
                                            | ConnectionState::Failed
                                    ),
                                    |d| {
                                        d.child(
                                            div()
                                                .id("reconnect-btn")
                                                .px_2()
                                                .py_1()
                                                .rounded(px(4.0))
                                                .bg(theme::ACCENT)
                                                .text_color(rgb(0xffffff))
                                                .text_xs()
                                                .cursor_pointer()
                                                .child("Reconnect")
                                                .on_click(cx.listener(move |_this, _, _window, cx| {
                                                    cx.emit(TerminalPaneEvent::Reconnect(tab_id));
                                                })),
                                        )
                                    },
                                ),
                        )
                        .child(match term {
                            Some(entity) => div()
                                .flex_1()
                                .size_full()
                                .child(entity)
                                .into_any_element(),
                            None => div()
                                .flex_1()
                                .p_4()
                                .text_color(theme::TEXT_MUTED)
                                .child(msg)
                                .into_any_element(),
                        })
                        .into_any_element()
                }
            })
    }
}

#[derive(Clone, Debug)]
pub enum TerminalPaneEvent {
    Reconnect(uuid::Uuid),
}

impl EventEmitter<TerminalPaneEvent> for TerminalPane {}
