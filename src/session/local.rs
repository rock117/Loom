use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Spawn a local shell in a PTY and return reader/writer plus resize handle.
pub struct LocalPty {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
}

impl LocalPty {
    pub fn spawn(shell: &str, cwd: Option<&std::path::Path>) -> Result<Self> {
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
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let _child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn shell `{shell}`"))?;
        drop(pair.slave);

        let writer = pair.master.take_writer().context("pty writer")?;
        let reader = pair.master.try_clone_reader().context("pty reader")?;
        let master = Arc::new(Mutex::new(pair.master));

        Ok(Self {
            master,
            reader,
            writer,
        })
    }

    pub fn resize_callback(master: Arc<Mutex<Box<dyn MasterPty + Send>>>) -> impl Fn(usize, usize) + Send + Sync + 'static {
        move |cols: usize, rows: usize| {
            let _ = master.lock().resize(PtySize {
                cols: cols as u16,
                rows: rows as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }
}

/// Resolve default shell executable for this platform.
pub fn resolve_shell(configured: Option<&str>) -> String {
    if let Some(shell) = configured {
        if !shell.is_empty() {
            return shell.to_string();
        }
    }

    #[cfg(windows)]
    {
        for candidate in ["pwsh", "powershell", "cmd"] {
            if which_exists(candidate) {
                return candidate.to_string();
            }
        }
        "cmd".to_string()
    }

    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

#[cfg(windows)]
fn which_exists(name: &str) -> bool {
    std::process::Command::new("where")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
