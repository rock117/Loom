//! SFTP requests served on the same russh session as the interactive shell.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use flume::{Receiver, Sender};
use russh::client;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ssh::ClientHandler;
use super::transfer_filter::{FilterMatcher, TransferFilter};

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

/// Include/exclude globs plus optional remote/local compression for a transfer.
#[derive(Debug, Clone, Default)]
pub struct TransferOptions {
    pub filter: TransferFilter,
    pub compress: bool,
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

/// Result of resolving a typed/pasted Files path bar entry.
#[derive(Debug, Clone)]
pub enum ResolvedPath {
    Dir(String),
    File(RemoteEntry),
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
    /// Optional UI label update (e.g. archive filename after remote compress).
    pub label: Option<String>,
    /// Optional local path update (e.g. final archive path).
    pub local_path: Option<PathBuf>,
    /// Lightweight status phase (`Copying…`) — not a filename; prefer rare updates.
    pub phase: Option<String>,
}

/// Result of a finished upload/download.
#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub files: u32,
    pub bytes: u64,
    /// When set (compressed download), the archive path that was written.
    pub saved_as: Option<PathBuf>,
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
        label: None,
        local_path: None,
        phase: None,
    }
}

fn progress_phase(id: uuid::Uuid, phase: impl Into<String>) -> TransferProgress {
    TransferProgress {
        id,
        done: 0,
        total: None,
        files_done: None,
        files_total: None,
        overall_done: None,
        overall_total: None,
        label: None,
        local_path: None,
        phase: Some(phase.into()),
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
    /// Docker-over-SSH browse / mutate (exec on Handle; no SFTP).
    DockerHome {
        container: String,
        reply: Sender<Result<String>>,
    },
    DockerList {
        container: String,
        path: String,
        reply: Sender<Result<Vec<RemoteEntry>>>,
    },
    DockerMkdir {
        container: String,
        path: String,
        reply: Sender<Result<()>>,
    },
    DockerRemove {
        container: String,
        path: String,
        is_dir: bool,
        reply: Sender<Result<()>>,
    },
    DockerRename {
        container: String,
        from: String,
        to: String,
        reply: Sender<Result<()>>,
    },
    DockerChmod {
        container: String,
        path: String,
        mode: u32,
        reply: Sender<Result<()>>,
    },
    DockerProbe {
        container: String,
        reply: Sender<Result<crate::session::host_info::HostSnapshot>>,
    },
    DockerResolvePath {
        container: String,
        path: String,
        reply: Sender<Result<ResolvedPath, String>>,
    },
    /// Stat a remote path for the Files address bar (dir → navigate, file → download).
    ResolvePath {
        path: String,
        reply: Sender<Result<ResolvedPath, String>>,
    },
}

/// One file/folder upload or download.
///
/// Transfers never ride the interactive shell connection: each job opens its own SSH
/// connection, runs the SFTP copy, then disconnects. See [`spawn_transfer`].
pub enum TransferJob {
    Download {
        id: uuid::Uuid,
        remote: String,
        local: PathBuf,
        options: TransferOptions,
    },
    Upload {
        id: uuid::Uuid,
        local: PathBuf,
        remote_dir: String,
        options: TransferOptions,
    },
    /// Stage with remote `docker cp`, then SFTP pull.
    DockerDownload {
        id: uuid::Uuid,
        container: String,
        remote: String,
        local: PathBuf,
    },
    /// Stage with SFTP push, then remote `docker cp`.
    DockerUpload {
        id: uuid::Uuid,
        container: String,
        local: PathBuf,
        remote_dir: String,
    },
}

/// Everything needed to dial a fresh SSH connection for a transfer.
///
/// Cloned from the pane's live session so a transfer still works when the shell
/// connection dies (and so it can run while the shell keeps streaming).
#[derive(Clone)]
pub struct TransferTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: super::ssh::SshAuthMaterial,
    pub jump: Option<super::ssh::SshJumpParams>,
}

