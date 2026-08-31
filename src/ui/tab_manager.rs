use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use gpui::prelude::*;
use gpui::*;
use portable_pty::ChildKiller;
use uuid::Uuid;

use crate::model::{ConnectionState, Profile, ProfileKind, SshAuth};
use crate::platform;
use crate::session::credentials;
use crate::session::local::{LocalPty, resolve_shell, teardown_pty};
use crate::session::ssh::{self, SshAuthMaterial, SshConnectParams};
use crate::shared::theme;
use crate::terminal::{ColorPalette, TerminalConfig, TerminalView, TerminalViewEvent};
use crate::ui::pane_layout::{PaneLayout, RemoveResult, SplitDirection};
use crate::ui::workspace_store::WorkspaceStore;

pub struct PaneSession {
    pub id: Uuid,
    /// `None` = ephemeral (not in sidebar, not restored on restart).
    pub profile_id: Option<Uuid>,
    /// Connection snapshot for spawn / reconnect / Save to….
    pub kind: ProfileKind,
    /// Display name (profile name or ephemeral label).
    pub label: String,
    pub state: ConnectionState,
    pub status_message: String,
    pub terminal: Option<Entity<TerminalView>>,
    pub pty_master: Option<Arc<parking_lot::Mutex<Box<dyn portable_pty::MasterPty + Send>>>>,
    pub pty_killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    pub ssh_shutdown: Option<flume::Sender<()>>,
    /// Same-session SFTP handle (SSH panes only).
    pub ssh_sftp: Option<crate::session::sftp::SftpHandle>,
    _term_subscriptions: Vec<Subscription>,
}

pub struct TabSession {
    pub id: Uuid,
    pub title: String,
    pub panes: HashMap<Uuid, PaneSession>,
    pub layout: PaneLayout,
    pub focused: Uuid,
    /// When set, only this leaf is shown full-size (Zed zoom); layout tree is kept.
    pub zoomed: Option<Uuid>,
}

impl TabSession {
    pub fn focused_pane(&self) -> Option<&PaneSession> {
        self.panes.get(&self.focused)
    }

    pub fn focused_pane_mut(&mut self) -> Option<&mut PaneSession> {
        self.panes.get_mut(&self.focused)
    }

    pub fn display_state(&self) -> ConnectionState {
        self.focused_pane()
            .map(|p| p.state)
            .unwrap_or(ConnectionState::Idle)
    }
}

pub struct TabManager {
    pub tabs: Vec<TabSession>,
    pub active: Option<Uuid>,
    pub font_size: f32,
    pub show_line_numbers: bool,
    store: Entity<WorkspaceStore>,
}

