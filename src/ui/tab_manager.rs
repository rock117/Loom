use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;
use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};
use portable_pty::ChildKiller;
use uuid::Uuid;

use crate::model::{ConnectionState, Profile, ProfileKind};
use crate::platform;
use crate::session::local::{LocalPty, resolve_shell, teardown_pty};
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
    pub pty_killer: Option<Box<dyn ChildKiller + Send + Sync>>,
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
                pty_killer: None,
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
                    pty_killer: None,
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
        let killer = pty.killer;
        let resize = LocalPty::resize_callback(master.clone());

        let family = if font_family.trim().is_empty() {
            platform::monospace_font_family()
        } else {
            font_family
        };
        let config = terminal_config(self.font_size, family);
        let terminal = cx.new(|cx| {
            TerminalView::new(pty.writer, pty.reader, config, cx).with_resize_callback(resize)
        });
        terminal.read(cx).focus_handle().focus(window);

        let cwd_label = cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .or_else(|| LocalPty::default_cwd().map(|p| p.display().to_string()))
            .unwrap_or_else(|| ".".into());
        let shell_short = std::path::Path::new(&shell)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&shell);

        Ok(TabSession {
            id: Uuid::new_v4(),
            profile_id: profile.id,
            title: profile.name.clone(),
            state: ConnectionState::Connected,
            status_message: format!("{shell_short} · {cwd_label}"),
            terminal: Some(terminal),
            pty_master: Some(master),
            pty_killer: Some(killer),
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

        let old_master = self.tabs[idx].pty_master.take();
        let old_killer = self.tabs[idx].pty_killer.take();
        self.tabs[idx].terminal = None;
        teardown_pty(old_killer, old_master);

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
        let tab = self.tabs.remove(pos);
        let master = tab.pty_master;
        let killer = tab.pty_killer;
        drop(tab.terminal);
        teardown_pty(killer, master);

        self.active = if self.tabs.is_empty() {
            None
        } else {
            Some(self.tabs[pos.min(self.tabs.len() - 1)].id)
        };
        cx.notify();
    }

    pub fn teardown_all(&mut self) {
        let tabs = std::mem::take(&mut self.tabs);
        self.active = None;
        for tab in tabs {
            drop(tab.terminal);
            teardown_pty(tab.pty_killer, tab.pty_master);
        }
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
            (
                s.settings.default_shell.clone(),
                s.settings.font_family.clone(),
            )
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

impl Drop for TabManager {
    fn drop(&mut self) {
        self.teardown_all();
    }
}

fn terminal_config(font_size: f32, font_family: &str) -> TerminalConfig {
    let colors = ColorPalette::builder()
        .background(0x1A, 0x1A, 0x1C)
        .foreground(0xE8, 0xE6, 0xE3)
        .cursor(0xE8, 0xE6, 0xE3)
        .black(0x0C, 0x0C, 0x0C)
        .red(0xC5, 0x0F, 0x1F)
        .green(0x13, 0xA1, 0x0E)
        .yellow(0xC1, 0x9C, 0x00)
        .blue(0x00, 0x37, 0xDA)
        .magenta(0x88, 0x17, 0x98)
        .cyan(0x3A, 0x96, 0xDD)
        .white(0xCC, 0xCC, 0xCC)
        .bright_black(0x76, 0x76, 0x76)
        .bright_red(0xE7, 0x48, 0x56)
        .bright_green(0x16, 0xC6, 0x0C)
        .bright_yellow(0xF9, 0xF1, 0xA5)
        .bright_blue(0x3B, 0x78, 0xFF)
        .bright_magenta(0xB4, 0x00, 0x9E)
        .bright_cyan(0x61, 0xD6, 0xD6)
        .bright_white(0xF2, 0xF2, 0xF2)
        .build();

    TerminalConfig {
        font_family: font_family.into(),
        font_size: px(font_size),
        cols: 80,
        rows: 24,
        scrollback: 10_000,
        line_height_multiplier: 1.2,
        padding: Edges::all(px(theme::SPACE_2)),
        colors,
    }
}

pub fn state_color(state: ConnectionState) -> Hsla {
    match state {
        ConnectionState::Connected => theme::SUCCESS,
        ConnectionState::Connecting => theme::TEXT_MUTED,
        ConnectionState::Failed => theme::DANGER,
        ConnectionState::Disconnected | ConnectionState::Idle => theme::TEXT_MUTED,
    }
}
