//! Right context panel — Files (SFTP) + Info (see docs/CONTEXT_PANEL.md).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{ConnectionState, ProfileKind};
use crate::platform;
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
    Running {
        done: u64,
        total: Option<u64>,
        files_done: Option<u32>,
        files_total: Option<u32>,
    },
    Done {
        /// Present for folder transfers (file count).
        files: Option<u32>,
    },
    Failed(String),
}

struct TransferRow {
    id: Uuid,
    label: String,
    direction: TransferDir,
    status: TransferStatus,
    /// Local path for Reveal (download destination or upload source).
    local_path: Option<PathBuf>,
    is_dir: bool,
    /// When the SFTP transfer actually started (after file dialogs).
    started_at: Option<std::time::Instant>,
    /// Set when the transfer finishes or fails.
    elapsed: Option<std::time::Duration>,
    bytes_done: u64,
    bytes_total: Option<u64>,
}

struct TransferMenu {
    id: Uuid,
    position: Point<Pixels>,
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
    /// Bumps on each List so stale replies are ignored.
    list_gen: u64,
    transfers: Vec<TransferRow>,
    transfer_menu: Option<TransferMenu>,
    /// Height share for the file list vs Transfers footer (0.35..=0.9).
    list_ratio: f32,
    files_body_bounds: Option<Bounds<Pixels>>,
    files_sash_drag: bool,
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
        let list_ratio = store
            .read(cx)
            .ui_state
            .context_files_list_ratio
            .clamp(0.35, 0.9);
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
            list_gen: 0,
            transfers: Vec::new(),
            transfer_menu: None,
            list_ratio,
            files_body_bounds: None,
            files_sash_drag: false,
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
                self.list_gen = self.list_gen.wrapping_add(1);
                self.cwd = None;
                self.home = None;
                self.entries.clear();
                self.selected = None;
                self.error = None;
                self.listing = false;
                self.transfers.clear();
                self.transfer_menu = None;
            }
            return;
        };
        if self.bound_pane == Some(pane_id) {
            return;
        }
        self.bound_pane = Some(pane_id);
        self.list_gen = self.list_gen.wrapping_add(1);
        self.cwd = None;
        self.home = None;
        self.entries.clear();
        self.selected = None;
        self.error = None;
        self.listing = false;
        // Keep transfer rows from other panes visible until cleared by user;
        // in-flight ops on the previous pane fail once its SFTP handle is dropped.
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
        self.list_gen = self.list_gen.wrapping_add(1);
        let req_gen = self.list_gen;
        self.listing = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                if req_gen != this.list_gen {
                    return;
                }
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
                    files_done: if is_dir { Some(0) } else { None },
                    files_total: None,
                },
                local_path: None,
                is_dir,
                started_at: None,
                elapsed: None,
                bytes_done: 0,
                bytes_total: if is_dir { None } else { Some(entry.size) },
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

            this.update(cx, |this, cx| {
                if let Some(row) = this.transfers.iter_mut().find(|t| t.id == id) {
                    row.local_path = Some(local.clone());
                    row.started_at = Some(std::time::Instant::now());
                }
                cx.notify();
            })
            .ok();

            let (progress_tx, progress_rx) = flume::bounded::<TransferProgress>(64);
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
                                Ok(Ok(outcome)) => {
                                    let files = if is_dir {
                                        Some(outcome.files)
                                    } else {
                                        None
                                    };
                                    this.finish_transfer(id, files, Some(outcome.bytes));
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
                            files_done: None,
                            files_total: None,
                        },
                        local_path: Some(local.clone()),
                        is_dir: false,
                        started_at: Some(std::time::Instant::now()),
                        elapsed: None,
                        bytes_done: 0,
                        bytes_total: None,
                    },
                );
                cx.notify();
            })
            .ok();

            let (progress_tx, progress_rx) = flume::bounded::<TransferProgress>(64);
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
                                Ok(Ok(outcome)) => {
                                    this.finish_transfer(id, None, Some(outcome.bytes));
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
            let (prev_files, prev_total) = match &row.status {
                TransferStatus::Running {
                    files_done,
                    files_total,
                    ..
                } => (*files_done, *files_total),
                TransferStatus::Done { files } => (*files, *files),
                TransferStatus::Failed(_) => (None, None),
            };
            row.status = TransferStatus::Running {
                done: p.done,
                total: p.total,
                files_done: p.files_done.or(prev_files),
                files_total: p.files_total.or(prev_total),
            };
            if let Some(b) = p.overall_done {
                row.bytes_done = b;
            } else if !row.is_dir {
                row.bytes_done = p.done;
            }
            if let Some(t) = p.overall_total {
                row.bytes_total = Some(t);
            } else if let Some(t) = p.total {
                row.bytes_total = Some(t);
            }
            if row.started_at.is_none() {
                row.started_at = Some(std::time::Instant::now());
            }
        }
    }

    fn finish_transfer(&mut self, id: Uuid, files: Option<u32>, bytes: Option<u64>) {
        if let Some(row) = self.transfers.iter_mut().find(|t| t.id == id) {
            let files = files.or_else(|| match &row.status {
                TransferStatus::Running { files_done, .. } if row.is_dir => *files_done,
                _ => None,
            });
            if let Some(b) = bytes {
                row.bytes_done = b;
                if row.bytes_total.is_none() {
                    row.bytes_total = Some(b);
                }
            }
            row.elapsed = Some(
                row.started_at
                    .map(|t| t.elapsed())
                    .unwrap_or_default(),
            );
            row.status = TransferStatus::Done { files };
        }
    }

    fn fail_transfer(&mut self, id: Uuid, msg: String) {
        if let Some(row) = self.transfers.iter_mut().find(|t| t.id == id) {
            row.elapsed = row.started_at.map(|t| t.elapsed());
            row.status = TransferStatus::Failed(msg);
        }
    }

    fn remove_transfer(&mut self, id: Uuid) {
        self.transfers.retain(|t| t.id != id);
        if self.transfer_menu.as_ref().is_some_and(|m| m.id == id) {
            self.transfer_menu = None;
        }
    }

    fn clear_transfers(&mut self) {
        self.transfers.clear();
        self.transfer_menu = None;
    }

    fn reveal_transfer(&self, id: Uuid) {
        let Some(path) = self
            .transfers
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.local_path.as_ref())
        else {
            return;
        };
        let _ = platform::reveal_in_file_manager(path);
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
            .child(self.render_files_split(entries, selected, cx))
            .into_any_element()
    }

    /// File list + Transfers with a draggable vertical sash.
    fn render_files_split(
        &self,
        entries: Vec<RemoteEntry>,
        selected: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let list_ratio = self.list_ratio.clamp(0.35, 0.9);
        let view = cx.entity();

        div()
            .id("ctx-files-body")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        view.update(cx, |this, _| {
                            this.files_body_bounds = Some(bounds);
                        });
                        bounds
                    },
                    |_bounds, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("ctx-file-list")
                    .h(relative(list_ratio))
                    .w_full()
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
            .child(
                div()
                    .id("ctx-files-sash")
                    .h(px(4.0))
                    .w_full()
                    .flex_shrink_0()
                    .cursor(CursorStyle::ResizeRow)
                    .bg(theme::BORDER)
                    .hover(|s| s.bg(theme::ACCENT))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.files_sash_drag = true;
                            cx.notify();
                        }),
                    ),
            )
            .child(self.render_transfers(1.0 - list_ratio, cx))
    }

    fn render_transfers(&self, height_ratio: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let has_transfers = !self.transfers.is_empty();
        div()
            .id("ctx-transfers")
            .h(relative(height_ratio.clamp(0.1, 0.65)))
            .w_full()
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(theme::SPACE_1))
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::TEXT_MUTED)
                            .child("Transfers"),
                    )
                    .when(has_transfers, |d| {
                        d.child(
                            div()
                                .id("ctx-transfers-clear")
                                .px(px(theme::SPACE_1))
                                .rounded(px(theme::RADIUS_SM))
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                                .child("Clear")
                                .tooltip(|_, cx| Tooltip::text("Clear all transfers", cx))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clear_transfers();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(self.transfers.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_DISABLED)
                        .child("No transfers yet"),
                )
            })
            .children(self.transfers.iter().map(|row| {
                let id = row.id;
                let arrow = match row.direction {
                    TransferDir::Download => "↓",
                    TransferDir::Upload => "↑",
                };
                let status = transfer_status_label(row);
                let color = match &row.status {
                    TransferStatus::Failed(_) => theme::DANGER,
                    TransferStatus::Done { .. } => theme::ICON_LOCAL,
                    _ => theme::TEXT_MUTED,
                };
                div()
                    .id(SharedString::from(format!("xfer-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1))
                    .px(px(2.0))
                    .rounded(px(theme::RADIUS_SM))
                    .text_xs()
                    .hover(|s| s.bg(theme::HOVER))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.transfer_menu = Some(TransferMenu {
                                id,
                                position: event.position,
                            });
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    )
                    .child(div().text_color(theme::TEXT_MUTED).flex_shrink_0().child(arrow))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme::TEXT)
                            .overflow_hidden()
                            .child(row.label.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_color(color)
                            .child(status),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("xfer-rm-{id}")))
                            .flex_shrink_0()
                            .px(px(4.0))
                            .rounded(px(theme::RADIUS_SM))
                            .text_color(theme::TEXT_MUTED)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                            .child("×")
                            .tooltip(|_, cx| Tooltip::text("Remove", cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_transfer(id);
                                cx.notify();
                            })),
                    )
            }))
    }

    fn render_transfer_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.transfer_menu.as_ref()?;
        let id = menu.id;
        let position = menu.position;
        let row = self.transfers.iter().find(|t| t.id == id)?;
        let can_reveal = row
            .local_path
            .as_ref()
            .is_some_and(|p| p.exists());

        Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(Edges {
                        top: px(4.0),
                        right: px(4.0),
                        bottom: px(4.0),
                        left: px(4.0),
                    })
                    .child(
                        div()
                            .min_w(px(180.0))
                            .p(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS))
                            .bg(theme::ELEVATED)
                            .border_1()
                            .border_color(theme::BORDER)
                            .shadow_md()
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(self.transfer_menu_item(
                                "xfer-ctx-reveal",
                                "Reveal in File Explorer",
                                can_reveal,
                                cx,
                                move |this, _cx| {
                                    this.reveal_transfer(id);
                                },
                            ))
                            .child(self.transfer_menu_item(
                                "xfer-ctx-remove",
                                "Remove",
                                true,
                                cx,
                                move |this, cx| {
                                    this.remove_transfer(id);
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .into_any_element(),
        )
    }

    fn transfer_menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_1))
            .rounded(px(theme::RADIUS_SM))
            .text_sm()
            .when(enabled, |d| {
                d.text_color(theme::TEXT)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
            })
            .when(!enabled, |d| d.text_color(theme::TEXT_DISABLED))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if !enabled {
                    return;
                }
                on_click(this, cx);
                this.transfer_menu = None;
                cx.notify();
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

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        let m = (secs / 60.0).floor() as u32;
        let s = (secs % 60.0).floor() as u32;
        format!("{m}m{s:02}s")
    } else {
        let h = (secs / 3600.0).floor() as u32;
        let m = ((secs % 3600.0) / 60.0).floor() as u32;
        format!("{h}h{m:02}m")
    }
}

