//! SFTP requests served on the same russh session as the interactive shell.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use flume::{Receiver, Sender};
use russh::client;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ssh::ClientHandler;

/// Shared flag: UI sets true on Remove/Clear so the transfer lane aborts and frees up.
pub type TransferCancel = Arc<AtomicBool>;

pub fn transfer_cancel_flag() -> TransferCancel {
    Arc::new(AtomicBool::new(false))
}

fn ensure_not_cancelled(cancel: &TransferCancel) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    Ok(())
}

/// One remote directory entry for the Files browser.
#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Last modified time as Unix seconds, when the backend provides it.
    pub mtime: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub id: uuid::Uuid,
    pub done: u64,
    pub total: Option<u64>,
    /// Files completed so far (recursive folder transfers).
    pub files_done: Option<u32>,
    /// Total files in a recursive transfer (after / during pre-count).
    pub files_total: Option<u32>,
    /// Bytes transferred for the whole job (folders accumulate).
    pub overall_done: Option<u64>,
    /// Total bytes for the whole job (file size or sum of folder files).
    pub overall_total: Option<u64>,
}

/// Result of a finished upload/download.
#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub files: u32,
    pub bytes: u64,
}

fn progress_msg(
    id: uuid::Uuid,
    done: u64,
    total: Option<u64>,
    files_done: Option<u32>,
    files_total: Option<u32>,
    overall_done: Option<u64>,
    overall_total: Option<u64>,
) -> TransferProgress {
    TransferProgress {
        id,
        done,
        total,
        files_done,
        files_total,
        overall_done,
        overall_total,
    }
}

pub enum SftpRequest {
    /// Resolve session home (canonicalize ".").
    Home {
        reply: Sender<Result<String>>,
    },
    List {
        path: String,
        reply: Sender<Result<Vec<RemoteEntry>>>,
    },
    Download {
        id: uuid::Uuid,
        remote: String,
        local: PathBuf,
        progress: Sender<TransferProgress>,
        reply: Sender<Result<TransferOutcome>>,
        cancel: TransferCancel,
    },
    Upload {
        id: uuid::Uuid,
        local: PathBuf,
        remote_dir: String,
        progress: Sender<TransferProgress>,
        reply: Sender<Result<TransferOutcome>>,
        cancel: TransferCancel,
    },
    /// Create a remote directory (browse lane).
    Mkdir {
        path: String,
        reply: Sender<Result<()>>,
    },
    /// Remove a file or directory tree (browse lane).
    Remove {
        path: String,
        is_dir: bool,
        reply: Sender<Result<()>>,
    },
    Rename {
        from: String,
        to: String,
        reply: Sender<Result<()>>,
    },
    /// Set Unix permission bits (e.g. 0o755). File type bits are preserved when possible.
    Chmod {
        path: String,
        mode: u32,
        reply: Sender<Result<()>>,
    },
    /// One-shot remote host metrics (runs on the SSH Handle, not SFTP).
    HostProbe {
        reply: Sender<Result<crate::session::host_info::HostSnapshot>>,
    },
}

/// Cloneable handle used by the UI to talk to the SSH thread's SFTP pool.
#[derive(Clone)]
pub struct SftpHandle {
    tx: Sender<SftpRequest>,
}

impl SftpHandle {
    pub fn request(&self, req: SftpRequest) -> Result<()> {
        self.tx
            .send(req)
            .map_err(|_| anyhow::anyhow!("SFTP channel closed"))
    }
}

pub fn channel_pair() -> (SftpHandle, Receiver<SftpRequest>) {
    let (tx, rx) = flume::unbounded();
    (SftpHandle { tx }, rx)
}

/// Max concurrent SFTP subsystem channels across the whole app.
const GLOBAL_SFTP_CHANNEL_BUDGET: usize = 12;
/// Close an idle lane session after this long with no requests.
const LANE_IDLE_SECS: u64 = 90;

