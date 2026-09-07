//! Overlay: pick an installed WSL distro → save as a Local profile (`wsl.exe -d …`).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::Profile;
use crate::session::wsl;
use crate::shared::theme;
use crate::ui::rename_edit::{RenameEdit, typed_text_from_keystroke};
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum WslFormEvent {
    Close,
    Saved {
        profile_id: Uuid,
        connect: bool,
    },
}

#[derive(Clone)]
enum ListState {
    Loading,
    Ready(Vec<String>),
    Error(String),
}

pub struct WslForm {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    list: ListState,
    selected: Option<String>,
    /// Profile display name (editable).
    name: RenameEdit,
    /// When true, changing distro no longer overwrites [`Self::name`].
    name_touched: bool,
    error: Option<String>,
    selecting: bool,
    name_bounds: Option<Bounds<Pixels>>,
    _caret_blink: Option<Task<()>>,
    _load_task: Option<Task<()>>,
}

impl WslForm {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let mut form = Self {
            store,
            focus_handle: cx.focus_handle(),
            list: ListState::Loading,
            selected: None,
            name: field_edit(""),
            name_touched: false,
            error: None,
            selecting: false,
            name_bounds: None,
            _caret_blink: None,
            _load_task: None,
        };
        form.refresh(cx);
        form
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.name = field_edit("");
        self.name_touched = false;
        self.error = None;
        self.selecting = false;
        self.refresh(cx);
        self.start_caret_blink(cx);
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        self.name.caret_visible = true;
        self._caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.name.caret_visible = !this.name.caret_visible;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.list = ListState::Loading;
        self.selected = None;
        if !self.name_touched {
            self.name = field_edit("");
        }
        self.error = None;
        cx.notify();

        let (tx, rx) = flume::bounded(1);
        let _ = std::thread::Builder::new()
            .name("loom-wsl-list".into())
            .spawn(move || {
                let result = wsl::list_distros();
                let _ = tx.send(result);
            });

