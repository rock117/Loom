//! Terminal scrollback / viewport text search (Ctrl+F).

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use gpui::prelude::*;
use gpui::*;

use super::TerminalView;
use crate::shared::theme;
use crate::ui::rename_edit::{RenameEdit, typed_text_from_keystroke};

/// Approximate monospace width for find-query hit-testing (`text_xs`).
const FIND_CHAR_W: f32 = 7.0;

/// In-view find bar state (owned by [`TerminalView`]).
pub struct FindState {
    pub query: RenameEdit,
    pub status: SharedString,
    pub focus_handle: FocusHandle,
    /// Mouse-drag selection in the query field.
    selecting: bool,
    pub(super) query_bounds: Option<Bounds<Pixels>>,
}

impl FindState {
    pub fn new(focus_handle: FocusHandle) -> Self {
        let mut query = RenameEdit::new("");
        query.clear_selection();
        Self {
            query,
            status: SharedString::default(),
            focus_handle,
            selecting: false,
            query_bounds: None,
        }
    }
}

/// Escape regex metacharacters so user input is matched literally.
pub fn escape_literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for c in query.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '#'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn collect_matches<T>(term: &alacritty_terminal::Term<T>, regex: &mut RegexSearch) -> Vec<Match> {
    let history = term.history_size() as i32;
    let start = AlacPoint::new(Line(-history), Column(0));
    let last_line = term.screen_lines() as i32 - 1;
    let last_col = term.columns().saturating_sub(1);
    let end = AlacPoint::new(Line(last_line), Column(last_col));
    RegexIter::new(start, end, Direction::Right, term, regex).collect()
}

impl TerminalView {
    pub fn is_find_open(&self) -> bool {
        self.find.is_some()
    }

