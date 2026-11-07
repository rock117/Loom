//! Modal when an SSH host key no longer matches `known_hosts.json` (TOFU change).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::session::known_hosts::HostKeyMismatch;
use crate::shared::theme;

#[derive(Clone, Debug)]
pub enum HostKeyPromptEvent {
    Cancel,
    /// User accepted the new fingerprint; caller should persist then reconnect.
    Trust {
        pane_id: Uuid,
        mismatch: HostKeyMismatch,
    },
}

pub struct HostKeyPrompt {
    focus_handle: FocusHandle,
    pub pane_id: Uuid,
    pub mismatch: HostKeyMismatch,
}

impl HostKeyPrompt {
    pub fn new(pane_id: Uuid, mismatch: HostKeyMismatch, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            pane_id,
            mismatch,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn trust(&mut self, cx: &mut Context<Self>) {
        cx.emit(HostKeyPromptEvent::Trust {
            pane_id: self.pane_id,
            mismatch: self.mismatch.clone(),
        });
    }
}

impl Focusable for HostKeyPrompt {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HostKeyPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let endpoint = self.mismatch.endpoint();
        let expected = self.mismatch.expected.clone();
        let offered = self.mismatch.offered.clone();

        div()
            .id("ssh-host-key-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(HostKeyPromptEvent::Cancel)),
            )
            .child(
                div()
                    .id("ssh-host-key-card")
                    .w(px(460.0))
                    .p_5()
                    .rounded(px(8.0))
                    .bg(theme::PANEL_BG)
                    .border_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .track_focus(&self.focus_handle)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.focus_handle.focus(window);
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        let key = &event.keystroke.key;
                        if key == "escape" {
                            cx.emit(HostKeyPromptEvent::Cancel);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" {
                            this.trust(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::TEXT)
                            .child("Host key changed"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(format!(
                                "The key for {endpoint} does not match Loom’s known_hosts. \
                                 This is normal after a reinstall — or a sign of a MITM attack. \
                                 Compare fingerprints before trusting."
                            )),
                    )
                    .child(fingerprint_block("Previously trusted", &expected))
                    .child(fingerprint_block("Offered now", &offered))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_1()
                            .child(
                                div()
                                    .id("hk-cancel")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .text_sm()
                                    .text_color(theme::TEXT_MUTED)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::HOVER))
                                    .child("Cancel")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(HostKeyPromptEvent::Cancel);
                                    })),
                            )
                            .child(
                                div()
                                    .id("hk-trust")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ACCENT)
                                    .text_sm()
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.9))
                                    .child("Trust and reconnect")
                                    .on_click(cx.listener(|this, _, _, cx| this.trust(cx))),
                            ),
                    ),
            )
    }
}

fn fingerprint_block(label: &str, fp: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::TEXT_MUTED)
                .child(label.to_string()),
        )
        .child(
            div()
                .w_full()
                .px(px(theme::SPACE_2))
                .py(px(theme::SPACE_1))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::ELEVATED)
                .border_1()
                .border_color(theme::BORDER)
                .text_xs()
                .font_family("Consolas")
                .text_color(theme::TEXT)
                .child(fp.to_string()),
        )
}

impl EventEmitter<HostKeyPromptEvent> for HostKeyPrompt {}