static OPEN_SFTP_CHANNELS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn try_acquire_channel_budget() -> bool {
    use std::sync::atomic::Ordering;
    loop {
        let cur = OPEN_SFTP_CHANNELS.load(Ordering::SeqCst);
        if cur >= GLOBAL_SFTP_CHANNEL_BUDGET {
            return false;
        }
        if OPEN_SFTP_CHANNELS
            .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
    }
}

fn release_channel_budget() {
    use std::sync::atomic::Ordering;
    OPEN_SFTP_CHANNELS.fetch_sub(1, Ordering::SeqCst);
}

/// Holds one budget unit; released on drop.
struct ChannelBudgetGuard;

impl Drop for ChannelBudgetGuard {
    fn drop(&mut self) {
        release_channel_budget();
    }
}

#[derive(Clone, Copy)]
enum LaneKind {
    Browse,
    Transfer,
}

fn is_browse_request(req: &SftpRequest) -> bool {
    matches!(
        req,
        SftpRequest::Home { .. }
            | SftpRequest::List { .. }
            | SftpRequest::Mkdir { .. }
            | SftpRequest::Remove { .. }
            | SftpRequest::Rename { .. }
            | SftpRequest::Chmod { .. }
    )
}

fn is_host_probe(req: &SftpRequest) -> bool {
    matches!(req, SftpRequest::HostProbe { .. })
}

/// Dual-lane SFTP pool on one SSH handle: browse and transfer never block each other.
/// Sessions are opened lazily and closed after idle; closing `req_rx` tears the pool down.
pub async fn run_sftp_worker(
    session: Arc<client::Handle<ClientHandler>>,
    req_rx: Receiver<SftpRequest>,
) {
    let (browse_tx, browse_rx) = flume::unbounded::<SftpRequest>();
    let (transfer_tx, transfer_rx) = flume::unbounded::<SftpRequest>();

    let browse_session = Arc::clone(&session);
    let transfer_session = Arc::clone(&session);
    let browse_task = tokio::spawn(async move {
        run_lane(browse_session, browse_rx, LaneKind::Browse).await;
    });
    let transfer_task = tokio::spawn(async move {
        run_lane(transfer_session, transfer_rx, LaneKind::Transfer).await;
    });

    while let Ok(req) = req_rx.recv_async().await {
        if is_host_probe(&req) {
            let SftpRequest::HostProbe { reply } = req else {
                continue;
            };
            let session = Arc::clone(&session);
            tokio::spawn(async move {
                let result = crate::session::host_info::collect_via_ssh(&session).await;
                let _ = reply.send(result);
            });
            continue;
        }
        if is_browse_request(&req) {
            if browse_tx.send(req).is_err() {
                break;
            }
        } else if transfer_tx.send(req).is_err() {
            break;
        }
    }

    // Dropping senders ends both lanes; await so channels are closed before SSH teardown races.
    drop(browse_tx);
    drop(transfer_tx);
    let _ = browse_task.await;
    let _ = transfer_task.await;
}

