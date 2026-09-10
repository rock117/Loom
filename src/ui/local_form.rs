//! Overlay to edit a Local / WSL profile (name + start directory).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::ProfileKind;
use crate::shared::theme;
use crate::ui::rename_edit::{RenameEdit, typed_text_from_keystroke};
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum LocalFormEvent {
    Close,
    Saved { profile_id: Uuid },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Cwd,
}

pub struct LocalForm {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    editing: Option<Uuid>,
    name: RenameEdit,
    cwd: RenameEdit,
    /// True when profile is WSL (`wsl.exe` + args) — footnote for cwd semantics.
    is_wsl: bool,
    error: Option<String>,
    field: Field,
    selecting: bool,
    field_bounds: [Option<Bounds<Pixels>>; 2],
    _caret_blink: Option<Task<()>>,
}

impl LocalForm {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let mut form = Self {
            store,
            focus_handle: cx.focus_handle(),
            editing: None,
            name: field_edit(""),
            cwd: field_edit(""),
            is_wsl: false,
            error: None,
            field: Field::Name,
            selecting: false,
            field_bounds: [None; 2],
            _caret_blink: None,
        };
        form.start_caret_blink(cx);
        form
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
        let ProfileKind::Local { cwd, .. } = &profile.kind else {
            self.error = Some("Not a Local profile".into());
            cx.notify();
            return;
        };

