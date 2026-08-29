//! Interactive SSH session bridged to std Read/Write for TerminalView.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use flume::{Receiver, Sender};
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use tokio::runtime::Builder as RtBuilder;

use super::known_hosts;

/// Auth material resolved before connect (password never written to profile JSON).
#[derive(Debug, Clone)]
pub enum SshAuthMaterial {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

pub struct SshConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuthMaterial,
    pub cols: u32,
    pub rows: u32,
}

/// Handles returned to the UI after a successful SSH shell start.
pub struct SshSessionHandles {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub resize: Arc<dyn Fn(usize, usize) + Send + Sync>,
    /// Drop or send to request disconnect (best-effort).
    pub shutdown: Sender<()>,
    /// Same-session SFTP (opens subsystem channel on first use).
    pub sftp: crate::session::sftp::SftpHandle,
}

struct ChannelReader {
    rx: Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.buf.is_empty() {
            match self.rx.recv() {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        return Ok(0);
                    }
                    self.buf.extend(chunk);
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len());
        for (i, b) in self.buf.drain(..n).enumerate() {
            out[i] = b;
        }
        Ok(n)
    }
}

struct ChannelWriter {
    tx: Sender<Vec<u8>>,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.tx
            .send(buf.to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct ClientHandler {
    host: String,
    port: u16,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = server_public_key.fingerprint(HashAlg::Sha256).to_string();
        match known_hosts::check_and_record(&self.host, self.port, &fp) {
            Ok(()) => Ok(true),
            Err(err) => {
                eprintln!("loom: SSH host key rejected: {err:#}");
                Ok(false)
            }
        }
    }
}

/// Blocking connect: spawns a dedicated Tokio thread that owns the session.
pub fn connect_blocking(params: SshConnectParams) -> Result<SshSessionHandles> {
    let (stdin_tx, stdin_rx) = flume::unbounded::<Vec<u8>>();
    let (stdout_tx, stdout_rx) = flume::unbounded::<Vec<u8>>();
    let (resize_tx, resize_rx) = flume::unbounded::<(u32, u32)>();
    let (shutdown_tx, shutdown_rx) = flume::bounded::<()>(1);
    let (ready_tx, ready_rx) = flume::bounded::<Result<()>>(1);
    let (sftp_handle, sftp_rx) = crate::session::sftp::channel_pair();

    let host = params.host.clone();
    let port = params.port;
    let user = params.user.clone();
    let auth = params.auth.clone();
    let cols = params.cols.max(1);
    let rows = params.rows.max(1);

    thread::Builder::new()
        .name("loom-ssh".into())
        .spawn(move || {
            let rt = match RtBuilder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("loom-ssh-worker")
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("tokio runtime: {err}")));
                    return;
                }
            };
            rt.block_on(async move {
                match run_session(
                    host,
                    port,
                    user,
                    auth,
                    cols,
                    rows,
                    stdin_rx,
                    stdout_tx.clone(),
                    resize_rx,
                    shutdown_rx,
                    sftp_rx,
                    &ready_tx,
                )
                .await
                {
                    Ok(()) => {
                        let _ = stdout_tx.send(Vec::new());
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("{err:#}")));
                        let _ = stdout_tx.send(Vec::new());
                        eprintln!("loom: SSH session ended: {err:#}");
                    }
                }
            });
        })
        .context("spawn SSH thread")?;

    match ready_rx.recv_timeout(Duration::from_secs(45)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err),
        Err(_) => bail!("SSH connection timed out"),
    }

    let resize = Arc::new(move |c: usize, r: usize| {
        let _ = resize_tx.send((c.max(1) as u32, r.max(1) as u32));
    });

    Ok(SshSessionHandles {
        reader: Box::new(ChannelReader {
            rx: stdout_rx,
            buf: VecDeque::new(),
        }),
        writer: Box::new(ChannelWriter { tx: stdin_tx }),
        resize,
        shutdown: shutdown_tx,
        sftp: sftp_handle,
    })
}

