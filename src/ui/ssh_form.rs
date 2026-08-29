//! Overlay to create an SSH profile (password stored in OS keyring when remembered).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{Profile, ProfileKind, SshAuth};
use crate::session::credentials;
use crate::shared::theme;
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum SshFormEvent {
    Close,
    Created {
        profile_id: Uuid,
        connect: bool,
        /// Present when password was not saved to the keyring (one-shot connect).
        oneshot_password: Option<String>,
    },
}

pub struct SshForm {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    name: String,
    host: String,
    port: String,
    user: String,
    password: String,
    remember: bool,
    error: Option<String>,
    field: Field,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Host,
    Port,
    User,
    Password,
}

impl SshForm {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        Self {
            store,
            focus_handle: cx.focus_handle(),
            name: String::new(),
            host: String::new(),
            port: "22".into(),
            user: whoami_user(),
            password: String::new(),
            remember: true,
            error: None,
            field: Field::Host,
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.name.clear();
        self.host.clear();
        self.port = "22".into();
        self.user = whoami_user();
        self.password.clear();
        self.remember = true;
        self.error = None;
        self.field = Field::Host;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn append_char(&mut self, event: &KeyDownEvent) -> bool {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
        {
            return false;
        }
        let Some(typed) = event.keystroke.key_char.as_deref() else {
            return false;
        };
        let target = match self.field {
            Field::Name => &mut self.name,
            Field::Host => &mut self.host,
            Field::Port => &mut self.port,
            Field::User => &mut self.user,
            Field::Password => &mut self.password,
        };
        let mut any = false;
        for ch in typed.chars() {
            if !ch.is_control() {
                target.push(ch);
                any = true;
            }
        }
        any
    }

    fn submit(&mut self, connect: bool, cx: &mut Context<Self>) {
        self.error = None;
        let host = self.host.trim().to_string();
        if host.is_empty() {
            self.error = Some("Host is required".into());
            cx.notify();
            return;
        }
        let user = self.user.trim().to_string();
        if user.is_empty() {
            self.error = Some("User is required".into());
            cx.notify();
            return;
        }
        let port: u16 = match self.port.trim().parse() {
            Ok(0) | Err(_) => {
                self.error = Some("Port must be 1–65535".into());
                cx.notify();
                return;
            }
            Ok(p) => p,
        };
        let name = {
            let n = self.name.trim();
            if n.is_empty() {
                format!("{user}@{host}")
            } else {
                n.to_string()
            }
        };

        if self.password.is_empty() {
            self.error = Some("Password is required (saved to OS keyring if Remember is on)".into());
            cx.notify();
            return;
        }

        let profile = Profile {
            id: Uuid::new_v4(),
            name,
            kind: ProfileKind::Ssh {
                host,
                port,
                user,
                auth: SshAuth::Password {
                    remember: self.remember,
                },
            },
        };
        let id = profile.id;

        if self.remember {
            if let Err(err) = credentials::set_password(id, &self.password) {
                self.error = Some(format!("Could not store password: {err:#}"));
                cx.notify();
                return;
            }
        }

        let ok = self.store.update(cx, |s, cx| s.add_ssh_profile(profile, cx));
        if !ok {
            if self.remember {
                let _ = credentials::delete_password(id);
            }
            self.error = Some("No group available for the new profile".into());
            cx.notify();
            return;
        }

        let oneshot = if self.remember {
            None
        } else {
            Some(self.password.clone())
        };
        cx.emit(SshFormEvent::Created {
            profile_id: id,
            connect,
            oneshot_password: oneshot,
        });
    }

    fn field_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: &str,
        active: bool,
        secret: bool,
    ) -> impl IntoElement {
        let display = if secret && !value.is_empty() {
            "•".repeat(value.chars().count().min(24))
        } else if value.is_empty() {
            String::new()
        } else {
            value.to_string()
        };
        let caret = if active { "|" } else { "" };
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::TEXT_MUTED)
                    .child(label),
            )
            .child(
                div()
                    .w_full()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .rounded(px(theme::RADIUS_SM))
                    .bg(theme::ELEVATED)
                    .border_1()
                    .border_color(if active {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .text_sm()
                    .text_color(if display.is_empty() && !active {
                        theme::TEXT_DISABLED
                    } else {
                        theme::TEXT
                    })
                    .child(if display.is_empty() && !active {
                        SharedString::from("…")
                    } else {
                        SharedString::from(format!("{display}{caret}"))
                    }),
            )
    }
}

