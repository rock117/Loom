//! Local PTY session helpers (portable-pty). Child/killer kept for clean teardown.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::platform;

/// Spawn a local shell in a PTY and return reader/writer plus resize/teardown handles.
pub struct LocalPty {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
}

impl LocalPty {
    pub fn spawn(shell: &str, cwd: Option<&Path>) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let resolved_cwd = cwd
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());
        if let Some(ref dir) = resolved_cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn shell `{shell}`"))?;
        drop(pair.slave);

        let killer = child.clone_killer();
        // Keep the Child alive on a background wait thread so Drop does not
        // block the UI; killer is used for explicit teardown.
        thread::Builder::new()
            .name("pty-child-wait".into())
            .spawn(move || {
                let mut child = child;
                let _ = child.wait();
            })
            .context("spawn pty wait thread")?;

        let writer = pair.master.take_writer().context("pty writer")?;
        let reader = pair.master.try_clone_reader().context("pty reader")?;
        let master = Arc::new(Mutex::new(pair.master));

        Ok(Self {
            master,
            reader,
            writer,
            killer,
        })
    }

    pub fn default_cwd() -> Option<PathBuf> {
        std::env::current_dir().ok()
    }

    pub fn resize_callback(
        master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    ) -> impl Fn(usize, usize) + Send + Sync + 'static {
        move |cols: usize, rows: usize| {
            if let Err(error) = master.lock().resize(PtySize {
                cols: cols as u16,
                rows: rows as u16,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                eprintln!("loom: pty resize failed: {error}");
            }
        }
    }
}

/// Resolve default shell executable for this platform.
pub fn resolve_shell(configured: Option<&str>) -> String {
    platform::resolve_shell(configured)
}

/// Tear down a ConPTY/session off the UI thread (kill then drop master).
pub fn teardown_pty(
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
) {
    thread::Builder::new()
        .name("pty-teardown".into())
        .spawn(move || {
            if let Some(mut killer) = killer {
                let _ = killer.kill();
            }
            drop(master);
        })
        .ok();
}
