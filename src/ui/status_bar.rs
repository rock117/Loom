//! Window-bottom status bar (Zed / VS Code style).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{ConnectionState, ProfileKind};
use crate::shared::theme;
use crate::ui::tab_manager::TabManager;
use crate::ui::tooltip::Tooltip;
use crate::ui::workspace_store::WorkspaceStore;

const ICON: f32 = 12.0;
/// Soft cap for the cwd segment label; full path stays in the tooltip.
const CWD_LABEL_MAX_CHARS: usize = 56;

pub struct StatusBar {
    pub store: Entity<WorkspaceStore>,
    pub tabs: Entity<TabManager>,
    toast: Option<SharedString>,
    _toast_clear: Option<Task<()>>,
    _observe_store: Subscription,
    _observe_tabs: Subscription,
    _observe_terminal: Option<Subscription>,
    /// Polls `process_cwd` for the focused Local pane so the cwd label tracks `cd`
    /// even when OSC hooks are missing (not used on quit — see WINDOW_CLOSE_HANG).
    _cwd_poll: Option<Task<()>>,
}

#[derive(Clone, Debug)]
pub enum StatusBarEvent {
    Reconnect(Uuid),
    OpenSettings,
    EditSshProfile(Uuid),
    EditLocalProfile(Uuid),
    /// Toggle profiles sidebar visibility (Zed left-dock).
    ToggleSidebar,
    /// Toggle right context panel visibility.
    ToggleContextPanel,
    /// Focus Context → Info port-forward section.
    FocusPortForwards,
}

impl StatusBar {
    pub fn new(
        store: Entity<WorkspaceStore>,
        tabs: Entity<TabManager>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        let tabs_for_observe = tabs.clone();
        let _observe_tabs = cx.observe(&tabs_for_observe, |this, _tabs, cx| {
            this.resync_terminal_observe(cx);
            cx.notify();
        });
        let mut bar = Self {
            store,
            tabs,
            toast: None,
            _toast_clear: None,
            _observe_store,
            _observe_tabs,
            _observe_terminal: None,
            _cwd_poll: None,
        };
        bar.resync_terminal_observe(cx);
        bar.start_cwd_poll(cx);
        bar
    }

    fn start_cwd_poll(&mut self, cx: &mut Context<Self>) {
        self._cwd_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(800))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.poll_focused_local_cwd(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Refresh live cwd for focused Local/WSL only (SSH / Docker stay OSC-only —
    /// `process_cwd` on `docker.exe` is the host client cwd, not the container).
    fn poll_focused_local_cwd(&mut self, cx: &mut Context<Self>) {
        let term = self.tabs.read(cx).active.and_then(|id| {
            self.tabs
                .read(cx)
                .tabs
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.focused_pane())
                .filter(|p| {
                    p.kind.is_local() && !p.kind.is_docker_local() && p.terminal.is_some()
                })
                .and_then(|p| p.terminal.clone())
        });
        let Some(term) = term else {
            return;
        };
        term.update(cx, |view, cx| {
            view.refresh_working_directory(cx);
        });
    }

    fn resync_terminal_observe(&mut self, cx: &mut Context<Self>) {
        let term = self.tabs.read(cx).active.and_then(|id| {
            self.tabs
                .read(cx)
                .tabs
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.focused_pane())
                .and_then(|p| p.terminal.clone())
        });
        self._observe_terminal = term.map(|entity| {
            cx.observe(&entity, |_this, _term, cx| cx.notify())
        });
    }