        self._load_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.list = match result {
                    Ok(Ok(names)) if names.is_empty() => ListState::Error(
                        "No WSL distributions found. Install one from the Microsoft Store, then Refresh."
                            .into(),
                    ),
                    Ok(Ok(names)) => {
                        let first = names.first().cloned();
                        this.selected = first.clone();
                        if let Some(distro) = first {
                            this.apply_distro_default_name(&distro, cx);
                        }
                        ListState::Ready(names)
                    }
                    Ok(Err(err)) => ListState::Error(format!("{err:#}")),
                    Err(_) => ListState::Error("WSL list cancelled".into()),
                };
                this.start_caret_blink(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn apply_distro_default_name(&mut self, distro: &str, cx: &App) {
        if self.name_touched {
            return;
        }
        let suggested = self.unique_profile_name(distro, cx);
        self.name = field_edit(suggested);
    }

    fn select_distro(&mut self, distro: String, cx: &mut Context<Self>) {
        self.apply_distro_default_name(&distro, cx);
        self.selected = Some(distro);
        self.error = None;
        cx.notify();
    }

    fn unique_profile_name(&self, base: &str, cx: &App) -> String {
        let existing = self.store.read(cx).workspace.all_profile_names();
        if !existing.iter().any(|n| n == base) {
            return base.to_string();
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base} {n}");
            if !existing.iter().any(|e| e == &candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn resolve_save_name(&self, distro: &str, cx: &App) -> String {
        let typed = self.name.text.trim();
        let base = if typed.is_empty() { distro } else { typed };
        self.unique_profile_name(base, cx)
    }

    fn save(&mut self, connect: bool, cx: &mut Context<Self>) {
        let Some(distro) = self.selected.clone() else {
            self.error = Some("Select a distribution".into());
            cx.notify();
            return;
        };
        let name = self.resolve_save_name(&distro, cx);
        if name.is_empty() {
            self.error = Some("Name is required".into());
            cx.notify();
            return;
        }
        let profile = Profile::new_wsl(name, &distro);
        let id = self.store.update(cx, |s, cx| {
            let target = s.insert_target();
            s.place_profile(profile, target, cx)
        });
        cx.emit(WslFormEvent::Saved {
            profile_id: id,
            connect,
        });
    }

    fn handle_edit_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let chord = mods.control || mods.platform;
        let shift = mods.shift;

        if chord && key.eq_ignore_ascii_case("a") {
            self.name.select_all();
            self.name_touched = true;
            return true;
        }
        if chord && key.eq_ignore_ascii_case("c") {
            let text = if self.name.has_selection() {
                self.name.selected_text()
            } else {
                self.name.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return true;
        }
        if chord && key.eq_ignore_ascii_case("x") {
            let text = if self.name.has_selection() {
                self.name.selected_text()
            } else {
                self.name.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            if self.name.has_selection() {
                self.name.delete_selection();
            } else {
                self.name.text.clear();
                self.name.cursor = 0;
                self.name.anchor = 0;
            }
            self.name_touched = true;
            return true;
        }
        if (chord && key.eq_ignore_ascii_case("v"))
            || (mods.shift && key.eq_ignore_ascii_case("insert"))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                let cleaned = text.replace('\r', "").replace('\n', "");
                if !cleaned.is_empty() {
                    self.name.insert(&cleaned);
                    self.name_touched = true;
                }
            }
            return true;
        }
        if key == "backspace" {
            self.name.backspace();
            self.name_touched = true;
            return true;
        }
        if key == "delete" {
            self.name.delete_forward();
            self.name_touched = true;
            return true;
        }
        if key == "left" {
            self.name.move_left(shift);
            return true;
        }
        if key == "right" {
            self.name.move_right(shift);
            return true;
        }
        if key == "home" {
            self.name.move_home(shift);
            return true;
        }
        if key == "end" {
            self.name.move_end(shift);
            return true;
        }
        if chord {
            return false;
        }
        if let Some(cleaned) = typed_text_from_keystroke(&event.keystroke) {
            self.name.insert(&cleaned);
            self.name_touched = true;
            return true;
        }
        false
    }

    fn index_at_pointer(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.name_bounds else {
            return self.name.char_len();
        };
        let pad: f32 = theme::SPACE_2;
        let local_x: f32 = (position.x - bounds.origin.x).into();
        char_index_at_x(&self.name, local_x - pad)
    }

    fn begin_mouse_select(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let extend = event.modifiers.shift;
        if event.click_count >= 2 {
            self.name.select_all();
            self.selecting = false;
        } else {
            let idx = self.index_at_pointer(event.position);
            self.name.set_caret(idx, extend);
            self.selecting = true;
        }
        self.name.caret_visible = true;
        cx.notify();
    }

    fn update_mouse_select(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let idx = self.index_at_pointer(position);
        self.name.set_caret(idx, true);
        self.name.caret_visible = true;
        cx.notify();
    }

    fn end_mouse_select(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
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
}

impl Focusable for WslForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<WslFormEvent> for WslForm {}

impl EntityInputHandler for WslForm {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let utf16: Vec<u16> = self.name.text.encode_utf16().collect();
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
        let (lo, hi) = self.name.sel_range();
        let start = Self::char_to_utf16(&self.name.text, lo);
        let end = Self::char_to_utf16(&self.name.text, hi);
        Some(UTF16Selection {
            range: start..end,
            reversed: self.name.cursor < self.name.anchor,
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
            let lo = Self::utf16_to_char(&self.name.text, r.start);
            let hi = Self::utf16_to_char(&self.name.text, r.end);
            self.name.anchor = lo;
            self.name.cursor = hi;
        }
        let cleaned = text.replace('\r', "").replace('\n', "");
        if !cleaned.is_empty() || had_range {
            if cleaned.is_empty() {
                self.name.delete_selection();
            } else {
                self.name.insert(&cleaned);
            }
            self.name_touched = true;
            self.name.caret_visible = true;
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
        // Preedit stays with the OS IME; commit arrives via replace_text_in_range.
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.name_bounds
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (lo, _) = self.name.sel_range();
        Some(Self::char_to_utf16(&self.name.text, lo))
    }
}

impl Render for WslForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.clone();
        let selected = self.selected.clone();
        let can_save = matches!(&list, ListState::Ready(_)) && selected.is_some();
        let err = self.error.clone();
        let name_edit = self.name.clone();
        let view = cx.entity();

        div()
            .id("wsl-form-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(WslFormEvent::Close)),
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
                    .id("wsl-form-card")
                    .w(px(400.0))
                    .max_h(px(520.0))
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
                        let key = event.keystroke.key.as_str();
                        if key == "escape" {
                            cx.emit(WslFormEvent::Close);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter"
                            && matches!(&this.list, ListState::Ready(_))
                            && this.selected.is_some()
                        {
                            this.save(true, cx);
                            cx.stop_propagation();
                            return;
                        }
                        if this.handle_edit_key(event, cx) {
                            this.name.caret_visible = true;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::TEXT)
                                    .child("New WSL Profile"),
                            )
                            .child(
                                div()
                                    .id("wsl-refresh")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                                    .cursor_pointer()
                                    .child("Refresh")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.name_touched = false;
                                        this.refresh(cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(
                                "Pick a distro and set the profile name. Runs wsl.exe -d <distro>.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("Name"),
                            )
                            .child(
                                div()
                                    .id("wsl-name-field")
                                    .relative()
                                    .w_full()
                                    .px(px(theme::SPACE_2))
                                    .py(px(theme::SPACE_1))
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::ELEVATED)
                                    .border_1()
                                    .border_color(theme::ACCENT)
                                    .cursor_text()
                                    .overflow_hidden()
                                    .child({
                                        let view = view.clone();
                                        let view_paint = view.clone();
                                        canvas(
                                            move |bounds, _, cx| {
                                                view.update(cx, |this, _| {
                                                    this.name_bounds = Some(bounds);
                                                });
                                                bounds
                                            },
                                            move |bounds, _, window, cx| {
                                                let focus =
                                                    view_paint.read(cx).focus_handle.clone();
                                                window.handle_input(
                                                    &focus,
                                                    ElementInputHandler::new(
                                                        bounds,
                                                        view_paint.clone(),
                                                    ),
                                                    cx,
                                                );
                                            },
                                        )
                                        .absolute()
                                        .size_full()
                                    })
                                    .child(name_edit.into_element_bare())
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                            this.begin_mouse_select(event, window, cx);
                                            cx.stop_propagation();
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child("Distribution"),
                    )
                    .child(
                        div()
                            .id("wsl-distro-list")
                            .flex_1()
                            .min_h(px(140.0))
                            .max_h(px(240.0))
                            .overflow_y_scroll()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme::BORDER_SUBTLE)
                            .bg(theme::BG)
                            .p_1()
                            .child(match &list {
                                ListState::Loading => div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("Scanning installed distributions…")
                                    .into_any_element(),
                                ListState::Error(msg) => div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(theme::DANGER)
                                    .child(msg.clone())
                                    .into_any_element(),
                                ListState::Ready(names) => div()
                                    .flex()
                                    .flex_col()
                                    .gap_0()
                                    .children(names.iter().map(|name| {
                                        let name = name.clone();
                                        let is_sel = selected.as_ref() == Some(&name);
                                        let row_id = SharedString::from(format!("wsl-d-{name}"));
                                        div()
                                            .id(row_id)
                                            .px_3()
                                            .py_2()
                                            .rounded(px(4.0))
                                            .cursor_pointer()
                                            .bg(if is_sel {
                                                theme::SELECTION
                                            } else {
                                                hsla(0.0, 0.0, 0.0, 0.0)
                                            })
                                            .hover(|s| {
                                                if is_sel {
                                                    s
                                                } else {
                                                    s.bg(theme::HOVER)
                                                }
                                            })
                                            .text_sm()
                                            .text_color(theme::TEXT)
                                            .child(name.clone())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.select_distro(name.clone(), cx);
                                                cx.stop_propagation();
                                            }))
                                    }))
                                    .into_any_element(),
                            }),
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
                            .child(form_btn("Cancel", false, cx, |_, _, cx| {
                                cx.emit(WslFormEvent::Close);
                            }))
                            .child(form_btn("Save", false, cx, move |this, _, cx| {
                                if can_save {
                                    this.save(false, cx);
                                }
                            }))
                            .child(form_btn("Save & Open", true, cx, move |this, _, cx| {
                                if can_save {
                                    this.save(true, cx);
                                }
                            })),
                    ),
            )
    }
}

fn field_edit(text: impl Into<String>) -> RenameEdit {
    let mut edit = RenameEdit::new(text);
    edit.move_end(false);
    edit
}

fn char_index_at_x(edit: &RenameEdit, local_x: f32) -> usize {
    const AVG_CHAR_W: f32 = 7.4;
    if local_x <= 0.0 {
        return 0;
    }
    let idx = (local_x / AVG_CHAR_W).round() as usize;
    idx.min(edit.char_len())
}

fn form_btn(
    label: &'static str,
    primary: bool,
    cx: &mut Context<WslForm>,
    on_click: impl Fn(&mut WslForm, &mut Window, &mut Context<WslForm>) + 'static,
) -> impl IntoElement {
    let id = SharedString::from(format!("wsl-btn-{label}"));
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded(px(theme::RADIUS_SM))
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(theme::ACCENT)
                .text_color(rgb(0xffffff))
                .hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT)
                .hover(|s| s.bg(theme::HOVER))
        })
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            on_click(this, window, cx);
            cx.stop_propagation();
        }))
}