fn whoami_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".into())
}

impl Focusable for SshForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SshForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let err = self.error.clone();
        div()
            .id("ssh-form-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(SshFormEvent::Close)),
            )
            .child(
                div()
                    .id("ssh-form-card")
                    .w(px(420.0))
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
                            cx.emit(SshFormEvent::Close);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" {
                            this.submit(true, cx);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "tab" {
                            this.field = match this.field {
                                Field::Name => Field::Host,
                                Field::Host => Field::Port,
                                Field::Port => Field::User,
                                Field::User => Field::Password,
                                Field::Password => Field::Name,
                            };
                            cx.notify();
                            cx.stop_propagation();
                            return;
                        }
                        if key == "backspace" {
                            match this.field {
                                Field::Name => {
                                    this.name.pop();
                                }
                                Field::Host => {
                                    this.host.pop();
                                }
                                Field::Port => {
                                    this.port.pop();
                                }
                                Field::User => {
                                    this.user.pop();
                                }
                                Field::Password => {
                                    this.password.pop();
                                }
                            }
                            cx.notify();
                            cx.stop_propagation();
                        } else if this.append_char(event) {
                            cx.notify();
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::TEXT)
                            .child("New SSH profile"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child("Tab switches fields · Enter connects · Esc cancels"),
                    )
                    .child(
                        div()
                            .id("f-name")
                            .cursor_pointer()
                            .child(self.field_row(
                                "ssh-name",
                                "Name (optional)",
                                &self.name,
                                self.field == Field::Name,
                                false,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.field = Field::Name;
                                this.focus_handle.focus(window);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("f-host")
                            .cursor_pointer()
                            .child(self.field_row(
                                "ssh-host",
                                "Host",
                                &self.host,
                                self.field == Field::Host,
                                false,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.field = Field::Host;
                                this.focus_handle.focus(window);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("f-port")
                                    .flex_1()
                                    .cursor_pointer()
                                    .child(self.field_row(
                                        "ssh-port",
                                        "Port",
                                        &self.port,
                                        self.field == Field::Port,
                                        false,
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.field = Field::Port;
                                        this.focus_handle.focus(window);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("f-user")
                                    .flex_1()
                                    .cursor_pointer()
                                    .child(self.field_row(
                                        "ssh-user",
                                        "User",
                                        &self.user,
                                        self.field == Field::User,
                                        false,
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.field = Field::User;
                                        this.focus_handle.focus(window);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("f-pass")
                            .cursor_pointer()
                            .child(self.field_row(
                                "ssh-pass",
                                "Password",
                                &self.password,
                                self.field == Field::Password,
                                true,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.field = Field::Password;
                                this.focus_handle.focus(window);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("ssh-remember")
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .child(if self.remember {
                                        "[x] Remember password (OS keyring)"
                                    } else {
                                        "[ ] Remember password (OS keyring)"
                                    }),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remember = !this.remember;
                                cx.notify();
                            })),
                    )
                    .when_some(err, |d, msg| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::DANGER)
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_2()
                            .child(
                                div()
                                    .id("ssh-cancel")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .text_sm()
                                    .text_color(theme::TEXT_MUTED)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::HOVER))
                                    .child("Cancel")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(SshFormEvent::Close);
                                    })),
                            )
                            .child(
                                div()
                                    .id("ssh-save")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ELEVATED)
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::HOVER))
                                    .child("Save")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit(false, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("ssh-connect")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ACCENT)
                                    .text_sm()
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.9))
                                    .child("Save & Connect")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit(true, cx);
                                    })),
                            ),
                    ),
            )
    }
}

impl EventEmitter<SshFormEvent> for SshForm {}
