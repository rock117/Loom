use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;
use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};
use uuid::Uuid;

use crate::model::{ConnectionState, Profile, ProfileKind};
use crate::session::local::{LocalPty, resolve_shell};
use crate::shared::theme;
use crate::ui::workspace_store::WorkspaceStore;

pub struct TabSession {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub title: String,
    pub state: ConnectionState,
    pub status_message: String,
    pub terminal: Option<Entity<TerminalView>>,
    pub pty_master: Option<Arc<parking_lot::Mutex<Box<dyn portable_pty::MasterPty + Send>>>>,
}

pub struct TabManager {
    pub tabs: Vec<TabSession>,
    pub active: Option<Uuid>,
    pub font_size: f32,
}

impl TabManager {
    pub fn new(font_size: f32) -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            font_size,
        }
    }

    pub fn active_index(&self) -> usize {
        self.active
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .unwrap_or(0)
    }

    pub fn open_profile(
        &mut self,
        profile: &Profile,
        default_shell: Option<&str>,
        font_family: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !profile.kind.is_local() {
            let tab = TabSession {
                id: Uuid::new_v4(),
                profile_id: profile.id,
                title: profile.name.clone(),
                state: ConnectionState::Failed,
                status_message: "SSH is not enabled yet".into(),
                terminal: None,
                pty_master: None,
            };
            let id = tab.id;
            self.tabs.push(tab);
            self.active = Some(id);
            cx.notify();
            return;
        }

        match self.spawn_local(profile, default_shell, font_family, window, cx) {
            Ok(tab) => {
                let id = tab.id;
                self.tabs.push(tab);
                self.active = Some(id);
                cx.notify();
            }
            Err(err) => {
                let tab = TabSession {
                    id: Uuid::new_v4(),
                    profile_id: profile.id,
                    title: profile.name.clone(),
                    state: ConnectionState::Failed,
                    status_message: format!("{err:#}"),
                    terminal: None,
                    pty_master: None,
                };
                let id = tab.id;
                self.tabs.push(tab);
                self.active = Some(id);
                cx.notify();
            }
        }
    }

    fn spawn_local(
        &self,
        profile: &Profile,
        default_shell: Option<&str>,
        font_family: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<TabSession> {
        let ProfileKind::Local { shell, cwd, .. } = &profile.kind else {
            anyhow::bail!("not a local profile");
        };
        let configured = shell.as_deref().or(default_shell);
        let shell = resolve_shell(configured);
        let pty = LocalPty::spawn(&shell, cwd.as_deref())?;
        let master = pty.master.clone();
        let resize = LocalPty::resize_callback(master.clone());

        let config = terminal_config(self.font_size, font_family);
        let terminal = cx.new(|cx| {
            TerminalView::new(pty.writer, pty.reader, config, cx)
                .with_resize_callback(resize)
        });
        terminal.read(cx).focus_handle().focus(window);

        Ok(TabSession {
            id: Uuid::new_v4(),
            profile_id: profile.id,
            title: profile.name.clone(),
            state: ConnectionState::Connected,
            status_message: format!("local · {shell}"),
            terminal: Some(terminal),
            pty_master: Some(master),
        })
    }

    pub fn reconnect(
        &mut self,
        tab_id: Uuid,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let profile_id = self.tabs[idx].profile_id;
        let title = self.tabs[idx].title.clone();
        let (profile, default_shell, font_family) = {
            let s = store.read(cx);
            (
                s.workspace.find_profile(profile_id).cloned(),
                s.settings.default_shell.clone(),
                s.settings.font_family.clone(),
            )
        };
        let Some(profile) = profile else {
            self.tabs[idx].state = ConnectionState::Failed;
            self.tabs[idx].status_message = "profile missing".into();
            cx.notify();
            return;
        };

        self.tabs[idx].terminal = None;
        self.tabs[idx].pty_master = None;
        self.tabs[idx].state = ConnectionState::Connecting;
        self.tabs[idx].status_message = "reconnecting…".into();

        match self.spawn_local(
            &profile,
            default_shell.as_deref(),
            &font_family,
            window,
            cx,
        ) {
            Ok(mut fresh) => {
                fresh.id = tab_id;
                fresh.title = title;
                self.tabs[idx] = fresh;
            }
            Err(err) => {
                self.tabs[idx].state = ConnectionState::Failed;
                self.tabs[idx].status_message = format!("{err:#}");
            }
        }
        cx.notify();
    }

    pub fn close_active(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        self.close_tab(active, cx);
    }

    pub fn close_tab(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(pos);
        self.active = if self.tabs.is_empty() {
            None
        } else {
            Some(self.tabs[pos.min(self.tabs.len() - 1)].id)
        };
        cx.notify();
    }

    pub fn select_tab(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(id);
        if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
            if let Some(term) = &tab.terminal {
                term.read(cx).focus_handle().focus(window);
            }
        }
        cx.notify();
    }

    pub fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let i = self.active_index();
        let next = (i + 1) % self.tabs.len();
        let id = self.tabs[next].id;
        self.select_tab(id, window, cx);
    }

    pub fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let i = self.active_index();
        let prev = if i == 0 { self.tabs.len() - 1 } else { i - 1 };
        let id = self.tabs[prev].id;
        self.select_tab(id, window, cx);
    }

    pub fn duplicate_active(
        &mut self,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.active else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|t| t.id == active) else {
            return;
        };
        let profile_id = tab.profile_id;
        let profile = store.read(cx).workspace.find_profile(profile_id).cloned();
        let (default_shell, font_family) = {
            let s = store.read(cx);
            (s.settings.default_shell.clone(), s.settings.font_family.clone())
        };
        if let Some(profile) = profile {
            self.open_profile(
                &profile,
                default_shell.as_deref(),
                &font_family,
                window,
                cx,
            );
        }
    }

    pub fn begin_rename_active(&mut self, _cx: &mut Context<Self>) -> Option<String> {
        let active = self.active?;
        self.tabs
            .iter()
            .find(|t| t.id == active)
            .map(|t| t.title.clone())
    }

    pub fn apply_rename_active(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == active) {
            tab.title = title;
            cx.notify();
        }
    }

    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = size.clamp(8.0, 32.0);
        for tab in &self.tabs {
            if let Some(term) = &tab.terminal {
                term.update(cx, |terminal, cx| {
                    let mut config = terminal.config().clone();
                    config.font_size = px(self.font_size);
                    terminal.update_config(config, cx);
                });
            }
        }
        cx.notify();
    }

    pub fn snapshot_for_persist(&self) -> (Vec<(Uuid, Option<String>)>, usize) {
        let tabs = self
            .tabs
            .iter()
            .map(|t| (t.profile_id, Some(t.title.clone())))
            .collect();
        (tabs, self.active_index())
    }
}

