use gpui::prelude::*;
use gpui::*;

use crate::model::{ProfileKind, export_workspace_to, import_workspace_from};
use crate::shared::actions::*;
use crate::shared::theme;
use crate::ui::app_bus::{AppBus, AppBusEvent};
use crate::ui::context_panel::{ContextPanel, ContextPanelEvent};
use crate::ui::password_prompt::{PasswordPrompt, PasswordPromptEvent};
use crate::ui::persistence::Persistence;
use crate::ui::settings::{SettingsEvent, SettingsPanel};
use crate::ui::sidebar::{Sidebar, SidebarEvent};
use crate::ui::ssh_form::{SshForm, SshFormEvent};
use crate::ui::status_bar::{StatusBar, StatusBarEvent};
use crate::ui::tab_bar::{TabBar, TabBarEvent};
use crate::ui::tab_manager::TabManager;
use crate::ui::terminal_pane::TerminalPane;
use crate::ui::workspace_store::{Selection, WorkspaceStore};

struct ContextResizeDrag {
    start_x: f32,
    start_width: f32,
}

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    store: Entity<WorkspaceStore>,
    app_bus: Entity<AppBus>,
    persistence: Entity<Persistence>,
    tabs: Entity<TabManager>,
    sidebar: Entity<Sidebar>,
    tab_bar: Entity<TabBar>,
    terminal_pane: Entity<TerminalPane>,
    context_panel: Entity<ContextPanel>,
    status_bar: Entity<StatusBar>,
    settings: Entity<SettingsPanel>,
    ssh_form: Entity<SshForm>,
    password_prompt: Option<Entity<PasswordPrompt>>,
    /// When set, password submit reconnects this tab instead of opening a new one.
    pending_reconnect_tab: Option<uuid::Uuid>,
    sidebar_width: f32,
    sidebar_visible: bool,
    resizing_sidebar: bool,
    context_panel_width: f32,
    context_panel_visible: bool,
    context_resize: Option<ContextResizeDrag>,
    show_settings: bool,
    show_ssh_form: bool,
    restore_profiles: Vec<uuid::Uuid>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(WorkspaceStore::load);
        let font_size = store.read(cx).ui_state.font_size.max(
            store.read(cx).settings.font_size,
        );
        let sidebar_width = store.read(cx).ui_state.sidebar_width.max(0.0);
        let sidebar_visible = store.read(cx).ui_state.sidebar_visible;
        let context_panel_width = store.read(cx).ui_state.context_panel_width.max(0.0);
        let context_panel_visible = store.read(cx).ui_state.context_panel_visible;
        let restore_profiles: Vec<uuid::Uuid> = store
            .read(cx)
            .ui_state
            .open_tabs
            .iter()
            .map(|t| t.profile_id)
            .collect();

        let show_line_numbers = store.read(cx).settings.show_line_numbers;
        let ansi_palette = store.read(cx).settings.ansi_palette;
        let app_bus = cx.new(|_| AppBus);
        let tabs = cx.new(|_cx| {
            TabManager::new(font_size, show_line_numbers, ansi_palette, app_bus.clone())
        });
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), cx));
        let tab_bar = cx.new(|cx| TabBar::new(tabs.clone(), store.clone(), cx));
        let terminal_pane = cx.new(|cx| TerminalPane::new(tabs.clone(), cx));
        let context_panel = cx.new(|cx| ContextPanel::new(store.clone(), tabs.clone(), cx));
        let status_bar = cx.new(|cx| StatusBar::new(store.clone(), tabs.clone(), cx));
        let settings = cx.new(|cx| SettingsPanel::new(store.clone(), cx));
        let ssh_form = cx.new(|cx| SshForm::new(store.clone(), cx));
        let workspace_weak = cx.weak_entity();
        let persistence = cx.new(|cx| {
            Persistence::new(app_bus.clone(), store.clone(), workspace_weak, cx)
        });

        // Scheme B: block close until Persistence flushes via WillQuit, then quit.
        let bus_for_close = app_bus.clone();
        let persistence_for_close = persistence.clone();
        window.on_window_should_close(cx, move |_window, cx| {
            if persistence_for_close.read(cx).allow_window_close() {
                return true;
            }
            AppBus::emit(&bus_for_close, AppBusEvent::WillQuit, cx);
            false
        });

        let mut view = Self {
            focus_handle: cx.focus_handle(),
            store: store.clone(),
            app_bus: app_bus.clone(),
            persistence: persistence.clone(),
            tabs: tabs.clone(),
            sidebar: sidebar.clone(),
            tab_bar: tab_bar.clone(),
            terminal_pane: terminal_pane.clone(),
            context_panel: context_panel.clone(),
            status_bar: status_bar.clone(),
            settings: settings.clone(),
            ssh_form: ssh_form.clone(),
            password_prompt: None,
            pending_reconnect_tab: None,
            sidebar_width,
            sidebar_visible,
            resizing_sidebar: false,
            context_panel_width,
            context_panel_visible,
            context_resize: None,
            show_settings: false,
            show_ssh_form: false,
            restore_profiles,
            _subscriptions: Vec::new(),
        };

        view._subscriptions.push(cx.subscribe_in(
            &app_bus,
            window,
            |this, _, event, window, cx| match event {
                AppBusEvent::SplitPane { pane_id, direction } => {
                    let pane_id = *pane_id;
                    let direction = *direction;
                    let store = this.store.clone();
                    this.tabs.update(cx, |m, cx| {
                        m.split_pane(pane_id, direction, &store, window, cx);
                    });
                }
                _ => {}
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &sidebar,
            window,
            |this, _, event, window, cx| match event {
                SidebarEvent::OpenProfile(id) => this.open_profile_id(*id, window, cx),
                SidebarEvent::ShowProfileMenu(_) => {}
                SidebarEvent::OpenSettings => {
                    this.show_settings = true;
                    this.show_ssh_form = false;
                    this.password_prompt = None;
                    cx.notify();
                }
                SidebarEvent::OpenSshForm => {
                    this.ssh_form.update(cx, |f, cx| f.reset(cx));
                    this.show_ssh_form = true;
                    this.show_settings = false;
                    this.password_prompt = None;
                    cx.notify();
                    cx.defer_in(window, |this, window, cx| {
                        this.ssh_form.read(cx).focus(window);
                    });
                }
                SidebarEvent::EditSshProfile(id) => {
                    let id = *id;
                    this.ssh_form.update(cx, |f, cx| f.load_for_edit(id, cx));
                    this.show_ssh_form = true;
                    this.show_settings = false;
                    this.password_prompt = None;
                    cx.notify();
                    cx.defer_in(window, |this, window, cx| {
                        this.ssh_form.read(cx).focus(window);
                    });
                }
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &ssh_form,
            window,
            move |this, _, event: &SshFormEvent, window, cx| match event {
                SshFormEvent::Close => {
                    this.show_ssh_form = false;
                    cx.notify();
                }
                SshFormEvent::Saved {
                    profile_id,
                    connect,
                    oneshot_password,
                } => {
                    this.show_ssh_form = false;
                    if *connect {
                        this.connect_ssh(*profile_id, oneshot_password.clone(), window, cx);
                    }
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
                TabBarEvent::Split(direction) => {
                    let store = this.store.clone();
                    let direction = *direction;
                    this.tabs.update(cx, |m, cx| {
                        m.split_focused(direction, &store, window, cx);
                    });
                }
                TabBarEvent::DuplicateProfile(_profile_id) => {
                    let store = this.store.clone();
                    this.tabs.update(cx, |m, cx| {
                        m.duplicate_active_ephemeral(&store, window, cx);
                    });
                    this.persist_tabs(cx);
                }
                TabBarEvent::SaveTab { tab_id, group_id } => {
                    this.save_tab_to_group(*tab_id, *group_id, cx);
                }
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &status_bar,
            window,
            |this, _, event, window, cx| match event {
                StatusBarEvent::Reconnect(id) => {
                    let tab_id = *id;
                    let profile = {
                        let tabs = this.tabs.read(cx);
                        tabs.tabs
                            .iter()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.focused_pane())
                            .and_then(|p| {
                                p.profile_id.and_then(|pid| {
                                    this.store
                                        .read(cx)
                                        .workspace
                                        .find_profile(pid)
                                        .cloned()
                                })
                            })
                    };
                    if let Some(profile) = profile {
                        if TabManager::ssh_needs_password(&profile) {
                            this.pending_reconnect_tab = Some(tab_id);
                            this.show_password_prompt(
                                profile.id,
                                profile.name.clone(),
                                window,
                                cx,
                            );
                            return;
                        }
                    }
                    this.pending_reconnect_tab = None;
                    let store = this.store.clone();
                    this.tabs.update(cx, |m, cx| {
                        m.reconnect(tab_id, &store, window, cx);
                    });
                }
                StatusBarEvent::OpenSettings => {
                    this.show_settings = true;
                    this.show_ssh_form = false;
                    this.password_prompt = None;
                    cx.notify();
                }
                StatusBarEvent::EditSshProfile(id) => {
                    let id = *id;
                    this.ssh_form.update(cx, |f, cx| f.load_for_edit(id, cx));
                    this.show_ssh_form = true;
                    this.show_settings = false;
                    this.password_prompt = None;
                    cx.notify();
                    cx.defer_in(window, |this, window, cx| {
                        this.ssh_form.read(cx).focus(window);
                    });
                }
                StatusBarEvent::ToggleSidebar => {
                    this.toggle_sidebar(window, cx);
                }
                StatusBarEvent::ToggleContextPanel => {
                    this.toggle_context_panel(cx);
                }
            },
        ));

        view._subscriptions.push(cx.subscribe_in(
            &context_panel,
            window,
            |this, _, event, _window, cx| match event {
                ContextPanelEvent::Toast(msg) => {
                    this.set_toast(msg.clone(), cx);
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
                SettingsEvent::LineNumbersChanged(enabled) => {
                    this.tabs
                        .update(cx, |m, cx| m.set_show_line_numbers(*enabled, cx));
                }
                SettingsEvent::AnsiPaletteChanged(palette) => {
                    this.tabs
                        .update(cx, |m, cx| m.set_ansi_palette(*palette, cx));
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
        let Some(profile) = profile else {
            return;
        };

        if TabManager::ssh_needs_password(&profile) {
            self.show_password_prompt(profile.id, profile.name.clone(), window, cx);
            return;
        }

        let store = self.store.clone();
        self.tabs.update(cx, |m, cx| {
            m.open_profile(
                &profile,
                default_shell.as_deref(),
                &font_family,
                &store,
                window,
                cx,
            );
        });
        self.persist_tabs(cx);
    }

    fn connect_ssh(
        &mut self,
        profile_id: uuid::Uuid,
        oneshot_password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (profile, font_family) = {
            let s = self.store.read(cx);
            (
                s.workspace.find_profile(profile_id).cloned(),
                s.settings.font_family.clone(),
            )
        };
        let Some(profile) = profile else {
            return;
        };

        if let Some(password) = oneshot_password {
            self.tabs.update(cx, |m, cx| {
                m.open_ssh_with_password(&profile, password, &font_family, cx);
            });
            self.persist_tabs(cx);
            return;
        }

        if TabManager::ssh_needs_password(&profile) {
            self.show_password_prompt(profile.id, profile.name.clone(), window, cx);
            return;
        }

        self.tabs.update(cx, |m, cx| {
            m.open_ssh_profile(&profile, &font_family, cx);
        });
        self.persist_tabs(cx);
    }

    fn show_password_prompt(
        &mut self,
        profile_id: uuid::Uuid,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = cx.new(|cx| PasswordPrompt::new(profile_id, title, cx));
        self._subscriptions.push(cx.subscribe(&prompt, {
            move |this, _, event: &PasswordPromptEvent, cx| match event {
                PasswordPromptEvent::Cancel => {
                    this.password_prompt = None;
                    this.pending_reconnect_tab = None;
                    cx.notify();
                }
                PasswordPromptEvent::Submit {
                    profile_id,
                    password,
                } => {
                    this.password_prompt = None;
                    let reconnect_tab = this.pending_reconnect_tab.take();
                    let (profile, font_family) = {
                        let s = this.store.read(cx);
                        (
                            s.workspace.find_profile(*profile_id).cloned(),
                            s.settings.font_family.clone(),
                        )
                    };
                    if let Some(profile) = profile {
                        if let Some(tab_id) = reconnect_tab {
                            let store = this.store.clone();
                            let password = password.clone();
                            this.tabs.update(cx, |m, cx| {
                                m.reconnect_with_password(tab_id, password, &store, cx);
                            });
                        } else {
                            this.tabs.update(cx, |m, cx| {
                                m.open_ssh_with_password(
                                    &profile,
                                    password.clone(),
                                    &font_family,
                                    cx,
                                );
                            });
                            this.persist_tabs(cx);
                        }
                    }
                    cx.notify();
                }
            }
        }));
        self.password_prompt = Some(prompt);
        self.show_settings = false;
        self.show_ssh_form = false;
        cx.notify();
        cx.defer_in(window, |this, window, cx| {
            if let Some(prompt) = this.password_prompt.as_ref() {
                prompt.read(cx).focus(window);
            }
        });
    }

    fn new_local_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        self.tabs.update(cx, |m, cx| {
            m.open_ephemeral_local(&store, window, cx);
        });
        self.persist_tabs(cx);
    }

    /// Persist an ephemeral tab as a Profile under root or a group.
    fn save_tab_to_group(
        &mut self,
        tab_id: uuid::Uuid,
        group_id: Option<uuid::Uuid>,
        cx: &mut Context<Self>,
    ) {
        use crate::model::Profile;
        use crate::ui::workspace_store::InsertTarget;

        let Some((mut kind, label, live_cwd)) =
            self.tabs.read(cx).tabs.iter().find(|t| t.id == tab_id).and_then(|t| {
                t.panes.get(&t.focused).map(|p| {
                    let cwd = p
                        .terminal
                        .as_ref()
                        .and_then(|term| term.read(cx).working_directory());
                    (p.kind.clone(), p.label.clone(), cwd)
                })
            })
        else {
            return;
        };
        // Already bound — nothing to save.
        if self
            .tabs
            .read(cx)
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.panes.get(&t.focused))
            .and_then(|p| p.profile_id)
            .is_some()
        {
            return;
        }

        if let (ProfileKind::Local { cwd: c, .. }, Some(path)) = (&mut kind, live_cwd) {
            *c = Some(path);
        }

        let names = self.store.read(cx).workspace.all_profile_names();
        let mut name = label;
        if names.iter().any(|n| n == &name) {
            let mut n = 2u32;
            loop {
                let candidate = format!("{name} ({n})");
                if !names.iter().any(|e| e == &candidate) {
                    name = candidate;
                    break;
                }
                n += 1;
            }
        }
        let profile = Profile {
            id: uuid::Uuid::new_v4(),
            name: name.clone(),
            kind,
        };
        let pid = profile.id;
        let target = match group_id {
            Some(gid) => InsertTarget::Group(gid),
            None => InsertTarget::Root,
        };
        self.store.update(cx, |s, cx| {
            s.place_profile(profile, target, cx);
        });
        self.tabs.update(cx, |m, cx| {
            m.bind_focused_to_profile(tab_id, pid, name, cx);
        });
        self.persist_tabs(cx);
    }

    /// Flush open tabs + Bound Local cwds to disk (also used on window release).
    pub fn flush_persist(&mut self, cx: &mut App) {
        let sidebar_width = self.sidebar_width;
        let sidebar_visible = self.sidebar_visible;
        let context_panel_width = self.context_panel_width;
        let context_panel_visible = self.context_panel_visible;
        let (tabs, active, font_size, local_cwds) = self.tabs.update(cx, |m, cx| {
            let cwds = m.bound_local_cwds(cx);
            let (tabs, active) = m.snapshot_for_persist();
            (tabs, active, m.font_size, cwds)
        });
        self.store.update(cx, |s, cx| {
            for (pid, cwd) in local_cwds {
                s.update_local_profile_cwd(pid, cwd, cx);
            }
            s.ui_state.sidebar_width = sidebar_width;
            s.ui_state.sidebar_visible = sidebar_visible;
            s.ui_state.context_panel_width = context_panel_width;
            s.ui_state.context_panel_visible = context_panel_visible;
            s.ui_state.font_size = font_size;
            s.settings.font_size = font_size;
            s.sync_open_tabs(&tabs, active);
            s.persist_now();
        });
    }

    fn persist_tabs(&mut self, cx: &mut Context<Self>) {
        AppBus::emit(&self.app_bus, AppBusEvent::PersistRequested, cx);
    }

    fn request_quit(&mut self, cx: &mut Context<Self>) {
        AppBus::emit(&self.app_bus, AppBusEvent::WillQuit, cx);
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        let visible = self.sidebar_visible;
        self.store.update(cx, |s, cx| {
            s.ui_state.sidebar_visible = visible;
            s.mark_dirty();
            cx.notify();
        });
        self.persist_tabs(cx);
        if visible {
            self.sidebar.read(cx).focus_handle(cx).focus(window);
        } else if let Some(term) = self
            .tabs
            .read(cx)
            .active_tab()
            .and_then(|t| t.focused_pane())
            .and_then(|p| p.terminal.as_ref())
        {
            term.read(cx).focus_handle().focus(window);
        }
        cx.notify();
    }

    fn toggle_context_panel(&mut self, cx: &mut Context<Self>) {
        self.context_panel_visible = !self.context_panel_visible;
        let visible = self.context_panel_visible;
        self.store.update(cx, |s, cx| {
            s.ui_state.context_panel_visible = visible;
            s.mark_dirty();
            cx.notify();
        });
        self.persist_tabs(cx);
        cx.notify();
    }

    fn set_toast(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        let msg = msg.into();
        self.status_bar.update(cx, |bar, cx| bar.set_toast(msg, cx));
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
                                this.set_toast(format!("Exported to {}", path.display()), cx);
                            }
                            Err(err) => {
                                this.set_toast(format!("Export failed: {err:#}"), cx);
                            }
                        }
                    }
                    None => {
                        this.set_toast("Export cancelled", cx);
                    }
                }
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
                                this.set_toast(format!("Imported from {}", path.display()), cx);
                            }
                            Err(err) => {
                                this.set_toast(format!("Import failed: {err:#}"), cx);
                            }
                        }
                    }
                    None => {
                        this.set_toast("Import cancelled", cx);
                    }
                }
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
        let tab_menus_open = self.tab_bar.read(cx).has_open_menu();
        div()
            .key_context("Loom")
            // Do not track_focus on the root — that steals keyboard focus from the
            // terminal. App shortcuts still dispatch via key_context while a child
            // (terminal / sidebar) holds focus.
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(theme::BG)
            .text_color(theme::TEXT)
            .on_action(cx.listener(|this, _: &NewLocalTab, window, cx| {
                this.new_local_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                // Zed-like: close focused pane first; last pane closes the tab.
                this.tabs.update(cx, |m, cx| m.close_active(window, cx));
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &SplitLeft, window, cx| {
                let store = this.store.clone();
                this.tabs.update(cx, |m, cx| {
                    m.split_focused(
                        crate::ui::pane_layout::SplitDirection::Left,
                        &store,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                let store = this.store.clone();
                this.tabs.update(cx, |m, cx| {
                    m.split_focused(
                        crate::ui::pane_layout::SplitDirection::Right,
                        &store,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &SplitUp, window, cx| {
                let store = this.store.clone();
                this.tabs.update(cx, |m, cx| {
                    m.split_focused(
                        crate::ui::pane_layout::SplitDirection::Up,
                        &store,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                let store = this.store.clone();
                this.tabs.update(cx, |m, cx| {
                    m.split_focused(
                        crate::ui::pane_layout::SplitDirection::Down,
                        &store,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &ActivatePaneLeft, window, cx| {
                this.tabs.update(cx, |m, cx| {
                    m.activate_pane_in_direction(
                        crate::ui::pane_layout::SplitDirection::Left,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &ActivatePaneRight, window, cx| {
                this.tabs.update(cx, |m, cx| {
                    m.activate_pane_in_direction(
                        crate::ui::pane_layout::SplitDirection::Right,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &ActivatePaneUp, window, cx| {
                this.tabs.update(cx, |m, cx| {
                    m.activate_pane_in_direction(
                        crate::ui::pane_layout::SplitDirection::Up,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, _: &ActivatePaneDown, window, cx| {
                this.tabs.update(cx, |m, cx| {
                    m.activate_pane_in_direction(
                        crate::ui::pane_layout::SplitDirection::Down,
                        window,
                        cx,
                    );
                });
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
                this.tabs.update(cx, |m, cx| {
                    m.duplicate_active_ephemeral(&store, window, cx);
                });
                this.persist_tabs(cx);
            }))
            .on_action(cx.listener(|this, _: &SaveWorkspace, _, cx| {
                this.persistence.update(cx, |p, cx| p.flush_now(cx));
                this.set_toast("Workspace saved", cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSettings, _, cx| {
                this.show_settings = !this.show_settings;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.toggle_sidebar(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleContextPanel, _, cx| {
                this.toggle_context_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &ExportWorkspace, _, cx| {
                this.export_workspace(cx);
            }))
            .on_action(cx.listener(|this, _: &ImportWorkspace, _, cx| {
                this.import_workspace(cx);
            }))
            .on_action(cx.listener(|this, _: &QuitApp, _, cx| {
                this.request_quit(cx);
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
            .on_action(cx.listener(|this, _: &RenameFocused, window, cx| {
                // Binding is `Renamable` (profile/group selected + sidebar focused).
                match this.store.read(cx).selection {
                    Selection::Profile(_) | Selection::Group(_) => {
                        this.sidebar.update(cx, |s, cx| {
                            s.begin_rename(cx);
                        });
                        this.sidebar.read(cx).focus_handle(cx).focus(window);
                    }
                    Selection::None => {}
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut changed = false;
                    if this.resizing_sidebar {
                        this.resizing_sidebar = false;
                        changed = true;
                    }
                    if this.context_resize.take().is_some() {
                        changed = true;
                    }
                    if changed {
                        this.persist_tabs(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.resizing_sidebar {
                    this.sidebar_width = f32::from(event.position.x).max(0.0);
                    cx.notify();
                }
                if let Some(drag) = this.context_resize.as_ref() {
                    let x: f32 = event.position.x.into();
                    let dx = drag.start_x - x;
                    this.context_panel_width = (drag.start_width + dx).max(0.0);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .when(self.sidebar_visible, |d| {
                        d.child(
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
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(self.tab_bar.clone())
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.terminal_pane.clone()),
                            ),
                    )
                    .when(self.context_panel_visible, |d| {
                        d.child(
                            div()
                                .id("context-resizer")
                                .w(px(4.0))
                                .h_full()
                                .cursor(CursorStyle::ResizeColumn)
                                .hover(|s| s.bg(theme::ACCENT))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                        this.context_resize = Some(ContextResizeDrag {
                                            start_x: event.position.x.into(),
                                            start_width: this.context_panel_width,
                                        });
                                        cx.notify();
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .w(px(self.context_panel_width))
                                .h_full()
                                .child(self.context_panel.clone()),
                        )
                    }),
            )
            .child(self.status_bar.clone())
            .when(self.show_settings, |d| d.child(self.settings.clone()))
            .when(self.show_ssh_form, |d| d.child(self.ssh_form.clone()))
            .when_some(self.password_prompt.clone(), |d, prompt| d.child(prompt))
            // Full-window backdrop under TabBar menus (priority 0); menu uses priority 1.
            .when(tab_menus_open, |d| {
                d.child(
                    deferred(
                        div()
                            .id("tab-menu-backdrop")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.tab_bar.update(cx, |tb, cx| tb.close_all_menus(cx));
                                    cx.stop_propagation();
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, _, _, cx| {
                                    this.tab_bar.update(cx, |tb, cx| tb.close_all_menus(cx));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .with_priority(0),
                )
            })
    }
}