    fn start_find_caret_blink(&mut self, cx: &mut Context<Self>) {
        self._find_caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if let Some(find) = this.find.as_mut() {
                            find.query.caret_visible = !find.query.caret_visible;
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_closed = self.find.is_none();
        if was_closed {
            self.find = Some(FindState::new(cx.focus_handle()));
            self.start_find_caret_blink(cx);
        }
        if let Some(find) = self.find.as_mut() {
            if !find.query.text.is_empty() {
                find.query.select_all();
            } else {
                find.query.caret_visible = true;
            }
            find.selecting = false;
            find.focus_handle.clone().focus(window);
        }
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        self._find_caret_blink = None;
        self.focus_handle.focus(window);
        cx.notify();
    }

    pub fn find_next(&mut self, cx: &mut Context<Self>) {
        self.run_find(Direction::Right, cx);
    }

    pub fn find_prev(&mut self, cx: &mut Context<Self>) {
        self.run_find(Direction::Left, cx);
    }

    fn run_find(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.query.text.is_empty() {
            find.status = "".into();
            self.state.with_term_mut(|term| term.selection = None);
            cx.notify();
            return;
        }

        let pattern = escape_literal(&find.query.text);
        let mut regex = match RegexSearch::new(&pattern) {
            Ok(r) => r,
            Err(_) => {
                find.status = "Invalid".into();
                cx.notify();
                return;
            }
        };

        let result = self.state.with_term_mut(|term| {
            let matches = collect_matches(term, &mut regex);
            if matches.is_empty() {
                term.selection = None;
                return (0usize, 0usize);
            }

            let current = term
                .selection
                .as_ref()
                .and_then(|s| s.to_range(term))
                .and_then(|range| {
                    matches
                        .iter()
                        .position(|m| *m.start() == range.start && *m.end() == range.end)
                });

            let idx = match direction {
                Direction::Right => current.map(|i| (i + 1) % matches.len()).unwrap_or(0),
                Direction::Left => current
                    .map(|i| {
                        if i == 0 {
                            matches.len() - 1
                        } else {
                            i - 1
                        }
                    })
                    .unwrap_or(matches.len() - 1),
            };

            let m = &matches[idx];
            let start = *m.start();
            let end = *m.end();
            let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
            sel.update(end, Side::Right);
            sel.include_all();
            term.selection = Some(sel);
            term.scroll_to_point(start);
            (idx + 1, matches.len())
        });

        if let Some(find) = self.find.as_mut() {
            find.status = if result.1 == 0 {
                "0/0".into()
            } else {
                format!("{}/{}", result.0, result.1).into()
            };
        }
        cx.notify();
    }

    fn find_char_index_at(&self, position: gpui::Point<Pixels>) -> usize {
        let Some(find) = self.find.as_ref() else {
            return 0;
        };
        let Some(bounds) = find.query_bounds else {
            return find.query.char_len();
        };
        let local_x = f32::from(position.x - bounds.origin.x - px(theme::SPACE_2));
        find.query.char_index_at_x(local_x, FIND_CHAR_W)
    }

    fn on_find_query_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        find.focus_handle.clone().focus(window);
        let idx = self.find_char_index_at(event.position);
        if let Some(find) = self.find.as_mut() {
            let extend = event.modifiers.shift;
            if extend {
                find.query.set_caret(idx, true);
                find.selecting = false;
            } else {
                find.query.set_caret(idx, false);
                find.selecting = true;
            }
            find.query.caret_visible = true;
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn on_find_query_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        if !find.selecting {
            return;
        }
        let idx = self.find_char_index_at(event.position);
        if let Some(find) = self.find.as_mut() {
            find.query.set_caret(idx, true);
            find.query.caret_visible = true;
        }
        cx.notify();
    }

    fn on_find_query_mouse_up(&mut self, cx: &mut Context<Self>) {
        if let Some(find) = self.find.as_mut() {
            find.selecting = false;
        }
        cx.notify();
    }

    pub(crate) fn render_find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let status = find.status.clone();
        let focus = find.focus_handle.clone();
        let empty_query = find.query.text.is_empty();
        let query_el = find.query.into_element_bare();

        Some(
            div()
                .id("terminal-find-bar")
                .absolute()
                .top_0()
                .right_0()
                .m(px(theme::SPACE_2))
                .flex()
                .items_center()
                .gap(px(theme::SPACE_1))
                .h(px(32.0))
                .px(px(theme::SPACE_2))
                .rounded(px(theme::RADIUS))
                .bg(theme::PANEL_BG)
                .border_1()
                .border_color(theme::BORDER)
                .shadow_md()
                .occlude()
                .track_focus(&focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_find_key_down(event, window, cx);
                }))
                .child(
                    div()
                        .id("find-query")
                        .relative()
                        .flex()
                        .items_center()
                        .w(px(220.0))
                        .h(px(22.0))
                        .px(px(theme::SPACE_2))
                        .rounded(px(theme::RADIUS_SM))
                        .bg(theme::BG)
                        .border_1()
                        .border_color(theme::ACCENT)
                        .text_xs()
                        .text_color(theme::TEXT)
                        .overflow_hidden()
                        .cursor_text()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                this.on_find_query_mouse_down(event, window, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            this.on_find_query_mouse_move(event, cx);
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.on_find_query_mouse_up(cx);
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.on_find_query_mouse_up(cx);
                            }),
                        )
                        .child({
                            let view = cx.entity();
                            canvas(
                                move |bounds, _, cx| {
                                    view.update(cx, |this, _| {
                                        if let Some(find) = this.find.as_mut() {
                                            find.query_bounds = Some(bounds);
                                        }
                                    });
                                    bounds
                                },
                                |_bounds, _, _, _| {},
                            )
                            .absolute()
                            .size_full()
                        })
                        .when(empty_query, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(px(theme::SPACE_2))
                                    .right(px(theme::SPACE_2))
                                    .text_color(theme::TEXT_DISABLED)
                                    .child("Search…"),
                            )
                        })
                        .child(query_el),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if status.is_empty() {
                            theme::TEXT_DISABLED
                        } else if status.as_ref() == "0/0" {
                            theme::DANGER
                        } else {
                            theme::TEXT_MUTED
                        })
                        .min_w(px(44.0))
                        .px(px(theme::SPACE_1))
                        .text_center()
                        .child(if status.is_empty() {
                            SharedString::from("—")
                        } else {
                            status
                        }),
                )
                .child(
                    div()
                        .w(px(1.0))
                        .h(px(16.0))
                        .bg(theme::BORDER)
                        .mx(px(theme::SPACE_1)),
                )
                .child(find_btn(
                    "find-prev",
                    "↑",
                    "Previous Match",
                    cx.listener(|this, _, _, cx| this.find_prev(cx)),
                ))
                .child(find_btn(
                    "find-next",
                    "↓",
                    "Next Match",
                    cx.listener(|this, _, _, cx| this.find_next(cx)),
                ))
                .child(find_btn(
                    "find-close",
                    "×",
                    "Close Find",
                    cx.listener(|this, _, window, cx| this.close_find(window, cx)),
                ))
                .into_any_element(),
        )
    }

    fn on_find_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let chord = mods.control || mods.platform;
        let shift = mods.shift;

        if key == "escape" {
            self.close_find(window, cx);
            cx.stop_propagation();
            return;
        }
        if key == "enter" {
            if shift {
                self.find_prev(cx);
            } else {
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if key == "f3" {
            if shift {
                self.find_prev(cx);
            } else {
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("f") {
            if let Some(find) = self.find.as_mut() {
                if !find.query.text.is_empty() {
                    find.query.select_all();
                }
                find.query.caret_visible = true;
                find.focus_handle.clone().focus(window);
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("a") {
            if let Some(find) = self.find.as_mut() {
                find.query.select_all();
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("c") {
            if let Some(find) = self.find.as_ref() {
                let text = if find.query.has_selection() {
                    find.query.selected_text()
                } else {
                    find.query.text.clone()
                };
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("x") {
            let text = self.find.as_ref().map(|f| {
                if f.query.has_selection() {
                    f.query.selected_text()
                } else {
                    f.query.text.clone()
                }
            });
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            if let Some(find) = self.find.as_mut() {
                if find.query.has_selection() {
                    find.query.delete_selection();
                } else {
                    find.query.text.clear();
                    find.query.cursor = 0;
                    find.query.anchor = 0;
                }
                find.query.caret_visible = true;
            }
            self.state.with_term_mut(|term| term.selection = None);
            self.find_next(cx);
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("g") {
            if shift {
                self.find_prev(cx);
            } else {
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("v") {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                let cleaned = text.replace('\r', "").replace('\n', "");
                if let Some(find) = self.find.as_mut() {
                    find.query.insert(&cleaned);
                }
                self.state.with_term_mut(|term| term.selection = None);
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if key == "left" {
            if let Some(find) = self.find.as_mut() {
                find.query.move_left(shift);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key == "right" {
            if let Some(find) = self.find.as_mut() {
                find.query.move_right(shift);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key == "home" {
            if let Some(find) = self.find.as_mut() {
                find.query.move_home(shift);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key == "end" {
            if let Some(find) = self.find.as_mut() {
                find.query.move_end(shift);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key == "backspace" {
            if let Some(find) = self.find.as_mut() {
                find.query.backspace();
            }
            self.state.with_term_mut(|term| term.selection = None);
            self.find_next(cx);
            cx.stop_propagation();
            return;
        }
        if key == "delete" {
            if let Some(find) = self.find.as_mut() {
                find.query.delete_forward();
            }
            self.state.with_term_mut(|term| term.selection = None);
            self.find_next(cx);
            cx.stop_propagation();
            return;
        }
        if !chord && !mods.alt {
            if let Some(typed) = typed_text_from_keystroke(&event.keystroke) {
                let cleaned: String = typed.chars().filter(|c| !c.is_control()).collect();
                if !cleaned.is_empty() {
                    if let Some(find) = self.find.as_mut() {
                        find.query.insert(&cleaned);
                    }
                    self.state.with_term_mut(|term| term.selection = None);
                    self.find_next(cx);
                    cx.stop_propagation();
                }
            }
        }
    }
}

fn find_btn(
    id: &'static str,
    label: &'static str,
    tip: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    use crate::ui::tooltip::Tooltip;

    div()
        .id(id)
        .size(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_SM))
        .text_xs()
        .text_color(theme::TEXT_MUTED)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
        .tooltip(move |_, cx| Tooltip::text(tip, cx))
        .child(label)
        .on_click(on_click)
}
