//! Local SSH port forwarding on a shared russh Handle (stage 1).
//! See docs/PORT_FORWARD.md.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use flume::{Receiver, Sender};
use parking_lot::Mutex;
use russh::ChannelOpenFailure;
use russh::client;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

use crate::model::{PortForwardKind, PortForwardRule};

use super::ssh::ClientHandler;

/// Runtime row shown in Info / status bar.
#[derive(Debug, Clone)]
pub struct ForwardRuntime {
    pub id: Uuid,
    pub name: String,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub temporary: bool,
    pub status: ForwardStatus,
    /// Set when a dial through SSH failed while still Listening.
    pub last_dial_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardStatus {
    Starting,
    Listening,
    Error(String),
    Stopped,
}

impl ForwardStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Starting => "Starting…",
            Self::Listening => "Listening",
            Self::Error(msg) => msg.as_str(),
            Self::Stopped => "Stopped",
        }
    }

    pub fn is_listening(&self) -> bool {
        matches!(self, Self::Listening)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

impl ForwardRuntime {
    /// OpenSSH `-L` flag for this runtime row (stage 1 = Local only).
    pub fn open_ssh_flag(&self) -> String {
        crate::model::open_ssh_forward_flag(
            PortForwardKind::Local,
            &self.bind_host,
            self.bind_port,
            &self.target_host,
            self.target_port,
        )
    }
}

#[derive(Debug, Clone)]
pub struct ForwardSnapshot {
    pub rows: Vec<ForwardRuntime>,
    /// Cached: server rejected TCP forwarding (AllowTcpForwarding).
    pub forwarding_denied: bool,
    pub generation: u64,
}

impl ForwardSnapshot {
    pub fn listening_count(&self) -> usize {
        self.rows.iter().filter(|r| r.status.is_listening()).count()
    }

    pub fn error_count(&self) -> usize {
        self.rows.iter().filter(|r| r.status.is_error()).count()
    }

    /// Compact status-bar label for forward failures (`None` = nothing to show).
    pub fn status_bar_error_label(&self) -> Option<String> {
        let errors: Vec<_> = self
            .rows
            .iter()
            .filter_map(|r| match &r.status {
                ForwardStatus::Error(msg) => Some((r.bind_port, msg.as_str())),
                _ => None,
            })
            .collect();

        if errors.is_empty() {
            return if self.forwarding_denied {
                Some("⇄ Forward denied".into())
            } else {
                None
            };
        }

        if errors.len() == 1 {
            let (port, msg) = errors[0];
            if msg.ends_with(" already in use") {
                return Some(format!("⇄ {port} in use"));
            }
            let short = truncate_status(msg, 36);
            return Some(format!("⇄ {short}"));
        }

        Some(format!("⇄ {} errors", errors.len()))
    }
}

fn truncate_status(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub(crate) struct SharedState {
    rows: Mutex<HashMap<Uuid, ForwardRuntime>>,
    forwarding_denied: AtomicBool,
    generation: AtomicU64,
    notify: Sender<()>,
}

impl SharedState {
    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.notify.try_send(());
    }

    fn upsert_row(&self, row: ForwardRuntime) {
        self.rows.lock().insert(row.id, row);
        self.bump();
    }

    fn update_row(&self, id: Uuid, f: impl FnOnce(&mut ForwardRuntime)) {
        let mut map = self.rows.lock();
        if let Some(row) = map.get_mut(&id) {
            f(row);
            drop(map);
            self.bump();
        }
    }

    fn remove_row(&self, id: Uuid) {
        self.rows.lock().remove(&id);
        self.bump();
    }

    fn snapshot(&self) -> ForwardSnapshot {
        let mut rows: Vec<_> = self.rows.lock().values().cloned().collect();
        rows.sort_by(|a, b| {
            a.bind_port
                .cmp(&b.bind_port)
                .then_with(|| a.name.cmp(&b.name))
        });
        ForwardSnapshot {
            rows,
            forwarding_denied: self.forwarding_denied.load(Ordering::SeqCst),
            generation: self.generation.load(Ordering::SeqCst),
        }
    }

    fn set_forwarding_denied(&self, denied: bool) {
        self.forwarding_denied.store(denied, Ordering::SeqCst);
        self.bump();
    }
}

pub(crate) enum ForwardCmd {
    Start {
        rule: PortForwardRule,
        temporary: bool,
        reply: Sender<Result<()>>,
    },
    Stop {
        id: Uuid,
        reply: Sender<Result<()>>,
    },
    /// Soft-stop then start again (same rule fields from current row).
    Retry {
        id: Uuid,
        reply: Sender<Result<()>>,
    },
}

/// Cloneable UI handle for the SSH-thread forward worker.
#[derive(Clone)]
pub struct ForwardHandle {
    tx: Sender<ForwardCmd>,
    state: Arc<SharedState>,
    /// Fired when runtime rows change (UI should `cx.notify`).
    pub changes: Receiver<()>,
}

impl ForwardHandle {
    pub fn snapshot(&self) -> ForwardSnapshot {
        self.state.snapshot()
    }

    pub fn start(&self, rule: PortForwardRule, temporary: bool) -> Result<()> {
        let (reply_tx, reply_rx) = flume::bounded(1);
        self.tx
            .send(ForwardCmd::Start {
                rule,
                temporary,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("forward channel closed"))?;
        reply_rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| anyhow::anyhow!("forward start timed out"))?
    }

    pub fn stop(&self, id: Uuid) -> Result<()> {
        let (reply_tx, reply_rx) = flume::bounded(1);
        self.tx
            .send(ForwardCmd::Stop {
                id,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("forward channel closed"))?;
        reply_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| anyhow::anyhow!("forward stop timed out"))?
    }

    pub fn retry(&self, id: Uuid) -> Result<()> {
        let (reply_tx, reply_rx) = flume::bounded(1);
        self.tx
            .send(ForwardCmd::Retry {
                id,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("forward channel closed"))?;
        reply_rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| anyhow::anyhow!("forward retry timed out"))?
    }
}

pub fn channel_pair() -> (ForwardHandle, Receiver<ForwardCmd>, Arc<SharedState>) {
    let (cmd_tx, cmd_rx) = flume::unbounded();
    let (notify_tx, notify_rx) = flume::unbounded();
    let state = Arc::new(SharedState {
        rows: Mutex::new(HashMap::new()),
        forwarding_denied: AtomicBool::new(false),
        generation: AtomicU64::new(0),
        notify: notify_tx,
    });
    let handle = ForwardHandle {
        tx: cmd_tx,
        state: Arc::clone(&state),
        changes: notify_rx,
    };
    (handle, cmd_rx, state)
}

/// Runs on the SSH Tokio runtime until `cmd_rx` closes (pane teardown).
pub async fn run_forward_worker(
    session: Arc<client::Handle<ClientHandler>>,
    cmd_rx: Receiver<ForwardCmd>,
    state: Arc<SharedState>,
) {
    let mut stops: HashMap<Uuid, Sender<()>> = HashMap::new();

    while let Ok(cmd) = cmd_rx.recv_async().await {
        match cmd {
            ForwardCmd::Start {
                rule,
                temporary,
                reply,
            } => {
                let result = start_local(
                    Arc::clone(&session),
                    Arc::clone(&state),
                    &mut stops,
                    rule,
                    temporary,
                )
                .await;
                let _ = reply.send(result);
            }
            ForwardCmd::Stop { id, reply } => {
                if let Some(tx) = stops.remove(&id) {
                    let _ = tx.send(());
                }
                state.remove_row(id);
                let _ = reply.send(Ok(()));
            }
            ForwardCmd::Retry { id, reply } => {
                let rule = {
                    let map = state.rows.lock();
                    map.get(&id).map(|r| PortForwardRule {
                        id: r.id,
                        kind: PortForwardKind::Local,
                        bind_host: r.bind_host.clone(),
                        bind_port: r.bind_port,
                        target_host: r.target_host.clone(),
                        target_port: r.target_port,
                        name: r.name.clone(),
                        enabled: true,
                    })
                };
                let temporary = state
                    .rows
                    .lock()
                    .get(&id)
                    .map(|r| r.temporary)
                    .unwrap_or(false);
                if let Some(tx) = stops.remove(&id) {
                    let _ = tx.send(());
                }
                state.remove_row(id);
                let result = match rule {
                    Some(rule) => {
                        start_local(
                            Arc::clone(&session),
                            Arc::clone(&state),
                            &mut stops,
                            rule,
                            temporary,
                        )
                        .await
                    }
                    None => Err(anyhow::anyhow!("forward rule not found")),
                };
                let _ = reply.send(result);
            }
        }
    }

    for (_, tx) in stops.drain() {
        let _ = tx.send(());
    }
    state.rows.lock().clear();
    state.bump();
}

async fn start_local(
    session: Arc<client::Handle<ClientHandler>>,
    state: Arc<SharedState>,
    stops: &mut HashMap<Uuid, Sender<()>>,
    rule: PortForwardRule,
    temporary: bool,
) -> Result<()> {
    if !matches!(rule.kind, PortForwardKind::Local) {
        bail!("only Local forwards are supported in this release");
    }
    if rule.bind_port == 0 {
        bail!("bind port must be 1–65535");
    }
    if rule.target_port == 0 {
        bail!("target port must be 1–65535");
    }

    // Replace existing rule with same id.
    if let Some(tx) = stops.remove(&rule.id) {
        let _ = tx.send(());
    }

    state.upsert_row(ForwardRuntime {
        id: rule.id,
        name: rule.name.clone(),
        bind_host: rule.bind_host.clone(),
        bind_port: rule.bind_port,
        target_host: rule.target_host.clone(),
        target_port: rule.target_port,
        temporary,
        status: ForwardStatus::Starting,
        last_dial_error: None,
    });

    let bind_addr = format!("{}:{}", rule.bind_host.trim(), rule.bind_port);
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(err) => {
            let msg = if err.kind() == std::io::ErrorKind::AddrInUse {
                format!("{} already in use", rule.bind_port)
            } else {
                format!("bind {bind_addr}: {err}")
            };
            state.update_row(rule.id, |r| {
                r.status = ForwardStatus::Error(msg.clone());
            });
            bail!("{msg}");
        }
    };

    let (stop_tx, stop_rx) = flume::bounded::<()>(1);
    stops.insert(rule.id, stop_tx);

    state.update_row(rule.id, |r| {
        r.status = ForwardStatus::Listening;
        r.last_dial_error = None;
    });

    let target_host = rule.target_host.clone();
    let target_port = rule.target_port;
    let rule_id = rule.id;
    let state_loop = Arc::clone(&state);
    let session_loop = Arc::clone(&session);

    tokio::spawn(async move {
        accept_loop(
            listener,
            session_loop,
            state_loop,
            rule_id,
            target_host,
            target_port,
            stop_rx,
        )
        .await;
    });

    Ok(())
}

