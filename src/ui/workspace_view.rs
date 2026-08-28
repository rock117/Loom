use gpui::prelude::*;
use gpui::*;

use crate::model::{export_workspace_to, import_workspace_from};
use crate::shared::actions::*;
use crate::shared::theme;
use crate::ui::settings::{SettingsEvent, SettingsPanel};
use crate::ui::sidebar::{Sidebar, SidebarEvent};
use crate::ui::tab_bar::{TabBar, TabBarEvent};
use crate::ui::tab_manager::TabManager;
use crate::ui::terminal_pane::{TerminalPane, TerminalPaneEvent};
use crate::ui::workspace_store::{Selection, WorkspaceStore};

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    store: Entity<WorkspaceStore>,
    tabs: Entity<TabManager>,
    sidebar: Entity<Sidebar>,
    tab_bar: Entity<TabBar>,
    terminal_pane: Entity<TerminalPane>,
    settings: Entity<SettingsPanel>,
    sidebar_width: f32,
    resizing_sidebar: bool,
    show_settings: bool,
    status_message: Option<SharedString>,
    restore_profiles: Vec<uuid::Uuid>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(WorkspaceStore::load);
        let font_size = store.read(cx).ui_state.font_size.max(
            store.read(cx).settings.font_size,
        );
        let sidebar_width = store
            .read(cx)
            .ui_state
            .sidebar_width
            .clamp(theme::SIDEBAR_MIN, theme::SIDEBAR_MAX);
        let restore_profiles: Vec<uuid::Uuid> = store
            .read(cx)
            .ui_state
            .open_tabs
            .iter()
            .map(|t| t.profile_id)
            .collect();

        let tabs = cx.new(|_cx| TabManager::new(font_size));
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), cx));
        let tab_bar = cx.new(|cx| TabBar::new(tabs.clone(), cx));
        let terminal_pane = cx.new(|cx| TerminalPane::new(tabs.clone(), cx));
        let settings = cx.new(|cx| SettingsPanel::new(store.clone(), cx));

        let mut view = Self {
            focus_handle: cx.focus_handle(),
            store: store.clone(),
            tabs: tabs.clone(),
            sidebar: sidebar.clone(),
            tab_bar: tab_bar.clone(),
            terminal_pane: terminal_pane.clone(),
            settings: settings.clone(),
            sidebar_width,
            resizing_sidebar: false,
            show_settings: false,
            status_message: None,
            restore_profiles,
            _subscriptions: Vec::new(),
        };

        view._subscriptions.push(cx.subscribe_in(
            &sidebar,
            window,
            |this, _, event, window, cx| match event {
                SidebarEvent::OpenProfile(id) => this.open_profile_id(*id, window, cx),
                SidebarEvent::ShowProfileMenu(_) => {}
                SidebarEvent::OpenSettings => {
                    this.show_settings = true;
                    cx.notify();
                }
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &tab_bar,
            window,
            |this, _, event, window, cx| match event {
                TabBarEvent::NewTab => this.new_local_tab(window, cx),
                TabBarEvent::Changed => this.persist_tabs(cx),
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &terminal_pane,
            window,
            |this, _, event, window, cx| match event {
                TerminalPaneEvent::Reconnect(id) => {
                    let store = this.store.clone();
                    this.tabs.update(cx, |m, cx| {
                        m.reconnect(*id, &store, window, cx);
                    });
                }
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &settings,
            window,
            |this, _, event, _window, cx| match event {
                SettingsEvent::Close => {
                    this.show_settings = false;
                    cx.notify();
                }
                SettingsEvent::Export => this.export_workspace(cx),
                SettingsEvent::Import => this.import_workspace(cx),
                SettingsEvent::FontSizeChanged(size) => {
                    this.tabs.update(cx, |m, cx| m.set_font_size(*size, cx));
                    this.persist_tabs(cx);
                }
            },
        ));

        let to_restore = view.restore_profiles.clone();
        view.restore_profiles.clear();
        cx.defer_in(window, move |this, window, cx| {
            for pid in to_restore {
                this.open_profile_id(pid, window, cx);
            }
        });

        view
    }

    fn open_profile_id(&mut self, id: uuid::Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let (profile, default_shell, font_family) = {
            let s = self.store.read(cx);
            (
                s.workspace.find_profile(id).cloned(),
                s.settings.default_shell.clone(),
                s.settings.font_family.clone(),
            )
        };
        if let Some(profile) = profile {
            self.tabs.update(cx, |m, cx| {
                m.open_profile(
                    &profile,
                    default_shell.as_deref(),
                    &font_family,
                    window,
                    cx,
                );
            });
            self.persist_tabs(cx);
        }
    }

    fn new_local_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.store.read(cx).default_local_profile_id().or_else(|| {
            self.store.update(cx, |s, cx| s.add_local_profile(cx))
        });
        if let Some(id) = id {
            self.open_profile_id(id, window, cx);
        }
    }

    fn persist_tabs(&mut self, cx: &mut Context<Self>) {
        let sidebar_width = self.sidebar_width;
        let (tabs, active, font_size) = {
            let manager = self.tabs.read(cx);
            let (tabs, active) = manager.snapshot_for_persist();
            (tabs, active, manager.font_size)
        };
        self.store.update(cx, |s, _cx| {
            s.ui_state.sidebar_width = sidebar_width;
            s.ui_state.font_size = font_size;
            s.settings.font_size = font_size;
            s.sync_open_tabs(&tabs, active);
            s.persist_now();
        });
    }

    fn export_workspace(&mut self, cx: &mut Context<Self>) {
        let ws = self.store.read(cx).workspace.clone();
        cx.spawn(async move |this, cx| {
            let path = rfd::AsyncFileDialog::new()
                .set_title("Export Loom workspace")
                .set_file_name("loom-workspace.json")
                .add_filter("JSON", &["json"])
                .save_file()
                .await;

            this.update(cx, |this, cx| {
                match path {
                    Some(handle) => {
                        let path = handle.path().to_path_buf();
                        match export_workspace_to(&path, &ws) {
                            Ok(()) => {
                                this.status_message =
                                    Some(format!("Exported to {}", path.display()).into());
                            }
                            Err(err) => {
                                this.status_message =
                                    Some(format!("Export failed: {err:#}").into());
                            }
                        }
                    }
                    None => {
                        this.status_message = Some("Export cancelled".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn import_workspace(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let path = rfd::AsyncFileDialog::new()
                .set_title("Import Loom workspace (replaces current)")
                .add_filter("JSON", &["json"])
                .pick_file()
                .await;

            this.update(cx, |this, cx| {
                match path {
                    Some(handle) => {
                        let path = handle.path().to_path_buf();
                        match import_workspace_from(&path) {
                            Ok(ws) => {
                                this.store.update(cx, |s, cx| s.replace_workspace(ws, cx));
                                this.tabs.update(cx, |m, cx| {
                                    m.tabs.clear();
                                    m.active = None;
                                    cx.notify();
                                });
                                this.show_settings = false;
                                this.status_message =
                                    Some(format!("Imported from {}", path.display()).into());
                            }
                            Err(err) => {
                                this.status_message =
                                    Some(format!("Import failed: {err:#}").into());
                            }
                        }
                    }
                    None => {
                        this.status_message = Some("Import cancelled".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status_message.clone();

        div()
            .key_context("Loom")
            // Do not track_focus on the root — that steals keyboard focus from the
            // terminal. App shortcuts still dispatch via key_context while a child
            // (terminal / sidebar) holds focus.
            .size_full()
            .flex()
            .relative()
            .bg(theme::BG)
            .text_color(theme::TEXT)
            .on_action(cx.listener(|this, _: &NewLocalTab, window, cx| {
                this.new_local_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                this.tabs.update(cx, |m, cx| m.close_active(cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.tabs.update(cx, |m, cx| m.next_tab(window, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                this.tabs.update(cx, |m, cx| m.prev_tab(window, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &DuplicateTab, window, cx| {
                let store = this.store.clone();
                this.tabs
                    .update(cx, |m, cx| m.duplicate_active(&store, window, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &SaveWorkspace, _, cx| {
                this.persist_tabs(cx);
                this.store.update(cx, |s, _| s.persist_now());
                this.status_message = Some("Workspace saved".into());
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.sidebar.update(cx, |s, cx| s.focus_search(window, cx));
            }))
            .on_action(cx.listener(|this, _: &ToggleSettings, _, cx| {
                this.show_settings = !this.show_settings;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportWorkspace, _, cx| {
                this.export_workspace(cx);
            }))
            .on_action(cx.listener(|this, _: &ImportWorkspace, _, cx| {
                this.import_workspace(cx);
            }))
            .on_action(cx.listener(|_this, _: &QuitApp, _, cx| {
                cx.quit();
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                let size = this.tabs.read(cx).font_size + 1.0;
                this.tabs.update(cx, |m, cx| m.set_font_size(size, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                let size = this.tabs.read(cx).font_size - 1.0;
                this.tabs.update(cx, |m, cx| m.set_font_size(size, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.tabs.update(cx, |m, cx| m.set_font_size(14.0, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &RenameFocused, _, cx| {
                match this.store.read(cx).selection {
                    Selection::Profile(_) | Selection::Group(_) => {
                        this.store.update(cx, |s, cx| s.begin_rename(cx));
                    }
                    Selection::None => {
                        if let Some(title) =
                            this.tabs.update(cx, |m, cx| m.begin_rename_active(cx))
                        {
                            this.tabs.update(cx, |m, cx| {
                                m.apply_rename_active(format!("{title}*"), cx);
                            });
                            this.persist_tabs(cx);
                        }
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing_sidebar {
                        this.resizing_sidebar = false;
                        this.persist_tabs(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.resizing_sidebar {
                    this.sidebar_width = event
                        .position
                        .x
                        .clamp(px(theme::SIDEBAR_MIN), px(theme::SIDEBAR_MAX))
                        .into();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .w(px(self.sidebar_width))
                    .h_full()
                    .child(self.sidebar.clone()),
            )
            .child(
                div()
                    .id("sidebar-resizer")
                    .w(px(4.0))
                    .h_full()
                    .cursor(CursorStyle::ResizeColumn)
                    .hover(|s| s.bg(theme::ACCENT))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.resizing_sidebar = true;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.tab_bar.clone())
                    .child(div().flex_1().min_h_0().child(self.terminal_pane.clone()))
                    .when_some(status, |d, msg| {
                        d.child(
                            div()
                                .id("status-toast")
                                .px_3()
                                .py_1()
                                .bg(theme::PANEL_BG)
                                .border_t_1()
                                .border_color(theme::BORDER)
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .cursor_pointer()
                                .child(msg)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.status_message = None;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(self.show_settings, |d| d.child(self.settings.clone()))
    }
}