impl TransferTarget {
    /// Concurrency key: one queue per SSH endpoint (account-independent — the server's
    /// `MaxSessions` is per `sshd`, not per user).
    fn host_key(&self) -> String {
        format!("{}:{}", self.host.trim().to_ascii_lowercase(), self.port)
    }
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

/// Max concurrent **browse** SFTP channels across the whole app. Transfers use their own
/// connections and are limited per host by `MAX_CONCURRENT_PER_HOST` instead.
const GLOBAL_SFTP_CHANNEL_BUDGET: usize = 12;
/// Close an idle browse session after this long with no requests.
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

fn is_host_probe(req: &SftpRequest) -> bool {
    matches!(req, SftpRequest::HostProbe { .. })
}

fn is_docker_exec_request(req: &SftpRequest) -> bool {
    matches!(
        req,
        SftpRequest::DockerHome { .. }
            | SftpRequest::DockerList { .. }
            | SftpRequest::DockerMkdir { .. }
            | SftpRequest::DockerRemove { .. }
            | SftpRequest::DockerRename { .. }
            | SftpRequest::DockerChmod { .. }
            | SftpRequest::DockerProbe { .. }
            | SftpRequest::DockerResolvePath { .. }
    )
}

/// Per-host transfer scheduler.
///
/// Browsing still shares the shell connection (one SFTP channel, lazy + idle-reclaimed);
/// transfers each dial their own SSH connection, capped per host so a burst of downloads
/// cannot exhaust the server's `MaxSessions`/`MaxStartups`. Excess jobs wait their turn.
const MAX_CONCURRENT_PER_HOST: usize = 10;
/// Timeout for the per-transfer SSH handshake (connect + auth, jump hops included).
const TRANSFER_CONNECT_TIMEOUT_SECS: u64 = 45;

/// Shared runtime for transfers. Jobs are queued as tasks (no thread per waiting job).
fn transfer_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RT: std::sync::OnceLock<Option<tokio::runtime::Runtime>> =
        std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            // One worker per allowed concurrent transfer: SFTP framing + crypto is CPU work
            // that must not starve the other in-flight jobs.
            .worker_threads(MAX_CONCURRENT_PER_HOST)
            .thread_name("loom-transfer")
            .build()
            .ok()
    })
    .as_ref()
    .ok_or_else(|| anyhow::anyhow!("transfer runtime unavailable"))
}

/// One semaphore per SSH endpoint. Keyed by host:port (not per user) because the server's
/// `MaxSessions` limit applies per `sshd` process.
///
/// Entries are intentionally never evicted: the map is bounded by the number of distinct
/// hosts the user connects to, and removing a semaphore while jobs wait on it would let a
/// later job bypass the cap.
fn host_semaphore(key: &str) -> Arc<tokio::sync::Semaphore> {
    static HOST_SEMAPHORES: parking_lot::Mutex<
        Option<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>,
    > = parking_lot::Mutex::new(None);
    let mut guard = HOST_SEMAPHORES.lock();
    guard
        .get_or_insert_with(std::collections::HashMap::new)
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PER_HOST)))
        .clone()
}

/// Wait for a concurrency slot on this host. `cancel` (or a closed progress channel, i.e.
/// the UI abandoned the job) aborts a queued job.
async fn acquire_host_slot(
    key: &str,
    cancel: &TransferCancel,
    progress: &Sender<TransferProgress>,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    let sem = host_semaphore(key);
    loop {
        ensure_not_cancelled(cancel)?;
        if progress.is_disconnected() {
            bail!("cancelled");
        }
        match Arc::clone(&sem).try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                // Queued: wait for a slot, but wake periodically to honor cancel.
                tokio::select! {
                    res = Arc::clone(&sem).acquire_owned() => {
                        return res.map_err(|_| anyhow::anyhow!("transfer queue closed"));
                    }
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
            }
            Err(tokio::sync::TryAcquireError::Closed) => {
                bail!("transfer queue closed");
            }
        }
    }
}

/// Run one transfer on its own SSH connection, then close it.
///
/// Spawns onto the shared transfer runtime and reports back through `reply`, so a queued
/// job costs no OS thread. Never call this from the UI thread's executor.
pub fn spawn_transfer(
    target: TransferTarget,
    job: TransferJob,
    progress: Sender<TransferProgress>,
    cancel: TransferCancel,
    reply: Sender<Result<TransferOutcome>>,
) {
    let key = target.host_key();
    let runtime = match transfer_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            let _ = reply.send(Err(err));
            return;
        }
    };
    let cancel_task = Arc::clone(&cancel);
    runtime.spawn(async move {
        let outcome = run_transfer(&key, target, job, progress, cancel_task).await;
        let _ = reply.send(outcome);
    });
}

