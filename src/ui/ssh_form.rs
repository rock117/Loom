//! Overlay to create / edit an SSH profile (password stored in OS keyring when remembered).

use std::path::PathBuf;

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
    Saved {
        profile_id: Uuid,
        connect: bool,
        /// Present when password was not saved to the keyring (one-shot connect).
        oneshot_password: Option<String>,
    },
}

pub struct SshForm {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    /// `None` = create new; `Some(id)` = edit existing SSH profile.
    editing: Option<Uuid>,
    name: String,
    host: String,
    port: String,
    user: String,
    password: String,
    key_path: String,
    use_private_key: bool,
    /// Keyring already has a password for this profile (edit mode).
    has_stored_password: bool,
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
    KeyPath,
}

impl SshForm {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        Self {
            store,
            focus_handle: cx.focus_handle(),
            editing: None,
            name: String::new(),
            host: String::new(),
            port: "22".into(),
            user: whoami_user(),
            password: String::new(),
            key_path: String::new(),
            use_private_key: false,
            has_stored_password: false,
            remember: true,
            error: None,
            field: Field::Host,
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.name.clear();
        self.host.clear();
        self.port = "22".into();
        self.user = whoami_user();
        self.password.clear();
        self.key_path.clear();
        self.use_private_key = false;
        self.has_stored_password = false;
        self.remember = true;
        self.error = None;
        self.field = Field::Host;
        cx.notify();
    }