async fn run_lane(
    session: Arc<client::Handle<ClientHandler>>,
    req_rx: Receiver<SftpRequest>,
    kind: LaneKind,
) {
    let idle = std::time::Duration::from_secs(LANE_IDLE_SECS);
    let mut sftp: Option<(SftpSession, ChannelBudgetGuard)> = None;

    loop {
        if sftp.is_some() {
            tokio::select! {
                biased;
                msg = req_rx.recv_async() => {
                    match msg {
                        Ok(req) => {
                            if let Err(err) = ensure_session(&session, &mut sftp, kind).await {
                                reply_open_err(&req, err);
                                continue;
                            }
                            let Some((ref s, _)) = sftp else { continue };
                            dispatch_request(s, req).await;
                        }
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(idle) => {
                    // Idle reclaim: drop session and return global budget.
                    sftp = None;
                }
            }
        } else {
            let Ok(req) = req_rx.recv_async().await else {
                break;
            };
            if let Err(err) = ensure_session(&session, &mut sftp, kind).await {
                reply_open_err(&req, err);
                continue;
            }
            let Some((ref s, _)) = sftp else { continue };
            dispatch_request(s, req).await;
        }
    }

    // Lane ends when req_rx closes; budget returns via ChannelBudgetGuard::drop.
}

async fn ensure_session(
    session: &client::Handle<ClientHandler>,
    sftp: &mut Option<(SftpSession, ChannelBudgetGuard)>,
    kind: LaneKind,
) -> Result<()> {
    if sftp.is_some() {
        return Ok(());
    }
    if !try_acquire_channel_budget() {
        let label = match kind {
            LaneKind::Browse => "browse",
            LaneKind::Transfer => "transfer",
        };
        bail!(
            "SFTP {label} unavailable: too many open SFTP channels (max {GLOBAL_SFTP_CHANNEL_BUDGET})"
        );
    }
    let guard = ChannelBudgetGuard;
    match open_sftp(session).await {
        Ok(s) => {
            *sftp = Some((s, guard));
            Ok(())
        }
        Err(err) => {
            drop(guard);
            Err(err)
        }
    }
}

async fn dispatch_request(sftp: &SftpSession, req: SftpRequest) {
    match req {
        SftpRequest::Home { reply } => {
            let _ = reply.send(
                sftp.canonicalize(".")
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
            );
        }
        SftpRequest::List { path, reply } => {
            let _ = reply.send(list_dir(sftp, &path).await);
        }
        SftpRequest::Download {
            id,
            remote,
            local,
            progress,
            reply,
            cancel,
        } => {
            let _ = reply.send(download_path(sftp, id, &remote, &local, &progress, &cancel).await);
        }
        SftpRequest::Upload {
            id,
            local,
            remote_dir,
            progress,
            reply,
            cancel,
        } => {
            let _ = reply.send(upload_path(sftp, id, &local, &remote_dir, &progress, &cancel).await);
        }
        SftpRequest::Mkdir { path, reply } => {
            let _ = reply.send(
                sftp.create_dir(path.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("mkdir {path}: {e}")),
            );
        }
        SftpRequest::Remove {
            path,
            is_dir,
            reply,
        } => {
            let _ = reply.send(remove_path(sftp, &path, is_dir).await);
        }
        SftpRequest::Rename { from, to, reply } => {
            let _ = reply.send(
                sftp.rename(from.clone(), to.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("rename {from} → {to}: {e}")),
            );
        }
        SftpRequest::Chmod { path, mode, reply } => {
            let _ = reply.send(chmod_path(sftp, &path, mode).await);
        }
        SftpRequest::HostProbe { reply } => {
            let _ = reply.send(Err(anyhow::anyhow!(
                "host probe must not run on SFTP lane"
            )));
        }
    }
}

async fn open_sftp(session: &client::Handle<ClientHandler>) -> Result<SftpSession> {
    let channel = session
        .channel_open_session()
        .await
        .context("open SFTP channel")?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .context("request sftp subsystem")?;
    SftpSession::new(channel.into_stream())
        .await
        .context("SFTP handshake")
}

fn reply_open_err(req: &SftpRequest, err: anyhow::Error) {
    let msg = format!("{err:#}");
    match req {
        SftpRequest::Home { reply } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::List { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::Download { reply, .. } | SftpRequest::Upload { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::Mkdir { reply, .. }
        | SftpRequest::Remove { reply, .. }
        | SftpRequest::Rename { reply, .. }
        | SftpRequest::Chmod { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::HostProbe { reply } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
    }
}

async fn remove_path(sftp: &SftpSession, path: &str, is_dir: bool) -> Result<()> {
    if is_dir {
        let entries = list_dir(sftp, path).await?;
        for entry in entries {
            Box::pin(remove_path(sftp, &entry.path, entry.is_dir)).await?;
        }
        sftp.remove_dir(path.to_string())
            .await
            .with_context(|| format!("rmdir {path}"))?;
    } else {
        sftp.remove_file(path.to_string())
            .await
            .with_context(|| format!("remove {path}"))?;
    }
    Ok(())
}

async fn chmod_path(sftp: &SftpSession, path: &str, mode: u32) -> Result<()> {
    let mut attrs = sftp
        .metadata(path.to_string())
        .await
        .with_context(|| format!("stat {path}"))?;
    // Preserve file-type bits (0o170000); apply permission bits from `mode`.
    const IFMT: u32 = 0o170000;
    let type_bits = attrs.permissions.unwrap_or(0) & IFMT;
    let perm_bits = mode & 0o7777;
    attrs.permissions = Some(type_bits | perm_bits);
    sftp.set_metadata(path.to_string(), attrs)
        .await
        .with_context(|| format!("chmod {path}"))?;
    Ok(())
}

async fn list_dir(sftp: &SftpSession, path: &str) -> Result<Vec<RemoteEntry>> {
    let mut out = Vec::new();
    for entry in sftp
        .read_dir(path.to_string())
        .await
        .with_context(|| format!("read_dir {path}"))?
    {
        let name = entry.file_name();
        let is_dir = entry.file_type().is_dir();
        let meta = entry.metadata();
        let size = meta.size.unwrap_or(0);
        let mtime = meta.mtime.map(|t| t as u64);
        out.push(RemoteEntry {
            name,
            path: entry.path(),
            is_dir,
            size,
            mtime,
        });
    }
    Ok(out)
}

async fn count_remote_files(
    sftp: &SftpSession,
    remote: &str,
    id: uuid::Uuid,
    progress: &Sender<TransferProgress>,
    found: &mut u32,
    bytes: &mut u64,
    cancel: &TransferCancel,
) -> Result<u32> {
    ensure_not_cancelled(cancel)?;
    let meta = sftp
        .metadata(remote.to_string())
        .await
        .with_context(|| format!("stat {remote}"))?;
    if meta.file_type().is_dir() {
        let mut n = 0u32;
        for entry in sftp
            .read_dir(remote.to_string())
            .await
            .with_context(|| format!("read_dir {remote}"))?
        {
            ensure_not_cancelled(cancel)?;
            n += Box::pin(count_remote_files(
                sftp,
                &entry.path(),
                id,
                progress,
                found,
                bytes,
                cancel,
            ))
            .await?;
        }
        Ok(n)
    } else {
        *found += 1;
        *bytes += meta.size.unwrap_or(0);
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(0),
            Some(*found),
            Some(0),
            Some(*bytes),
        ));
        Ok(1)
    }
}

async fn count_local_files(
    local: &Path,
    id: uuid::Uuid,
    progress: &Sender<TransferProgress>,
    found: &mut u32,
    bytes: &mut u64,
    cancel: &TransferCancel,
) -> Result<u32> {
    ensure_not_cancelled(cancel)?;
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let mut n = 0u32;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            ensure_not_cancelled(cancel)?;
            n += Box::pin(count_local_files(
                &entry.path(),
                id,
                progress,
                found,
                bytes,
                cancel,
            ))
            .await?;
        }
        Ok(n)
    } else if meta.is_file() {
        *found += 1;
        *bytes += meta.len();
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(0),
            Some(*found),
            Some(0),
            Some(*bytes),
        ));
        Ok(1)
    } else {
        Ok(0)
    }
}