async fn run_transfer(
    key: &str,
    target: TransferTarget,
    job: TransferJob,
    progress: Sender<TransferProgress>,
    cancel: TransferCancel,
) -> Result<TransferOutcome> {
    // Queue first: no socket / handshake until this host has a free slot.
    let _slot = acquire_host_slot(key, &cancel, &progress).await?;
    let id = job.id();

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    // A dial/auth failure happens before we own a session, so there is nothing to close.
    // Bounded: an unreachable host must not pin the host slot forever.
    let connected = tokio::time::timeout(
        Duration::from_secs(TRANSFER_CONNECT_TIMEOUT_SECS),
        super::ssh::connect_handles(
            config,
            &target.host,
            target.port,
            &target.user,
            target.auth,
            target.jump,
        ),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "connect to {}:{} timed out after {TRANSFER_CONNECT_TIMEOUT_SECS}s",
            target.host,
            target.port
        )
    })?;
    let (session, _jump) = connected?;
    let session = Arc::new(session);

    let transfer = async {
        let sftp = open_sftp(&session).await?;
        match job {
            TransferJob::Download {
                remote,
                local,
                options,
                ..
            } => {
                download_path(
                    &session,
                    &sftp,
                    id,
                    &remote,
                    &local,
                    &options,
                    &progress,
                    &cancel,
                )
                .await
            }
            TransferJob::Upload {
                local,
                remote_dir,
                options,
                ..
            } => upload_path(&sftp, id, &local, &remote_dir, &options, &progress, &cancel).await,
            TransferJob::DockerDownload {
                container,
                remote,
                local,
                ..
            } => {
                docker_download_staged(
                    &session,
                    &sftp,
                    id,
                    &container,
                    &remote,
                    &local,
                    progress,
                    Arc::clone(&cancel),
                )
                .await
            }
            TransferJob::DockerUpload {
                container,
                local,
                remote_dir,
                ..
            } => {
                docker_upload_staged(
                    &session,
                    &sftp,
                    id,
                    &container,
                    &local,
                    &remote_dir,
                    progress,
                    Arc::clone(&cancel),
                )
                .await
            }
        }
    };

    // Cancel must win even mid-`read`: a half-open socket would otherwise hold this host
    // slot (and the connection) until the OS gives up. The flag alone is only checked
    // between chunks.
    let result = tokio::select! {
        res = transfer => res,
        _ = wait_cancelled(&cancel) => Err(anyhow::anyhow!("cancelled")),
    };

    // Close this job's connection (and jump tunnel) regardless of outcome.
    // Bounded wait: a half-open socket must not pin the host slot forever.
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        session.disconnect(russh::Disconnect::ByApplication, "transfer done", ""),
    )
    .await;
    result
}

/// Resolve once `cancel` is set. Polls (10 Hz) because the flag is a plain atomic shared
/// with the UI thread — no async primitive to await on.
async fn wait_cancelled(cancel: &TransferCancel) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

impl TransferJob {
    fn id(&self) -> uuid::Uuid {
        match self {
            Self::Download { id, .. }
            | Self::Upload { id, .. }
            | Self::DockerDownload { id, .. }
            | Self::DockerUpload { id, .. } => *id,
        }
    }
}

/// Browse-only SFTP worker on the shell connection.
///
/// One lazily opened subsystem channel, reopened after idling, closed with the session.
pub async fn run_sftp_worker(
    session: Arc<client::Handle<ClientHandler>>,
    req_rx: Receiver<SftpRequest>,
) {
    let mut sftp: Option<(SftpSession, ChannelBudgetGuard)> = None;
    let idle = std::time::Duration::from_secs(LANE_IDLE_SECS);

    loop {
        let req = if sftp.is_some() {
            tokio::select! {
                biased;
                msg = req_rx.recv_async() => match msg {
                    Ok(req) => req,
                    Err(_) => break,
                },
                _ = tokio::time::sleep(idle) => {
                    // Idle reclaim: drop the channel and return global budget.
                    sftp = None;
                    continue;
                }
            }
        } else {
            match req_rx.recv_async().await {
                Ok(req) => req,
                Err(_) => break,
            }
        };

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
        if is_docker_exec_request(&req) {
            let session = Arc::clone(&session);
            tokio::spawn(async move {
                dispatch_docker_exec(&session, req).await;
            });
            continue;
        }
        if let Err(err) = ensure_session(&session, &mut sftp).await {
            reply_open_err(&req, err);
            continue;
        }
        let Some((ref s, _)) = sftp else { continue };
        dispatch_request(s, req).await;
    }
}