impl TabManager {
    pub fn new(
        font_size: f32,
        show_line_numbers: bool,
        store: Entity<WorkspaceStore>,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            font_size,
            show_line_numbers,
            store,
        }
    }

    pub fn active_index(&self) -> usize {
        self.active
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .unwrap_or(0)
    }

    pub fn active_tab(&self) -> Option<&TabSession> {
        let id = self.active?;
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut TabSession> {
        let id = self.active?;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn open_profile(
        &mut self,
        profile: &Profile,
        default_shell: Option<&str>,
        font_family: &str,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &profile.kind {
            ProfileKind::Local { .. } => {
                match self.spawn_local(
                    Some(profile.id),
                    &profile.kind,
                    &profile.name,
                    default_shell,
                    font_family,
                    store,
                    None,
                    window,
                    cx,
                ) {
                    Ok(pane) => {
                        let title = self.unique_tab_title(&profile.name);
                        let tab = wrap_pane_as_tab(title, pane);
                        let id = tab.id;
                        self.tabs.push(tab);
                        self.active = Some(id);
                        cx.notify();
                    }
                    Err(err) => self.push_failed(profile, format!("{err:#}"), cx),
                }
            }
            ProfileKind::Ssh { .. } => match resolve_ssh_auth(profile, None) {
                Ok(Some(auth)) => self.begin_ssh(profile, auth, font_family, cx),
                Ok(None) => self.push_failed(profile, "password required".into(), cx),
                Err(err) => self.push_failed(profile, format!("{err:#}"), cx),
            },
        }
    }

    /// Open an SSH profile when credentials are already available (keyring).
    pub fn open_ssh_profile(
        &mut self,
        profile: &Profile,
        font_family: &str,
        cx: &mut Context<Self>,
    ) {
        match resolve_ssh_auth(profile, None) {
            Ok(Some(auth)) => self.begin_ssh(profile, auth, font_family, cx),
            Ok(None) => self.push_failed(profile, "password required".into(), cx),
            Err(err) => self.push_failed(profile, format!("{err:#}"), cx),
        }
    }

    pub fn open_ssh_with_password(
        &mut self,
        profile: &Profile,
        password: String,
        font_family: &str,
        cx: &mut Context<Self>,
    ) {
        match resolve_ssh_auth(profile, Some(password)) {
            Ok(Some(auth)) => self.begin_ssh(profile, auth, font_family, cx),
            Ok(None) => self.push_failed(profile, "password required".into(), cx),
            Err(err) => self.push_failed(profile, format!("{err:#}"), cx),
        }
    }

    pub fn ssh_needs_password(profile: &Profile) -> bool {
        matches!(
            &profile.kind,
            ProfileKind::Ssh {
                auth: SshAuth::Password { .. },
                ..
            }
        ) && credentials::needs_password_prompt(profile.id)
    }

    fn push_failed(&mut self, profile: &Profile, message: String, cx: &mut Context<Self>) {
        let pane = PaneSession {
            id: Uuid::new_v4(),
            profile_id: Some(profile.id),
            kind: profile.kind.clone(),
            label: profile.name.clone(),
            state: ConnectionState::Failed,
            status_message: message,
            terminal: None,
            pty_master: None,
            pty_killer: None,
            ssh_shutdown: None,
            ssh_sftp: None,
            _term_subscriptions: Vec::new(),
        };
        let tab = wrap_pane_as_tab(self.unique_tab_title(&profile.name), pane);
        let id = tab.id;
        self.tabs.push(tab);
        self.active = Some(id);
        cx.notify();
    }

    fn begin_ssh(
        &mut self,
        profile: &Profile,
        auth: SshAuthMaterial,
        font_family: &str,
        cx: &mut Context<Self>,
    ) {
        let ProfileKind::Ssh {
            host, port, user, ..
        } = &profile.kind
        else {
            return;
        };

        let pane_id = Uuid::new_v4();
        let pane = PaneSession {
            id: pane_id,
            profile_id: Some(profile.id),
            kind: profile.kind.clone(),
            label: profile.name.clone(),
            state: ConnectionState::Connecting,
            status_message: format!("connecting to {user}@{host}:{port}…"),
            terminal: None,
            pty_master: None,
            pty_killer: None,
            ssh_shutdown: None,
            ssh_sftp: None,
            _term_subscriptions: Vec::new(),
        };
        let tab = wrap_pane_as_tab(self.unique_tab_title(&profile.name), pane);
        let tab_id = tab.id;
        self.tabs.push(tab);
        self.active = Some(tab_id);
        cx.notify();

        self.spawn_ssh_connect(tab_id, pane_id, profile, auth, font_family, cx);
    }

    fn spawn_ssh_connect(
        &mut self,
        tab_id: Uuid,
        pane_id: Uuid,
        profile: &Profile,
        auth: SshAuthMaterial,
        font_family: &str,
        cx: &mut Context<Self>,
    ) {
        self.spawn_ssh_connect_kind(tab_id, pane_id, &profile.kind, auth, font_family, cx);
    }

    fn spawn_ssh_connect_kind(
        &mut self,
        tab_id: Uuid,
        pane_id: Uuid,
        kind: &ProfileKind,
        auth: SshAuthMaterial,
        font_family: &str,
        cx: &mut Context<Self>,
    ) {
        let ProfileKind::Ssh {
            host, port, user, ..
        } = kind
        else {
            return;
        };

        let params = SshConnectParams {
            host: host.clone(),
            port: *port,
            user: user.clone(),
            auth,
            cols: 80,
            rows: 24,
        };
        let family = if font_family.trim().is_empty() {
            platform::monospace_font_family().to_string()
        } else {
            font_family.to_string()
        };
        let font_size = self.font_size;
        let show_line_numbers = self.show_line_numbers;
        let label = format!("{user}@{host}:{port}");

        let (tx, rx) = flume::bounded(1);
        let _ = thread::Builder::new()
            .name("loom-ssh-connect".into())
            .spawn(move || {
                let result = ssh::connect_blocking(params);
                let _ = tx.send(result);
            });

        cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_id) else {
                    return;
                };
                let Some(pane) = tab.panes.get_mut(&pane_id) else {
                    return;
                };
                match result {
                    Ok(Ok(handles)) => {
                        let config = terminal_config(font_size, &family, show_line_numbers);
                        let resize = handles.resize.clone();
                        let terminal = cx.new(|cx| {
                            TerminalView::new(handles.writer, handles.reader, config, cx)
                                .with_resize_callback(move |c, r| resize(c, r))
                        });
                        let term_subs = wire_terminal_session(&terminal, None, cx);
                        pane.terminal = Some(terminal);
                        pane._term_subscriptions = term_subs;
                        pane.ssh_shutdown = Some(handles.shutdown);
                        pane.ssh_sftp = Some(handles.sftp);
                        pane.state = ConnectionState::Connected;
                        pane.status_message = format!("ssh · {label}");
                    }
                    Ok(Err(err)) => {
                        pane.state = ConnectionState::Failed;
                        pane.status_message = format!("{err:#}");
                    }
                    Err(_) => {
                        pane.state = ConnectionState::Failed;
                        pane.status_message = "SSH connect cancelled".into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn spawn_local(
        &self,
        profile_id: Option<Uuid>,
        kind: &ProfileKind,
        label: &str,
        default_shell: Option<&str>,
        font_family: &str,
        store: &Entity<WorkspaceStore>,
        cwd_override: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<PaneSession> {
        let ProfileKind::Local { shell, cwd, .. } = kind else {
            anyhow::bail!("not a local profile");
        };
        let configured = shell.as_deref().or(default_shell);
        let shell = resolve_shell(configured);
        let (proxy_mode, proxy_url, proxy_no) = {
            let s = store.read(cx);
            (
                s.settings.local_proxy_mode,
                s.settings.local_proxy_url.clone(),
                s.settings.local_proxy_no_proxy.clone(),
            )
        };
        let spawn_cwd = cwd_override.as_deref().or(cwd.as_deref());
        let pty = LocalPty::spawn(
            &shell,
            spawn_cwd,
            proxy_mode,
            proxy_url.as_deref(),
            proxy_no.as_deref(),
        )?;
        let master = pty.master.clone();
        let killer = pty.killer;
        let resize = LocalPty::resize_callback(master.clone());

        let family = if font_family.trim().is_empty() {
            platform::monospace_font_family()
        } else {
            font_family
        };
        let config = terminal_config(self.font_size, family, self.show_line_numbers);
        let working_dir = spawn_cwd
            .map(std::path::Path::to_path_buf)
            .or_else(|| LocalPty::default_cwd());
        let terminal = cx.new(|cx| {
            TerminalView::new(pty.writer, pty.reader, config, cx)
                .with_resize_callback(resize)
                .with_shell_pid(pty.shell_pid)
        });
        let term_subs = wire_terminal_session(&terminal, working_dir.clone(), cx);
        terminal.read(cx).focus_handle().focus(window);

        let cwd_label = working_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".into());
        let shell_short = std::path::Path::new(&shell)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&shell);

        let mut kind = kind.clone();
        if let ProfileKind::Local {
            cwd: ref mut c, ..
        } = kind
        {
            *c = working_dir.clone();
        }

        Ok(PaneSession {
            id: Uuid::new_v4(),
            profile_id,
            kind,
            label: label.to_string(),
            state: ConnectionState::Connected,
            status_message: format!("{shell_short} · {cwd_label}"),
            terminal: Some(terminal),
            pty_master: Some(master),
            pty_killer: Some(killer),
            ssh_shutdown: None,
            ssh_sftp: None,
            _term_subscriptions: term_subs,
        })
    }

    pub fn split_focused(
        &mut self,
        direction: SplitDirection,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.active else {
            return;
        };
        let Some(tab_idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let focused = self.tabs[tab_idx].focused;
        let Some(source) = self.tabs[tab_idx].panes.get(&focused).map(|p| {
            (
                p.kind.clone(),
                p.label.clone(),
                p.profile_id,
                p.terminal
                    .as_ref()
                    .and_then(|t| t.read(cx).working_directory()),
            )
        }) else {
            return;
        };
        let (kind, label, auth_profile_id, live_cwd) = source;

        let (default_shell, font_family) = {
            let s = store.read(cx);
            (s.settings.default_shell.clone(), s.settings.font_family.clone())
        };

        match &kind {
            ProfileKind::Local { .. } => {
                match self.spawn_local(
                    None,
                    &kind,
                    &label,
                    default_shell.as_deref(),
                    &font_family,
                    store,
                    live_cwd,
                    window,
                    cx,
                ) {
                    Ok(pane) => {
                        let new_id = pane.id;
                        let tab = &mut self.tabs[tab_idx];
                        tab.panes.insert(new_id, pane);
                        if tab.layout.split(focused, direction, new_id) {
                            tab.focused = new_id;
                        }
                        cx.notify();
                    }
                    Err(err) => {
                        // Do not mark the source pane Failed — only report on status.
                        if let Some(pane) = self.tabs[tab_idx].focused_pane_mut() {
                            pane.status_message = format!("split failed: {err:#}");
                        }
                        cx.notify();
                    }
                }
            }
            ProfileKind::Ssh { host, port, user, .. } => {
                let pseudo = Profile {
                    id: auth_profile_id.unwrap_or_else(Uuid::nil),
                    name: label.clone(),
                    kind: kind.clone(),
                };
                match resolve_ssh_auth(&pseudo, None) {
                    Ok(Some(auth)) => {
                        let new_id = Uuid::new_v4();
                        let pane = PaneSession {
                            id: new_id,
                            profile_id: None,
                            kind: kind.clone(),
                            label: label.clone(),
                            state: ConnectionState::Connecting,
                            status_message: format!("connecting to {user}@{host}:{port}…"),
                            terminal: None,
                            pty_master: None,
                            pty_killer: None,
                            ssh_shutdown: None,
                            ssh_sftp: None,
                            _term_subscriptions: Vec::new(),
                        };
                        let tab = &mut self.tabs[tab_idx];
                        tab.panes.insert(new_id, pane);
                        if tab.layout.split(focused, direction, new_id) {
                            tab.focused = new_id;
                        }
                        cx.notify();
                        self.spawn_ssh_connect_kind(
                            tab_id,
                            new_id,
                            &kind,
                            auth,
                            &font_family,
                            cx,
                        );
                    }
                    Ok(None) => {
                        if let Some(pane) = self.tabs[tab_idx].focused_pane_mut() {
                            pane.status_message = "password required for split".into();
                        }
                        cx.notify();
                    }
                    Err(err) => {
                        if let Some(pane) = self.tabs[tab_idx].focused_pane_mut() {
                            pane.status_message = format!("split failed: {err:#}");
                        }
                        cx.notify();
                    }
                }
            }
        }
    }

    pub fn activate_pane_in_direction(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let focused = tab.focused;
        let Some(next) = tab.layout.adjacent_leaf(focused, direction) else {
            return;
        };
        self.focus_pane(next, window, cx);
    }

    pub fn focus_pane(&mut self, pane_id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if !tab.panes.contains_key(&pane_id) {
            return;
        }
        tab.focused = pane_id;
        if tab.zoomed.is_some() {
            tab.zoomed = Some(pane_id);
        }
        if let Some(term) = tab.panes.get(&pane_id).and_then(|p| p.terminal.as_ref()) {
            term.read(cx).focus_handle().focus(window);
        }
        cx.notify();
    }

    pub fn set_split_ratio(&mut self, split_id: Uuid, ratio: f32, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.layout.set_ratio(split_id, ratio) {
            cx.notify();
        }
    }

    pub fn reconnect(
        &mut self,
        tab_id: Uuid,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reconnect_inner(tab_id, store, None, Some(window), cx);
    }

    /// SSH password reconnect (no Window required — used after modal submit).
    pub fn reconnect_with_password(
        &mut self,
        tab_id: Uuid,
        password: String,
        store: &Entity<WorkspaceStore>,
        cx: &mut Context<Self>,
    ) {
        self.reconnect_inner(tab_id, store, Some(password), None, cx);
    }

    fn reconnect_inner(
        &mut self,
        tab_id: Uuid,
        store: &Entity<WorkspaceStore>,
        password: Option<String>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let focused = self.tabs[idx].focused;
        let Some((profile_id, kind, label)) = self.tabs[idx].panes.get(&focused).map(|p| {
            (p.profile_id, p.kind.clone(), p.label.clone())
        }) else {
            return;
        };

        let (default_shell, font_family) = {
            let s = store.read(cx);
            (s.settings.default_shell.clone(), s.settings.font_family.clone())
        };

        // Prefer live Profile fields when Bound; otherwise use pane snapshot.
        let kind = profile_id
            .and_then(|pid| store.read(cx).workspace.find_profile(pid).map(|p| p.kind.clone()))
            .unwrap_or(kind);
        let label = profile_id
            .and_then(|pid| {
                store
                    .read(cx)
                    .workspace
                    .find_profile(pid)
                    .map(|p| p.name.clone())
            })
            .unwrap_or(label);

        if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
            teardown_pane_io(pane);
            drop(pane.terminal.take());
            pane.kind = kind.clone();
            pane.label = label.clone();
        }

        match &kind {
            ProfileKind::Local { .. } => {
                let Some(window) = window else {
                    if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                        pane.state = ConnectionState::Failed;
                        pane.status_message = "reconnect requires window focus".into();
                    }
                    cx.notify();
                    return;
                };
                if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                    pane.state = ConnectionState::Connecting;
                    pane.status_message = "reconnecting…".into();
                }
                match self.spawn_local(
                    profile_id,
                    &kind,
                    &label,
                    default_shell.as_deref(),
                    &font_family,
                    store,
                    None,
                    window,
                    cx,
                ) {
                    Ok(mut fresh) => {
                        fresh.id = focused;
                        self.tabs[idx].panes.insert(focused, fresh);
                    }
                    Err(err) => {
                        if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                            pane.state = ConnectionState::Failed;
                            pane.status_message = format!("{err:#}");
                        }
                    }
                }
                cx.notify();
            }
            ProfileKind::Ssh { host, port, user, .. } => {
                let pseudo = Profile {
                    id: profile_id.unwrap_or_else(Uuid::nil),
                    name: label.clone(),
                    kind: kind.clone(),
                };
                match resolve_ssh_auth(&pseudo, password) {
                    Ok(Some(auth)) => {
                        if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                            pane.profile_id = profile_id;
                            pane.state = ConnectionState::Connecting;
                            pane.status_message =
                                format!("connecting to {user}@{host}:{port}…");
                        }
                        cx.notify();
                        self.spawn_ssh_connect_kind(
                            tab_id,
                            focused,
                            &kind,
                            auth,
                            &font_family,
                            cx,
                        );
                    }
                    Ok(None) => {
                        if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                            pane.state = ConnectionState::Failed;
                            pane.status_message = "password required".into();
                        }
                        cx.notify();
                    }
                    Err(err) => {
                        if let Some(pane) = self.tabs[idx].panes.get_mut(&focused) {
                            pane.state = ConnectionState::Failed;
                            pane.status_message = format!("{err:#}");
                        }
                        cx.notify();
                    }
                }
            }
        }
    }

    fn on_pane_session_ended(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        let Some(pane) = self
            .tabs
            .iter_mut()
            .find_map(|t| t.panes.get_mut(&pane_id))
        else {
            return;
        };
        // Already reconnecting / failed — don't clobber a newer Connecting state.
        if matches!(
            pane.state,
            ConnectionState::Connecting | ConnectionState::Failed | ConnectionState::Disconnected
        ) {
            // Still drop IO if a late Exit races with reconnect teardown.
            if pane.ssh_sftp.is_some() || pane.ssh_shutdown.is_some() || pane.pty_master.is_some()
            {
                let master = pane.pty_master.take();
                let killer = pane.pty_killer.take();
                drop(pane.ssh_sftp.take());
                if let Some(tx) = pane.ssh_shutdown.take() {
                    let _ = tx.send(());
                }
                teardown_pty(killer, master);
            }
            cx.notify();
            return;
        }
        pane.state = ConnectionState::Failed;
        pane.status_message = "disconnected — use Reconnect in the status bar".into();
        let master = pane.pty_master.take();
        let killer = pane.pty_killer.take();
        drop(pane.ssh_sftp.take());
        if let Some(tx) = pane.ssh_shutdown.take() {
            let _ = tx.send(());
        }
        teardown_pty(killer, master);
        // Keep `terminal` so the last output and disconnect banner remain visible.
        cx.notify();
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|t| t.id == active) else {
            return;
        };
        let pane_id = tab.focused;
        self.close_pane(pane_id, Some(window), cx);
    }

    fn pane_id_for_terminal(&self, term: &Entity<TerminalView>) -> Option<Uuid> {
        for tab in &self.tabs {
            for (id, pane) in &tab.panes {
                if pane
                    .terminal
                    .as_ref()
                    .is_some_and(|t| t.entity_id() == term.entity_id())
                {
                    return Some(*id);
                }
            }
        }
        None
    }

    fn focus_pane_id(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if tab.panes.contains_key(&pane_id) {
                tab.focused = pane_id;
                // While zoomed, follow focus so the visible pane matches.
                if tab.zoomed.is_some() {
                    tab.zoomed = Some(pane_id);
                }
                self.active = Some(tab.id);
                cx.notify();
                return;
            }
        }
    }

    /// Toggle Zed-style zoom: focused pane fills the tab; layout tree stays intact.
    pub fn toggle_zoom_focused(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.layout.leaf_count() <= 1 {
            tab.zoomed = None;
            cx.notify();
            return;
        }
        if tab.zoomed == Some(tab.focused) {
            tab.zoomed = None;
        } else {
            tab.zoomed = Some(tab.focused);
        }
        cx.notify();
    }

    pub fn active_zoomed(&self) -> bool {
        self.active_tab()
            .is_some_and(|t| t.zoomed.is_some() && t.layout.leaf_count() > 1)
    }

    pub fn can_zoom_active(&self) -> bool {
        self.active_tab()
            .is_some_and(|t| t.layout.leaf_count() > 1 || t.zoomed.is_some())
    }

    /// Close a specific pane (or its tab when it is the last pane).
    pub fn close_pane(
        &mut self,
        pane_id: Uuid,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_idx) = self
            .tabs
            .iter()
            .position(|t| t.panes.contains_key(&pane_id))
        else {
            return;
        };
        let tab_id = self.tabs[tab_idx].id;
        self.tabs[tab_idx].focused = pane_id;
        match self.tabs[tab_idx].layout.remove_leaf(pane_id) {
            RemoveResult::RemovedRoot => {
                self.close_tab(tab_id, cx);
            }
            RemoveResult::Collapsed { focus } => {
                if let Some(mut pane) = self.tabs[tab_idx].panes.remove(&pane_id) {
                    teardown_pane_io(&mut pane);
                    drop(pane.terminal);
                }
                self.tabs[tab_idx].focused = focus;
                // Drop zoom if the zoomed leaf is gone or only one pane remains.
                let clear_zoom = self.tabs[tab_idx].zoomed == Some(pane_id)
                    || self.tabs[tab_idx].layout.leaf_count() <= 1;
                if clear_zoom {
                    self.tabs[tab_idx].zoomed = None;
                }
                if let Some(window) = window {
                    if let Some(term) = self.tabs[tab_idx]
                        .panes
                        .get(&focus)
                        .and_then(|p| p.terminal.as_ref())
                    {
                        term.read(cx).focus_handle().focus(window);
                    }
                }
                cx.notify();
            }
            RemoveResult::NotFound => {}
        }
    }

    /// Close the focused pane (or the whole tab when it is the last pane).
    pub fn close_focused_pane(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|t| t.id == active) else {
            return;
        };
        let pane_id = tab.focused;
        self.close_pane(pane_id, window, cx);
    }

    pub fn close_tab(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let mut tab = self.tabs.remove(pos);
        teardown_tab_io(&mut tab);

        self.active = if self.tabs.is_empty() {
            None
        } else {
            Some(self.tabs[pos.min(self.tabs.len() - 1)].id)
        };
        cx.notify();
    }

    /// Close every tab except `keep`, tearing down PTY/SSH/SFTP for each.
    pub fn close_other_tabs(&mut self, keep: Uuid, cx: &mut Context<Self>) {
        let ids: Vec<Uuid> = self
            .tabs
            .iter()
            .map(|t| t.id)
            .filter(|&id| id != keep)
            .collect();
        for id in ids {
            self.close_tab(id, cx);
        }
        if self.tabs.iter().any(|t| t.id == keep) {
            self.active = Some(keep);
        }
        cx.notify();
    }

    /// Close tabs to the left of `id` (lower indices).
    pub fn close_tabs_to_left(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let ids: Vec<Uuid> = self.tabs[..pos].iter().map(|t| t.id).collect();
        for tid in ids {
            self.close_tab(tid, cx);
        }
        if self.tabs.iter().any(|t| t.id == id) {
            self.active = Some(id);
        }
        cx.notify();
    }

    /// Close tabs to the right of `id` (higher indices).
    pub fn close_tabs_to_right(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let ids: Vec<Uuid> = self.tabs[pos + 1..].iter().map(|t| t.id).collect();
        for tid in ids {
            self.close_tab(tid, cx);
        }
        if self.tabs.iter().any(|t| t.id == id) {
            self.active = Some(id);
        }
        cx.notify();
    }

    /// Close all tabs and release session resources.
    pub fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.teardown_all();
        cx.notify();
    }

    /// Profile id for the focused pane of a tab (fallback: any pane). `None` if ephemeral.
    pub fn profile_id_for_tab(&self, tab_id: Uuid) -> Option<Uuid> {
        let tab = self.tabs.iter().find(|t| t.id == tab_id)?;
        tab.focused_pane()
            .and_then(|p| p.profile_id)
            .or_else(|| tab.panes.values().find_map(|p| p.profile_id))
    }

    /// Prefer `base`, then `base (2)`, `base (3)`, … so duplicate tabs stay distinguishable.
    fn unique_tab_title(&self, base: &str) -> String {
        if !self.tabs.iter().any(|t| t.title == base) {
            return base.to_string();
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{base} ({n})");
            if !self.tabs.iter().any(|t| t.title == candidate) {
                return candidate;
            }
            n = n.saturating_add(1);
            if n > 10_000 {
                return format!("{base} ({})", Uuid::new_v4());
            }
        }
    }

    pub fn teardown_all(&mut self) {
        let mut tabs = std::mem::take(&mut self.tabs);
        self.active = None;
        for mut tab in tabs.drain(..) {
            teardown_tab_io(&mut tab);
        }
    }

    pub fn select_tab(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(id);
        if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
            if let Some(term) = tab.focused_pane().and_then(|p| p.terminal.as_ref()) {
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
        self.duplicate_active_ephemeral(store, window, cx);
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
            for pane in tab.panes.values() {
                if let Some(term) = &pane.terminal {
                    term.update(cx, |terminal, cx| {
                        let mut config = terminal.config().clone();
                        config.font_size = px(self.font_size);
                        terminal.update_config(config, cx);
                    });
                }
            }
        }
        cx.notify();
    }

    pub fn set_show_line_numbers(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.show_line_numbers = enabled;
        for tab in &self.tabs {
            for pane in tab.panes.values() {
                if let Some(term) = &pane.terminal {
                    term.update(cx, |terminal, cx| {
                        let mut config = terminal.config().clone();
                        config.show_line_numbers = enabled;
                        terminal.update_config(config, cx);
                    });
                }
            }
        }
        cx.notify();
    }

    /// Bound Local panes → (profile_id, live cwd) for persistence.
    pub fn bound_local_cwds(&self, cx: &mut Context<Self>) -> Vec<(Uuid, std::path::PathBuf)> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            for pane in tab.panes.values() {
                let Some(pid) = pane.profile_id else {
                    continue;
                };
                if !pane.kind.is_local() {
                    continue;
                }
                let Some(term) = pane.terminal.as_ref() else {
                    continue;
                };
                let Some(cwd) = term.update(cx, |view, _| {
                    view.refresh_working_directory();
                    view.working_directory()
                }) else {
                    continue;
                };
                out.push((pid, cwd));
            }
        }
        out
    }

    pub fn snapshot_for_persist(&self) -> (Vec<(Uuid, Option<String>)>, usize) {
        // Only Bound tabs — ephemeral sessions are not restored.
        let tabs: Vec<(Uuid, Option<String>)> = self
            .tabs
            .iter()
            .filter_map(|t| {
                let pid = t.focused_pane().and_then(|p| p.profile_id)?;
                Some((pid, Some(t.title.clone())))
            })
            .collect();
        let active = self
            .active
            .and_then(|id| {
                self.tabs
                    .iter()
                    .filter(|t| t.focused_pane().and_then(|p| p.profile_id).is_some())
                    .position(|t| t.id == id)
            })
            .unwrap_or(0);
        (tabs, active)
    }

    /// Ctrl+T / +: ephemeral Local session (not in sidebar).
    pub fn open_ephemeral_local(
        &mut self,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (default_shell, font_family) = {
            let s = store.read(cx);
            (s.settings.default_shell.clone(), s.settings.font_family.clone())
        };
        let kind = ProfileKind::Local {
            shell: default_shell.clone(),
            cwd: None,
            env: Vec::new(),
        };
        let label = "Shell";
        match self.spawn_local(
            None,
            &kind,
            label,
            default_shell.as_deref(),
            &font_family,
            store,
            None,
            window,
            cx,
        ) {
            Ok(pane) => {
                let title = self.unique_tab_title(label);
                let tab = wrap_pane_as_tab(title, pane);
                let id = tab.id;
                self.tabs.push(tab);
                self.active = Some(id);
                cx.notify();
            }
            Err(err) => {
                eprintln!("loom: ephemeral local failed: {err:#}");
            }
        }
    }

    /// Duplicate focused session as a new ephemeral tab.
    pub fn duplicate_active_ephemeral(
        &mut self,
        store: &Entity<WorkspaceStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(source) = tab.focused_pane() else {
            return;
        };
        let kind = source.kind.clone();
        let label = source.label.clone();
        let auth_pid = source.profile_id;
        let live_cwd = source
            .terminal
            .as_ref()
            .and_then(|t| t.read(cx).working_directory());

        let (default_shell, font_family) = {
            let s = store.read(cx);
            (s.settings.default_shell.clone(), s.settings.font_family.clone())
        };

        match &kind {
            ProfileKind::Local { .. } => {
                match self.spawn_local(
                    None,
                    &kind,
                    &label,
                    default_shell.as_deref(),
                    &font_family,
                    store,
                    live_cwd,
                    window,
                    cx,
                ) {
                    Ok(pane) => {
                        let title = self.unique_tab_title(&label);
                        let tab = wrap_pane_as_tab(title, pane);
                        let id = tab.id;
                        self.tabs.push(tab);
                        self.active = Some(id);
                        cx.notify();
                    }
                    Err(err) => eprintln!("loom: duplicate local failed: {err:#}"),
                }
            }
            ProfileKind::Ssh { host, port, user, .. } => {
                let pseudo = Profile {
                    id: auth_pid.unwrap_or_else(Uuid::nil),
                    name: label.clone(),
                    kind: kind.clone(),
                };
                match resolve_ssh_auth(&pseudo, None) {
                    Ok(Some(auth)) => {
                        let pane_id = Uuid::new_v4();
                        let pane = PaneSession {
                            id: pane_id,
                            profile_id: None,
                            kind: kind.clone(),
                            label: label.clone(),
                            state: ConnectionState::Connecting,
                            status_message: format!("connecting to {user}@{host}:{port}…"),
                            terminal: None,
                            pty_master: None,
                            pty_killer: None,
                            ssh_shutdown: None,
                            ssh_sftp: None,
                            _term_subscriptions: Vec::new(),
                        };
                        let title = self.unique_tab_title(&label);
                        let tab = wrap_pane_as_tab(title, pane);
                        let tab_id = tab.id;
                        self.tabs.push(tab);
                        self.active = Some(tab_id);
                        cx.notify();
                        self.spawn_ssh_connect_kind(
                            tab_id,
                            pane_id,
                            &kind,
                            auth,
                            &font_family,
                            cx,
                        );
                    }
                    Ok(None) => {
                        // Fall back to opening Bound profile so password prompt can run.
                        if let Some(pid) = auth_pid {
                            let _ = pid;
                        }
                        eprintln!("loom: duplicate SSH needs password — open from sidebar or save first");
                    }
                    Err(err) => eprintln!("loom: duplicate SSH failed: {err:#}"),
                }
            }
        }
    }

    /// Bind an ephemeral pane to a newly saved profile id.
    pub fn bind_focused_to_profile(
        &mut self,
        tab_id: Uuid,
        profile_id: Uuid,
        label: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        if let Some(pane) = tab.panes.get_mut(&tab.focused) {
            pane.profile_id = Some(profile_id);
            pane.label = label.clone();
        }
        tab.title = label;
        cx.notify();
    }
}

