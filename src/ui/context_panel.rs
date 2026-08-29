//! Right context panel — Files (SFTP) + Info (see docs/CONTEXT_PANEL.md).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{ConnectionState, ProfileKind};
use crate::session::sftp::{
    RemoteEntry, SftpHandle, SftpRequest, TransferProgress, parent_remote,
};
use crate::shared::theme;
use crate::ui::tab_manager::TabManager;
use crate::ui::tooltip::Tooltip;
use crate::ui::workspace_store::WorkspaceStore;

const ICON: f32 = 13.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelTab {
    Files,
    Info,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferDir {
    Download,
    Upload,
}

#[derive(Clone)]
enum TransferStatus {
    Running { done: u64, total: Option<u64> },
    Done,
    Failed(String),
}

struct TransferRow {
    id: Uuid,
    label: String,
    direction: TransferDir,
    status: TransferStatus,
}

pub struct ContextPanel {
    store: Entity<WorkspaceStore>,
    tabs: Entity<TabManager>,
    active_tab: PanelTab,
    /// Remote cwd for the Files browser.
    cwd: Option<String>,
    home: Option<String>,
    entries: Vec<RemoteEntry>,
    selected: Option<String>,
    listing: bool,
    error: Option<String>,
    /// Pane id we last bound SFTP state to.
    bound_pane: Option<Uuid>,
    transfers: Vec<TransferRow>,
    focus_handle: FocusHandle,
    _observe_store: Subscription,
    _observe_tabs: Subscription,
}

#[derive(Clone, Debug)]
pub enum ContextPanelEvent {
    /// Reserved for future actions (kept for subscribe wiring).
    #[allow(dead_code)]
    None,
}

impl ContextPanel {
    pub fn new(
        store: Entity<WorkspaceStore>,
        tabs: Entity<TabManager>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _observe_store = cx.observe(&store, |_this, _store, cx| cx.notify());
        let _observe_tabs = cx.observe(&tabs, |this, _tabs, cx| {
            this.sync_session(cx);
            cx.notify();
        });
        let mut panel = Self {
            store,
            tabs,
            active_tab: PanelTab::Files,
            cwd: None,
            home: None,
            entries: Vec::new(),
            selected: None,
            listing: false,
            error: None,
            bound_pane: None,
            transfers: Vec::new(),
            focus_handle: cx.focus_handle(),
            _observe_store,
            _observe_tabs,
        };
        panel.sync_session(cx);
        panel
    }

    fn focused_sftp(&self, cx: &App) -> Option<(Uuid, SftpHandle)> {
        let tab = self.tabs.read(cx).active_tab()?;
        let pane = tab.focused_pane()?;
        let id = pane.id;
        let sftp = pane.ssh_sftp.clone()?;
        Some((id, sftp))
    }

    fn sync_session(&mut self, cx: &mut Context<Self>) {
        let Some((pane_id, sftp)) = self.focused_sftp(cx) else {
            if self.bound_pane.take().is_some() {
                self.cwd = None;
                self.home = None;
                self.entries.clear();
                self.selected = None;
                self.error = None;
            }
            return;
        };
        if self.bound_pane == Some(pane_id) {
            return;
        }
        self.bound_pane = Some(pane_id);
        self.cwd = None;
        self.home = None;
        self.entries.clear();
        self.selected = None;
        self.error = None;
        self.go_home(sftp, cx);
    }

    fn go_home(&mut self, sftp: SftpHandle, cx: &mut Context<Self>) {
        let (tx, rx) = flume::bounded(1);
        if sftp.request(SftpRequest::Home { reply: tx }).is_err() {
            self.error = Some("SFTP unavailable".into());
            return;
        }
        self.listing = true;
        cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.listing = false;
                match result {
                    Ok(Ok(home)) => {
                        this.home = Some(home.clone());
                        this.load_dir(home, cx);
                    }
                    Ok(Err(err)) => {
                        this.error = Some(format!("{err:#}"));
                        cx.notify();
                    }
                    Err(_) => {
                        this.error = Some("SFTP home request cancelled".into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_dir(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((_, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };
        let (tx, rx) = flume::bounded(1);
        if sftp
            .request(SftpRequest::List {
                path: path.clone(),
                reply: tx,
            })
            .is_err()
        {
            self.error = Some("SFTP unavailable".into());
            cx.notify();
            return;
        }
        self.listing = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.listing = false;
                match result {
                    Ok(Ok(entries)) => {
                        this.cwd = Some(path);
                        this.entries = entries;
                        this.selected = None;
                        this.error = None;
                    }
                    Ok(Err(err)) => {
                        this.error = Some(format!("{err:#}"));
                    }
                    Err(_) => {
                        this.error = Some("List cancelled".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.cwd.clone() else {
            return;
        };
        if let Some(parent) = parent_remote(&cwd) {
            self.load_dir(parent, cx);
        }
    }

    fn enter_or_download(&mut self, entry: RemoteEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.load_dir(entry.path, cx);
        } else {
            self.download_entry(entry, cx);
        }
    }

    fn download_selected(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| e.path == path).cloned() else {
            return;
        };
        self.download_entry(entry, cx);
    }

    fn download_entry(&mut self, entry: RemoteEntry, cx: &mut Context<Self>) {
        let Some((_, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };
        let remote = entry.path.clone();
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let id = Uuid::new_v4();

        self.transfers.insert(
            0,
            TransferRow {
                id,
                label: name.clone(),
                direction: TransferDir::Download,
                status: TransferStatus::Running {
                    done: 0,
                    total: if is_dir { None } else { Some(entry.size) },
                },
            },
        );
        cx.notify();

        cx.spawn(async move |this, cx| {
            let local = if is_dir {
                rfd::AsyncFileDialog::new()
                    .set_title("Download folder to…")
                    .pick_folder()
                    .await
                    .map(|h| h.path().join(&name))
            } else {
                rfd::AsyncFileDialog::new()
                    .set_title("Save file")
                    .set_file_name(&name)
                    .save_file()
                    .await
                    .map(|h| h.path().to_path_buf())
            };

            let Some(local) = local else {
                this.update(cx, |this, cx| {
                    this.fail_transfer(id, "Cancelled".into());
                    cx.notify();
                })
                .ok();
                return;
            };

            let (progress_tx, progress_rx) = flume::unbounded::<TransferProgress>();
            let (reply_tx, reply_rx) = flume::bounded(1);
            if sftp
                .request(SftpRequest::Download {
                    id,
                    remote,
                    local,
                    progress: progress_tx,
                    reply: reply_tx,
                })
                .is_err()
            {
                this.update(cx, |this, cx| {
                    this.fail_transfer(id, "SFTP unavailable".into());
                    cx.notify();
                })
                .ok();
                return;
            }

            loop {
                tokio::select! {
                    p = progress_rx.recv_async() => {
                        match p {
                            Ok(p) => {
                                this.update(cx, |this, cx| {
                                    this.update_transfer_progress(p);
                                    cx.notify();
                                }).ok();
                            }
                            Err(_) => {}
                        }
                    }
                    result = reply_rx.recv_async() => {
                        this.update(cx, |this, cx| {
                            match result {
                                Ok(Ok(())) => this.finish_transfer(id),
                                Ok(Err(err)) => this.fail_transfer(id, format!("{err:#}")),
                                Err(_) => this.fail_transfer(id, "Transfer cancelled".into()),
                            }
                            cx.notify();
                        }).ok();
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn upload(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.cwd.clone() else {
            self.error = Some("Open a remote folder first".into());
            cx.notify();
            return;
        };
        let Some((_, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };
        let id = Uuid::new_v4();

        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Upload to remote")
                .pick_file()
                .await;
            let Some(handle) = picked else {
                return;
            };
            let local: PathBuf = handle.path().to_path_buf();
            let label = local
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| local.display().to_string());

            this.update(cx, |this, cx| {
                this.transfers.insert(
                    0,
                    TransferRow {
                        id,
                        label,
                        direction: TransferDir::Upload,
                        status: TransferStatus::Running {
                            done: 0,
                            total: None,
                        },
                    },
                );
                cx.notify();
            })
            .ok();

            let (progress_tx, progress_rx) = flume::unbounded::<TransferProgress>();
            let (reply_tx, reply_rx) = flume::bounded(1);
            if sftp
                .request(SftpRequest::Upload {
                    id,
                    local,
                    remote_dir: cwd,
                    progress: progress_tx,
                    reply: reply_tx,
                })
                .is_err()
            {
                this.update(cx, |this, cx| {
                    this.fail_transfer(id, "SFTP unavailable".into());
                    cx.notify();
                })
                .ok();
                return;
            }

            loop {
                tokio::select! {
                    p = progress_rx.recv_async() => {
                        match p {
                            Ok(p) => {
                                this.update(cx, |this, cx| {
                                    this.update_transfer_progress(p);
                                    cx.notify();
                                }).ok();
                            }
                            Err(_) => {}
                        }
                    }
                    result = reply_rx.recv_async() => {
                        this.update(cx, |this, cx| {
                            match result {
                                Ok(Ok(())) => {
                                    this.finish_transfer(id);
                                    if let Some(cwd) = this.cwd.clone() {
                                        this.load_dir(cwd, cx);
                                    }
                                }
                                Ok(Err(err)) => this.fail_transfer(id, format!("{err:#}")),
                                Err(_) => this.fail_transfer(id, "Transfer cancelled".into()),
                            }
                            cx.notify();
                        }).ok();
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn update_transfer_progress(&mut self, p: TransferProgress) {
        if let Some(row) = self.transfers.iter_mut().find(|t| t.id == p.id) {
            row.status = TransferStatus::Running {
                done: p.done,
                total: p.total,
            };
        }
    }

    fn finish_transfer(&mut self, id: Uuid) {
        if let Some(row) = self.transfers.iter_mut().find(|t| t.id == id) {
            row.status = TransferStatus::Done;
        }
    }

    fn fail_transfer(&mut self, id: Uuid, msg: String) {
        if let Some(row) = self.transfers.iter_mut().find(|t| t.id == id) {
            row.status = TransferStatus::Failed(msg);
        }
    }

    fn tab_btn(
        &self,
        id: &'static str,
        label: &'static str,
        tab: PanelTab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.active_tab == tab;
        div()
            .id(id)
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .text_xs()
            .cursor_pointer()
            .when(active, |d| {
                d.bg(theme::HOVER)
                    .text_color(theme::TEXT)
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |d| {
                d.text_color(theme::TEXT_MUTED)
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.active_tab = tab;
                if tab == PanelTab::Files {
                    this.sync_session(cx);
                }
                cx.notify();
            }))
    }

    fn nav_btn(
        &self,
        id: &'static str,
        label: &'static str,
        tip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .text_xs()
            .when(enabled, |d| {
                d.text_color(theme::TEXT_MUTED)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                    .tooltip(move |_, cx| Tooltip::text(tip, cx))
                    .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
            })
            .when(!enabled, |d| d.text_color(theme::TEXT_DISABLED))
            .child(label)
    }

    fn render_files(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let has_sftp = self.focused_sftp(cx).is_some();
        if !has_sftp {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p(px(theme::SPACE_2))
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::TEXT)
                        .child("Files"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child(
                            "Remote file browser is available on SSH sessions. \
                             Open an SSH profile, then browse, download, and upload here.",
                        ),
                )
                .into_any_element();
        }

        let cwd = self.cwd.clone().unwrap_or_else(|| "…".into());
        let can_up = self
            .cwd
            .as_ref()
            .and_then(|p| parent_remote(p))
            .is_some();
        let listing = self.listing;
        let error = self.error.clone();
        let entries = self.entries.clone();
        let selected = self.selected.clone();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(theme::SPACE_1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(self.nav_btn("ctx-up", "↑", "Parent folder", can_up, cx, |this, cx| {
                        this.go_up(cx);
                    }))
                    .child(self.nav_btn("ctx-home", "⌂", "Home", true, cx, |this, cx| {
                        if let Some((_, sftp)) = this.focused_sftp(cx) {
                            this.go_home(sftp, cx);
                        }
                    }))
                    .child(self.nav_btn("ctx-refresh", "↻", "Refresh", true, cx, |this, cx| {
                        if let Some(cwd) = this.cwd.clone() {
                            this.load_dir(cwd, cx);
                        }
                    }))
                    .child(div().flex_1())
                    .child(self.nav_btn(
                        "ctx-download",
                        "↓",
                        "Download selected",
                        selected.is_some(),
                        cx,
                        |this, cx| this.download_selected(cx),
                    ))
                    .child(self.nav_btn("ctx-upload", "↑+", "Upload file…", true, cx, |this, cx| {
                        this.upload(cx);
                    })),
            )
            .child(
                div()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .rounded(px(theme::RADIUS_SM))
                    .bg(theme::BG)
                    .border_1()
                    .border_color(theme::BORDER_SUBTLE)
                    .text_xs()
                    .text_color(theme::TEXT_MUTED)
                    .overflow_hidden()
                    .child(cwd),
            )
            .when_some(error, |d, err| {
                d.child(
                    div()
                        .px(px(theme::SPACE_2))
                        .text_xs()
                        .text_color(theme::DANGER)
                        .child(err),
                )
            })
            .when(listing, |d| {
                d.child(
                    div()
                        .px(px(theme::SPACE_2))
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child("Loading…"),
                )
            })
            .child(
                div()
                    .id("ctx-file-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(theme::BORDER_SUBTLE)
                    .rounded(px(theme::RADIUS_SM))
                    .children(entries.into_iter().map(|entry| {
                        let path = entry.path.clone();
                        let is_sel = selected.as_deref() == Some(path.as_str());
                        let icon = if entry.is_dir { "📁" } else { "📄" };
                        let size = if entry.is_dir {
                            String::new()
                        } else {
                            format_size(entry.size)
                        };
                        let entry_click = entry.clone();
                        div()
                            .id(SharedString::from(format!("file-{path}")))
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_1))
                            .px(px(theme::SPACE_2))
                            .py(px(theme::SPACE_1))
                            .cursor_pointer()
                            .when(is_sel, |d| d.bg(theme::HOVER))
                            .hover(|s| s.bg(theme::HOVER))
                            .child(div().text_xs().child(icon))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(theme::TEXT)
                                    .overflow_hidden()
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(size),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                this.selected = Some(path.clone());
                                if event.click_count() >= 2 {
                                    this.enter_or_download(entry_click.clone(), cx);
                                }
                                cx.notify();
                            }))
                    })),
            )
            .child(self.render_transfers())
            .into_any_element()
    }

    fn render_transfers(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .max_h(px(120.0))
            .border_t_1()
            .border_color(theme::BORDER_SUBTLE)
            .pt(px(theme::SPACE_1))
            .gap(px(2.0))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::TEXT_MUTED)
                    .child("Transfers"),
            )
            .when(self.transfers.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_DISABLED)
                        .child("No transfers yet"),
                )
            })
            .children(self.transfers.iter().take(6).map(|row| {
                let arrow = match row.direction {
                    TransferDir::Download => "↓",
                    TransferDir::Upload => "↑",
                };
                let status = match &row.status {
                    TransferStatus::Running { done, total } => match total {
                        Some(t) if *t > 0 => {
                            format!("{}%", ((*done as f64 / *t as f64) * 100.0) as u32)
                        }
                        _ => format_size(*done),
                    },
                    TransferStatus::Done => "Done".into(),
                    TransferStatus::Failed(msg) => {
                        let short = if msg.len() > 24 {
                            format!("{}…", &msg[..24])
                        } else {
                            msg.clone()
                        };
                        short
                    }
                };
                let color = match &row.status {
                    TransferStatus::Failed(_) => theme::DANGER,
                    TransferStatus::Done => theme::ICON_LOCAL,
                    _ => theme::TEXT_MUTED,
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1))
                    .text_xs()
                    .child(div().text_color(theme::TEXT_MUTED).child(arrow))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme::TEXT)
                            .overflow_hidden()
                            .child(row.label.clone()),
                    )
                    .child(div().text_color(color).child(status))
            }))
    }

    fn render_info(&self, cx: &mut Context<Self>) -> AnyElement {
        let manager = self.tabs.read(cx);
        let Some(tab) = manager.active_tab() else {
            return div()
                .p(px(theme::SPACE_2))
                .text_xs()
                .text_color(theme::TEXT_MUTED)
                .child("No session open.")
                .into_any_element();
        };
        let pane = tab.focused_pane();
        let state = pane.map(|p| p.state).unwrap_or(ConnectionState::Idle);
        let profile = pane.and_then(|p| {
            self.store
                .read(cx)
                .workspace
                .find_profile(p.profile_id)
                .cloned()
        });
        let (kind_label, detail) = match profile.as_ref().map(|p| &p.kind) {
            Some(ProfileKind::Ssh {
                host, port, user, ..
            }) => ("SSH".into(), format!("{user}@{host}:{port}")),
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
                    .unwrap_or_else(|| "default shell".into());
                ("Local".into(), shell_label)
            }
            None => ("—".into(), tab.title.clone()),
        };
        let name = profile
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| tab.title.clone());
        let cwd = pane
            .and_then(|p| p.terminal.as_ref())
            .and_then(|t| t.read(cx).working_directory())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".into());
        let (cols, rows) = pane
            .and_then(|p| p.terminal.as_ref())
            .map(|t| t.read(cx).dimensions())
            .unwrap_or((0, 0));
        let size = if cols > 0 && rows > 0 {
            format!("{cols}×{rows}")
        } else {
            "—".into()
        };
        let state_label = match state {
            ConnectionState::Connected => "Connected",
            ConnectionState::Connecting => "Connecting",
            ConnectionState::Disconnected => "Disconnected",
            ConnectionState::Failed => "Failed",
            ConnectionState::Idle => "Idle",
        };

        fn row(label: &str, value: String) -> Div {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child(label.to_string()),
                )
                .child(div().text_xs().text_color(theme::TEXT).child(value))
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .p(px(theme::SPACE_2))
            .gap(px(theme::SPACE_2))
            .child(row("Profile", name))
            .child(row("Kind", kind_label))
            .child(row("Target", detail))
            .child(row("State", state_label.into()))
            .child(row("Working directory", cwd))
            .child(row("Size", size))
            .into_any_element()
    }
}

fn format_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n = n as f64;
    if n >= GB {
        format!("{:.1} GB", n / GB)
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else if n >= KB {
        format!("{:.0} KB", n / KB)
    } else {
        format!("{n:.0} B")
    }
}

impl Focusable for ContextPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ContextPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep Files browser in sync when the focused pane changes.
        self.sync_session(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::SIDEBAR_BG)
            .border_l_1()
            .border_color(theme::BORDER)
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(32.0))
                    .px(px(theme::SPACE_2))
                    .border_b_1()
                    .border_color(theme::BORDER_SUBTLE)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::TEXT_MUTED)
                            .child("Context"),
                    )
                    .child(
                        svg()
                            .path("icons/ui/panel-right.svg")
                            .size(px(ICON))
                            .text_color(theme::TEXT_MUTED),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .px(px(theme::SPACE_1))
                    .py(px(theme::SPACE_1))
                    .border_b_1()
                    .border_color(theme::BORDER_SUBTLE)
                    .child(self.tab_btn("ctx-tab-files", "Files", PanelTab::Files, cx))
                    .child(self.tab_btn("ctx-tab-info", "Info", PanelTab::Info, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .p(px(theme::SPACE_1))
                    .child(match self.active_tab {
                        PanelTab::Files => self.render_files(cx),
                        PanelTab::Info => self.render_info(cx),
                    }),
            )
    }
}

impl EventEmitter<ContextPanelEvent> for ContextPanel {}