async fn accept_loop(
    listener: TcpListener,
    session: Arc<client::Handle<ClientHandler>>,
    state: Arc<SharedState>,
    rule_id: Uuid,
    target_host: String,
    target_port: u16,
    stop_rx: Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = stop_rx.recv_async() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((tcp, peer)) => {
                        let session = Arc::clone(&session);
                        let state = Arc::clone(&state);
                        let target_host = target_host.clone();
                        tokio::spawn(async move {
                            if let Err(err) = bridge_one(
                                session,
                                tcp,
                                peer,
                                &target_host,
                                target_port,
                            ).await {
                                let msg = format_bridge_error(&err);
                                if is_forwarding_denied(&err) {
                                    state.set_forwarding_denied(true);
                                }
                                state.update_row(rule_id, |r| {
                                    r.last_dial_error = Some(msg);
                                });
                            }
                        });
                    }
                    Err(err) => {
                        state.update_row(rule_id, |r| {
                            r.status = ForwardStatus::Error(format!("accept: {err}"));
                        });
                        break;
                    }
                }
            }
        }
    }
}

async fn bridge_one(
    session: Arc<client::Handle<ClientHandler>>,
    mut tcp: TcpStream,
    peer: SocketAddr,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let channel = session
        .channel_open_direct_tcpip(
            target_host.to_string(),
            u32::from(target_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await
        .context("open direct-tcpip")?;

    let mut ssh = channel.into_stream();
    match tokio::io::copy_bidirectional(&mut tcp, &mut ssh).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => {
            // Best-effort half-close.
            let _ = tcp.shutdown().await;
            return Err(err).context("forward copy");
        }
    }
    Ok(())
}

fn is_forwarding_denied(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(russh::Error::ChannelOpenFailure(
            ChannelOpenFailure::AdministrativelyProhibited,
        )) = cause.downcast_ref::<russh::Error>()
        {
            return true;
        }
        let s = cause.to_string().to_lowercase();
        if s.contains("administratively prohibited")
            || s.contains("forwarding disabled")
            || s.contains("port forwarding is disabled")
        {
            return true;
        }
    }
    false
}

fn format_bridge_error(err: &anyhow::Error) -> String {
    if is_forwarding_denied(err) {
        return "Server disabled TCP forwarding (AllowTcpForwarding)".into();
    }
    for cause in err.chain() {
        if let Some(russh::Error::ChannelOpenFailure(reason)) =
            cause.downcast_ref::<russh::Error>()
        {
            return match reason {
                ChannelOpenFailure::ConnectFailed => {
                    "Target unreachable (connection refused)".into()
                }
                ChannelOpenFailure::ResourceShortage => "SSH channel resource shortage".into(),
                ChannelOpenFailure::UnknownChannelType => "SSH rejected channel type".into(),
                ChannelOpenFailure::AdministrativelyProhibited => {
                    "Server disabled TCP forwarding (AllowTcpForwarding)".into()
                }
                _ => format!("SSH channel open failed: {reason:?}"),
            };
        }
    }
    format!("{err:#}")
}