async fn download_path(
    sftp: &SftpSession,
    id: uuid::Uuid,
    remote: &str,
    local: &Path,
    progress: &Sender<TransferProgress>,
    cancel: &TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(cancel)?;
    let meta = sftp
        .metadata(remote.to_string())
        .await
        .with_context(|| format!("stat {remote}"))?;
    if meta.file_type().is_dir() {
        let mut found = 0u32;
        let mut bytes_total = 0u64;
        let files_total = count_remote_files(
            sftp,
            remote,
            id,
            progress,
            &mut found,
            &mut bytes_total,
            cancel,
        )
        .await?;
        ensure_not_cancelled(cancel)?;
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(0),
            Some(files_total),
            Some(0),
            Some(bytes_total),
        ));
        let mut files_done = 0u32;
        let mut bytes_done = 0u64;
        download_tree(
            sftp,
            id,
            remote,
            local,
            progress,
            &mut files_done,
            files_total,
            &mut bytes_done,
            bytes_total,
            cancel,
        )
        .await?;
        Ok(TransferOutcome {
            files: files_done,
            bytes: bytes_done,
        })
    } else {
        let bytes =
            download_file(sftp, id, remote, local, meta.size, progress, None, None, cancel)
                .await?;
        Ok(TransferOutcome { files: 1, bytes })
    }
}