impl Drop for TabManager {
    fn drop(&mut self) {
        self.teardown_all();
    }
}

fn wire_terminal_session(
    terminal: &Entity<TerminalView>,
    working_dir: Option<std::path::PathBuf>,
    cx: &mut Context<TabManager>,
) -> Vec<Subscription> {
    terminal.update(cx, |t, cx| {
        t.set_working_directory(working_dir, cx);
    });
    let sub = cx.subscribe(terminal, |this, term, event: &TerminalViewEvent, cx| {
        let Some(pane_id) = this.pane_id_for_terminal(&term) else {
            return;
        };
        match event {
            TerminalViewEvent::FocusRequested => {
                this.focus_pane_id(pane_id, cx);
            }
            TerminalViewEvent::CloseRequested => {
                this.close_pane(pane_id, None, cx);
            }
            TerminalViewEvent::SessionEnded => {
                this.on_pane_session_ended(pane_id, cx);
            }
            TerminalViewEvent::WorkingDirectoryChanged(path) => {
                let Some(pane) = this
                    .tabs
                    .iter_mut()
                    .find_map(|t| t.panes.get_mut(&pane_id))
                else {
                    return;
                };
                let Some(pid) = pane.profile_id else {
                    return;
                };
                if !pane.kind.is_local() {
                    return;
                }
                if let ProfileKind::Local { cwd, .. } = &mut pane.kind {
                    *cwd = Some(path.clone());
                }
                let store = this.store.clone();
                store.update(cx, |s, cx| {
                    s.update_local_profile_cwd(pid, path.clone(), cx);
                    s.persist_now();
                });
            }
        }
    });
    vec![sub]
}

