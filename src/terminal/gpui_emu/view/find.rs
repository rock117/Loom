//! Terminal scrollback / viewport text search (Ctrl+F).

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use gpui::prelude::*;
use gpui::*;

use super::TerminalView;
use crate::shared::theme;

/// In-view find bar state (owned by [`TerminalView`]).
pub struct FindState {
    pub query: String,
    pub status: SharedString,
    pub focus_handle: FocusHandle,
    pub caret_visible: bool,
}

impl FindState {
    pub fn new(focus_handle: FocusHandle) -> Self {
        Self {
            query: String::new(),
            status: SharedString::default(),
            focus_handle,
            caret_visible: true,
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
    let start = Point::new(Line(-history), Column(0));
    let last_line = term.screen_lines() as i32 - 1;
    let last_col = term.columns().saturating_sub(1);
    let end = Point::new(Line(last_line), Column(last_col));
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
                            find.caret_visible = !find.caret_visible;
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
            find.caret_visible = true;
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
        if find.query.is_empty() {
            find.status = "".into();
            self.state.with_term_mut(|term| term.selection = None);
            cx.notify();
            return;
        }

        let pattern = escape_literal(&find.query);
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

    pub(crate) fn render_find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let query = find.query.clone();
        let status = find.status.clone();
        let caret_visible = find.caret_visible;
        let focus = find.focus_handle.clone();
        let empty_query = query.is_empty();

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
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(query),
                                )
                                .child(
                                    div()
                                        .w(px(1.0))
                                        .h(px(13.0))
                                        .flex_shrink_0()
                                        .bg(if caret_visible {
                                            theme::TEXT
                                        } else {
                                            theme::TEXT.opacity(0.0)
                                        }),
                                ),
                        ),
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
                    cx.listener(|this, _, _, cx| this.find_prev(cx)),
                ))
                .child(find_btn(
                    "find-next",
                    "↓",
                    cx.listener(|this, _, _, cx| this.find_next(cx)),
                ))
                .child(find_btn(
                    "find-close",
                    "×",
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

        if key == "escape" {
            self.close_find(window, cx);
            cx.stop_propagation();
            return;
        }
        if key == "enter" {
            if mods.shift {
                self.find_prev(cx);
            } else {
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if key == "f3" {
            if mods.shift {
                self.find_prev(cx);
            } else {
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("f") {
            if let Some(find) = self.find.as_mut() {
                find.caret_visible = true;
                find.focus_handle.clone().focus(window);
            }
            cx.stop_propagation();
            return;
        }
        if chord && key.eq_ignore_ascii_case("g") {
            if mods.shift {
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
                    find.query.push_str(&cleaned);
                    find.caret_visible = true;
                }
                self.state.with_term_mut(|term| term.selection = None);
                self.find_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if key == "backspace" {
            if let Some(find) = self.find.as_mut() {
                find.query.pop();
                find.caret_visible = true;
            }
            self.state.with_term_mut(|term| term.selection = None);
            self.find_next(cx);
            cx.stop_propagation();
            return;
        }
        if !chord
            && !mods.alt
            && let Some(typed) = event.keystroke.key_char.as_deref()
        {
            let mut any = false;
            if let Some(find) = self.find.as_mut() {
                for ch in typed.chars() {
                    if !ch.is_control() {
                        find.query.push(ch);
                        any = true;
                    }
                }
                if any {
                    find.caret_visible = true;
                }
            }
            if any {
                self.state.with_term_mut(|term| term.selection = None);
                self.find_next(cx);
                cx.stop_propagation();
            }
        }
    }
}

fn find_btn(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
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
        .child(label)
        .on_click(on_click)
}
