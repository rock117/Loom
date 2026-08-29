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
        cmd.env("TERM_PROGRAM", "Loom");
        configure_cwd_reporting(&mut cmd, shell);

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

/// Ask common local shells to emit OSC cwd reports after each prompt.
fn configure_cwd_reporting(cmd: &mut CommandBuilder, shell: &str) {
    let stem = Path::new(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match stem.as_str() {
        // ConEmu / Windows Terminal style — Loom parses OSC 9;9.
        "cmd" => {
            cmd.env("PROMPT", "$E]9;9;$P$E\\$P$G");
        }
        // Wrap the existing prompt so user profiles still run; emit OSC 7 each time.
        "pwsh" | "powershell" => {
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(PWSH_OSC7_HOOK);
        }
        "bash" => {
            let existing = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            let inject =
                r#"printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "$PWD""#;
            let combined = if existing.is_empty() {
                inject.to_string()
            } else {
                format!("{existing}; {inject}")
            };
            cmd.env("PROMPT_COMMAND", combined);
        }
        "zsh" => {
            // precmd via ZDOTDIR would be invasive; set a simple chpwd hook via env script — skip.
            // Many zsh setups already send OSC 7; users without it can enable manually.
        }
        _ => {}
    }
}

/// PowerShell: preserve prior `prompt`, prepend OSC 7 with `file:///C:/...` URI.
const PWSH_OSC7_HOOK: &str = r#"
if (-not $global:__LoomOsc7Hooked) {
  $global:__LoomOsc7Hooked = $true
  $global:__LoomPrevPrompt = $function:prompt
  function global:prompt {
    try {
      $p = (Get-Location).ProviderPath
      $uriPath = ($p -replace '\\', '/')
      if ($uriPath -match '^[A-Za-z]:') { $uriPath = '/' + $uriPath }
      $uri = 'file://' + $uriPath
      [Console]::Write([char]27 + ']7;' + $uri + [char]7)
    } catch {}
    if ($global:__LoomPrevPrompt) { & $global:__LoomPrevPrompt } else { 'PS> ' }
  }
}
"#;

/// Resolve default shell executable for this platform.
pub fn resolve_shell(configured: Option<&str>) -> String {
    platform::resolve_shell(configured)
}
