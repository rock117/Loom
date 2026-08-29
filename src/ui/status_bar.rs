//! Window-bottom status bar (Zed / VS Code style).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{ConnectionState, ProfileKind};
use crate::shared::theme;
use crate::ui::tab_manager::TabManager;
use crate::ui::workspace_store::WorkspaceStore;

const ICON: f32 = 12.0;

pub struct StatusBar {
    pub store: Entity<WorkspaceStore>,
    pub tabs: Entity<TabManager>,
    toast: Option<SharedString>,
    _observe_store: Subscription,
    _observe_tabs: Subscription,
    _observe_terminal: Option<Subscription>,
}

#[derive(Clone, Debug)]
pub enum StatusBarEvent {
    Reconnect(Uuid),
    OpenSettings,
    EditSshProfile(Uuid),
    /// Toggle profiles sidebar visibility (Zed left-dock).
    ToggleSidebar,
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
            _observe_store,
            _observe_tabs,
            _observe_terminal: None,
        };
        bar.resync_terminal_observe(cx);
        bar
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
        cx.notify();
    }

    pub fn clear_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
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
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(child)
    }
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tabs.read(cx);
        let font_size = manager.font_size;
        let sidebar_visible = self.store.read(cx).ui_state.sidebar_visible;
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
            let profile_id = pane.map(|p| p.profile_id);
            let profile = profile_id.and_then(|pid| {
                self.store.read(cx).workspace.find_profile(pid).cloned()
            });

            let (kind_icon, kind_color, target, is_ssh) = match profile.as_ref().map(|p| &p.kind) {
                Some(ProfileKind::Ssh {
                    host, port, user, ..
                }) => (
                    "icons/ui/remote.svg",
                    theme::ICON_REMOTE,
                    format!("{user}@{host}:{port}"),
                    true,
                ),
                Some(ProfileKind::Local { shell, .. }) => {
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
                    (
                        "icons/ui/terminal.svg",
                        theme::ICON_LOCAL,
                        shell_label,
                        false,
                    )
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
                ConnectionState::Disconnected => "Disconnected",
                ConnectionState::Failed => "Failed",
                ConnectionState::Idle => "Idle",
            };

            let needs_reconnect = matches!(
                state,
                ConnectionState::Disconnected | ConnectionState::Failed
            );

            let (cols, rows) = pane
                .and_then(|p| p.terminal.as_ref())
                .map(|t| t.read(cx).dimensions())
                .unwrap_or((0, 0));

            let left = div()
                .flex()
                .items_center()
                .h_full()
                .gap(px(2.0))
                .child(Self::segment(
                    "sb-session",
                    cx,
                    move |_, _, cx| {
                        if is_ssh {
                            if let Some(profile_id) = profile_id {
                                cx.emit(StatusBarEvent::EditSshProfile(profile_id));
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
                .when(needs_reconnect, |d| {
                    d.child(Self::segment(
                        "sb-reconnect",
                        cx,
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
        let panel_toggle = div()
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
            }));

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
                                .text_color(theme::TEXT_MUTED)
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
                    )),
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

impl EventEmitter<StatusBarEvent> for StatusBar {}