fn wrap_pane_as_tab(title: String, pane: PaneSession) -> TabSession {
    let pane_id = pane.id;
    let mut panes = HashMap::new();
    panes.insert(pane_id, pane);
    TabSession {
        id: Uuid::new_v4(),
        title,
        panes,
        layout: PaneLayout::leaf(pane_id),
        focused: pane_id,
        zoomed: None,
    }
}

fn teardown_pane_io(pane: &mut PaneSession) {
    // Drop SFTP handle first so the pool shuts down and returns channel budget
    // before (or as) the SSH session disconnects.
    drop(pane.ssh_sftp.take());
    if let Some(tx) = pane.ssh_shutdown.take() {
        let _ = tx.send(());
    }
    let master = pane.pty_master.take();
    let killer = pane.pty_killer.take();
    teardown_pty(killer, master);
}

fn teardown_tab_io(tab: &mut TabSession) {
    for pane in tab.panes.values_mut() {
        teardown_pane_io(pane);
        drop(pane.terminal.take());
    }
}

fn resolve_ssh_auth(
    profile: &Profile,
    password_override: Option<String>,
) -> anyhow::Result<Option<SshAuthMaterial>> {
    let ProfileKind::Ssh { auth, .. } = &profile.kind else {
        anyhow::bail!("not an SSH profile");
    };
    match auth {
        SshAuth::Password { .. } => {
            if let Some(p) = password_override {
                return Ok(Some(SshAuthMaterial::Password(p)));
            }
            Ok(credentials::get_password(profile.id)?.map(SshAuthMaterial::Password))
        }
        SshAuth::PrivateKey { path } => Ok(Some(SshAuthMaterial::PrivateKey {
            path: path.clone(),
            passphrase: None,
        })),
    }
}

fn terminal_config(font_size: f32, font_family: &str, show_line_numbers: bool) -> TerminalConfig {
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
        show_line_numbers,
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
