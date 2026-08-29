//! SFTP requests served on the same russh session as the interactive shell.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use flume::{Receiver, Sender};
use russh::client;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ssh::ClientHandler;

/// One remote directory entry for the Files browser.
#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub id: uuid::Uuid,
    pub done: u64,
    pub total: Option<u64>,
    /// Files completed so far (recursive folder transfers).
    pub files_done: Option<u32>,
    /// Total files in a recursive transfer (after pre-count).
    pub files_total: Option<u32>,
}

/// Result of a finished upload/download.
#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub files: u32,
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
    },
    Upload {
        id: uuid::Uuid,
        local: PathBuf,
        remote_dir: String,
        progress: Sender<TransferProgress>,
        reply: Sender<Result<TransferOutcome>>,
    },
}

/// Cloneable handle used by the UI to talk to the SSH thread's SFTP worker.
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

/// Background task: open SFTP subsystem once, serve requests until the channel closes.
pub async fn run_sftp_worker(
    session: Arc<client::Handle<ClientHandler>>,
    req_rx: Receiver<SftpRequest>,
) {
    let mut sftp: Option<SftpSession> = None;

    while let Ok(req) = req_rx.recv_async().await {
        if sftp.is_none() {
            match open_sftp(&session).await {
                Ok(s) => sftp = Some(s),
                Err(err) => {
                    reply_open_err(&req, err);
                    continue;
                }
            }
        }
        let Some(sftp) = sftp.as_ref() else {
            continue;
        };
        match req {
            SftpRequest::Home { reply } => {
                let _ = reply.send(sftp.canonicalize(".").await.map_err(|e| anyhow::anyhow!("{e}")));
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
            } => {
                let _ = reply.send(download_path(sftp, id, &remote, &local, &progress).await);
            }
            SftpRequest::Upload {
                id,
                local,
                remote_dir,
                progress,
                reply,
            } => {
                let _ = reply.send(upload_path(sftp, id, &local, &remote_dir, &progress).await);
            }
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
    }
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
        let size = entry.metadata().size.unwrap_or(0);
        out.push(RemoteEntry {
            name,
            path: entry.path(),
            is_dir,
            size,
        });
    }
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(out)
}

async fn count_remote_files(sftp: &SftpSession, remote: &str) -> Result<u32> {
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
            n += Box::pin(count_remote_files(sftp, &entry.path())).await?;
        }
        Ok(n)
    } else {
        Ok(1)
    }
}

async fn count_local_files(local: &Path) -> Result<u32> {
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let mut n = 0u32;
        let mut rd = tokio::fs::read_dir(local)
            .await
            .with_context(|| format!("read_dir {}", local.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            n += Box::pin(count_local_files(&entry.path())).await?;
        }
        Ok(n)
    } else if meta.is_file() {
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
) -> Result<TransferOutcome> {
    let meta = sftp
        .metadata(remote.to_string())
        .await
        .with_context(|| format!("stat {remote}"))?;
    if meta.file_type().is_dir() {
        let files_total = count_remote_files(sftp, remote).await?;
        let _ = progress.send(TransferProgress {
            id,
            done: 0,
            total: None,
            files_done: Some(0),
            files_total: Some(files_total),
        });
        let mut files_done = 0u32;
        download_tree(sftp, id, remote, local, progress, &mut files_done, files_total).await?;
        Ok(TransferOutcome { files: files_done })
    } else {
        download_file(sftp, id, remote, local, meta.size, progress).await?;
        Ok(TransferOutcome { files: 1 })
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
) -> Result<()> {
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
            ))
            .await?;
        }
        Ok(())
    } else {
        download_file(sftp, id, remote, local, meta.size, progress).await?;
        *files_done += 1;
        let _ = progress.send(TransferProgress {
            id,
            done: 0,
            total: None,
            files_done: Some(*files_done),
            files_total: Some(files_total),
        });
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
) -> Result<()> {
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
    let _ = progress.send(TransferProgress {
        id,
        done,
        total,
        files_done: None,
        files_total: None,
    });
    loop {
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
        let _ = progress.send(TransferProgress {
            id,
            done,
            total,
            files_done: None,
            files_total: None,
        });
    }
    local_file.flush().await.ok();
    Ok(())
}

async fn upload_path(
    sftp: &SftpSession,
    id: uuid::Uuid,
    local: &Path,
    remote_dir: &str,
    progress: &Sender<TransferProgress>,
) -> Result<TransferOutcome> {
    let name = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid local path"))?;
    let remote = join_remote(remote_dir, name);
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    if meta.is_dir() {
        let files_total = count_local_files(local).await?;
        let _ = progress.send(TransferProgress {
            id,
            done: 0,
            total: None,
            files_done: Some(0),
            files_total: Some(files_total),
        });
        let mut files_done = 0u32;
        upload_tree(sftp, id, local, remote_dir, progress, &mut files_done, files_total).await?;
        Ok(TransferOutcome { files: files_done })
    } else if meta.is_file() {
        upload_file(sftp, id, local, &remote, meta.len(), progress).await?;
        Ok(TransferOutcome { files: 1 })
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
) -> Result<()> {
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
            Box::pin(upload_tree(
                sftp,
                id,
                &entry.path(),
                &remote,
                progress,
                files_done,
                files_total,
            ))
            .await?;
        }
        Ok(())
    } else if meta.is_file() {
        upload_file(sftp, id, local, &remote, meta.len(), progress).await?;
        *files_done += 1;
        let _ = progress.send(TransferProgress {
            id,
            done: 0,
            total: None,
            files_done: Some(*files_done),
            files_total: Some(files_total),
        });
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
) -> Result<()> {
    let mut local_file = tokio::fs::File::open(local)
        .await
        .with_context(|| format!("open {}", local.display()))?;
    let mut remote_file = sftp
        .create(remote.to_string())
        .await
        .with_context(|| format!("create {remote}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let _ = progress.send(TransferProgress {
        id,
        done,
        total: Some(total),
        files_done: None,
        files_total: None,
    });
    loop {
        let n = local_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..n])
            .await
            .with_context(|| format!("write {remote}"))?;
        done += n as u64;
        let _ = progress.send(TransferProgress {
            id,
            done,
            total: Some(total),
            files_done: None,
            files_total: None,
        });
    }
    remote_file.flush().await.ok();
    Ok(())
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
