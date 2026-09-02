//! Settings overlay — grouped sections, restrained chrome (Zed-like density).

use gpui::prelude::*;
use gpui::*;

use crate::model::AnsiPalette;
use crate::session::local_proxy::{self, LocalProxyMode};
use crate::shared::theme;
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    Close,
    Export,
    Import,
    FontSizeChanged(f32),
    LineNumbersChanged(bool),
    AnsiPaletteChanged(AnsiPalette),
}

pub struct SettingsPanel {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    editing_shell: bool,
    editing_font: bool,
    editing_proxy_url: bool,
    editing_no_proxy: bool,
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
            editing_proxy_url: false,
            editing_no_proxy: false,
            _observe_store,
        }
    }

    fn section_title(label: impl Into<SharedString>) -> impl IntoElement {
        div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::TEXT_MUTED)
            .child(label.into())
    }

    fn field_label(label: impl Into<SharedString>) -> impl IntoElement {
        div()
            .text_xs()
            .text_color(theme::TEXT_MUTED)
            .child(label.into())
    }

    fn ghost_icon_btn(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(28.0))
            .rounded(px(theme::RADIUS_SM))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(theme::TEXT_MUTED)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    fn step_btn(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(28.0))
            .h(px(28.0))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::BG)
            .border_1()
            .border_color(theme::BORDER)
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(theme::TEXT)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    fn secondary_btn(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .px_3()
            .py_2()
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::BG)
            .border_1()
            .border_color(theme::BORDER)
            .text_sm()
            .text_color(theme::TEXT)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).border_color(theme::BORDER_SUBTLE))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    fn shell_preset_selected(current: &Option<String>, preset: &str) -> bool {
        match (current.as_deref(), preset) {
            (None, "auto") => true,
            (Some(s), p) if s.eq_ignore_ascii_case(p) => true,
            _ => false,
        }
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.store.read(cx).settings.clone();
        let shell_value = settings.default_shell.clone();
        let shell_display = shell_value.clone().unwrap_or_default();
        let font_family = settings.font_family.clone();
        let font_size = settings.font_size;
        let show_line_numbers = settings.show_line_numbers;
        let ansi_palette = settings.ansi_palette;
        let proxy_mode = settings.local_proxy_mode;
        let proxy_url = settings.local_proxy_url.clone().unwrap_or_default();
        let no_proxy = settings.local_proxy_no_proxy.clone().unwrap_or_default();
        let detected = (proxy_mode == LocalProxyMode::Auto)
            .then(|| local_proxy::detect_proxy().map(|p| p.url))
            .flatten();
        let shell_is_custom = shell_value
            .as_ref()
            .is_some_and(|s| !matches!(s.as_str(), "pwsh" | "powershell" | "cmd"));
        // Cap card height so short laptop viewports can still reach Workspace via scroll.
        let card_max_h = window.viewport_size().height * 0.9;

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
                    .w(px(440.0))
                    .max_h(card_max_h)
                    .p_5()
                    .rounded(px(theme::RADIUS))
                    .bg(theme::PANEL_BG)
                    .border_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .overflow_hidden()
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
                        } else if this.editing_proxy_url {
                            if key == "enter" {
                                this.editing_proxy_url = false;
                                this.store.update(cx, |s, _| s.persist_now());
                                cx.notify();
                                cx.stop_propagation();
                            } else if key == "backspace" {
                                this.store.update(cx, |s, cx| {
                                    if let Some(url) = &mut s.settings.local_proxy_url {
                                        url.pop();
                                        if url.is_empty() {
                                            s.settings.local_proxy_url = None;
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
                                            let mut cur = s
                                                .settings
                                                .local_proxy_url
                                                .clone()
                                                .unwrap_or_default();
                                            cur.push(ch);
                                            s.settings.local_proxy_url = Some(cur);
                                            s.mark_dirty();
                                            cx.notify();
                                        });
                                        cx.stop_propagation();
                                    }
                                }
                            }
                        } else if this.editing_no_proxy {
                            if key == "enter" {
                                this.editing_no_proxy = false;
                                this.store.update(cx, |s, _| s.persist_now());
                                cx.notify();
                                cx.stop_propagation();
                            } else if key == "backspace" {
                                this.store.update(cx, |s, cx| {
                                    if let Some(np) = &mut s.settings.local_proxy_no_proxy {
                                        np.pop();
                                        if np.is_empty() {
                                            s.settings.local_proxy_no_proxy = None;
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
                                            let mut cur = s
                                                .settings
                                                .local_proxy_no_proxy
                                                .clone()
                                                .unwrap_or_default();
                                            cur.push(ch);
                                            s.settings.local_proxy_no_proxy = Some(cur);
                                            s.mark_dirty();
                                            cx.notify();
                                        });
                                        cx.stop_propagation();
                                    }
                                }
                            }
                        }
                    }))
                    // Header
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
                            .child(Self::ghost_icon_btn("settings-close", "✕", cx, |_, _, cx| {
                                cx.emit(SettingsEvent::Close);
                            })),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_4()
                            // Terminal
                            .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(Self::section_title("TERMINAL"))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(Self::field_label("Default shell"))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_1()
                                            .children(
                                                ["auto", "pwsh", "powershell", "cmd"].into_iter().map(
                                                    |name| {
                                                        let selected = Self::shell_preset_selected(
                                                            &shell_value,
                                                            name,
                                                        );
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "shell-preset-{name}"
                                                            )))
                                                            .px_2()
                                                            .py_1()
                                                            .rounded(px(theme::RADIUS_SM))
                                                            .bg(if selected {
                                                                theme::SELECTION
                                                            } else {
                                                                theme::BG
                                                            })
                                                            .border_1()
                                                            .border_color(if selected {
                                                                theme::ACCENT
                                                            } else {
                                                                theme::BORDER
                                                            })
                                                            .text_xs()
                                                            .text_color(if selected {
                                                                theme::TEXT
                                                            } else {
                                                                theme::TEXT_MUTED
                                                            })
                                                            .cursor_pointer()
                                                            .hover(|s| {
                                                                if selected {
                                                                    s
                                                                } else {
                                                                    s.bg(theme::HOVER)
                                                                        .text_color(theme::TEXT)
                                                                }
                                                            })
                                                            .child(name.to_string())
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.store.update(cx, |s, cx| {
                                                                        s.settings.default_shell =
                                                                            if name == "auto" {
                                                                                None
                                                                            } else {
                                                                                Some(
                                                                                    name.to_string(),
                                                                                )
                                                                            };
                                                                        s.mark_dirty();
                                                                        s.persist_now();
                                                                        cx.notify();
                                                                    });
                                                                    this.editing_shell = false;
                                                                    cx.notify();
                                                                },
                                                            ))
                                                    },
                                                ),
                                            ),
                                    )
                                    .child(Self::field_label(if shell_is_custom {
                                        "Custom executable"
                                    } else {
                                        "Custom executable (optional)"
                                    }))
                                    .child(
                                        div()
                                            .id("settings-shell")
                                            .px_3()
                                            .py_2()
                                            .rounded(px(theme::RADIUS_SM))
                                            .bg(theme::BG)
                                            .border_1()
                                            .border_color(if self.editing_shell {
                                                theme::ACCENT
                                            } else {
                                                theme::BORDER
                                            })
                                            .text_sm()
                                            .text_color(if shell_display.is_empty()
                                                && !self.editing_shell
                                            {
                                                theme::TEXT_DISABLED
                                            } else {
                                                theme::TEXT
                                            })
                                            .cursor_pointer()
                                            .child(if self.editing_shell {
                                                format!("{shell_display}|")
                                            } else if shell_display.is_empty() {
                                                "e.g. C:\\Windows\\System32\\cmd.exe".into()
                                            } else {
                                                shell_display
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.editing_shell = true;
                                                this.editing_font = false;
                                                this.focus_handle.focus(window);
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    // Appearance
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(Self::section_title("APPEARANCE"))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(Self::field_label("Font family"))
                                    .child(
                                        div()
                                            .id("settings-font")
                                            .px_3()
                                            .py_2()
                                            .rounded(px(theme::RADIUS_SM))
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
                                            .child("Font size"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(Self::step_btn(
                                                "font-minus",
                                                "−",
                                                cx,
                                                |this, _, cx| {
                                                    this.store.update(cx, |s, cx| {
                                                        s.settings.font_size =
                                                            (s.settings.font_size - 1.0).max(8.0);
                                                        s.mark_dirty();
                                                        s.persist_now();
                                                        cx.notify();
                                                    });
                                                    let size =
                                                        this.store.read(cx).settings.font_size;
                                                    cx.emit(SettingsEvent::FontSizeChanged(size));
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .min_w(px(28.0))
                                                    .text_sm()
                                                    .text_color(theme::TEXT)
                                                    .text_center()
                                                    .child(format!("{font_size:.0}")),
                                            )
                                            .child(Self::step_btn(
                                                "font-plus",
                                                "+",
                                                cx,
                                                |this, _, cx| {
                                                    this.store.update(cx, |s, cx| {
                                                        s.settings.font_size =
                                                            (s.settings.font_size + 1.0).min(32.0);
                                                        s.mark_dirty();
                                                        s.persist_now();
                                                        cx.notify();
                                                    });
                                                    let size =
                                                        this.store.read(cx).settings.font_size;
                                                    cx.emit(SettingsEvent::FontSizeChanged(size));
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
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
                                                    .child(
                                                        "Scrollback gutter (1 = oldest line)",
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("settings-line-numbers")
                                            .px_3()
                                            .py_1()
                                            .rounded(px(theme::RADIUS_SM))
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
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.0))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme::TEXT)
                                                    .child("ANSI colors"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::TEXT_MUTED)
                                                    .child(
                                                        "How ANSI colors are drawn. Does not change the remote shell.",
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_1()
                                            .children(AnsiPalette::ALL.into_iter().map(|preset| {
                                                let selected = ansi_palette == preset;
                                                let label = preset.as_str().to_string();
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "ansi-palette-{label}"
                                                    )))
                                                    .px_2()
                                                    .py_1()
                                                    .rounded(px(theme::RADIUS_SM))
                                                    .bg(if selected {
                                                        theme::SELECTION
                                                    } else {
                                                        theme::BG
                                                    })
                                                    .border_1()
                                                    .border_color(if selected {
                                                        theme::ACCENT
                                                    } else {
                                                        theme::BORDER
                                                    })
                                                    .text_xs()
                                                    .text_color(if selected {
                                                        theme::TEXT
                                                    } else {
                                                        theme::TEXT_MUTED
                                                    })
                                                    .cursor_pointer()
                                                    .child(label)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.store.update(cx, |s, cx| {
                                                            s.settings.ansi_palette = preset;
                                                            s.mark_dirty();
                                                            s.persist_now();
                                                            cx.notify();
                                                        });
                                                        cx.emit(SettingsEvent::AnsiPaletteChanged(
                                                            preset,
                                                        ));
                                                        cx.notify();
                                                    }))
                                            })),
                                    ),
                            ),
                    )
                    // Local proxy
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(Self::section_title("LOCAL PROXY"))
                            .child(Self::field_label(
                                "Injected into new Local shells only (not SSH)",
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(
                                        [
                                            LocalProxyMode::Off,
                                            LocalProxyMode::Auto,
                                            LocalProxyMode::Manual,
                                        ]
                                        .into_iter()
                                        .map(|mode| {
                                            let selected = proxy_mode == mode;
                                            let label = mode.as_str().to_string();
                                            div()
                                                .id(SharedString::from(format!(
                                                    "proxy-mode-{label}"
                                                )))
                                                .px_2()
                                                .py_1()
                                                .rounded(px(theme::RADIUS_SM))
                                                .bg(if selected {
                                                    theme::SELECTION
                                                } else {
                                                    theme::BG
                                                })
                                                .border_1()
                                                .border_color(if selected {
                                                    theme::ACCENT
                                                } else {
                                                    theme::BORDER
                                                })
                                                .text_xs()
                                                .text_color(if selected {
                                                    theme::TEXT
                                                } else {
                                                    theme::TEXT_MUTED
                                                })
                                                .cursor_pointer()
                                                .child(label)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.store.update(cx, |s, cx| {
                                                        s.settings.local_proxy_mode = mode;
                                                        s.mark_dirty();
                                                        s.persist_now();
                                                        cx.notify();
                                                    });
                                                    this.editing_proxy_url = false;
                                                    cx.notify();
                                                }))
                                        }),
                                    ),
                            )
                            .when(proxy_mode == LocalProxyMode::Auto, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(match &detected {
                                            Some(url) => format!("Detected: {url}"),
                                            None => "Detected: (none)".into(),
                                        }),
                                )
                            })
                            .when(proxy_mode == LocalProxyMode::Manual, |d| {
                                d.child(Self::field_label("Proxy URL"))
                                    .child(
                                        div()
                                            .id("settings-proxy-url")
                                            .px_3()
                                            .py_2()
                                            .rounded(px(theme::RADIUS_SM))
                                            .bg(theme::BG)
                                            .border_1()
                                            .border_color(if self.editing_proxy_url {
                                                theme::ACCENT
                                            } else {
                                                theme::BORDER
                                            })
                                            .text_sm()
                                            .text_color(if proxy_url.is_empty()
                                                && !self.editing_proxy_url
                                            {
                                                theme::TEXT_DISABLED
                                            } else {
                                                theme::TEXT
                                            })
                                            .cursor_pointer()
                                            .child(if self.editing_proxy_url {
                                                format!("{proxy_url}|")
                                            } else if proxy_url.is_empty() {
                                                "e.g. http://127.0.0.1:7890".into()
                                            } else {
                                                proxy_url.clone()
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.editing_proxy_url = true;
                                                this.editing_no_proxy = false;
                                                this.editing_shell = false;
                                                this.editing_font = false;
                                                this.focus_handle.focus(window);
                                                cx.notify();
                                            })),
                                    )
                            })
                            .when(
                                matches!(
                                    proxy_mode,
                                    LocalProxyMode::Auto | LocalProxyMode::Manual
                                ),
                                |d| {
                                    d.child(Self::field_label("No proxy (optional)"))
                                        .child(
                                            div()
                                                .id("settings-no-proxy")
                                                .px_3()
                                                .py_2()
                                                .rounded(px(theme::RADIUS_SM))
                                                .bg(theme::BG)
                                                .border_1()
                                                .border_color(if self.editing_no_proxy {
                                                    theme::ACCENT
                                                } else {
                                                    theme::BORDER
                                                })
                                                .text_sm()
                                                .text_color(if no_proxy.is_empty()
                                                    && !self.editing_no_proxy
                                                {
                                                    theme::TEXT_DISABLED
                                                } else {
                                                    theme::TEXT
                                                })
                                                .cursor_pointer()
                                                .child(if self.editing_no_proxy {
                                                    format!("{no_proxy}|")
                                                } else if no_proxy.is_empty() {
                                                    "localhost,127.0.0.1".into()
                                                } else {
                                                    no_proxy
                                                })
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.editing_no_proxy = true;
                                                    this.editing_proxy_url = false;
                                                    this.editing_shell = false;
                                                    this.editing_font = false;
                                                    this.focus_handle.focus(window);
                                                    cx.notify();
                                                })),
                                        )
                                },
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(
                                        "Applies to newly opened Local tabs (or Local Reconnect).",
                                    ),
                            ),
                    )
                    // Workspace
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(Self::section_title("WORKSPACE"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(Self::secondary_btn(
                                        "btn-export",
                                        "Export workspace…",
                                        cx,
                                        |_, _, cx| cx.emit(SettingsEvent::Export),
                                    ))
                                    .child(Self::secondary_btn(
                                        "btn-import",
                                        "Import workspace…",
                                        cx,
                                        |_, _, cx| cx.emit(SettingsEvent::Import),
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(
                                        "Import replaces the current workspace after confirmation in the file dialog flow.",
                                    ),
                            ),
                            ),
                    ),
            )
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}
