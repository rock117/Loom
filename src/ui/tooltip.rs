//! Simple hover tooltip views for icon controls (Zed-style).

use gpui::prelude::*;
use gpui::*;

use crate::shared::theme;

/// Compact label shown after the GPUI hover delay.
pub struct Tooltip {
    text: SharedString,
    key: Option<SharedString>,
}

impl Tooltip {
    pub fn text(text: impl Into<SharedString>, cx: &mut App) -> AnyView {
        cx.new(|_| Self {
            text: text.into(),
            key: None,
        })
        .into()
    }

    /// Label plus muted shortcut, e.g. `Toggle Sidebar` · `Ctrl+B`.
    pub fn with_key(
        text: impl Into<SharedString>,
        key: impl Into<SharedString>,
        cx: &mut App,
    ) -> AnyView {
        cx.new(|_| Self {
            text: text.into(),
            key: Some(key.into()),
        })
        .into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.key.clone();
        div()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::BORDER)
            .shadow_md()
            .text_xs()
            .text_color(theme::TEXT)
            .child(self.text.clone())
            .when_some(key, |d, key| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child(key),
                )
            })
    }
}
