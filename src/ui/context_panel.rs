//! Right context panel — Files (SFTP / local FS) + Info (see docs/CONTEXT_PANEL.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{ConnectionState, ProfileKind};
use crate::platform;
use crate::session::host_info::{self, HostSnapshot};
use crate::session::local_fs;
use crate::session::sftp::{
    RemoteEntry, SftpHandle, SftpRequest, TransferCancel, TransferProgress, join_remote,
    parent_remote, transfer_cancel_flag,
};
use crate::shared::theme;
use crate::ui::rename_edit::RenameEdit;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilesKind {
    Sftp,
    Local,
}

#[derive(Clone)]
enum TransferStatus {
    /// Request is on the transfer lane but the worker has not started it yet.
    Queued,
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
    /// Set on Remove/Clear so the SFTP worker aborts and frees the transfer lane.
    cancel: TransferCancel,
}

struct TransferMenu {
    id: Uuid,
    position: Point<Pixels>,
}

struct EntryMenu {
    path: String,
    position: Point<Pixels>,
}

enum FilesPrompt {
    NewFolder {
        edit: RenameEdit,
    },
    Rename {
        path: String,
        #[allow(dead_code)]
        is_dir: bool,
        edit: RenameEdit,
    },
    Chmod {
        path: String,
        edit: RenameEdit,
    },
    ConfirmDelete {
        path: String,
        is_dir: bool,
        name: String,
    },
}

pub struct ContextPanel {
    store: Entity<WorkspaceStore>,
    tabs: Entity<TabManager>,
    active_tab: PanelTab,
    /// Remote or local cwd for the Files browser.
    cwd: Option<String>,
    home: Option<String>,
    entries: Vec<RemoteEntry>,
    selected: Option<String>,
    listing: bool,
    error: Option<String>,
    /// Pane id we last bound file state to.
    bound_pane: Option<Uuid>,
    files_kind: Option<FilesKind>,
    /// Bumps on each List so stale replies are ignored.
    list_gen: u64,
    /// Transfers keyed by SSH pane id (not shared across servers/tabs).
    transfers_by_pane: HashMap<Uuid, Vec<TransferRow>>,
    transfer_menu: Option<TransferMenu>,
    entry_menu: Option<EntryMenu>,
    prompt: Option<FilesPrompt>,
    /// Host metrics for the Info tab (manual refresh).
    host_info: Option<HostSnapshot>,
    host_info_pane: Option<Uuid>,
    host_info_loading: bool,
    host_info_error: Option<String>,
    /// Height share for the file list vs Transfers footer (0.35..=0.9).
    list_ratio: f32,
    files_body_bounds: Option<Bounds<Pixels>>,
    files_sash_drag: bool,
    focus_handle: FocusHandle,
    _prompt_caret_blink: Option<Task<()>>,
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
            files_kind: None,
            list_gen: 0,
            transfers_by_pane: HashMap::new(),
            transfer_menu: None,
            entry_menu: None,
            prompt: None,
            host_info: None,
            host_info_pane: None,
            host_info_loading: false,
            host_info_error: None,
            list_ratio,
            files_body_bounds: None,
            files_sash_drag: false,
            focus_handle: cx.focus_handle(),
            _prompt_caret_blink: None,
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

    /// SSH SFTP, or a Local profile pane for filesystem browsing.
    fn focused_files(&self, cx: &App) -> Option<(Uuid, FilesKind, Option<SftpHandle>)> {
        let tab = self.tabs.read(cx).active_tab()?;
        let pane = tab.focused_pane()?;
        let id = pane.id;
        if let Some(sftp) = pane.ssh_sftp.clone() {
            return Some((id, FilesKind::Sftp, Some(sftp)));
        }
        let profile = self.store.read(cx).workspace.find_profile(pane.profile_id)?;
        if matches!(profile.kind, ProfileKind::Local { .. }) {
            return Some((id, FilesKind::Local, None));
        }
        None
    }

    fn reset_files_state(&mut self) {
        self.list_gen = self.list_gen.wrapping_add(1);
        self.cwd = None;
        self.home = None;
        self.entries.clear();
        self.selected = None;
        self.error = None;
        self.listing = false;
        self.transfer_menu = None;
        self.entry_menu = None;
        self.prompt = None;
        self._prompt_caret_blink = None;
        self.host_info = None;
        self.host_info_pane = None;
        self.host_info_loading = false;
        self.host_info_error = None;
    }