fn transfer_elapsed(row: &TransferRow) -> Option<std::time::Duration> {
    row.elapsed.or_else(|| row.started_at.map(|t| t.elapsed()))
}

fn transfer_status_label(row: &TransferRow) -> String {
    let mut parts: Vec<String> = Vec::new();

    match &row.status {
        TransferStatus::Running {
            done,
            total,
            files_done,
            files_total,
        } => {
            if row.is_dir || files_total.is_some() || files_done.is_some() {
                let d = files_done.unwrap_or(0);
                parts.push(match files_total {
                    Some(t) => format!("{d}/{t}"),
                    None => "…".into(),
                });
            } else {
                match total {
                    Some(t) if *t > 0 => {
                        parts.push(format!("{}%", ((*done as f64 / *t as f64) * 100.0) as u32));
                    }
                    _ => {}
                }
            }
        }
        TransferStatus::Done { files } => {
            parts.push("Done".into());
            if let Some(n) = files {
                parts.push(format!("{n}/{n}"));
            }
        }
        TransferStatus::Failed(msg) => {
            let short = if msg.len() > 20 {
                format!("{}…", &msg[..20])
            } else {
                msg.clone()
            };
            parts.push(short);
        }
    }

    let size_label = match (row.bytes_done, row.bytes_total) {
        (done, Some(total)) if total > 0 && done > 0 && done < total => {
            Some(format!("{} / {}", format_size(done), format_size(total)))
        }
        (_, Some(total)) if total > 0 => Some(format_size(total)),
        (done, _) if done > 0 => Some(format_size(done)),
        _ => None,
    };
    if let Some(s) = size_label {
        parts.push(s);
    }

    if let Some(d) = transfer_elapsed(row) {
        if !matches!(&row.status, TransferStatus::Failed(_)) || row.started_at.is_some() {
            parts.push(format_duration(d));
        }
    }

    if parts.is_empty() {
        "…".into()
    } else {
        parts.join(" · ")
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
        let transfer_menu = self.render_transfer_menu(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::SIDEBAR_BG)
            .border_l_1()
            .border_color(theme::BORDER)
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.transfer_menu.is_some() {
                        this.transfer_menu = None;
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key.as_str() == "escape" && this.transfer_menu.is_some() {
                    this.transfer_menu = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !this.files_sash_drag {
                        return;
                    }
                    this.files_sash_drag = false;
                    let ratio = this.list_ratio.clamp(0.35, 0.9);
                    this.list_ratio = ratio;
                    this.store.update(cx, |s, _| {
                        s.ui_state.context_files_list_ratio = ratio;
                        s.persist_now();
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !this.files_sash_drag {
                    return;
                }
                let Some(bounds) = this.files_body_bounds else {
                    return;
                };
                let h: f32 = bounds.size.height.into();
                if h <= 0.0 {
                    return;
                }
                let y: f32 = (event.position.y - bounds.origin.y).into();
                this.list_ratio = (y / h).clamp(0.35, 0.9);
                cx.notify();
            }))
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
            .when_some(transfer_menu, |d, menu| d.child(menu))
    }
}

impl EventEmitter<ContextPanelEvent> for ContextPanel {}