async fn run_session(
    host: String,
    port: u16,
    user: String,
    auth: SshAuthMaterial,
    cols: u32,
    rows: u32,
    stdin_rx: Receiver<Vec<u8>>,
    stdout_tx: Sender<Vec<u8>>,
    resize_rx: Receiver<(u32, u32)>,
    shutdown_rx: Receiver<()>,
    sftp_rx: Receiver<crate::session::sftp::SftpRequest>,
    ready_tx: &Sender<Result<()>>,
) -> Result<()> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });

    let handler = ClientHandler {
        host: host.clone(),
        port,
    };

    let mut session = client::connect(config, (host.as_str(), port), handler)
        .await
        .with_context(|| format!("connect to {host}:{port}"))?;

    authenticate(&mut session, &user, auth)
        .await
        .context("SSH authentication")?;

    let mut channel = session
        .channel_open_session()
        .await
        .context("open session channel")?;

    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .context("request PTY")?;
    channel
        .request_shell(true)
        .await
        .context("request shell")?;

    // Share Handle via Arc so SFTP can open another channel while the shell runs.
    let session = Arc::new(session);
    tokio::spawn(crate::session::sftp::run_sftp_worker(
        Arc::clone(&session),
        sftp_rx,
    ));

    if ready_tx.send(Ok(())).is_err() {
        return Ok(());
    }

    let mut stdin_closed = false;
    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.recv_async() => {
                let _ = session
                    .disconnect(Disconnect::ByApplication, "closed", "")
                    .await;
                break;
            }
            msg = stdin_rx.recv_async(), if !stdin_closed => {
                match msg {
                    Ok(data) => {
                        if data.is_empty() {
                            stdin_closed = true;
                            let _ = channel.eof().await;
                        } else if let Err(err) = channel.data(&data[..]).await {
                            bail!("send to SSH: {err}");
                        }
                    }
                    Err(_) => {
                        stdin_closed = true;
                        let _ = channel.eof().await;
                    }
                }
            }
            resize = resize_rx.recv_async() => {
                if let Ok((c, r)) = resize {
                    let _ = channel.window_change(c, r, 0, 0).await;
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if stdout_tx.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        if stdout_tx.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => {
                        break;
                    }
                    Some(ChannelMsg::Eof) => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn authenticate(
    session: &mut client::Handle<ClientHandler>,
    user: &str,
    auth: SshAuthMaterial,
) -> Result<()> {
    match auth {
        SshAuthMaterial::Password(password) => {
            let res = session
                .authenticate_password(user, password.clone())
                .await
                .context("password auth")?;
            if !res.success() {
                // Some servers want keyboard-interactive instead.
                let kbd = session
                    .authenticate_keyboard_interactive_start(user, None)
                    .await;
                if let Ok(mut response) = kbd {
                    loop {
                        match response {
                            KeyboardInteractiveAuthResponse::Success => break,
                            KeyboardInteractiveAuthResponse::Failure { .. } => {
                                bail!("authentication failed");
                            }
                            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                                let answers: Vec<String> = prompts
                                    .iter()
                                    .map(|p| {
                                        if p.prompt.to_lowercase().contains("password") {
                                            password.clone()
                                        } else {
                                            String::new()
                                        }
                                    })
                                    .collect();
                                response = session
                                    .authenticate_keyboard_interactive_respond(answers)
                                    .await
                                    .context("keyboard-interactive")?;
                            }
                        }
                    }
                } else {
                    bail!("authentication failed");
                }
            }
        }
        SshAuthMaterial::PrivateKey { path, passphrase } => {
            let key_pair = load_secret_key(&path, passphrase.as_deref())
                .with_context(|| format!("load key {}", path.display()))?;
            let hash = session.best_supported_rsa_hash().await?.flatten();
            let res = session
                .authenticate_publickey(
                    user,
                    PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash),
                )
                .await
                .context("publickey auth")?;
            if !res.success() {
                bail!("public key authentication failed");
            }
        }
    }
    Ok(())
}