    pub fn load_for_edit(&mut self, profile_id: Uuid, cx: &mut Context<Self>) {
        let profile = self
            .store
            .read(cx)
            .workspace
            .find_profile(profile_id)
            .cloned();
        let Some(profile) = profile else {
            self.error = Some("Profile not found".into());
            cx.notify();
            return;
        };
        let ProfileKind::Ssh {
            host,
            port,
            user,
            auth,
        } = profile.kind
        else {
            self.error = Some("Not an SSH profile".into());
            cx.notify();
            return;
        };

        self.editing = Some(profile_id);
        self.name = profile.name;
        self.host = host;
        self.port = port.to_string();
        self.user = user;
        self.password.clear();
        self.error = None;
        self.field = Field::Host;

        match auth {
            SshAuth::Password { remember } => {
                self.use_private_key = false;
                self.remember = remember;
                self.key_path.clear();
                self.has_stored_password = credentials::get_password(profile_id)
                    .ok()
                    .flatten()
                    .is_some();
            }
            SshAuth::PrivateKey { path } => {
                self.use_private_key = true;
                self.remember = false;
                self.key_path = path.display().to_string();
                self.has_stored_password = false;
            }
        }
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
        let target = self.active_field_mut();
        let mut any = false;
        for ch in typed.chars() {
            if !ch.is_control() {
                target.push(ch);
                any = true;
            }
        }
        any
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            Field::Name => &mut self.name,
            Field::Host => &mut self.host,
            Field::Port => &mut self.port,
            Field::User => &mut self.user,
            Field::Password => &mut self.password,
            Field::KeyPath => &mut self.key_path,
        }
    }

    fn active_field(&self) -> &str {
        match self.field {
            Field::Name => &self.name,
            Field::Host => &self.host,
            Field::Port => &self.port,
            Field::User => &self.user,
            Field::Password => &self.password,
            Field::KeyPath => &self.key_path,
        }
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let cleaned = text.replace('\r', "").replace('\n', "");
        if cleaned.is_empty() {
            return false;
        }
        self.active_field_mut().push_str(&cleaned);
        true
    }

    fn copy_field(&self, cx: &mut Context<Self>) -> bool {
        let text = self.active_field();
        if text.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        true
    }

    fn cut_field(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.copy_field(cx) {
            return false;
        }
        self.active_field_mut().clear();
        true
    }

    fn handle_clipboard_keys(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let chord = mods.control || mods.platform;
        if !chord {
            if mods.shift && key.eq_ignore_ascii_case("insert") {
                return self.paste_clipboard(cx);
            }
            return false;
        }
        if key.eq_ignore_ascii_case("v") {
            return self.paste_clipboard(cx);
        }
        if key.eq_ignore_ascii_case("c") {
            return self.copy_field(cx);
        }
        if key.eq_ignore_ascii_case("x") {
            return self.cut_field(cx);
        }
        false
    }

    fn cycle_field(&mut self) {
        self.field = if self.use_private_key {
            match self.field {
                Field::Name => Field::Host,
                Field::Host => Field::Port,
                Field::Port => Field::User,
                Field::User => Field::KeyPath,
                Field::KeyPath | Field::Password => Field::Name,
            }
        } else {
            match self.field {
                Field::Name => Field::Host,
                Field::Host => Field::Port,
                Field::Port => Field::User,
                Field::User => Field::Password,
                Field::Password | Field::KeyPath => Field::Name,
            }
        };
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

        let id = self.editing.unwrap_or_else(Uuid::new_v4);
        let creating = self.editing.is_none();

        let (auth, oneshot) = if self.use_private_key {
            let path = self.key_path.trim().to_string();
            if path.is_empty() {
                self.error = Some("Private key path is required".into());
                cx.notify();
                return;
            }
            let _ = credentials::delete_password(id);
            (
                SshAuth::PrivateKey {
                    path: PathBuf::from(path),
                },
                None,
            )
        } else {
            if self.password.is_empty() {
                if creating || !self.has_stored_password {
                    self.error = Some(
                        "Password is required (saved to OS keyring if Remember is on)".into(),
                    );
                    cx.notify();
                    return;
                }
            }

            if !self.password.is_empty() {
                if self.remember {
                    if let Err(err) = credentials::set_password(id, &self.password) {
                        self.error = Some(format!("Could not store password: {err:#}"));
                        cx.notify();
                        return;
                    }
                } else {
                    let _ = credentials::delete_password(id);
                }
            } else if !self.remember {
                let _ = credentials::delete_password(id);
            }

            let oneshot = if !self.password.is_empty() && !self.remember {
                Some(self.password.clone())
            } else {
                None
            };

            (
                SshAuth::Password {
                    remember: self.remember,
                },
                oneshot,
            )
        };

        let ok = if creating {
            let profile = Profile {
                id,
                name,
                kind: ProfileKind::Ssh {
                    host,
                    port,
                    user,
                    auth,
                },
            };
            let ok = self.store.update(cx, |s, cx| s.add_ssh_profile(profile, cx));
            if !ok && self.remember && !self.use_private_key {
                let _ = credentials::delete_password(id);
            }
            ok
        } else {
            self.store.update(cx, |s, cx| {
                s.update_ssh_profile(id, name, host, port, user, auth, cx)
            })
        };

        if !ok {
            self.error = Some(if creating {
                "No group available for the new profile".into()
            } else {
                "Could not update SSH profile".into()
            });
            cx.notify();
            return;
        }

        cx.emit(SshFormEvent::Saved {
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
        empty_hint: &str,
    ) -> impl IntoElement {
        let display = if secret && !value.is_empty() {
            "•".repeat(value.chars().count().min(24))
        } else if value.is_empty() {
            String::new()
        } else {
            value.to_string()
        };
        let caret = if active { "|" } else { "" };
        let show_hint = display.is_empty() && !active;
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
                    .text_color(if show_hint {
                        theme::TEXT_DISABLED
                    } else {
                        theme::TEXT
                    })
                    .child(if show_hint {
                        SharedString::from(empty_hint.to_string())
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
        let editing = self.editing.is_some();
        let title = if editing {
            "SSH profile settings"
        } else {
            "New SSH profile"
        };
        let pass_label = if editing && self.has_stored_password {
            "Password (blank = keep saved)"
        } else {
            "Password"
        };
        let pass_hint = if editing && self.has_stored_password {
            "••••••••"
        } else {
            "…"
        };

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
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, window, cx| {
                            this.focus_handle.focus(window);
                            if this.paste_clipboard(cx) {
                                cx.notify();
                            }
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
                        if this.handle_clipboard_keys(event, cx) {
                            cx.notify();
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" {
                            this.submit(true, cx);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "tab" {
                            this.cycle_field();
                            cx.notify();
                            cx.stop_propagation();
                            return;
                        }
                        if key == "backspace" {
                            this.active_field_mut().pop();
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
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child("Tab · Ctrl+C/V · Enter connects · Esc"),
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
                                "…",
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
                                "…",
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
                                        "…",
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
                                        "…",
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
                            .id("ssh-auth-mode")
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .child(if self.use_private_key {
                                        "Auth: private key  (click to use password)"
                                    } else {
                                        "Auth: password  (click to use private key)"
                                    }),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.use_private_key = !this.use_private_key;
                                this.field = if this.use_private_key {
                                    Field::KeyPath
                                } else {
                                    Field::Password
                                };
                                cx.notify();
                            })),
                    )
                    .when(!self.use_private_key, |d| {
                        d.child(
                            div()
                                .id("f-pass")
                                .cursor_pointer()
                                .child(self.field_row(
                                    "ssh-pass",
                                    pass_label,
                                    &self.password,
                                    self.field == Field::Password,
                                    true,
                                    pass_hint,
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
                    })
                    .when(self.use_private_key, |d| {
                        d.child(
                            div()
                                .id("f-key")
                                .cursor_pointer()
                                .child(self.field_row(
                                    "ssh-key",
                                    "Private key path",
                                    &self.key_path,
                                    self.field == Field::KeyPath,
                                    false,
                                    "C:\\Users\\…\\.ssh\\id_ed25519",
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.field = Field::KeyPath;
                                    this.focus_handle.focus(window);
                                    cx.notify();
                                })),
                        )
                    })
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
                                    .child(if editing {
                                        "Save & Connect"
                                    } else {
                                        "Save & Connect"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit(true, cx);
                                    })),
                            ),
                    ),
            )
    }
}

impl EventEmitter<SshFormEvent> for SshForm {}
