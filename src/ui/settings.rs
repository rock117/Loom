use gpui::prelude::*;
use gpui::*;

use crate::shared::theme;
use crate::ui::widgets::AccentButton;
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    Close,
    Export,
    Import,
    FontSizeChanged(f32),
    LineNumbersChanged(bool),
}

pub struct SettingsPanel {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    editing_shell: bool,
    editing_font: bool,
    _observe_store: Subscription,
}

impl SettingsPanel {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        Self {
            store,
            focus_handle: cx.focus_handle(),
            editing_shell: false,
            editing_font: false,
            _observe_store,
        }
    }

    fn row_button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded(px(4.0))
            .bg(theme::ACCENT)
            .text_color(rgb(0xffffff))
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.store.read(cx).settings.clone();
        let shell_label = settings
            .default_shell
            .clone()
            .unwrap_or_else(|| "auto".into());
        let font_family = settings.font_family.clone();
        let font_size = settings.font_size;
        let show_line_numbers = settings.show_line_numbers;

        div()
            .id("settings-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)),
            )
            .child(
                div()
                    .id("settings-card")
                    .w(px(420.0))
                    .p_5()
                    .rounded(px(8.0))
                    .bg(theme::PANEL_BG)
                    .border_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .track_focus(&self.focus_handle)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        let key = &event.keystroke.key;
                        if key == "escape" {
                            cx.emit(SettingsEvent::Close);
                            cx.stop_propagation();
                            return;
                        }
                        if this.editing_shell {
                            if key == "enter" {
                                this.editing_shell = false;
                                this.store.update(cx, |s, _| s.persist_now());
                                cx.notify();
                                cx.stop_propagation();
                            } else if key == "backspace" {
                                this.store.update(cx, |s, cx| {
                                    if let Some(shell) = &mut s.settings.default_shell {
                                        shell.pop();
                                        if shell.is_empty() {
                                            s.settings.default_shell = None;
                                        }
                                    }
                                    s.mark_dirty();
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            } else if key.len() == 1 {
                                if let Some(ch) = key.chars().next() {
                                    if !ch.is_control() {
                                        this.store.update(cx, |s, cx| {
                                            let mut cur =
                                                s.settings.default_shell.clone().unwrap_or_default();
                                            cur.push(ch);
                                            s.settings.default_shell = Some(cur);
                                            s.mark_dirty();
                                            cx.notify();
                                        });
                                        cx.stop_propagation();
                                    }
                                }
                            }
                        } else if this.editing_font {
                            if key == "enter" {
                                this.editing_font = false;
                                this.store.update(cx, |s, _| s.persist_now());
                                cx.notify();
                                cx.stop_propagation();
                            } else if key == "backspace" {
                                this.store.update(cx, |s, cx| {
                                    s.settings.font_family.pop();
                                    s.mark_dirty();
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            } else if key.len() == 1 {
                                if let Some(ch) = key.chars().next() {
                                    if !ch.is_control() {
                                        this.store.update(cx, |s, cx| {
                                            s.settings.font_family.push(ch);
                                            s.mark_dirty();
                                            cx.notify();
                                        });
                                        cx.stop_propagation();
                                    }
                                }
                            }
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::TEXT)
                                    .child("Settings"),
                            )
                            .child(AccentButton::new(
                                "settings-close",
                                "Close",
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(SettingsEvent::Close);
                                }),
                            )),
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
                                    .child("Default shell (empty = auto)"),
                            )
                            .child(
                                div()
                                    .id("settings-shell")
                                    .px_3()
                                    .py_2()
                                    .rounded(px(4.0))
                                    .bg(theme::BG)
                                    .border_1()
                                    .border_color(if self.editing_shell {
                                        theme::ACCENT
                                    } else {
                                        theme::BORDER
                                    })
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .cursor_pointer()
                                    .child(if self.editing_shell {
                                        format!("{shell_label}|")
                                    } else {
                                        shell_label
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.editing_shell = true;
                                        this.editing_font = false;
                                        this.focus_handle.focus(window);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .children(
                                        ["auto", "pwsh", "powershell", "cmd"].into_iter().map(
                                            |name| {
                                                let label = name.to_string();
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "shell-preset-{name}"
                                                    )))
                                                    .px_2()
                                                    .py_1()
                                                    .rounded(px(4.0))
                                                    .bg(theme::BG)
                                                    .border_1()
                                                    .border_color(theme::BORDER)
                                                    .text_xs()
                                                    .text_color(theme::TEXT_MUTED)
                                                    .cursor_pointer()
                                                    .child(label.clone())
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.store.update(cx, |s, cx| {
                                                            s.settings.default_shell =
                                                                if name == "auto" {
                                                                    None
                                                                } else {
                                                                    Some(name.to_string())
                                                                };
                                                            s.mark_dirty();
                                                            s.persist_now();
                                                            cx.notify();
                                                        });
                                                        this.editing_shell = false;
                                                        cx.notify();
                                                    }))
                                            },
                                        ),
                                    ),
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
                                    .child("Font family"),
                            )
                            .child(
                                div()
                                    .id("settings-font")
                                    .px_3()
                                    .py_2()
                                    .rounded(px(4.0))
                                    .bg(theme::BG)
                                    .border_1()
                                    .border_color(if self.editing_font {
                                        theme::ACCENT
                                    } else {
                                        theme::BORDER
                                    })
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .cursor_pointer()
                                    .child(if self.editing_font {
                                        format!("{font_family}|")
                                    } else {
                                        font_family
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.editing_font = true;
                                        this.editing_shell = false;
                                        this.focus_handle.focus(window);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::TEXT)
                                    .child(format!("Font size: {font_size:.0}")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(Self::row_button("font-minus", "−", cx, |this, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.settings.font_size =
                                                (s.settings.font_size - 1.0).max(8.0);
                                            s.mark_dirty();
                                            s.persist_now();
                                            cx.notify();
                                        });
                                        let size = this.store.read(cx).settings.font_size;
                                        cx.emit(SettingsEvent::FontSizeChanged(size));
                                    }))
                                    .child(Self::row_button("font-plus", "+", cx, |this, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.settings.font_size =
                                                (s.settings.font_size + 1.0).min(32.0);
                                            s.mark_dirty();
                                            s.persist_now();
                                            cx.notify();
                                        });
                                        let size = this.store.read(cx).settings.font_size;
                                        cx.emit(SettingsEvent::FontSizeChanged(size));
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.0))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme::TEXT)
                                                    .child("Line numbers"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::TEXT_MUTED)
                                                    .child("Scrollback gutter (1 = oldest line)"),
                                            ),
                                    )
                            .child(
                                div()
                                    .id("settings-line-numbers")
                                    .px_3()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .bg(if show_line_numbers {
                                        theme::ACCENT
                                    } else {
                                        theme::BG
                                    })
                                    .border_1()
                                    .border_color(if show_line_numbers {
                                        theme::ACCENT
                                    } else {
                                        theme::BORDER
                                    })
                                    .text_sm()
                                    .text_color(if show_line_numbers {
                                        rgb(0xffffff).into()
                                    } else {
                                        theme::TEXT_MUTED
                                    })
                                    .cursor_pointer()
                                    .child(if show_line_numbers { "On" } else { "Off" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let enabled = this.store.update(cx, |s, cx| {
                                            s.settings.show_line_numbers =
                                                !s.settings.show_line_numbers;
                                            let v = s.settings.show_line_numbers;
                                            s.mark_dirty();
                                            s.persist_now();
                                            cx.notify();
                                            v
                                        });
                                        cx.emit(SettingsEvent::LineNumbersChanged(enabled));
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Self::row_button("btn-export", "Export workspace…", cx, |_, _, cx| {
                                cx.emit(SettingsEvent::Export);
                            }))
                            .child(Self::row_button("btn-import", "Import workspace…", cx, |_, _, cx| {
                                cx.emit(SettingsEvent::Import);
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child("Import replaces the current workspace after confirmation in the file dialog flow."),
                    ),
            )
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}