async fn download_tree(
    sftp: &SftpSession,
    id: uuid::Uuid,
    remote: &str,
    local: &Path,
    progress: &Sender<TransferProgress>,
    files_done: &mut u32,
    files_total: u32,
    bytes_done: &mut u64,
    bytes_total: u64,
    cancel: &TransferCancel,
) -> Result<()> {
    ensure_not_cancelled(cancel)?;
    let meta = sftp
        .metadata(remote.to_string())
        .await
        .with_context(|| format!("stat {remote}"))?;
    if meta.file_type().is_dir() {
        tokio::fs::create_dir_all(local)
            .await
            .with_context(|| format!("mkdir {}", local.display()))?;
        for entry in sftp
            .read_dir(remote.to_string())
            .await
            .with_context(|| format!("read_dir {remote}"))?
        {
            ensure_not_cancelled(cancel)?;
            let name = entry.file_name();
            let child_remote = entry.path();
            let child_local = local.join(&name);
            Box::pin(download_tree(
                sftp,
                id,
                &child_remote,
                &child_local,
                progress,
                files_done,
                files_total,
                bytes_done,
                bytes_total,
                cancel,
            ))
            .await?;
        }
        Ok(())
    } else {
        let n = download_file(
            sftp,
            id,
            remote,
            local,
            meta.size,
            progress,
            Some(*bytes_done),
            Some(bytes_total),
            cancel,
        )
        .await?;
        *files_done += 1;
        *bytes_done += n;
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(*files_done),
            Some(files_total),
            Some(*bytes_done),
            Some(bytes_total),
        ));
        Ok(())
    }
}

async fn download_file(
    sftp: &SftpSession,
    id: uuid::Uuid,
    remote: &str,
    local: &Path,
    total: Option<u64>,
    progress: &Sender<TransferProgress>,
    overall_base: Option<u64>,
    overall_total: Option<u64>,
    cancel: &TransferCancel,
) -> Result<u64> {
    ensure_not_cancelled(cancel)?;
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut remote_file = sftp
        .open(remote.to_string())
        .await
        .with_context(|| format!("open {remote}"))?;
    let mut local_file = tokio::fs::File::create(local)
        .await
        .with_context(|| format!("create {}", local.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let base = overall_base.unwrap_or(0);
    let _ = progress.try_send(progress_msg(
        id,
        done,
        total,
        None,
        None,
        Some(base),
        overall_total.or(total),
    ));
    loop {
        ensure_not_cancelled(cancel)?;
        let n = remote_file
            .read(&mut buf)
            .await
            .with_context(|| format!("read {remote}"))?;
        if n == 0 {
            break;
        }
        local_file
            .write_all(&buf[..n])
            .await
            .with_context(|| format!("write {}", local.display()))?;
        done += n as u64;
        let _ = progress.try_send(progress_msg(
            id,
            done,
            total,
            None,
            None,
            Some(base + done),
            overall_total.or(total),
        ));
    }
    local_file.flush().await.ok();
    Ok(done)
}

async fn upload_path(
    sftp: &SftpSession,
    id: uuid::Uuid,
    local: &Path,
    remote_dir: &str,
    progress: &Sender<TransferProgress>,
    cancel: &TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(cancel)?;
    let name = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid local path"))?;
    let remote = join_remote(remote_dir, name);
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let mut found = 0u32;
        let mut bytes_total = 0u64;
        let files_total =
            count_local_files(local, id, progress, &mut found, &mut bytes_total, cancel).await?;
        ensure_not_cancelled(cancel)?;
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(0),
            Some(files_total),
            Some(0),
            Some(bytes_total),
        ));
        let mut files_done = 0u32;
        let mut bytes_done = 0u64;
        upload_tree(
            sftp,
            id,
            local,
            remote_dir,
            progress,
            &mut files_done,
            files_total,
            &mut bytes_done,
            bytes_total,
            cancel,
        )
        .await?;
        Ok(TransferOutcome {
            files: files_done,
            bytes: bytes_done,
        })
    } else if meta.is_file() {
        let bytes = upload_file(
            sftp,
            id,
            local,
            &remote,
            meta.len(),
            progress,
            None,
            Some(meta.len()),
            cancel,
        )
        .await?;
        Ok(TransferOutcome { files: 1, bytes })
    } else {
        bail!("unsupported local entry {}", local.display());
    }
}

