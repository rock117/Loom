use gpui::prelude::*;
use gpui::*;

use crate::shared::theme;

/// Small clickable control used across sidebar / settings / tab chrome.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    label: SharedString,
    muted: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl IconButton {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            muted: false,
            on_click: Box::new(on_click),
        }
    }

    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = self.on_click;
        div()
            .id(self.id)
            .px_2()
            .py_1()
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::PANEL_BG)
            .border_1()
            .border_color(theme::BORDER)
            .text_color(if self.muted {
                theme::TEXT_MUTED
            } else {
                theme::TEXT
            })
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(theme::HOVER))
            .child(self.label)
            .on_click(move |event, window, cx| on_click(event, window, cx))
    }
}

/// Primary accent action button.
#[derive(IntoElement)]
pub struct AccentButton {
    id: ElementId,
    label: SharedString,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl AccentButton {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            on_click: Box::new(on_click),
        }
    }
}

impl RenderOnce for AccentButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = self.on_click;
        div()
            .id(self.id)
            .px_3()
            .py_1()
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::ACCENT)
            .text_color(rgb(0xffffff))
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.opacity(0.9))
            .child(self.label)
            .on_click(move |event, window, cx| on_click(event, window, cx))
    }
}