        self.editing = Some(profile_id);
        self.name = field_edit(profile.name);
        self.cwd = field_edit(
            cwd.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        );
        self.is_wsl = profile.kind.is_wsl_local();
        self.error = None;
        self.field = Field::Name;
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
                if this
                    .update(cx, |this, cx| {
                        this.active_edit_mut().caret_visible =
                            !this.active_edit().caret_visible;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn active_edit_mut(&mut self) -> &mut RenameEdit {
        match self.field {
            Field::Name => &mut self.name,
            Field::Cwd => &mut self.cwd,
        }
    }

    fn active_edit(&self) -> &RenameEdit {
        match self.field {
            Field::Name => &self.name,
            Field::Cwd => &self.cwd,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(pid) = self.editing else {
            self.error = Some("Nothing to save".into());
            cx.notify();
            return;
        };
        let name = self.name.text.trim().to_string();
        if name.is_empty() {
            self.error = Some("Name is required".into());
            cx.notify();
            return;
        }
        let cwd_raw = self.cwd.text.trim();
        let cwd = if cwd_raw.is_empty() {
            None
        } else {
            Some(PathBuf::from(cwd_raw))
        };
        let ok = self.store.update(cx, |s, cx| {
            s.update_local_profile(pid, name, cwd, cx)
        });
        if !ok {
            self.error = Some("Could not update profile".into());
            cx.notify();
            return;
        }
        cx.emit(LocalFormEvent::Saved { profile_id: pid });
    }

    fn pick_cwd_folder(&mut self, cx: &mut Context<Self>) {
        let start = {
            let t = self.cwd.text.trim();
            if t.is_empty() {
                None
            } else {
                Some(PathBuf::from(t))
            }
        };
        cx.spawn(async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Start directory");
            if let Some(dir) = start.filter(|p| p.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog.pick_folder().await;
            let Some(handle) = picked else {
                return;
            };
            let path = handle.path().to_path_buf();
            this.update(cx, |this, cx| {
                this.cwd = field_edit(path.display().to_string());
                this.field = Field::Cwd;
                this.active_edit_mut().move_end(false);
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        if typed_text_from_keystroke(&event.keystroke).is_some() {
            return false;
        }
        false
    }
}

impl Focusable for LocalForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<LocalFormEvent> for LocalForm {}

impl EntityInputHandler for LocalForm {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = &self.active_edit().text;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let start = range.start.min(utf16.len());
        let end = range.end.min(utf16.len());
        *adjusted_range = Some(start..end);
        String::from_utf16(&utf16[start..end]).ok()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let edit = self.active_edit();
        let (lo, hi) = edit.sel_range();
        let start = char_to_utf16(&edit.text, lo);
        let end = char_to_utf16(&edit.text, hi);
        Some(UTF16Selection {
            range: start..end,
            reversed: edit.cursor < edit.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_range = range.is_some();
        if let Some(r) = range {
            let edit = self.active_edit_mut();
            let lo = utf16_to_char(&edit.text, r.start);
            let hi = utf16_to_char(&edit.text, r.end);
            edit.anchor = lo;
            edit.cursor = hi;
        }
        let cleaned = text.replace('\r', "").replace('\n', "");
        if !cleaned.is_empty() || had_range {
            let edit = self.active_edit_mut();
            if cleaned.is_empty() {
                edit.delete_selection();
            } else {
                edit.insert(&cleaned);
            }
            edit.caret_visible = true;
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let i = match self.field {
            Field::Name => 0,
            Field::Cwd => 1,
        };
        self.field_bounds[i]
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.active_edit();
        let (lo, _) = edit.sel_range();
        Some(char_to_utf16(&edit.text, lo))
    }
}

fn field_edit(s: impl Into<String>) -> RenameEdit {
    let mut e = RenameEdit::new(s);
    e.move_end(false);
    e
}

fn char_to_utf16(text: &str, char_idx: usize) -> usize {
    text.chars().take(char_idx).map(|c| c.len_utf16()).sum()
}

fn utf16_to_char(text: &str, utf16_idx: usize) -> usize {
    let mut u = 0;
    for (i, c) in text.chars().enumerate() {
        if u >= utf16_idx {
            return i;
        }
        u += c.len_utf16();
    }
    text.chars().count()
}

impl Render for LocalForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let name_active = self.field == Field::Name;
        let cwd_active = self.field == Field::Cwd;
        let name_edit = self.name.clone();
        let cwd_edit = self.cwd.clone();
        let is_wsl = self.is_wsl;
        let error = self.error.clone();

        div()
            .id("local-form-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(LocalFormEvent::Close)),
            )
            .child(
                div()
                    .id("local-form-card")
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
                            cx.emit(LocalFormEvent::Close);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" {
                            this.submit(cx);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "tab" {
                            this.field = match this.field {
                                Field::Name => Field::Cwd,
                                Field::Cwd => Field::Name,
                            };
                            this.active_edit_mut().select_all();
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
                            .child("Local profile"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(
                                "Start directory is used each time you open this profile. \
                                 cd in a session does not change it until you Save cwd to profile.",
                            ),
                    )
                    .child(Self::field_row(
                        "local-name",
                        "Name",
                        Field::Name,
                        &name_edit,
                        name_active,
                        0,
                        view.clone(),
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("Start directory"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(theme::SPACE_1))
                                    .child(
                                        div()
                                            .id("local-cwd-input")
                                            .relative()
                                            .flex_1()
                                            .min_w_0()
                                            .px(px(theme::SPACE_2))
                                            .py(px(theme::SPACE_1))
                                            .rounded(px(theme::RADIUS_SM))
                                            .bg(theme::ELEVATED)
                                            .border_1()
                                            .border_color(if cwd_active {
                                                theme::ACCENT
                                            } else {
                                                theme::BORDER
                                            })
                                            .text_sm()
                                            .overflow_hidden()
                                            .cursor_text()
                                            .child({
                                                let view = view.clone();
                                                let view_paint = view.clone();
                                                canvas(
                                                    move |bounds, _, cx| {
                                                        view.update(cx, |this, _| {
                                                            this.field_bounds[1] = Some(bounds);
                                                        });
                                                        bounds
                                                    },
                                                    move |bounds, _, window, cx| {
                                                        let form = view_paint.read(cx);
                                                        if form.field == Field::Cwd {
                                                            let focus =
                                                                form.focus_handle.clone();
                                                            window.handle_input(
                                                                &focus,
                                                                ElementInputHandler::new(
                                                                    bounds,
                                                                    view_paint.clone(),
                                                                ),
                                                                cx,
                                                            );
                                                        }
                                                    },
                                                )
                                                .absolute()
                                                .size_full()
                                            })
                                            .child(if cwd_active {
                                                cwd_edit.into_element_bare()
                                            } else if cwd_edit.text.is_empty() {
                                                div()
                                                    .text_sm()
                                                    .text_color(theme::TEXT_DISABLED)
                                                    .child("(default / home)")
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .text_sm()
                                                    .text_color(theme::TEXT)
                                                    .child(cwd_edit.text.clone())
                                                    .into_any_element()
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, window, cx| {
                                                    this.field = Field::Cwd;
                                                    this.focus_handle.focus(window);
                                                    this.cwd.move_end(false);
                                                    cx.notify();
                                                    cx.stop_propagation();
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("local-cwd-browse")
                                            .px(px(theme::SPACE_2))
                                            .py(px(theme::SPACE_1))
                                            .rounded(px(theme::RADIUS_SM))
                                            .border_1()
                                            .border_color(theme::BORDER)
                                            .text_xs()
                                            .text_color(theme::TEXT)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(theme::HOVER))
                                            .child("Browse…")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, _, cx| {
                                                    this.pick_cwd_folder(cx);
                                                    cx.stop_propagation();
                                                }),
                                            ),
                                    ),
                            )
                            .when(is_wsl, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(
                                            "WSL: Windows path for the wsl.exe process \
                                             (often maps to /mnt/… inside the distro).",
                                        ),
                                )
                            }),
                    )
                    .when_some(error, |d, msg| {
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
                            .child(
                                div()
                                    .id("local-cancel")
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_1))
                                    .rounded(px(theme::RADIUS_SM))
                                    .border_1()
                                    .border_color(theme::BORDER)
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .cursor_pointer()
                                    .child("Cancel")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|_, _, _, cx| {
                                            cx.emit(LocalFormEvent::Close);
                                            cx.stop_propagation();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("local-save")
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_1))
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ACCENT)
                                    .text_sm()
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .child("Save")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.submit(cx);
                                            cx.stop_propagation();
                                        }),
                                    ),
                            ),
                    ),
            )
    }
}

impl LocalForm {
    fn field_row(
        id: &'static str,
        label: &'static str,
        field: Field,
        edit: &RenameEdit,
        active: bool,
        fi: usize,
        view: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view_paint = view.clone();
        div()
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
                    .id(id)
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
                    .child({
                        let view = view.clone();
                        let view_paint = view_paint.clone();
                        canvas(
                            move |bounds, _, cx| {
                                view.update(cx, |this, _| {
                                    this.field_bounds[fi] = Some(bounds);
                                });
                                bounds
                            },
                            move |bounds, _, window, cx| {
                                let form = view_paint.read(cx);
                                if form.field == field {
                                    let focus = form.focus_handle.clone();
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, view_paint.clone()),
                                        cx,
                                    );
                                }
                            },
                        )
                        .absolute()
                        .size_full()
                    })
                    .child(if active {
                        edit.into_element_bare()
                    } else {
                        div()
                            .text_sm()
                            .text_color(theme::TEXT)
                            .child(edit.text.clone())
                            .into_any_element()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.field = field;
                            this.focus_handle.focus(window);
                            this.active_edit_mut().move_end(false);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}
