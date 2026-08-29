//! Modal password prompt for SSH when no stored credential exists.

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::session::credentials;
use crate::shared::theme;

#[derive(Clone, Debug)]
pub enum PasswordPromptEvent {
    Cancel,
    Submit {
        profile_id: Uuid,
        password: String,
    },
}

pub struct PasswordPrompt {
    focus_handle: FocusHandle,
    pub profile_id: Uuid,
    pub title: String,
    password: String,
    remember: bool,
    error: Option<String>,
}

impl PasswordPrompt {
    pub fn new(profile_id: Uuid, title: impl Into<String>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            profile_id,
            title: title.into(),
            password: String::new(),
            remember: true,
            error: None,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.password.is_empty() {
            self.error = Some("Password required".into());
            cx.notify();
            return;
        }
        if self.remember {
            if let Err(err) = credentials::set_password(self.profile_id, &self.password) {
                self.error = Some(format!("Could not store password: {err:#}"));
                cx.notify();
                return;
            }
        }
        cx.emit(PasswordPromptEvent::Submit {
            profile_id: self.profile_id,
            password: self.password.clone(),
        });
    }
}

impl Focusable for PasswordPrompt {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PasswordPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let masked = if self.password.is_empty() {
            String::new()
        } else {
            "•".repeat(self.password.chars().count().min(32))
        };
        let err = self.error.clone();
        let title = self.title.clone();

        div()
            .id("ssh-password-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PasswordPromptEvent::Cancel)),
            )
            .child(
                div()
                    .id("ssh-password-card")
                    .w(px(360.0))
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
                            cx.emit(PasswordPromptEvent::Cancel);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" {
                            this.submit(cx);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "backspace" {
                            this.password.pop();
                            cx.notify();
                            cx.stop_propagation();
                        } else if !event.keystroke.modifiers.control
                            && !event.keystroke.modifiers.alt
                            && !event.keystroke.modifiers.platform
                        {
                            if let Some(typed) = event.keystroke.key_char.as_deref() {
                                let mut any = false;
                                for ch in typed.chars() {
                                    if !ch.is_control() {
                                        this.password.push(ch);
                                        any = true;
                                    }
                                }
                                if any {
                                    cx.notify();
                                    cx.stop_propagation();
                                }
                            }
                        }
                    }))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::TEXT)
                            .child("SSH password"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(title),
                    )
                    .child(
                        div()
                            .w_full()
                            .px(px(theme::SPACE_2))
                            .py(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS_SM))
                            .bg(theme::ELEVATED)
                            .border_1()
                            .border_color(theme::ACCENT)
                            .text_sm()
                            .text_color(theme::TEXT)
                            .child(format!("{masked}|")),
                    )
                    .child(
                        div()
                            .id("pw-remember")
                            .cursor_pointer()
                            .text_sm()
                            .text_color(theme::TEXT)
                            .child(if self.remember {
                                "[x] Remember password"
                            } else {
                                "[ ] Remember password"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remember = !this.remember;
                                cx.notify();
                            })),
                    )
                    .when_some(err, |d, msg| {
                        d.child(div().text_xs().text_color(theme::DANGER).child(msg))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("pw-cancel")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .text_sm()
                                    .text_color(theme::TEXT_MUTED)
                                    .cursor_pointer()
                                    .child("Cancel")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(PasswordPromptEvent::Cancel);
                                    })),
                            )
                            .child(
                                div()
                                    .id("pw-ok")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ACCENT)
                                    .text_sm()
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .child("Connect")
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            ),
                    ),
            )
    }
}

impl EventEmitter<PasswordPromptEvent> for PasswordPrompt {}
