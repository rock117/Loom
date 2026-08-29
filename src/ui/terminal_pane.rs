use gpui::prelude::*;
use gpui::*;

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
                    .flex_col()
                    .gap(px(theme::SPACE_2))
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
                    let msg = tab.status_message.clone();
                    match &tab.terminal {
                        Some(entity) => div()
                            .flex_1()
                            .size_full()
                            .min_h_0()
                            .child(entity.clone())
                            .into_any_element(),
                        None => div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p(px(theme::SPACE_4))
                            .text_sm()
                            .text_color(theme::TEXT_MUTED)
                            .child(msg)
                            .into_any_element(),
                    }
                }
            })
    }
}