fn terminal_config(font_size: f32, font_family: &str) -> TerminalConfig {
    let colors = ColorPalette::builder()
        .background(0x1A, 0x1A, 0x1C)
        .foreground(0xE8, 0xE6, 0xE3)
        .cursor(0xE8, 0xE6, 0xE3)
        .black(0x10, 0x10, 0x10)
        .red(0xEF, 0xA6, 0xA2)
        .green(0x80, 0xC9, 0x90)
        .yellow(0xA6, 0x94, 0x60)
        .blue(0xA3, 0xB8, 0xEF)
        .magenta(0xE6, 0xA3, 0xDC)
        .cyan(0x50, 0xCA, 0xCD)
        .white(0x80, 0x80, 0x80)
        .bright_black(0x39, 0x41, 0x4E)
        .bright_red(0xE0, 0xAF, 0x85)
        .bright_green(0x5A, 0xCC, 0xAF)
        .bright_yellow(0xC8, 0xC8, 0x74)
        .bright_blue(0xCC, 0xAC, 0xED)
        .bright_magenta(0xF2, 0xA1, 0xC2)
        .bright_cyan(0x74, 0xC3, 0xE4)
        .bright_white(0xC0, 0xC0, 0xC0)
        .build();

    let family = if font_family.trim().is_empty() {
        if cfg!(windows) {
            "Cascadia Mono"
        } else {
            "monospace"
        }
    } else {
        font_family
    };

    TerminalConfig {
        font_family: family.into(),
        font_size: px(font_size),
        cols: 80,
        rows: 24,
        scrollback: 10_000,
        line_height_multiplier: 1.15,
        padding: Edges::all(px(8.0)),
        colors,
    }
}

/// Small helper used by status strip colors.
pub fn state_color(state: ConnectionState) -> Hsla {
    match state {
        ConnectionState::Connected => theme::ACCENT,
        ConnectionState::Connecting => theme::TEXT_MUTED,
        ConnectionState::Failed => theme::DANGER,
        ConnectionState::Disconnected | ConnectionState::Idle => theme::TEXT_MUTED,
    }
}