async fn ensure_session(
    session: &client::Handle<ClientHandler>,
    sftp: &mut Option<(SftpSession, ChannelBudgetGuard)>,
) -> Result<()> {
    if sftp.is_some() {
        return Ok(());
    }
    if !try_acquire_channel_budget() {
        bail!(
            "SFTP unavailable: too many open SFTP channels (max {GLOBAL_SFTP_CHANNEL_BUDGET})"
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
        SftpRequest::DockerHome { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(
                "docker browse must not run on SFTP lane"
            )));
        }
        SftpRequest::DockerList { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(
                "docker browse must not run on SFTP lane"
            )));
        }
        SftpRequest::DockerMkdir { reply, .. }
        | SftpRequest::DockerRemove { reply, .. }
        | SftpRequest::DockerRename { reply, .. }
        | SftpRequest::DockerChmod { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(
                "docker browse must not run on SFTP lane"
            )));
        }
        SftpRequest::DockerProbe { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(
                "docker probe must not run on SFTP lane"
            )));
        }
        SftpRequest::DockerResolvePath { reply, .. } => {
            let _ = reply.send(Err("docker resolve must not run on SFTP lane".into()));
        }
        SftpRequest::ResolvePath { path, reply } => {
            let _ = reply.send(resolve_path(sftp, &path).await);
        }
    }
}

async fn dispatch_docker_exec(session: &client::Handle<ClientHandler>, req: SftpRequest) {
    match req {
        SftpRequest::DockerHome { container, reply } => {
            let _ = reply.send(crate::session::docker_ssh::home_dir(session, &container).await);
        }
        SftpRequest::DockerList {
            container,
            path,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::list_dir(session, &container, &path).await,
            );
        }
        SftpRequest::DockerMkdir {
            container,
            path,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::create_dir(session, &container, &path).await,
            );
        }
        SftpRequest::DockerRemove {
            container,
            path,
            is_dir,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::remove_path(session, &container, &path, is_dir)
                    .await,
            );
        }
        SftpRequest::DockerRename {
            container,
            from,
            to,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::rename(session, &container, &from, &to).await,
            );
        }
        SftpRequest::DockerChmod {
            container,
            path,
            mode,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::chmod(session, &container, &path, mode).await,
            );
        }
        SftpRequest::DockerProbe { container, reply } => {
            let _ = reply.send(
                crate::session::docker_ssh::container_snapshot(session, &container).await,
            );
        }
        SftpRequest::DockerResolvePath {
            container,
            path,
            reply,
        } => {
            let _ = reply.send(
                crate::session::docker_ssh::resolve_existing_path(session, &container, &path)
                    .await,
            );
        }
        _ => {}
    }
}

async fn docker_download_staged(
    session: &client::Handle<ClientHandler>,
    sftp: &SftpSession,
    id: uuid::Uuid,
    container: &str,
    remote: &str,
    local: &Path,
    progress: Sender<TransferProgress>,
    cancel: TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(&cancel)?;
    let name = path_basename(remote);
    let tmp = crate::session::docker_ssh::mktemp_dir(session).await?;
    let staged = format!("{tmp}/{name}");
    let _ = progress.try_send(progress_phase(id, "Copying from container…"));
    if let Err(err) =
        crate::session::docker_ssh::docker_cp_out(session, container, remote, &staged).await
    {
        let _ = crate::session::docker_ssh::rm_rf(session, &tmp).await;
        return Err(err);
    }
    let _ = progress.try_send(progress_phase(id, "Downloading…"));
    let result = download_path(
        session,
        sftp,
        id,
        &staged,
        local,
        &TransferOptions::default(),
        &progress,
        &cancel,
    )
    .await;
    let _ = crate::session::docker_ssh::rm_rf(session, &tmp).await;
    result
}

