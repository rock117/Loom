//! Terminal right-click context menu.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;

use crate::platform;
use crate::shared::theme;

use super::TerminalView;

#[derive(Clone, Debug)]
pub enum TerminalViewEvent {
    /// Focus the pane that owns this terminal (e.g. before a context action).
    FocusRequested,
    /// Close the pane that owns this terminal (or its tab when it is the last pane).
    CloseRequested,
    /// PTY / SSH channel ended — host should mark the pane failed and offer reconnect.
    SessionEnded,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl TerminalView {
    pub fn set_working_directory(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.working_directory = path;
        cx.notify();
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.working_directory.clone()
    }

    /// Prefer live process cwd (local), else keep OSC / spawn cwd.
    pub(super) fn refresh_working_directory(&mut self) {
        if let Some(pid) = self.shell_pid {
            if let Some(cwd) = platform::process_cwd(pid) {
                self.working_directory = Some(cwd);
            }
        }
    }

    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.is_some() {
            self.context_menu = None;
            cx.notify();
        }
    }

    pub(super) fn open_context_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.refresh_working_directory();
        self.context_menu = Some(position);
        cx.notify();
    }

    fn has_selection(&self) -> bool {
        self.state
            .with_term(|term| term.selection_to_string().is_some_and(|s| !s.is_empty()))
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};

        self.state.with_term_mut(|term| {
            let history = term.history_size() as i32;
            let cols = term.columns().saturating_sub(1);
            let bottom = term.bottommost_line();
            let start = AlacPoint::new(Line(-history), Column(0));
            let end = AlacPoint::new(bottom, Column(cols));
            let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
            selection.update(end, Side::Right);
            selection.include_all();
            term.selection = Some(selection);
        });
        self.copy_selection_to_clipboard(cx);
        cx.notify();
    }

    fn copy_working_directory(&mut self, cx: &mut Context<Self>) {
        self.refresh_working_directory();
        let Some(path) = self.working_directory.as_ref() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(
            path.to_string_lossy().into_owned(),
        ));
    }

    fn reveal_working_directory(&mut self) {
        self.refresh_working_directory();
        let Some(path) = self.working_directory.as_ref() else {
            return;
        };
        let _ = platform::reveal_in_file_manager(path);
    }

    fn menu_shell(&self) -> Div {
        div()
            .min_w(px(200.0))
            .p(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS))
            .bg(theme::ELEVATED)
            .border_1()
            .border_color(theme::BORDER)
            .shadow_md()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    fn menu_divider(&self) -> impl IntoElement {
        div()
            .h(px(1.0))
            .my(px(theme::SPACE_1))
            .bg(theme::BORDER_SUBTLE)
    }

    fn menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .text_sm()
            .when(enabled, |d| {
                d.text_color(theme::TEXT)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
            })
            .when(!enabled, |d| d.text_color(theme::TEXT_DISABLED))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                if !enabled {
                    return;
                }
                on_click(this, window, cx);
                this.context_menu = None;
                cx.notify();
            }))
    }

    pub(super) fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let position = self.context_menu?;
        let has_sel = self.has_selection();
        let has_path = self.working_directory.is_some();
        let can_copy_path = has_path;
        let can_reveal = self
            .working_directory
            .as_ref()
            .is_some_and(|p| p.exists());

        Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(Edges {
                        top: px(4.0),
                        right: px(4.0),
                        bottom: px(4.0),
                        left: px(4.0),
                    })
                    .child(
                        self.menu_shell()
                            .child(self.menu_item("term-ctx-copy", "Copy", has_sel, cx, |this, _, cx| {
                                this.copy_selection_to_clipboard(cx);
                            }))
                            .child(self.menu_item(
                                "term-ctx-paste",
                                "Paste",
                                true,
                                cx,
                                |this, _, cx| {
                                    this.paste_from_clipboard(cx);
                                },
                            ))
                            .child(self.menu_divider())
                            .child(self.menu_item(
                                "term-ctx-copy-path",
                                "Copy Path",
                                can_copy_path,
                                cx,
                                |this, _, cx| {
                                    this.copy_working_directory(cx);
                                },
                            ))
                            .child(self.menu_item(
                                "term-ctx-reveal",
                                "Reveal in File Explorer",
                                can_reveal,
                                cx,
                                |this, _, _| {
                                    this.reveal_working_directory();
                                },
                            ))
                            .child(self.menu_divider())
                            .child(self.menu_item(
                                "term-ctx-find",
                                "Find…",
                                true,
                                cx,
                                |this, window, cx| {
                                    this.open_find(window, cx);
                                },
                            ))
                            .child(self.menu_item(
                                "term-ctx-select-all",
                                "Select All",
                                true,
                                cx,
                                |this, _, cx| {
                                    this.select_all(cx);
                                },
                            ))
                            .child(self.menu_divider())
                            .child(self.menu_item(
                                "term-ctx-close",
                                "Close",
                                true,
                                cx,
                                |_this, _, cx| {
                                    cx.emit(TerminalViewEvent::CloseRequested);
                                },
                            )),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
