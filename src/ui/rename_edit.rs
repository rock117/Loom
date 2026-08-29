//! In-place rename editor (Zed-like: select-all on start, blinking caret).

use gpui::prelude::*;
use gpui::*;

use crate::shared::theme;

#[derive(Clone, Debug)]
pub struct RenameEdit {
    pub text: String,
    /// Cursor position in Unicode scalar values (0..=char_len).
    pub cursor: usize,
    /// Selection anchor; range is `min(anchor, cursor)..max(anchor, cursor)`.
    pub anchor: usize,
    pub caret_visible: bool,
}

impl RenameEdit {
    /// Start renaming with the entire name selected (caret at end).
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.chars().count();
        Self {
            text,
            cursor: len,
            anchor: 0,
            caret_visible: true,
        }
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn sel_range(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.cursor
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.char_len();
        self.caret_visible = true;
    }

    pub fn clear_selection(&mut self) {
        self.anchor = self.cursor;
    }

    fn clamp_cursor(&mut self) {
        let len = self.char_len();
        if self.cursor > len {
            self.cursor = len;
        }
        if self.anchor > len {
            self.anchor = len;
        }
    }

    /// Delete the current selection; returns true if something was removed.
    pub fn delete_selection(&mut self) -> bool {
        let (lo, hi) = self.sel_range();
        if lo == hi {
            return false;
        }
        let before: String = self.text.chars().take(lo).collect();
        let after: String = self.text.chars().skip(hi).collect();
        self.text = before + &after;
        self.cursor = lo;
        self.anchor = lo;
        self.caret_visible = true;
        true
    }

    pub fn insert(&mut self, typed: &str) {
        self.delete_selection();
        let before: String = self.text.chars().take(self.cursor).collect();
        let after: String = self.text.chars().skip(self.cursor).collect();
        let insert_len = typed.chars().count();
        self.text = before + typed + &after;
        self.cursor += insert_len;
        self.anchor = self.cursor;
        self.caret_visible = true;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let before: String = self.text.chars().take(self.cursor - 1).collect();
        let after: String = self.text.chars().skip(self.cursor).collect();
        self.text = before + &after;
        self.cursor -= 1;
        self.anchor = self.cursor;
        self.caret_visible = true;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor >= self.char_len() {
            return;
        }
        let before: String = self.text.chars().take(self.cursor).collect();
        let after: String = self.text.chars().skip(self.cursor + 1).collect();
        self.text = before + &after;
        self.anchor = self.cursor;
        self.caret_visible = true;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let (lo, _) = self.sel_range();
            self.cursor = lo;
            self.anchor = lo;
            self.caret_visible = true;
            return;
        }
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        if !extend {
            self.anchor = self.cursor;
        }
        self.caret_visible = true;
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let (_, hi) = self.sel_range();
            self.cursor = hi;
            self.anchor = hi;
            self.caret_visible = true;
            return;
        }
        if self.cursor < self.char_len() {
            self.cursor += 1;
        }
        if !extend {
            self.anchor = self.cursor;
        }
        self.caret_visible = true;
    }

    pub fn move_home(&mut self, extend: bool) {
        self.cursor = 0;
        if !extend {
            self.anchor = 0;
        }
        self.caret_visible = true;
    }

    pub fn move_end(&mut self, extend: bool) {
        self.cursor = self.char_len();
        if !extend {
            self.anchor = self.cursor;
        }
        self.caret_visible = true;
    }

    pub fn selected_text(&self) -> String {
        let (lo, hi) = self.sel_range();
        self.text.chars().skip(lo).take(hi - lo).collect()
    }

    /// Render label with selection highlight and blinking caret.
    pub fn into_element(&self) -> AnyElement {
        let (lo, hi) = self.sel_range();
        let cursor = self.cursor.min(self.char_len());
        let before_sel: String = self.text.chars().take(lo).collect();
        let selected: String = self.text.chars().skip(lo).take(hi - lo).collect();
        let after_sel: String = self.text.chars().skip(hi).collect();

        let caret = |visible: bool| {
            div()
                .w(px(1.0))
                .h(px(13.0))
                .flex_shrink_0()
                .bg(if visible {
                    theme::TEXT
                } else {
                    theme::TEXT.opacity(0.0)
                })
        };

        let sel_el = |s: String| {
            div()
                .bg(theme::ACCENT.opacity(0.45))
                .rounded(px(2.0))
                .child(s)
        };

        let mut row = div()
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .h(px(ROW_RENAME))
            .px(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::ACCENT)
            .text_xs()
            .text_color(theme::TEXT);

        // Compose: text + caret at `cursor`.
        if lo == hi {
            let before: String = self.text.chars().take(cursor).collect();
            let after: String = self.text.chars().skip(cursor).collect();
            row = row
                .child(div().whitespace_nowrap().child(before))
                .child(caret(self.caret_visible))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(after),
                );
        } else if cursor <= lo {
            let before_c: String = before_sel.chars().take(cursor).collect();
            let mid: String = before_sel.chars().skip(cursor).collect();
            row = row
                .child(div().whitespace_nowrap().child(before_c))
                .child(caret(self.caret_visible))
                .child(div().whitespace_nowrap().child(mid))
                .child(sel_el(selected))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(after_sel),
                );
        } else if cursor >= hi {
            let after_before_c: String = after_sel.chars().take(cursor - hi).collect();
            let after_after_c: String = after_sel.chars().skip(cursor - hi).collect();
            row = row
                .child(div().whitespace_nowrap().child(before_sel))
                .child(sel_el(selected))
                .child(div().whitespace_nowrap().child(after_before_c))
                .child(caret(self.caret_visible))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(after_after_c),
                );
        } else {
            // Caret inside selection.
            let sel_before: String = selected.chars().take(cursor - lo).collect();
            let sel_after: String = selected.chars().skip(cursor - lo).collect();
            row = row
                .child(div().whitespace_nowrap().child(before_sel))
                .child(sel_el(sel_before))
                .child(caret(self.caret_visible))
                .child(sel_el(sel_after))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(after_sel),
                );
        }
        row.into_any_element()
    }
}

const ROW_RENAME: f32 = 18.0;