async fn docker_upload_staged(
    session: &client::Handle<ClientHandler>,
    sftp: &SftpSession,
    id: uuid::Uuid,
    container: &str,
    local: &Path,
    remote_dir: &str,
    progress: Sender<TransferProgress>,
    cancel: TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(&cancel)?;
    let name = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid local path"))?;
    let tmp = crate::session::docker_ssh::mktemp_dir(session).await?;
    let staged_dir = tmp.clone();
    let _ = progress.try_send(progress_phase(id, "Uploading…"));
    let outcome = upload_path(
        sftp,
        id,
        local,
        &staged_dir,
        &TransferOptions::default(),
        &progress,
        &cancel,
    )
    .await;
    let staged = format!("{tmp}/{name}");
    match outcome {
        Ok(o) => {
            let _ = progress.try_send(progress_phase(id, "Copying into container…"));
            if let Err(err) =
                crate::session::docker_ssh::docker_cp_in(session, container, &staged, remote_dir)
                    .await
            {
                let _ = crate::session::docker_ssh::rm_rf(session, &tmp).await;
                return Err(err);
            }
            let _ = crate::session::docker_ssh::rm_rf(session, &tmp).await;
            Ok(o)
        }
        Err(err) => {
            let _ = crate::session::docker_ssh::rm_rf(session, &tmp).await;
            Err(err)
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
        SftpRequest::Home { reply }
        | SftpRequest::DockerHome { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::List { reply, .. } | SftpRequest::DockerList { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::Mkdir { reply, .. }
        | SftpRequest::Remove { reply, .. }
        | SftpRequest::Rename { reply, .. }
        | SftpRequest::Chmod { reply, .. }
        | SftpRequest::DockerMkdir { reply, .. }
        | SftpRequest::DockerRemove { reply, .. }
        | SftpRequest::DockerRename { reply, .. }
        | SftpRequest::DockerChmod { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::HostProbe { reply } | SftpRequest::DockerProbe { reply, .. } => {
            let _ = reply.send(Err(anyhow::anyhow!(msg)));
        }
        SftpRequest::DockerResolvePath { reply, .. }
        | SftpRequest::ResolvePath { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
    }
}

async fn resolve_path(sftp: &SftpSession, path: &str) -> Result<ResolvedPath, String> {
    let path = path.trim().trim_matches('"').trim();
    if path.is_empty() {
        return Err("Path is empty".into());
    }
    let meta = sftp
        .metadata(path.to_string())
        .await
        .map_err(|_| "Path does not exist".to_string())?;
    if meta.is_dir() {
        return Ok(ResolvedPath::Dir(path.to_string()));
    }
    Ok(ResolvedPath::File(RemoteEntry {
        name: path_basename(path),
        path: path.to_string(),
        is_dir: false,
        size: meta.size.unwrap_or(0),
        mtime: meta.mtime.map(|t| t as u64),
    }))
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

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn path_basename(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path).to_string()
}

async fn count_remote_files(
    sftp: &SftpSession,
    remote: &str,
    rel: &str,
    filter: &FilterMatcher,
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
    let is_dir = meta.file_type().is_dir();
    if !filter.allows(rel, is_dir) {
        return Ok(0);
    }
    if is_dir {
        let mut n = 0u32;
        for entry in sftp
            .read_dir(remote.to_string())
            .await
            .with_context(|| format!("read_dir {remote}"))?
        {
            ensure_not_cancelled(cancel)?;
            let name = entry.file_name();
            let child_rel = join_rel(rel, &name);
            n += Box::pin(count_remote_files(
                sftp,
                &entry.path(),
                &child_rel,
                filter,
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
    rel: &str,
    filter: &FilterMatcher,
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
    let is_dir = meta.is_dir();
    if !filter.allows(rel, is_dir) {
        return Ok(0);
    }
    if is_dir {
        let mut n = 0u32;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            ensure_not_cancelled(cancel)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let child_rel = join_rel(rel, &name);
            n += Box::pin(count_local_files(
                &entry.path(),
                &child_rel,
                filter,
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
    session: &client::Handle<ClientHandler>,
    sftp: &SftpSession,
    id: uuid::Uuid,
    remote: &str,
    local: &Path,
    options: &TransferOptions,
    progress: &Sender<TransferProgress>,
    cancel: &TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(cancel)?;
    let _ = (session, options.compress); // compress downloads removed; originals only
    // Never overwrite: Explorer-style `name (1).ext` when the dest already exists.
    let requested = local.to_path_buf();
    let dest = crate::session::local_fs::unique_path(&requested);
    let saved_as = (dest != requested).then_some(dest.clone());
    let local = dest.as_path();

    let filter = options.filter.matcher()?;
    let root_rel = path_basename(remote);
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
            &root_rel,
            &filter,
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
            &root_rel,
            &filter,
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
            saved_as,
        })
    } else {
        if !filter.allows(&root_rel, false) {
            bail!("remote file excluded by filter");
        }
        let bytes =
            download_file(sftp, id, remote, local, meta.size, progress, None, None, cancel)
                .await?;
        Ok(TransferOutcome {
            files: 1,
            bytes,
            saved_as,
        })
    }
}

async fn download_tree(
    sftp: &SftpSession,
    id: uuid::Uuid,
    remote: &str,
    local: &Path,
    rel: &str,
    filter: &FilterMatcher,
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
    let is_dir = meta.file_type().is_dir();
    if !filter.allows(rel, is_dir) {
        return Ok(());
    }
    if is_dir {
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
            let child_rel = join_rel(rel, &name);
            Box::pin(download_tree(
                sftp,
                id,
                &child_remote,
                &child_local,
                &child_rel,
                filter,
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
        .with_context(|| format!("open remote file {remote}"))?;
    let mut local_file = tokio::fs::File::create(local)
        .await
        .with_context(|| format!("create local file {}", local.display()))?;
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
    options: &TransferOptions,
    progress: &Sender<TransferProgress>,
    cancel: &TransferCancel,
) -> Result<TransferOutcome> {
    ensure_not_cancelled(cancel)?;
    let name = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid local path"))?;
    let _ = options.compress; // UI no longer offers compress; always transfer as-is

    let filter = options.filter.matcher()?;
    let root_rel = name.to_string();
    // Top-level only: avoid clobbering an existing remote file/folder of the same name.
    let remote_name = unique_remote_basename(sftp, remote_dir, name).await?;
    let remote = join_remote(remote_dir, &remote_name);
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let mut found = 0u32;
        let mut bytes_total = 0u64;
        let files_total = count_local_files(
            local,
            &root_rel,
            &filter,
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
        // Seed the unique root, then recurse with children under that name.
        let _ = sftp.create_dir(remote.clone()).await;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            ensure_not_cancelled(cancel)?;
            let child_name = entry.file_name();
            let child_name = child_name.to_string_lossy();
            let child_rel = join_rel(&root_rel, &child_name);
            Box::pin(upload_tree(
                sftp,
                id,
                &entry.path(),
                &remote,
                &child_rel,
                &filter,
                progress,
                &mut files_done,
                files_total,
                &mut bytes_done,
                bytes_total,
                cancel,
            ))
            .await?;
        }
        Ok(TransferOutcome {
            files: files_done,
            bytes: bytes_done,
            saved_as: None,
        })
    } else if meta.is_file() {
        if !filter.allows(&root_rel, false) {
            bail!("local file excluded by filter");
        }
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
        Ok(TransferOutcome {
            files: 1,
            bytes,
            saved_as: None,
        })
    } else {
        bail!("unsupported local entry {}", local.display());
    }
}

async fn upload_tree(
    sftp: &SftpSession,
    id: uuid::Uuid,
    local: &Path,
    remote_dir: &str,
    rel: &str,
    filter: &FilterMatcher,
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
    if !filter.allows(rel, meta.is_dir()) {
        return Ok(());
    }
    if meta.is_dir() {
        let _ = sftp.create_dir(remote.clone()).await;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            ensure_not_cancelled(cancel)?;
            let child_name = entry.file_name();
            let child_name = child_name.to_string_lossy();
            let child_rel = join_rel(rel, &child_name);
            Box::pin(upload_tree(
                sftp,
                id,
                &entry.path(),
                &remote,
                &child_rel,
                filter,
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
        .with_context(|| format!("create remote file {remote}"))?;
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
            .with_context(|| format!("write remote file {remote}"))?;
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

/// Pick `name`, `name (1)`, `name (2)`, … under `remote_dir` that does not exist yet.
async fn unique_remote_basename(
    sftp: &SftpSession,
    remote_dir: &str,
    name: &str,
) -> Result<String> {
    let first = join_remote(remote_dir, name);
    if sftp.metadata(first).await.is_err() {
        return Ok(name.to_string());
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() && !e.contains('/') => {
            (s.to_string(), Some(e.to_string()))
        }
        _ => (name.to_string(), None),
    };
    for n in 1u32..10_000 {
        let candidate = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let path = join_remote(remote_dir, &candidate);
        if sftp.metadata(path).await.is_err() {
            return Ok(candidate);
        }
    }
    bail!("could not find a free remote name for {name}");
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
