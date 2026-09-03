//! Overlay to create / edit an SSH profile (password stored in OS keyring when remembered).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{Profile, ProfileKind, SshAuth};
use crate::session::credentials;
use crate::shared::theme;
use crate::ui::rename_edit::{RenameEdit, typed_text_from_keystroke};
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
    name: RenameEdit,
    host: RenameEdit,
    port: RenameEdit,
    user: RenameEdit,
    password: RenameEdit,
    key_path: RenameEdit,
    use_private_key: bool,
    /// Keyring already has a password for this profile (edit mode).
    has_stored_password: bool,
    remember: bool,
    error: Option<String>,
    field: Field,
    /// Mouse-drag text selection in the active field.
    selecting: bool,
    field_bounds: [Option<Bounds<Pixels>>; 6],
    _caret_blink: Option<Task<()>>,
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
        let mut form = Self {
            store,
            focus_handle: cx.focus_handle(),
            editing: None,
            name: field_edit(""),
            host: field_edit(""),
            port: field_edit("22"),
            user: field_edit(whoami_user()),
            password: field_edit(""),
            key_path: field_edit(""),
            use_private_key: false,
            has_stored_password: false,
            remember: true,
            error: None,
            field: Field::Host,
            selecting: false,
            field_bounds: [None; 6],
            _caret_blink: None,
        };
        form.field = Field::Host;
        form.start_caret_blink(cx);
        form
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.name = field_edit("");
        self.host = field_edit("");
        self.port = field_edit("22");
        self.user = field_edit(whoami_user());
        self.password = field_edit("");
        self.key_path = field_edit("");
        self.use_private_key = false;
        self.has_stored_password = false;
        self.remember = true;
        self.error = None;
        self.field = Field::Host;
        self.selecting = false;
        self.start_caret_blink(cx);
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
        self.name = field_edit(profile.name);
        self.host = field_edit(host);
        self.port = field_edit(port.to_string());
        self.user = field_edit(user);
        self.password = field_edit("");
        self.error = None;
        match auth {
            SshAuth::Password { remember } => {
                self.use_private_key = false;
                self.remember = remember;
                self.key_path = field_edit("");
                self.has_stored_password = credentials::get_password(profile_id)
                    .ok()
                    .flatten()
                    .is_some();
            }
            SshAuth::PrivateKey { path } => {
                self.use_private_key = true;
                self.remember = false;
                self.key_path = field_edit(path.display().to_string());
                self.has_stored_password = false;
            }
        }
        self.field = Field::Host;
        self.selecting = false;
        self.start_caret_blink(cx);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        self._caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        let edit = this.active_edit_mut();
                        edit.caret_visible = !edit.caret_visible;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    fn focus_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        if self.field != field {
            self.field = field;
            // Caret at end; do not select — user selects explicitly.
            self.active_edit_mut().move_end(false);
        } else {
            self.active_edit_mut().caret_visible = true;
        }
        self.selecting = false;
        self.start_caret_blink(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn begin_mouse_select(
        &mut self,
        field: Field,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.field = field;
        self.focus_handle.focus(window);
        let extend = event.modifiers.shift;
        if event.click_count >= 2 {
            self.active_edit_mut().select_all();
            self.selecting = false;
        } else {
            let idx = self.index_at_pointer(field, event.position);
            self.active_edit_mut().set_caret(idx, extend);
            self.selecting = true;
        }
        self.start_caret_blink(cx);
        cx.notify();
    }

    fn update_mouse_select(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let field = self.field;
        let idx = self.index_at_pointer(field, position);
        self.active_edit_mut().set_caret(idx, true);
        self.active_edit_mut().caret_visible = true;
        cx.notify();
    }

    fn end_mouse_select(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
    }

    fn index_at_pointer(&self, field: Field, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.field_bounds[field_idx(field)] else {
            return self.edit_for(field).char_len();
        };
        let pad: f32 = theme::SPACE_2;
        let local_x: f32 = (position.x - bounds.origin.x).into();
        char_index_at_x(self.edit_for(field), local_x - pad)
    }

    fn edit_for(&self, field: Field) -> &RenameEdit {
        match field {
            Field::Name => &self.name,
            Field::Host => &self.host,
            Field::Port => &self.port,
            Field::User => &self.user,
            Field::Password => &self.password,
            Field::KeyPath => &self.key_path,
        }
    }

    fn active_edit_mut(&mut self) -> &mut RenameEdit {
        match self.field {
            Field::Name => &mut self.name,
            Field::Host => &mut self.host,
            Field::Port => &mut self.port,
            Field::User => &mut self.user,
            Field::Password => &mut self.password,
            Field::KeyPath => &mut self.key_path,
        }
    }

    fn active_edit(&self) -> &RenameEdit {
        match self.field {
            Field::Name => &self.name,
            Field::Host => &self.host,
            Field::Port => &self.port,
            Field::User => &self.user,
            Field::Password => &self.password,
            Field::KeyPath => &self.key_path,
        }
    }

    fn handle_edit_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let chord = mods.control || mods.platform;
        let shift = mods.shift;

        if chord && key.eq_ignore_ascii_case("a") {
            self.active_edit_mut().select_all();
            return true;
        }
        if chord && key.eq_ignore_ascii_case("c") {
            let edit = self.active_edit();
            let text = if edit.has_selection() {
                edit.selected_text()
            } else {
                edit.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return true;
        }
        if chord && key.eq_ignore_ascii_case("x") {
            let text = {
                let edit = self.active_edit();
                if edit.has_selection() {
                    edit.selected_text()
                } else {
                    edit.text.clone()
                }
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            let edit = self.active_edit_mut();
            if edit.has_selection() {
                edit.delete_selection();
            } else {
                edit.text.clear();
                edit.cursor = 0;
                edit.anchor = 0;
            }
            return true;
        }
        if (chord && key.eq_ignore_ascii_case("v"))
            || (mods.shift && key.eq_ignore_ascii_case("insert"))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                let cleaned = text.replace('\r', "").replace('\n', "");
                if !cleaned.is_empty() {
                    self.active_edit_mut().insert(&cleaned);
                }
            }
            return true;
        }
        if key == "backspace" {
            self.active_edit_mut().backspace();
            return true;
        }
        if key == "delete" {
            self.active_edit_mut().delete_forward();
            return true;
        }
        if key == "left" {
            self.active_edit_mut().move_left(shift);
            return true;
        }
        if key == "right" {
            self.active_edit_mut().move_right(shift);
            return true;
        }
        if key == "home" {
            self.active_edit_mut().move_home(shift);
            return true;
        }
        if key == "end" {
            self.active_edit_mut().move_end(shift);
            return true;
        }
        if chord {
            return false;
        }
        if let Some(cleaned) = typed_text_from_keystroke(&event.keystroke) {
            self.active_edit_mut().insert(&cleaned);
            return true;
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
        self.active_edit_mut().move_end(false);
    }

    fn submit(&mut self, connect: bool, cx: &mut Context<Self>) {
        self.error = None;
        let host = self.host.text.trim().to_string();
        if host.is_empty() {
            self.error = Some("Host is required".into());
            cx.notify();
            return;
        }
        let user = self.user.text.trim().to_string();
        if user.is_empty() {
            self.error = Some("User is required".into());
            cx.notify();
            return;
        }
        let port: u16 = match self.port.text.trim().parse() {
            Ok(0) | Err(_) => {
                self.error = Some("Port must be 1–65535".into());
                cx.notify();
                return;
            }
            Ok(p) => p,
        };
        let name = {
            let n = self.name.text.trim();
            if n.is_empty() {
                format!("{user}@{host}")
            } else {
                n.to_string()
            }
        };

        let creating = self.editing.is_none();
        let id = self.editing.unwrap_or_else(Uuid::new_v4);

        let (auth, oneshot) = if self.use_private_key {
            let path = self.key_path.text.trim().to_string();
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
            if self.password.text.is_empty() {
                if creating || !self.has_stored_password {
                    self.error = Some(
                        "Password is required (saved to OS keyring if Remember is on)".into(),
                    );
                    cx.notify();
                    return;
                }
            }

            if !self.password.text.is_empty() {
                if self.remember {
                    if let Err(err) = credentials::set_password(id, &self.password.text) {
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

            let oneshot = if !self.password.text.is_empty() && !self.remember {
                Some(self.password.text.clone())
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
        field: Field,
        edit: &RenameEdit,
        active: bool,
        secret: bool,
        empty_hint: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let show_hint = edit.text.is_empty() && !active;
        let view = cx.entity();
        let fi = field_idx(field);
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
                    .id(SharedString::from(format!("{id}-input")))
                    .relative()
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
                    .overflow_hidden()
                    .cursor_text()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                view.update(cx, |this, _| {
                                    this.field_bounds[fi] = Some(bounds);
                                });
                                bounds
                            },
                            |_bounds, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .when(active && !secret, |d| d.child(edit.into_element_bare()))
                    .when(active && secret, |d| d.child(edit.into_element_bare_masked()))
                    .when(!active, |d| {
                        d.text_color(if show_hint {
                            theme::TEXT_DISABLED
                        } else {
                            theme::TEXT
                        })
                        .child(if show_hint {
                            empty_hint.to_string()
                        } else if secret {
                            "•".repeat(edit.char_len().min(24))
                        } else {
                            edit.text.clone()
                        })
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.begin_mouse_select(field, event, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}

fn whoami_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".into())
}

/// Caret at end, no selection (unlike [`RenameEdit::new`] which selects all).
fn field_edit(text: impl Into<String>) -> RenameEdit {
    let mut edit = RenameEdit::new(text);
    edit.move_end(false);
    edit
}

fn field_idx(field: Field) -> usize {
    match field {
        Field::Name => 0,
        Field::Host => 1,
        Field::Port => 2,
        Field::User => 3,
        Field::Password => 4,
        Field::KeyPath => 5,
    }
}

/// Approximate hit-test for proportional UI text (good enough for form fields).
fn char_index_at_x(edit: &RenameEdit, local_x: f32) -> usize {
    const AVG_CHAR_W: f32 = 7.4;
    if local_x <= 0.0 {
        return 0;
    }
    let idx = (local_x / AVG_CHAR_W).round() as usize;
    idx.min(edit.char_len())
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
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.update_mouse_select(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.end_mouse_select(cx);
                }),
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
                            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                                let cleaned = text.replace('\r', "").replace('\n', "");
                                if !cleaned.is_empty() {
                                    this.active_edit_mut().insert(&cleaned);
                                    cx.notify();
                                }
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.update_mouse_select(event.position, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.end_mouse_select(cx);
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
                            this.cycle_field();
                            this.start_caret_blink(cx);
                            cx.notify();
                            cx.stop_propagation();
                            return;
                        }
                        if this.handle_edit_key(event, cx) {
                            this.active_edit_mut().caret_visible = true;
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
                            .child("Tab · Ctrl+A/C/V/X · arrows · Enter connects · Esc"),
                    )
                    .child(self.field_row(
                        "ssh-name",
                        "Name (optional)",
                        Field::Name,
                        &self.name,
                        self.field == Field::Name,
                        false,
                        "…",
                        cx,
                    ))
                    .child(self.field_row(
                        "ssh-host",
                        "Host",
                        Field::Host,
                        &self.host,
                        self.field == Field::Host,
                        false,
                        "…",
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("f-port")
                                    .flex_1()
                                    .child(self.field_row(
                                        "ssh-port",
                                        "Port",
                                        Field::Port,
                                        &self.port,
                                        self.field == Field::Port,
                                        false,
                                        "…",
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .id("f-user")
                                    .flex_1()
                                    .child(self.field_row(
                                        "ssh-user",
                                        "User",
                                        Field::User,
                                        &self.user,
                                        self.field == Field::User,
                                        false,
                                        "…",
                                        cx,
                                    )),
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.use_private_key = !this.use_private_key;
                                let field = if this.use_private_key {
                                    Field::KeyPath
                                } else {
                                    Field::Password
                                };
                                this.focus_field(field, window, cx);
                            })),
                    )
                    .when(!self.use_private_key, |d| {
                        d.child(self.field_row(
                            "ssh-pass",
                            pass_label,
                            Field::Password,
                            &self.password,
                            self.field == Field::Password,
                            true,
                            pass_hint,
                            cx,
                        ))
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
                        d.child(self.field_row(
                            "ssh-key",
                            "Private key path",
                            Field::KeyPath,
                            &self.key_path,
                            self.field == Field::KeyPath,
                            false,
                            "C:\\Users\\…\\.ssh\\id_ed25519",
                            cx,
                        ))
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