async fn upload_tree(
    sftp: &SftpSession,
    id: uuid::Uuid,
    local: &Path,
    remote_dir: &str,
    progress: &Sender<TransferProgress>,
    files_done: &mut u32,
    files_total: u32,
    bytes_done: &mut u64,
    bytes_total: u64,
    cancel: &TransferCancel,
) -> Result<()> {
    ensure_not_cancelled(cancel)?;
    let name = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid local path"))?;
    let remote = join_remote(remote_dir, name);
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let _ = sftp.create_dir(remote.clone()).await;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            ensure_not_cancelled(cancel)?;
            Box::pin(upload_tree(
                sftp,
                id,
                &entry.path(),
                &remote,
                progress,
                files_done,
                files_total,
                bytes_done,
                bytes_total,
                cancel,
            ))
            .await?;
        }
        Ok(())
    } else if meta.is_file() {
        let n = upload_file(
            sftp,
            id,
            local,
            &remote,
            meta.len(),
            progress,
            Some(*bytes_done),
            Some(bytes_total),
            cancel,
        )
        .await?;
        *files_done += 1;
        *bytes_done += n;
        let _ = progress.try_send(progress_msg(
            id,
            0,
            None,
            Some(*files_done),
            Some(files_total),
            Some(*bytes_done),
            Some(bytes_total),
        ));
        Ok(())
    } else {
        bail!("unsupported local entry {}", local.display());
    }
}

async fn upload_file(
    sftp: &SftpSession,
    id: uuid::Uuid,
    local: &Path,
    remote: &str,
    total: u64,
    progress: &Sender<TransferProgress>,
    overall_base: Option<u64>,
    overall_total: Option<u64>,
    cancel: &TransferCancel,
) -> Result<u64> {
    ensure_not_cancelled(cancel)?;
    let mut local_file = tokio::fs::File::open(local)
        .await
        .with_context(|| format!("open {}", local.display()))?;
    let mut remote_file = sftp
        .create(remote.to_string())
        .await
        .with_context(|| format!("create {remote}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let base = overall_base.unwrap_or(0);
    let _ = progress.try_send(progress_msg(
        id,
        done,
        Some(total),
        None,
        None,
        Some(base),
        overall_total.or(Some(total)),
    ));
    loop {
        ensure_not_cancelled(cancel)?;
        let n = local_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..n])
            .await
            .with_context(|| format!("write {remote}"))?;
        done += n as u64;
        let _ = progress.try_send(progress_msg(
            id,
            done,
            Some(total),
            None,
            None,
            Some(base + done),
            overall_total.or(Some(total)),
        ));
    }
    remote_file.flush().await.ok();
    Ok(done)
}

pub fn join_remote(base: &str, name: &str) -> String {
    if base.is_empty() || base == "/" {
        if name.starts_with('/') {
            name.to_string()
        } else {
            format!("/{name}")
        }
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub fn parent_remote(path: &str) -> Option<String> {
    let p = path.trim_end_matches('/');
    if p.is_empty() || p == "/" {
        return None;
    }
    match p.rfind('/') {
        Some(0) => Some("/".into()),
        Some(i) => Some(p[..i].to_string()),
        None => Some("/".into()),
    }
}