    pub fn set_toast(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast = Some(msg.into());
        self._toast_clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(2500))
                .await;
            this.update(cx, |this, cx| {
                this.toast = None;
                this._toast_clear = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn clear_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
        self._toast_clear = None;
        cx.notify();
    }

    fn svg(path: &'static str, color: Hsla) -> impl IntoElement {
        svg()
            .path(path)
            .size(px(ICON))
            .flex_shrink_0()
            .text_color(color)
    }

    fn state_dot(state: ConnectionState) -> impl IntoElement {
        let color = match state {
            ConnectionState::Connected => theme::ICON_LOCAL,
            ConnectionState::Connecting | ConnectionState::Idle => theme::TEXT_MUTED,
            ConnectionState::Disconnected | ConnectionState::Failed => theme::DANGER,
        };
        div()
            .size(px(7.0))
            .rounded_full()
            .flex_shrink_0()
            .bg(color)
    }

    fn segment(
        id: impl Into<ElementId>,
        cx: &mut Context<Self>,
        tooltip: &'static str,
        key: Option<&'static str>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        child: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1))
            .px(px(theme::SPACE_2))
            .h_full()
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .tooltip(move |_, cx| {
                if let Some(key) = key {
                    Tooltip::with_key(tooltip, key, cx)
                } else {
                    Tooltip::text(tooltip, cx)
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(child)
    }

    /// Cached session cwd (OSC / pane kind) — no `process_cwd` on the paint path.
    fn cached_cwd(pane: Option<&crate::ui::tab_manager::PaneSession>, cx: &App) -> Option<PathBuf> {
        let pane = pane?;
        let from_term = pane
            .terminal
            .as_ref()
            .and_then(|t| t.read(cx).working_directory());
        // Docker: never fall back to Local spawn cwd (host path of docker.exe).
        // Show OSC-reported container path only; hide until then. Also drop any
        // stale host path left from older seeds / process_cwd polls.
        if pane.kind.is_docker() {
            return from_term.filter(|p| Self::docker_cwd_plausible(p));
        }
        if from_term.is_some() {
            return from_term;
        }
        match &pane.kind {
            ProfileKind::Local { cwd, .. } => cwd.clone(),
            ProfileKind::Ssh { .. } => None,
        }
    }

    /// Container paths are Unix-style; reject Windows drive / UNC (docker.exe client cwd).
    fn docker_cwd_plausible(path: &std::path::Path) -> bool {
        let s = path.to_string_lossy();
        if s.len() >= 2 && s.as_bytes()[1] == b':' {
            return false;
        }
        if s.starts_with(r"\\") {
            return false;
        }
        true
    }

    fn cwd_segment(
        cx: &mut Context<Self>,
        full: PathBuf,
    ) -> impl IntoElement {
        let full_str = full.display().to_string();
        let label = truncate_path_end(&full_str, CWD_LABEL_MAX_CHARS);
        let tip = format!("{full_str}\nClick to copy");
        let copy_path = full_str.clone();

        div()
            .id("sb-cwd")
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1))
            .px(px(theme::SPACE_2))
            .h_full()
            .max_w(px(420.0))
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .tooltip(move |_, cx| Tooltip::text(tip.clone(), cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()));
                this.set_toast("Copied path", cx);
            }))
            .child(Self::svg("icons/ui/folder.svg", theme::TEXT_MUTED))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::TEXT_MUTED)
                    .font_family(theme_font_mono())
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(label),
            )
    }
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let font_size = manager.font_size;
        let sidebar_visible = self.store.read(cx).ui_state.sidebar_visible;
        let context_visible = self.store.read(cx).ui_state.context_panel_visible;
        let active = manager
            .active
            .and_then(|id| manager.tabs.iter().find(|t| t.id == id));

        let toast = self.toast.clone();

        let (session_left, right_geom) = if let Some(tab) = active {
            let tab_id = tab.id;
            let pane = tab.focused_pane();
            let state = pane
                .map(|p| p.state)
                .unwrap_or(ConnectionState::Idle);
            let profile_id = pane.and_then(|p| p.profile_id);
            let profile = profile_id.and_then(|pid| {
                self.store.read(cx).workspace.find_profile(pid).cloned()
            });
            let kind_fallback = pane.map(|p| p.kind.clone());

            let (kind_icon, kind_color, target, is_ssh) = match profile
                .as_ref()
                .map(|p| &p.kind)
                .or(kind_fallback.as_ref())
            {
                Some(kind @ ProfileKind::Ssh {
                    host, port, user, ..
                }) => {
                    if kind.is_docker_ssh() {
                        let nice = kind
                            .docker_ssh_container_id()
                            .map(|id| {
                                if id.len() > 12 {
                                    format!("docker · {} @ {user}@{host}", &id[..12])
                                } else {
                                    format!("docker · {id} @ {user}@{host}")
                                }
                            })
                            .unwrap_or_else(|| format!("docker @ {user}@{host}"));
                        ("icons/ui/docker.svg", theme::ICON_DOCKER, nice, true)
                    } else {
                        (
                            "icons/ui/remote.svg",
                            theme::ICON_REMOTE,
                            format!("{user}@{host}:{port}"),
                            true,
                        )
                    }
                }
                Some(kind @ ProfileKind::Local { shell, args, .. }) => {
                    let shell_label = shell
                        .as_deref()
                        .map(|s| {
                            std::path::Path::new(s)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(s)
                                .to_string()
                        })
                        .unwrap_or_else(|| "Local".into());
                    let label = if args.is_empty() {
                        shell_label
                    } else {
                        format!("{shell_label} {}", args.join(" "))
                    };
                    if kind.is_wsl_local() {
                        ("icons/ui/wsl.svg", theme::ICON_WSL, label, false)
                    } else if kind.is_docker_local() {
                        let nice = crate::session::docker::container_id_from_args(args)
                            .map(|id| {
                                if id.len() > 12 {
                                    format!("docker · {}", &id[..12])
                                } else {
                                    format!("docker · {id}")
                                }
                            })
                            .unwrap_or_else(|| "docker".into());
                        ("icons/ui/docker.svg", theme::ICON_DOCKER, nice, false)
                    } else {
                        ("icons/ui/terminal.svg", theme::ICON_LOCAL, label, false)
                    }
                }
                None => (
                    "icons/ui/terminal.svg",
                    theme::TEXT_MUTED,
                    tab.title.clone(),
                    false,
                ),
            };

            let profile_name = profile
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| tab.title.clone());

            let state_label = match state {
                ConnectionState::Connected => "Connected",
                ConnectionState::Connecting => "Connecting",
                ConnectionState::Disconnected if !is_ssh => "Exited",
                ConnectionState::Disconnected => "Disconnected",
                ConnectionState::Failed if !is_ssh => "Exited",
                ConnectionState::Failed => "Failed",
                ConnectionState::Idle => "Idle",
            };

            let needs_reconnect = matches!(
                state,
                ConnectionState::Disconnected | ConnectionState::Failed
            );

            let fwd_snap = pane.and_then(|p| p.ssh_forwards.as_ref().map(|h| h.snapshot()));
            let fwd_listening = fwd_snap.as_ref().map(|s| s.listening_count()).unwrap_or(0);
            let fwd_error_label = fwd_snap
                .as_ref()
                .and_then(|s| s.status_bar_error_label());
            let fwd_label = match fwd_snap.as_ref() {
                Some(snap) if fwd_listening == 1 && fwd_error_label.is_none() => snap
                    .rows
                    .iter()
                    .find(|r| r.status.is_listening())
                    .map(|r| {
                        let endpoints = format!(
                            "{}:{} → {}:{}",
                            r.bind_host, r.bind_port, r.target_host, r.target_port
                        );
                        let name = r.name.trim();
                        if name.is_empty() {
                            format!("⇄ {endpoints}")
                        } else {
                            format!("⇄ {name} · {endpoints}")
                        }
                    })
                    .unwrap_or_else(|| format!("⇄ {fwd_listening}")),
                _ if fwd_listening > 0 && fwd_error_label.is_none() => {
                    format!("⇄ {fwd_listening}")
                }
                _ => String::new(),
            };
            let fwd_ok_tooltip = if fwd_listening == 1 {
                "Port forward — open Info"
            } else {
                "Port forwards — open Info"
            };

            let (cols, rows) = pane
                .and_then(|p| p.terminal.as_ref())
                .map(|t| t.read(cx).dimensions())
                .unwrap_or((0, 0));

            let cwd = Self::cached_cwd(pane, cx);

            let left = div()
                .flex()
                .items_center()
                .h_full()
                .gap(px(2.0))
                .child(Self::segment(
                    "sb-session",
                    cx,
                    if is_ssh {
                        "Edit SSH Profile"
                    } else if profile_id.is_some() {
                        "Edit Local Profile"
                    } else {
                        "Session"
                    },
                    None,
                    move |_, _, cx| {
                        if let Some(profile_id) = profile_id {
                            if is_ssh {
                                cx.emit(StatusBarEvent::EditSshProfile(profile_id));
                            } else {
                                cx.emit(StatusBarEvent::EditLocalProfile(profile_id));
                            }
                        }
                    },
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::SPACE_1))
                        .child(Self::state_dot(state))
                        .child(Self::svg(kind_icon, kind_color))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT)
                                .whitespace_nowrap()
                                .child(format!("{state_label}  ·  {target}")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .whitespace_nowrap()
                                .child(format!("·  {profile_name}")),
                        ),
                ))
                .when_some(cwd, |d, path| d.child(Self::cwd_segment(cx, path)))
                .when_some(fwd_error_label.clone(), |d, err_label| {
                    d.child(Self::segment(
                        "sb-fwd-err",
                        cx,
                        "Port forward error — open Info",
                        None,
                        |_, _, cx| {
                            cx.emit(StatusBarEvent::FocusPortForwards);
                        },
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::DANGER)
                                    .whitespace_nowrap()
                                    .child(err_label),
                            ),
                    ))
                })
                .when(fwd_error_label.is_none() && fwd_listening > 0, |d| {
                    d.child(Self::segment(
                        "sb-fwd",
                        cx,
                        fwd_ok_tooltip,
                        None,
                        |_, _, cx| {
                            cx.emit(StatusBarEvent::FocusPortForwards);
                        },
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .whitespace_nowrap()
                                    .child(fwd_label),
                            ),
                    ))
                })
                .when(needs_reconnect, |d| {
                    d.child(Self::segment(
                        "sb-reconnect",
                        cx,
                        "Reconnect",
                        None,
                        move |_, _, cx| {
                            cx.emit(StatusBarEvent::Reconnect(tab_id));
                        },
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .child(Self::svg("icons/ui/refresh-cw.svg", theme::DANGER))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::DANGER)
                                    .child("Reconnect"),
                            ),
                    ))
                });

            let geom = if cols > 0 && rows > 0 {
                format!("{cols}×{rows}")
            } else {
                String::new()
            };
            (left.into_any_element(), geom)
        } else {
            let left = div()
                .flex()
                .items_center()
                .px(px(theme::SPACE_2))
                .gap(px(theme::SPACE_1))
                .child(Self::svg("icons/ui/terminal.svg", theme::TEXT_DISABLED))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_DISABLED)
                        .child("No session"),
                );
            (left.into_any_element(), String::new())
        };

        // Zed-style: panel toggle is the leftmost status-bar control.
        let panel_toggle = {
            use crate::ui::tooltip::Tooltip;
            div()
                .id("sb-sidebar")
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .px(px(theme::SPACE_2))
                .rounded(px(theme::RADIUS_SM))
                .cursor_pointer()
                .when(sidebar_visible, |d| d.bg(theme::HOVER))
                .hover(|s| s.bg(theme::HOVER))
                .tooltip(|_, cx| Tooltip::with_key("Toggle Sidebar", "Ctrl+B", cx))
                .child(Self::svg(
                    "icons/ui/panel-left.svg",
                    if sidebar_visible {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTED
                    },
                ))
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(StatusBarEvent::ToggleSidebar);
                }))
        };

        div()
            .id("status-bar")
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(px(theme::STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .px(px(theme::STATUS_BAR_PAD_X))
            .bg(theme::PANEL_BG)
            .border_t_1()
            .border_color(theme::BORDER_SUBTLE)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap(px(2.0))
                    .child(panel_toggle)
                    .child(session_left),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap(px(2.0))
                    .when_some(toast, |d, msg| {
                        d.child(
                            div()
                                .id("sb-toast")
                                .px(px(theme::SPACE_2))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::SUCCESS)
                                .cursor_pointer()
                                .hover(|s| s.text_color(theme::TEXT))
                                .child(msg)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clear_toast(cx);
                                })),
                        )
                    })
                    .when(!right_geom.is_empty(), |d| {
                        d.child(
                            div()
                                .px(px(theme::SPACE_2))
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .font_family(theme_font_mono())
                                .child(right_geom),
                        )
                    })
                    .child(Self::segment(
                        "sb-font",
                        cx,
                        "Settings",
                        Some("Ctrl+,"),
                        |_, _, cx| {
                            cx.emit(StatusBarEvent::OpenSettings);
                        },
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .child(Self::svg("icons/ui/settings.svg", theme::TEXT_MUTED))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(format!("{font_size:.0}")),
                            ),
                    ))
                    .child({
                        use crate::ui::tooltip::Tooltip;
                        div()
                            .id("sb-context")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h_full()
                            .px(px(theme::SPACE_2))
                            .rounded(px(theme::RADIUS_SM))
                            .cursor_pointer()
                            .when(context_visible, |d| d.bg(theme::HOVER))
                            .hover(|s| s.bg(theme::HOVER))
                            .tooltip(|_, cx| {
                                Tooltip::with_key("Toggle Context Panel", "Ctrl+Shift+B", cx)
                            })
                            .child(Self::svg(
                                "icons/ui/panel-right.svg",
                                if context_visible {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_MUTED
                                },
                            ))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(StatusBarEvent::ToggleContextPanel);
                            }))
                    }),
            )
    }
}

fn theme_font_mono() -> SharedString {
    // Match terminal-ish digits without pulling platform module into render path noise.
    if cfg!(windows) {
        "Consolas".into()
    } else {
        "Menlo".into()
    }
}

/// Prefer the full path; if over `max_chars`, keep the prefix and ellipsize the tail.
fn truncate_path_end(path: &str, max_chars: usize) -> String {
    let count = path.chars().count();
    if count <= max_chars {
        return path.to_string();
    }
    if max_chars <= 1 {
        return "…".into();
    }
    let keep = max_chars - 1;
    let head: String = path.chars().take(keep).collect();
    format!("{head}…")
}

impl EventEmitter<StatusBarEvent> for StatusBar {}