    /// Drop transfer lists for panes that no longer exist; cancel their jobs.
    fn prune_closed_pane_transfers(&mut self, cx: &App) {
        let live: std::collections::HashSet<Uuid> = self
            .tabs
            .read(cx)
            .tabs
            .iter()
            .flat_map(|t| t.panes.keys().copied())
            .collect();
        let stale: Vec<Uuid> = self
            .transfers_by_pane
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for pane_id in stale {
            if let Some(rows) = self.transfers_by_pane.remove(&pane_id) {
                for row in rows {
                    row.cancel
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }

    fn pane_transfers(&self, pane_id: Uuid) -> &[TransferRow] {
        self.transfers_by_pane
            .get(&pane_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn current_transfers(&self) -> &[TransferRow] {
        match self.bound_pane {
            Some(id) => self.pane_transfers(id),
            None => &[],
        }
    }

    fn pane_transfers_mut(&mut self, pane_id: Uuid) -> &mut Vec<TransferRow> {
        self.transfers_by_pane.entry(pane_id).or_default()
    }

    fn find_transfer_mut(&mut self, id: Uuid) -> Option<&mut TransferRow> {
        for rows in self.transfers_by_pane.values_mut() {
            if let Some(row) = rows.iter_mut().find(|t| t.id == id) {
                return Some(row);
            }
        }
        None
    }

    fn find_transfer(&self, id: Uuid) -> Option<&TransferRow> {
        self.transfers_by_pane
            .values()
            .flat_map(|rows| rows.iter())
            .find(|t| t.id == id)
    }

    fn sync_session(&mut self, cx: &mut Context<Self>) {
        self.prune_closed_pane_transfers(cx);
        let Some((pane_id, kind, sftp)) = self.focused_files(cx) else {
            if self.bound_pane.take().is_some() || self.files_kind.take().is_some() {
                self.reset_files_state();
            }
            return;
        };
        if self.bound_pane == Some(pane_id) && self.files_kind == Some(kind) {
            return;
        }
        self.bound_pane = Some(pane_id);
        self.files_kind = Some(kind);
        self.reset_files_state();
        match kind {
            FilesKind::Sftp => {
                if let Some(sftp) = sftp {
                    self.go_home_sftp(sftp, cx);
                }
            }
            FilesKind::Local => self.go_home_local(cx),
        }
    }

    fn go_home_sftp(&mut self, sftp: SftpHandle, cx: &mut Context<Self>) {
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

    fn go_home_local(&mut self, cx: &mut Context<Self>) {
        let home = self
            .tabs
            .read(cx)
            .active_tab()
            .and_then(|t| t.focused_pane())
            .and_then(|p| p.terminal.as_ref())
            .and_then(|t| t.read(cx).working_directory())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(default_local_home);
        self.home = Some(home.clone());
        self.load_dir(home, cx);
    }

    fn go_home(&mut self, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => {
                if let Some((_, sftp)) = self.focused_sftp(cx) {
                    self.go_home_sftp(sftp, cx);
                }
            }
            Some(FilesKind::Local) => self.go_home_local(cx),
            None => {}
        }
    }

    fn load_dir(&mut self, path: String, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => self.load_dir_sftp(path, cx),
            Some(FilesKind::Local) => self.load_dir_local(path, cx),
            None => {
                self.error = Some("No file session".into());
                cx.notify();
            }
        }
    }

    fn load_dir_sftp(&mut self, path: String, cx: &mut Context<Self>) {
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

    fn load_dir_local(&mut self, path: String, cx: &mut Context<Self>) {
        self.list_gen = self.list_gen.wrapping_add(1);
        let req_gen = self.list_gen;
        self.listing = true;
        self.error = None;
        cx.notify();
        let path_buf = PathBuf::from(&path);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { local_fs::list_dir(&path_buf) })
                .await;
            this.update(cx, |this, cx| {
                if req_gen != this.list_gen {
                    return;
                }
                this.listing = false;
                match result {
                    Ok(entries) => {
                        this.cwd = Some(path);
                        this.entries = entries;
                        this.selected = None;
                        this.error = None;
                    }
                    Err(err) => {
                        this.error = Some(format!("{err:#}"));
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
        let parent = match self.files_kind {
            Some(FilesKind::Local) => local_fs::parent_path(&cwd),
            _ => parent_remote(&cwd),
        };
        if let Some(parent) = parent {
            self.load_dir(parent, cx);
        }
    }

    fn enter_or_download(&mut self, entry: RemoteEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.load_dir(entry.path, cx);
            return;
        }
        match self.files_kind {
            Some(FilesKind::Sftp) => self.download_entry(entry, cx),
            Some(FilesKind::Local) => {
                let _ = platform::reveal_in_file_manager(Path::new(&entry.path));
            }
            None => {}
        }
    }

    fn download_selected(&mut self, cx: &mut Context<Self>) {
        if self.files_kind != Some(FilesKind::Sftp) {
            return;
        }
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| e.path == path).cloned() else {
            return;
        };
        self.download_entry(entry, cx);
    }

    fn download_entry(&mut self, entry: RemoteEntry, cx: &mut Context<Self>) {
        let Some((pane_id, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };
        let remote = entry.path.clone();
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let id = Uuid::new_v4();
        let cancel = transfer_cancel_flag();

        self.pane_transfers_mut(pane_id).insert(
            0,
            TransferRow {
                id,
                label: name.clone(),
                direction: TransferDir::Download,
                status: TransferStatus::Queued,
                local_path: None,
                is_dir,
                started_at: None,
                elapsed: None,
                bytes_done: 0,
                bytes_total: if is_dir { None } else { Some(entry.size) },
                cancel: cancel.clone(),
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
                if let Some(row) = this.find_transfer_mut(id) {
                    row.local_path = Some(local.clone());
                    // Stay Queued until the transfer lane actually starts (first progress).
                    row.status = TransferStatus::Queued;
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
                    cancel,
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
                            // Row may already be gone after Remove/Clear — still OK.
                            if this.find_transfer(id).is_some() {
                                match result {
                                    Ok(Ok(outcome)) => {
                                        let files = if is_dir {
                                            Some(outcome.files)
                                        } else {
                                            None
                                        };
                                        this.finish_transfer(id, files, Some(outcome.bytes));
                                    }
                                    Ok(Err(err)) => {
                                        let msg = format!("{err:#}");
                                        if msg.contains("cancelled") {
                                            this.remove_transfer_silent(id);
                                        } else {
                                            this.fail_transfer(id, msg);
                                        }
                                    }
                                    Err(_) => this.fail_transfer(id, "Transfer cancelled".into()),
                                }
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

    fn upload_file(&mut self, cx: &mut Context<Self>) {
        if self.files_kind != Some(FilesKind::Sftp) {
            return;
        }
        let Some(cwd) = self.cwd.clone() else {
            self.error = Some("Open a remote folder first".into());
            cx.notify();
            return;
        };
        let Some((pane_id, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };

        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Upload file to remote")
                .pick_file()
                .await;
            let Some(handle) = picked else {
                return;
            };
            this.update(cx, |this, cx| {
                this.start_uploads(pane_id, sftp, cwd, vec![handle.path().to_path_buf()], cx);
            })
            .ok();
        })
        .detach();
    }

    fn upload_folder(&mut self, cx: &mut Context<Self>) {
        if self.files_kind != Some(FilesKind::Sftp) {
            return;
        }
        let Some(cwd) = self.cwd.clone() else {
            self.error = Some("Open a remote folder first".into());
            cx.notify();
            return;
        };
        let Some((pane_id, sftp)) = self.focused_sftp(cx) else {
            self.error = Some("No SSH session".into());
            cx.notify();
            return;
        };

        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Upload folder to remote")
                .pick_folder()
                .await;
            let Some(handle) = picked else {
                return;
            };
            this.update(cx, |this, cx| {
                this.start_uploads(pane_id, sftp, cwd, vec![handle.path().to_path_buf()], cx);
            })
            .ok();
        })
        .detach();
    }

    fn start_uploads(
        &mut self,
        pane_id: Uuid,
        sftp: SftpHandle,
        remote_dir: String,
        locals: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        for local in locals {
            self.start_one_upload(pane_id, sftp.clone(), remote_dir.clone(), local, cx);
        }
    }

    fn start_one_upload(
        &mut self,
        pane_id: Uuid,
        sftp: SftpHandle,
        remote_dir: String,
        local: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let id = Uuid::new_v4();
        let label = local
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| local.display().to_string());
        let is_dir = local.is_dir();
        let cancel = transfer_cancel_flag();

        self.pane_transfers_mut(pane_id).insert(
            0,
            TransferRow {
                id,
                label,
                direction: TransferDir::Upload,
                status: TransferStatus::Queued,
                local_path: Some(local.clone()),
                is_dir,
                started_at: None,
                elapsed: None,
                bytes_done: 0,
                bytes_total: None,
                cancel: cancel.clone(),
            },
        );
        cx.notify();

        cx.spawn(async move |this, cx| {
            let (progress_tx, progress_rx) = flume::bounded::<TransferProgress>(64);
            let (reply_tx, reply_rx) = flume::bounded(1);
            if sftp
                .request(SftpRequest::Upload {
                    id,
                    local,
                    remote_dir,
                    progress: progress_tx,
                    reply: reply_tx,
                    cancel,
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
                            if this.find_transfer(id).is_some() {
                                match result {
                                    Ok(Ok(outcome)) => {
                                        let files = if is_dir {
                                            Some(outcome.files)
                                        } else {
                                            None
                                        };
                                        this.finish_transfer(id, files, Some(outcome.bytes));
                                        if let Some(cwd) = this.cwd.clone() {
                                            this.load_dir(cwd, cx);
                                        }
                                    }
                                    Ok(Err(err)) => {
                                        let msg = format!("{err:#}");
                                        if msg.contains("cancelled") {
                                            this.remove_transfer_silent(id);
                                        } else {
                                            this.fail_transfer(id, msg);
                                        }
                                    }
                                    Err(_) => this.fail_transfer(id, "Transfer cancelled".into()),
                                }
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
        if let Some(row) = self.find_transfer_mut(p.id) {
            let (prev_files, prev_total) = match &row.status {
                TransferStatus::Running {
                    files_done,
                    files_total,
                    ..
                } => (*files_done, *files_total),
                TransferStatus::Queued | TransferStatus::Failed(_) => (None, None),
                TransferStatus::Done { files } => (*files, *files),
            };
            // First progress = left the queue and the worker is actually running this job.
            if row.started_at.is_none() {
                row.started_at = Some(std::time::Instant::now());
            }
            row.status = TransferStatus::Running {
                done: p.done,
                total: p.total.or(row.bytes_total),
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
        }
    }

    fn finish_transfer(&mut self, id: Uuid, files: Option<u32>, bytes: Option<u64>) {
        if let Some(row) = self.find_transfer_mut(id) {
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
        if let Some(row) = self.find_transfer_mut(id) {
            row.elapsed = row.started_at.map(|t| t.elapsed());
            row.status = TransferStatus::Failed(msg);
        }
    }

    fn remove_transfer(&mut self, id: Uuid) {
        if let Some(row) = self.find_transfer(id) {
            row.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.remove_transfer_silent(id);
    }

    fn remove_transfer_silent(&mut self, id: Uuid) {
        for rows in self.transfers_by_pane.values_mut() {
            rows.retain(|t| t.id != id);
        }
        if self.transfer_menu.as_ref().is_some_and(|m| m.id == id) {
            self.transfer_menu = None;
        }
    }

    fn clear_transfers(&mut self) {
        let Some(pane_id) = self.bound_pane else {
            return;
        };
        if let Some(rows) = self.transfers_by_pane.get_mut(&pane_id) {
            for row in rows.iter() {
                row.cancel
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            rows.clear();
        }
        self.transfer_menu = None;
    }

    fn reveal_transfer(&self, id: Uuid) {
        let Some(path) = self.find_transfer(id).and_then(|t| t.local_path.as_ref()) else {
            return;
        };
        let _ = platform::reveal_in_file_manager(path);
    }

    fn begin_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cwd.is_none() {
            self.error = Some("Open a folder first".into());
            cx.notify();
            return;
        }
        self.entry_menu = None;
        self.prompt = Some(FilesPrompt::NewFolder {
            edit: RenameEdit::new("New folder"),
        });
        self.start_prompt_caret_blink(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn begin_rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return;
        };
        self.entry_menu = None;
        self.prompt = Some(FilesPrompt::Rename {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            edit: RenameEdit::new(entry.name.clone()),
        });
        self.start_prompt_caret_blink(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn begin_chmod_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        self.entry_menu = None;
        self.prompt = Some(FilesPrompt::Chmod {
            path,
            edit: RenameEdit::new("755"),
        });
        self.start_prompt_caret_blink(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn begin_delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return;
        };
        self.entry_menu = None;
        self._prompt_caret_blink = None;
        self.prompt = Some(FilesPrompt::ConfirmDelete {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            name: entry.name.clone(),
        });
        cx.notify();
    }

    fn start_prompt_caret_blink(&mut self, cx: &mut Context<Self>) {
        self._prompt_caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if let Some(edit) = this.prompt_edit_mut() {
                            edit.caret_visible = !edit.caret_visible;
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    fn cancel_prompt(&mut self, cx: &mut Context<Self>) {
        self.prompt = None;
        self._prompt_caret_blink = None;
        cx.notify();
    }

    fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        self._prompt_caret_blink = None;
        match prompt {
            FilesPrompt::NewFolder { edit } => {
                let name = edit.text.trim().to_string();
                if name.is_empty() || name.contains('/') || name.contains('\\') {
                    self.error = Some("Invalid folder name".into());
                    self.prompt = Some(FilesPrompt::NewFolder { edit });
                    self.start_prompt_caret_blink(cx);
                    cx.notify();
                    return;
                }
                let Some(cwd) = self.cwd.clone() else {
                    return;
                };
                let path = match self.files_kind {
                    Some(FilesKind::Sftp) => join_remote(&cwd, &name),
                    Some(FilesKind::Local) => local_fs::join_child(&cwd, &name)
                        .to_string_lossy()
                        .into_owned(),
                    None => return,
                };
                self.run_mkdir(path, cx);
            }
            FilesPrompt::Rename { path, is_dir, edit } => {
                let name = edit.text.trim().to_string();
                if name.is_empty() || name.contains('/') || name.contains('\\') {
                    self.error = Some("Invalid name".into());
                    self.prompt = Some(FilesPrompt::Rename {
                        path,
                        is_dir,
                        edit: RenameEdit::new(name),
                    });
                    self.start_prompt_caret_blink(cx);
                    cx.notify();
                    return;
                }
                let parent = match self.files_kind {
                    Some(FilesKind::Local) => local_fs::parent_path(&path),
                    _ => parent_remote(&path),
                };
                let Some(parent) = parent.or_else(|| self.cwd.clone()) else {
                    return;
                };
                let to = match self.files_kind {
                    Some(FilesKind::Sftp) => join_remote(&parent, &name),
                    Some(FilesKind::Local) => local_fs::join_child(&parent, &name)
                        .to_string_lossy()
                        .into_owned(),
                    None => return,
                };
                self.run_rename(path, to, cx);
            }
            FilesPrompt::Chmod { path, edit } => match local_fs::parse_mode(&edit.text) {
                Ok(mode) => self.run_chmod(path, mode, cx),
                Err(err) => {
                    self.error = Some(format!("{err:#}"));
                    self.prompt = Some(FilesPrompt::Chmod { path, edit });
                    self.start_prompt_caret_blink(cx);
                    cx.notify();
                }
            },
            FilesPrompt::ConfirmDelete {
                path,
                is_dir,
                name: _,
            } => {
                self.run_remove(path, is_dir, cx);
            }
        }
    }

    fn run_mkdir(&mut self, path: String, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => {
                let Some((_, sftp)) = self.focused_sftp(cx) else {
                    return;
                };
                let (tx, rx) = flume::bounded(1);
                if sftp
                    .request(SftpRequest::Mkdir {
                        path: path.clone(),
                        reply: tx,
                    })
                    .is_err()
                {
                    self.error = Some("SFTP unavailable".into());
                    cx.notify();
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = rx.recv_async().await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(Ok(())) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Ok(Err(err)) => this.error = Some(format!("{err:#}")),
                            Err(_) => this.error = Some("Mkdir cancelled".into()),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Some(FilesKind::Local) => {
                let path_buf = PathBuf::from(&path);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { local_fs::create_dir(&path_buf) })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(()) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Err(err) => this.error = Some(format!("{err:#}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => {}
        }
    }

    fn run_rename(&mut self, from: String, to: String, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => {
                let Some((_, sftp)) = self.focused_sftp(cx) else {
                    return;
                };
                let (tx, rx) = flume::bounded(1);
                if sftp
                    .request(SftpRequest::Rename {
                        from: from.clone(),
                        to: to.clone(),
                        reply: tx,
                    })
                    .is_err()
                {
                    self.error = Some("SFTP unavailable".into());
                    cx.notify();
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = rx.recv_async().await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(Ok(())) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Ok(Err(err)) => this.error = Some(format!("{err:#}")),
                            Err(_) => this.error = Some("Rename cancelled".into()),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Some(FilesKind::Local) => {
                let from_p = PathBuf::from(&from);
                let to_p = PathBuf::from(&to);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { local_fs::rename(&from_p, &to_p) })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(()) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Err(err) => this.error = Some(format!("{err:#}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => {}
        }
    }

    fn run_chmod(&mut self, path: String, mode: u32, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => {
                let Some((_, sftp)) = self.focused_sftp(cx) else {
                    return;
                };
                let (tx, rx) = flume::bounded(1);
                if sftp
                    .request(SftpRequest::Chmod {
                        path: path.clone(),
                        mode,
                        reply: tx,
                    })
                    .is_err()
                {
                    self.error = Some("SFTP unavailable".into());
                    cx.notify();
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = rx.recv_async().await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(Ok(())) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Ok(Err(err)) => this.error = Some(format!("{err:#}")),
                            Err(_) => this.error = Some("Chmod cancelled".into()),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Some(FilesKind::Local) => {
                let path_buf = PathBuf::from(&path);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { local_fs::chmod(&path_buf, mode) })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(()) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Err(err) => this.error = Some(format!("{err:#}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => {}
        }
    }

    fn run_remove(&mut self, path: String, is_dir: bool, cx: &mut Context<Self>) {
        match self.files_kind {
            Some(FilesKind::Sftp) => {
                let Some((_, sftp)) = self.focused_sftp(cx) else {
                    return;
                };
                let (tx, rx) = flume::bounded(1);
                if sftp
                    .request(SftpRequest::Remove {
                        path: path.clone(),
                        is_dir,
                        reply: tx,
                    })
                    .is_err()
                {
                    self.error = Some("SFTP unavailable".into());
                    cx.notify();
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = rx.recv_async().await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(Ok(())) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Ok(Err(err)) => this.error = Some(format!("{err:#}")),
                            Err(_) => this.error = Some("Delete cancelled".into()),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Some(FilesKind::Local) => {
                let path_buf = PathBuf::from(&path);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { local_fs::remove_path(&path_buf, is_dir) })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(()) => {
                                if let Some(cwd) = this.cwd.clone() {
                                    this.load_dir(cwd, cx);
                                }
                            }
                            Err(err) => this.error = Some(format!("{err:#}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => {}
        }
    }

    fn prompt_title(prompt: &FilesPrompt) -> &'static str {
        match prompt {
            FilesPrompt::NewFolder { .. } => "New folder",
            FilesPrompt::Rename { .. } => "Rename",
            FilesPrompt::Chmod { .. } => "Permissions (octal)",
            FilesPrompt::ConfirmDelete { .. } => "Delete?",
        }
    }

    fn prompt_edit_mut(&mut self) -> Option<&mut RenameEdit> {
        match self.prompt.as_mut()? {
            FilesPrompt::NewFolder { edit }
            | FilesPrompt::Rename { edit, .. }
            | FilesPrompt::Chmod { edit, .. } => Some(edit),
            FilesPrompt::ConfirmDelete { .. } => None,
        }
    }

    fn handle_prompt_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.prompt.is_none() {
            return false;
        }
        if matches!(self.prompt, Some(FilesPrompt::ConfirmDelete { .. })) {
            let key = event.keystroke.key.as_str();
            if key == "escape" {
                self.cancel_prompt(cx);
                return true;
            }
            if key == "enter" {
                self.submit_prompt(cx);
                return true;
            }
            return false;
        }

        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let shift = mods.shift;
        let chord = mods.control || mods.platform;

        if key == "enter" {
            self.submit_prompt(cx);
            return true;
        }
        if key == "escape" {
            self.cancel_prompt(cx);
            return true;
        }

        let Some(edit) = self.prompt_edit_mut() else {
            return false;
        };

        if chord && key.eq_ignore_ascii_case("a") {
            edit.select_all();
            cx.notify();
            return true;
        }
        if chord && key.eq_ignore_ascii_case("c") {
            let text = if edit.has_selection() {
                edit.selected_text()
            } else {
                edit.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return true;
        }
        if chord && key.eq_ignore_ascii_case("x") {
            let text = if edit.has_selection() {
                edit.selected_text()
            } else {
                edit.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            if edit.has_selection() {
                edit.delete_selection();
            } else {
                edit.text.clear();
                edit.cursor = 0;
                edit.anchor = 0;
            }
            cx.notify();
            return true;
        }
        if (chord && key.eq_ignore_ascii_case("v"))
            || (mods.shift && key.eq_ignore_ascii_case("insert"))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                let cleaned = text.replace('\r', "").replace('\n', "");
                if !cleaned.is_empty() {
                    edit.insert(&cleaned);
                    cx.notify();
                }
            }
            return true;
        }
        if key == "backspace" {
            edit.backspace();
            cx.notify();
            return true;
        }
        if key == "delete" {
            edit.delete_forward();
            cx.notify();
            return true;
        }
        if key == "left" {
            edit.move_left(shift);
            cx.notify();
            return true;
        }
        if key == "right" {
            edit.move_right(shift);
            cx.notify();
            return true;
        }
        if key == "home" {
            edit.move_home(shift);
            cx.notify();
            return true;
        }
        if key == "end" {
            edit.move_end(shift);
            cx.notify();
            return true;
        }
        if let Some(typed) = event.keystroke.key_char.as_deref() {
            let cleaned = typed.replace('\r', "").replace('\n', "");
            if !cleaned.is_empty() && !chord {
                edit.insert(&cleaned);
                cx.notify();
                return true;
            }
        }
        false
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
                if tab == PanelTab::Info {
                    this.ensure_host_info(cx);
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
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
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
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .when(!enabled, |d| d.text_color(theme::TEXT_DISABLED))
            .child(label)
    }

    fn render_files(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let files = self.focused_files(cx);
        if files.is_none() {
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
                            "Open a Local or SSH session. Local panes browse the filesystem; \
                             SSH panes use SFTP for browse, upload, and download.",
                        ),
                )
                .into_any_element();
        }

        let is_sftp = self.files_kind == Some(FilesKind::Sftp);
        let cwd = self.cwd.clone().unwrap_or_else(|| "…".into());
        let can_up = self.cwd.as_ref().is_some_and(|p| match self.files_kind {
            Some(FilesKind::Local) => local_fs::parent_path(p).is_some(),
            _ => parent_remote(p).is_some(),
        });
        let listing = self.listing;
        let error = self.error.clone();
        let entries = self.entries.clone();
        let selected = self.selected.clone();
        let has_sel = selected.is_some();

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
                    .child(self.nav_btn("ctx-up", "↑", "Parent folder", can_up, cx, |this, _, cx| {
                        this.go_up(cx);
                    }))
                    .child(self.nav_btn("ctx-home", "⌂", "Home", true, cx, |this, _, cx| {
                        this.go_home(cx);
                    }))
                    .child(self.nav_btn("ctx-refresh", "↻", "Refresh", true, cx, |this, _, cx| {
                        if let Some(cwd) = this.cwd.clone() {
                            this.load_dir(cwd, cx);
                        }
                    }))
                    .child(self.nav_btn(
                        "ctx-mkdir",
                        "+",
                        "New folder",
                        self.cwd.is_some(),
                        cx,
                        |this, window, cx| this.begin_new_folder(window, cx),
                    ))
                    .child(div().flex_1())
                    .when(is_sftp, |d| {
                        d.child(self.nav_btn(
                            "ctx-download",
                            "↓",
                            "Download selected",
                            has_sel,
                            cx,
                            |this, _, cx| this.download_selected(cx),
                        ))
                        .child(self.nav_btn(
                            "ctx-upload",
                            "↑+",
                            "Upload file…",
                            true,
                            cx,
                            |this, _, cx| this.upload_file(cx),
                        ))
                        .child(self.nav_btn(
                            "ctx-upload-folder",
                            "⬆📁",
                            "Upload folder…",
                            true,
                            cx,
                            |this, _, cx| this.upload_folder(cx),
                        ))
                    }),
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
            .when_some(self.render_prompt_bar(cx), |d, bar| d.child(bar))
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
            .child(self.render_files_split(entries, selected, is_sftp, cx))
            .into_any_element()
    }

    fn render_prompt_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.prompt.as_ref()?;
        let title = Self::prompt_title(prompt);
        let confirm_label = match prompt {
            FilesPrompt::ConfirmDelete { name, is_dir, .. } => {
                if *is_dir {
                    format!("Delete folder “{name}”?")
                } else {
                    format!("Delete “{name}”?")
                }
            }
            _ => String::new(),
        };
        let edit_el = match prompt {
            FilesPrompt::NewFolder { edit }
            | FilesPrompt::Rename { edit, .. }
            | FilesPrompt::Chmod { edit, .. } => Some(edit.into_element()),
            FilesPrompt::ConfirmDelete { .. } => None,
        };

        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_1))
                .px(px(theme::SPACE_1))
                .py(px(theme::SPACE_1))
                .rounded(px(theme::RADIUS_SM))
                .border_1()
                .border_color(theme::BORDER)
                .bg(theme::ELEVATED)
                .track_focus(&self.focus_handle)
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::TEXT)
                        .child(title),
                )
                .when(!confirm_label.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(confirm_label),
                    )
                })
                .when_some(edit_el, |d, el| {
                    d.child(
                        div()
                            .id("ctx-prompt-input")
                            .w_full()
                            .child(el),
                    )
                })
                .child(
                    div()
                        .flex()
                        .gap(px(theme::SPACE_1))
                        .child(
                            div()
                                .id("ctx-prompt-ok")
                                .px(px(theme::SPACE_2))
                                .py(px(theme::SPACE_1))
                                .rounded(px(theme::RADIUS_SM))
                                .text_xs()
                                .bg(theme::ACCENT)
                                .text_color(theme::TEXT)
                                .cursor_pointer()
                                .child(if matches!(
                                    self.prompt,
                                    Some(FilesPrompt::ConfirmDelete { .. })
                                ) {
                                    "Delete"
                                } else {
                                    "OK"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.submit_prompt(cx))),
                        )
                        .child(
                            div()
                                .id("ctx-prompt-cancel")
                                .px(px(theme::SPACE_2))
                                .py(px(theme::SPACE_1))
                                .rounded(px(theme::RADIUS_SM))
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::HOVER))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_prompt(cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// File list + Transfers with a draggable vertical sash.
    fn render_files_split(
        &self,
        entries: Vec<RemoteEntry>,
        selected: Option<String>,
        accept_drops: bool,
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
                    .when(accept_drops, |d| {
                        d.drag_over::<ExternalPaths>(|style, _, _, _| {
                            style.bg(theme::ACCENT.opacity(0.2))
                        })
                        .can_drop(|dragged: &dyn std::any::Any, _, _| {
                            dragged.is::<ExternalPaths>()
                        })
                        .on_drop(cx.listener(
                            move |this, paths: &ExternalPaths, _, cx| {
                                if this.files_kind != Some(FilesKind::Sftp) {
                                    return;
                                }
                                let Some(cwd) = this.cwd.clone() else {
                                    this.error = Some("Open a remote folder first".into());
                                    cx.notify();
                                    return;
                                };
                                let Some((pane_id, sftp)) = this.focused_sftp(cx) else {
                                    return;
                                };
                                let locals: Vec<PathBuf> = paths.paths().to_vec();
                                if locals.is_empty() {
                                    return;
                                }
                                this.start_uploads(pane_id, sftp, cwd, locals, cx);
                            },
                        ))
                    })
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
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.selected = Some(path.clone());
                                    this.transfer_menu = None;
                                    this.entry_menu = Some(EntryMenu {
                                        path: path.clone(),
                                        position: event.position,
                                    });
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                this.entry_menu = None;
                                this.selected = Some(entry_click.path.clone());
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
        let transfers = self.current_transfers();
        let has_transfers = !transfers.is_empty();
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
            .when(transfers.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_DISABLED)
                        .child("No transfers yet"),
                )
            })
            .children(transfers.iter().map(|row| {
                let id = row.id;
                let arrow = match row.direction {
                    TransferDir::Download => "↓",
                    TransferDir::Upload => "↑",
                };
                let status = transfer_status_label(row);
                let color = match &row.status {
                    TransferStatus::Failed(_) => theme::DANGER,
                    TransferStatus::Done { .. } => theme::ICON_LOCAL,
                    TransferStatus::Queued => theme::TEXT_DISABLED,
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
        let row = self.find_transfer(id)?;
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
                                move |this, _, _cx| {
                                    this.reveal_transfer(id);
                                },
                            ))
                            .child(self.transfer_menu_item(
                                "xfer-ctx-remove",
                                "Remove",
                                true,
                                cx,
                                move |this, _, cx| {
                                    this.remove_transfer(id);
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .into_any_element(),
        )
    }

    fn render_entry_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.entry_menu.as_ref()?;
        let path = menu.path.clone();
        let position = menu.position;
        let entry = self.entries.iter().find(|e| e.path == path)?;
        let is_dir = entry.is_dir;
        let is_local = self.files_kind == Some(FilesKind::Local);

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
                            .min_w(px(160.0))
                            .p(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS))
                            .bg(theme::ELEVATED)
                            .border_1()
                            .border_color(theme::BORDER)
                            .shadow_md()
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(self.transfer_menu_item(
                                "file-ctx-new",
                                "New folder…",
                                true,
                                cx,
                                |this, window, cx| this.begin_new_folder(window, cx),
                            ))
                            .child(self.transfer_menu_item(
                                "file-ctx-rename",
                                "Rename…",
                                true,
                                cx,
                                |this, window, cx| this.begin_rename_selected(window, cx),
                            ))
                            .child(self.transfer_menu_item(
                                "file-ctx-chmod",
                                "Permissions…",
                                true,
                                cx,
                                |this, window, cx| this.begin_chmod_selected(window, cx),
                            ))
                            .child(self.transfer_menu_item(
                                "file-ctx-delete",
                                "Delete…",
                                true,
                                cx,
                                |this, _, cx| this.begin_delete_selected(cx),
                            ))
                            .when(is_local, |d| {
                                d.child(self.transfer_menu_item(
                                    "file-ctx-reveal",
                                    "Reveal in File Explorer",
                                    true,
                                    cx,
                                    move |this, _, _cx| {
                                        let _ = platform::reveal_in_file_manager(Path::new(&path));
                                        this.entry_menu = None;
                                    },
                                ))
                            })
                            .when(!is_dir && !is_local, |d| {
                                d.child(self.transfer_menu_item(
                                    "file-ctx-download",
                                    "Download",
                                    true,
                                    cx,
                                    |this, _, cx| this.download_selected(cx),
                                ))
                            }),
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
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
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
            .on_click(cx.listener(move |this, _, window, cx| {
                if !enabled {
                    return;
                }
                on_click(this, window, cx);
                this.transfer_menu = None;
                this.entry_menu = None;
                cx.notify();
            }))
    }

    fn refresh_host_info(&mut self, cx: &mut Context<Self>) {
        let Some((pane_id, kind, sftp)) = self.focused_files(cx) else {
            // SSH connecting without SFTP yet, or no pane — try SSH via focused pane sftp only.
            let tab = self.tabs.read(cx).active_tab();
            let pane = tab.and_then(|t| t.focused_pane());
            let Some(pane) = pane else {
                self.host_info_error = Some("No session open".into());
                cx.notify();
                return;
            };
            if let Some(sftp) = pane.ssh_sftp.clone() {
                self.start_ssh_host_probe(pane.id, sftp, cx);
                return;
            }
            // Local without going through focused_files? use profile kind.
            let is_local = self
                .store
                .read(cx)
                .workspace
                .find_profile(pane.profile_id)
                .is_some_and(|p| matches!(p.kind, ProfileKind::Local { .. }));
            if is_local {
                self.start_local_host_probe(pane.id, cx);
            } else {
                self.host_info_error = Some("Connect SSH to refresh host info".into());
                cx.notify();
            }
            return;
        };
        match kind {
            FilesKind::Local => self.start_local_host_probe(pane_id, cx),
            FilesKind::Sftp => {
                if let Some(sftp) = sftp {
                    self.start_ssh_host_probe(pane_id, sftp, cx);
                } else {
                    self.host_info_error = Some("SFTP unavailable".into());
                    cx.notify();
                }
            }
        }
    }

    /// Load host info when entering Info or after pane change (no auto interval).
    fn ensure_host_info(&mut self, cx: &mut Context<Self>) {
        if self.host_info_loading {
            return;
        }
        let pane_id = self
            .tabs
            .read(cx)
            .active_tab()
            .and_then(|t| t.focused_pane())
            .map(|p| p.id);
        let Some(pane_id) = pane_id else {
            self.host_info = None;
            self.host_info_pane = None;
            self.host_info_error = Some("No session open".into());
            return;
        };
        if self.host_info.is_some() && self.host_info_pane == Some(pane_id) {
            return;
        }
        self.refresh_host_info(cx);
    }

    fn start_local_host_probe(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        self.host_info_loading = true;
        self.host_info_error = None;
        self.host_info_pane = Some(pane_id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { host_info::collect_local() })
                .await;
            this.update(cx, |this, cx| {
                this.host_info_loading = false;
                match result {
                    Ok(snap) => {
                        this.host_info = Some(snap);
                        this.host_info_error = None;
                    }
                    Err(err) => {
                        this.host_info_error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_ssh_host_probe(&mut self, pane_id: Uuid, sftp: SftpHandle, cx: &mut Context<Self>) {
        self.host_info_loading = true;
        self.host_info_error = None;
        self.host_info_pane = Some(pane_id);
        cx.notify();
        let (tx, rx) = flume::bounded(1);
        if sftp
            .request(SftpRequest::HostProbe { reply: tx })
            .is_err()
        {
            self.host_info_loading = false;
            self.host_info_error = Some("SSH session unavailable".into());
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.host_info_loading = false;
                match result {
                    Ok(Ok(snap)) => {
                        this.host_info = Some(snap);
                        this.host_info_error = None;
                    }
                    Ok(Err(err)) => {
                        this.host_info_error = Some(format!("{err:#}"));
                    }
                    Err(_) => {
                        this.host_info_error = Some("Host probe cancelled".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_info(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_host_info(cx);

        let loading = self.host_info_loading;
        let err = self.host_info_error.clone();
        let snap = self.host_info.clone();

        // Industry pattern (narrow side panel): label+% · full-width continuous meter · used/total.
        // Severity on the fill (≥90% danger) — same idea as Activity Monitor / Task Manager.
        fn resource_meter(label: String, used: u64, total: u64, ratio: f32) -> Div {
            let ratio = ratio.clamp(0.0, 1.0);
            let pct = (ratio * 100.0) as u32;
            let fill = if ratio >= 0.9 {
                theme::DANGER
            } else {
                theme::ACCENT
            };
            let pct_color = if ratio >= 0.9 {
                theme::DANGER
            } else {
                theme::TEXT
            };
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(theme::SPACE_1))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(pct_color)
                                .child(format!("{pct}%")),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .bg(theme::BORDER_SUBTLE)
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .rounded(px(3.0))
                                .bg(fill)
                                .w(relative(ratio)),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child(format!(
                            "{} / {}",
                            host_info::format_bytes(used),
                            host_info::format_bytes(total)
                        )),
                )
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(theme::SPACE_2))
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
                            .child("Host"),
                    )
                    .child(
                        div()
                            .id("ctx-info-refresh")
                            .px(px(theme::SPACE_2))
                            .py(px(theme::SPACE_1))
                            .rounded(px(theme::RADIUS_SM))
                            .text_xs()
                            .text_color(if loading {
                                theme::TEXT_DISABLED
                            } else {
                                theme::TEXT_MUTED
                            })
                            .when(!loading, |d| {
                                d.cursor_pointer()
                                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                            })
                            .child(if loading { "…" } else { "↻" })
                            .tooltip(|_, cx| Tooltip::text("Refresh host info", cx))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.host_info_loading {
                                    return;
                                }
                                this.refresh_host_info(cx);
                            })),
                    ),
            )
            .when(loading && snap.is_none(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child("Loading host info…"),
                )
            })
            .when_some(err, |d, msg| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme::DANGER)
                        .child(msg),
                )
            })
            .when_some(snap, |d, s| {
                let os_line = if s.kernel.is_empty() {
                    s.os.clone()
                } else {
                    format!("{} · {}", s.os, s.kernel)
                };
                let cpu_line = {
                    let model = if s.cpu_model.is_empty() {
                        "—"
                    } else {
                        s.cpu_model.as_str()
                    };
                    // Keep the identity line short: cores (+ usage if known).
                    if let Some(pct) = s.cpu_usage_pct {
                        format!("{model} · {} cores · {pct:.0}%", s.cpu_cores)
                    } else {
                        format!("{model} · {} cores", s.cpu_cores)
                    }
                };
                let footer = match &s.load {
                    Some(load) if !load.is_empty() => {
                        format!(
                            "Load {load} · Up {}",
                            host_info::format_uptime(s.uptime_secs)
                        )
                    }
                    _ => format!("Up {}", host_info::format_uptime(s.uptime_secs)),
                };

                d.child(
                    // Identity: hostname as title, OS/CPU as muted lines.
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::TEXT)
                                .overflow_hidden()
                                .child(s.hostname.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .overflow_hidden()
                                .child(os_line),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .overflow_hidden()
                                .child(cpu_line),
                        ),
                )
                .child(
                    div()
                        .mt(px(theme::SPACE_1))
                        .pt(px(theme::SPACE_2))
                        .border_t_1()
                        .border_color(theme::BORDER_SUBTLE)
                        .flex()
                        .flex_col()
                        .gap(px(theme::SPACE_2))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::TEXT_MUTED)
                                .child("Resources"),
                        )
                        .child(resource_meter(
                            "Memory".into(),
                            s.mem_used,
                            s.mem_total,
                            s.mem_ratio(),
                        ))
                        .children(s.disks.into_iter().map(|d| {
                            resource_meter(
                                format!("Disk ({})", d.mount),
                                d.used,
                                d.total,
                                d.ratio(),
                            )
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::TEXT_MUTED)
                                .child(footer),
                        ),
                )
            })
            .into_any_element()
    }
}

fn default_local_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|p| PathBuf::from(p).to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "C:\\".into()
            } else {
                "/".into()
            }
        })
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

fn format_rate(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    } else if bytes_per_sec >= KB {
        format!("{:.0} KB/s", bytes_per_sec / KB)
    } else if bytes_per_sec > 0.0 {
        format!("{:.0} B/s", bytes_per_sec)
    } else {
        String::new()
    }
}

fn transfer_status_label(row: &TransferRow) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Folder jobs first walk the tree (files_done stays 0, bytes_done stays 0 while
    // files_total / bytes_total grow). Queued means the transfer lane has not started yet.
    let scanning = row.is_dir
        && row.bytes_done == 0
        && matches!(
            &row.status,
            TransferStatus::Running {
                files_done: Some(0) | None,
                ..
            }
        );

    match &row.status {
        TransferStatus::Queued => {
            parts.push("Queued".into());
        }
        TransferStatus::Running {
            done,
            total,
            files_done,
            files_total,
        } => {
            if scanning {
                parts.push(match files_total {
                    Some(t) => format!("Scanning · {t}"),
                    None => "Scanning…".into(),
                });
            } else if row.is_dir || files_total.is_some() || files_done.is_some() {
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
                    _ if *done > 0 => {
                        parts.push(format_size(*done));
                    }
                    _ => {
                        parts.push("…".into());
                    }
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
            let lower = msg.to_ascii_lowercase();
            if lower.contains("cancelled") || lower.contains("canceled") {
                parts.push("Cancelled".into());
            } else {
                let short = if msg.len() > 28 {
                    format!("{}…", &msg[..28])
                } else {
                    msg.clone()
                };
                parts.push(format!("Failed · {short}"));
            }
        }
    }

    // Queued: optional known size only (no rate / elapsed — not running yet).
    if matches!(&row.status, TransferStatus::Queued) {
        if let Some(total) = row.bytes_total.filter(|t| *t > 0) {
            parts.push(format_size(total));
        }
        return parts.join(" · ");
    }

    let size_label = match (row.bytes_done, row.bytes_total, scanning) {
        (done, Some(total), false) if total > 0 && done > 0 && done < total => {
            Some(format!("{} / {}", format_size(done), format_size(total)))
        }
        (_, Some(total), true) if total > 0 => Some(format_size(total)),
        (done, Some(total), false) if total > 0 && done >= total => Some(format_size(total)),
        (_, Some(total), false) if total > 0 && row.bytes_done == 0 => Some(format_size(total)),
        (done, None, false) if done > 0 => Some(format_size(done)),
        _ => None,
    };
    if let Some(s) = size_label {
        // Avoid duplicating size when Running already pushed format_size(done) above.
        if !parts.iter().any(|p| p == &s) {
            parts.push(s);
        }
    }

    // Live average rate while bytes are moving (not during folder scan).
    if !scanning
        && matches!(&row.status, TransferStatus::Running { .. })
        && row.bytes_done > 0
    {
        if let Some(started) = row.started_at {
            let secs = started.elapsed().as_secs_f64().max(0.001);
            let rate = format_rate(row.bytes_done as f64 / secs);
            if !rate.is_empty() {
                parts.push(rate);
            }
        }
    }

    if let Some(d) = transfer_elapsed(row) {
        if matches!(
            &row.status,
            TransferStatus::Running { .. } | TransferStatus::Done { .. }
        ) || (matches!(&row.status, TransferStatus::Failed(_)) && row.started_at.is_some())
        {
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
        let entry_menu = self.render_entry_menu(cx);

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
                    let mut changed = false;
                    if this.transfer_menu.is_some() {
                        this.transfer_menu = None;
                        changed = true;
                    }
                    if this.entry_menu.is_some() {
                        this.entry_menu = None;
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.handle_prompt_key(event, cx) {
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key.as_str() != "escape" {
                    return;
                }
                if this.transfer_menu.is_some() || this.entry_menu.is_some() {
                    this.transfer_menu = None;
                    this.entry_menu = None;
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
            .when_some(entry_menu, |d, menu| d.child(menu))
    }
}

impl EventEmitter<ContextPanelEvent> for ContextPanel {}
