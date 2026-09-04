//! Local PTY session helpers (portable-pty). Child/killer kept for clean teardown.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::platform;
use crate::session::local_proxy::{self, LocalProxyMode};

/// Spawn a local shell in a PTY and return reader/writer plus resize/teardown handles.
pub struct LocalPty {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
    /// Shell process id (for live cwd via PEB /proc — Zed-style fallback to OSC).
    pub shell_pid: Option<u32>,
}

impl LocalPty {
    pub fn spawn(
        shell: &str,
        cwd: Option<&Path>,
        proxy_mode: LocalProxyMode,
        proxy_url: Option<&str>,
        proxy_no_proxy: Option<&str>,
    ) -> Result<Self> {
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
        for (k, v) in local_proxy::proxy_env_vars(proxy_mode, proxy_url, proxy_no_proxy) {
            cmd.env(&k, &v);
        }

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

        let shell_pid = child.process_id();
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
            shell_pid,
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
///
/// OSC still matters for SSH / when process cwd is unavailable. Local Reveal/Copy
/// Path also refresh via [`crate::platform::process_cwd`] (Zed-style).
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
        // Wrap the existing prompt so user profiles still run; emit OSC 7 + 9;9.
        // One-liner + -EncodedCommand avoids CreateProcess mangling multiline -Command.
        "pwsh" | "powershell" => {
            cmd.arg("-NoExit");
            cmd.arg("-EncodedCommand");
            cmd.arg(pwsh_encoded_osc_hook());
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

/// PowerShell: preserve prior `prompt`; emit OSC 7 (file URI) and OSC 9;9 (Win path).
const PWSH_OSC_HOOK: &str = r#"
if (-not $global:__LoomOscHooked) {
  $global:__LoomOscHooked = $true
  $global:__LoomPrevPrompt = $function:prompt
  function global:prompt {
    try {
      $p = (Get-Location).ProviderPath
      $uriPath = ($p -replace '\\', '/')
      if ($uriPath -match '^[A-Za-z]:') { $uriPath = '/' + $uriPath }
      $esc = [char]27; $bel = [char]7
      [Console]::Out.Write("$esc]7;file://$uriPath$bel")
      [Console]::Out.Write("$esc]9;9;$p$bel")
    } catch {}
    if ($global:__LoomPrevPrompt) { & $global:__LoomPrevPrompt } else { 'PS> ' }
  }
}
"#;

fn pwsh_encoded_osc_hook() -> String {
    // PowerShell -EncodedCommand expects UTF-16LE bytes, base64-encoded.
    let utf16: Vec<u8> = PWSH_OSC_HOOK
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    base64_encode(&utf16)
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Resolve default shell executable for this platform.
pub use platform::{ResolvedShell, resolve_shell};
