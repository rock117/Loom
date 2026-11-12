//! Elevated SSH transfers / browse via `sudo`.
//!
//! SFTP has no sudo. When a normal transfer or directory list hits Permission
//! denied, Loom uses `sudo -n` / `sudo -S`:
//! - **Browse:** `find`/`test` under sudo (same TSV shape as Docker Files).
//! - **Transfer:** `sudo cp` into `/tmp`, then SFTP (or the reverse for upload).

use anyhow::{Context, Result, bail};
use russh::ChannelMsg;
use russh::client;

use super::sftp::{RemoteEntry, ResolvedPath, join_remote};
use super::ssh::{ClientHandler, shell_single_quote};

/// How to invoke sudo for an elevated transfer.
#[derive(Clone, Debug)]
pub enum SudoMode {
    /// Try `sudo -n` only (NOPASSWD / already-authorized).
    Passwordless,
    /// Try `sudo -n`, then `sudo -S` with this password.
    Password(String),
}

/// Run a non-interactive remote command; return stdout on exit 0.
pub async fn remote_exec(
    session: &client::Handle<ClientHandler>,
    command: &str,
) -> Result<String> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("open sudo-transfer exec channel")?;
    channel
        .exec(true, command)
        .await
        .context("exec remote command")?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { ref data }) => stdout.extend_from_slice(data),
            Some(ChannelMsg::ExtendedData { ref data, .. }) => stderr.extend_from_slice(data),
            Some(ChannelMsg::ExitStatus { exit_status }) => status = Some(exit_status),
            Some(ChannelMsg::Eof) => {}
            None => break,
            _ => {}
        }
    }
    if let Some(code) = status {
        if code != 0 {
            let err = String::from_utf8_lossy(&stderr);
            let out = String::from_utf8_lossy(&stdout);
            let detail = [err.trim(), out.trim()]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("remote command failed");
            bail!("{detail}");
        }
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// `sudo -S` with password on stdin, then run `inner` via `sh -c`.
async fn remote_exec_sudo(
    session: &client::Handle<ClientHandler>,
    inner: &str,
    password: &str,
) -> Result<String> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("open sudo -S channel")?;
    let wrapped = format!(
        "sudo -S -p '' -- sh -c {}",
        shell_single_quote(inner)
    );
    channel
        .exec(true, wrapped.as_str())
        .await
        .context("exec sudo -S")?;

    let mut payload = password.as_bytes().to_vec();
    payload.push(b'\n');
    channel
        .data(&payload[..])
        .await
        .context("send sudo password")?;
    let _ = channel.eof().await;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { ref data }) => stdout.extend_from_slice(data),
            Some(ChannelMsg::ExtendedData { ref data, .. }) => stderr.extend_from_slice(data),
            Some(ChannelMsg::ExitStatus { exit_status }) => status = Some(exit_status),
            Some(ChannelMsg::Eof) => {}
            None => break,
            _ => {}
        }
    }
    if let Some(code) = status {
        if code != 0 {
            let err = String::from_utf8_lossy(&stderr);
            let out = String::from_utf8_lossy(&stdout);
            let detail = [err.trim(), out.trim()]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("sudo failed");
            bail!("{detail}");
        }
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Run `inner` under sudo: passwordless first, then password if provided.
pub async fn sudo_sh(
    session: &client::Handle<ClientHandler>,
    inner: &str,
    mode: &SudoMode,
) -> Result<String> {
    let nopass = format!("sudo -n -- sh -c {}", shell_single_quote(inner));
    match remote_exec(session, &nopass).await {
        Ok(out) => return Ok(out),
        Err(err) => {
            if matches!(mode, SudoMode::Passwordless) {
                return Err(err).context("sudo requires a password");
            }
        }
    }
    match mode {
        SudoMode::Passwordless => unreachable!(),
        SudoMode::Password(password) => remote_exec_sudo(session, inner, password)
            .await
            .context("sudo"),
    }
}

pub async fn mktemp_dir(session: &client::Handle<ClientHandler>) -> Result<String> {
    let out = remote_exec(session, "mktemp -d /tmp/loom-sudo.XXXXXX").await?;
    let dir = out.trim().to_string();
    if dir.is_empty() {
        bail!("mktemp failed");
    }
    Ok(dir)
}

pub async fn rm_rf_sudo(
    session: &client::Handle<ClientHandler>,
    path: &str,
    mode: &SudoMode,
) -> Result<()> {
    let inner = format!("rm -rf -- {}", shell_single_quote(path));
    let _ = sudo_sh(session, &inner, mode).await;
    Ok(())
}

/// `sudo cp -a src dest` (dest parent must exist / be writable by root).
pub async fn sudo_cp_a(
    session: &client::Handle<ClientHandler>,
    src: &str,
    dest: &str,
    mode: &SudoMode,
) -> Result<()> {
    let inner = format!(
        "cp -a -- {} {}",
        shell_single_quote(src),
        shell_single_quote(dest)
    );
    sudo_sh(session, &inner, mode).await?;
    Ok(())
}

/// Pick a free basename under `remote_dir` using sudo tests (SFTP may be denied).
pub async fn unique_remote_basename_sudo(
    session: &client::Handle<ClientHandler>,
    remote_dir: &str,
    name: &str,
    mode: &SudoMode,
) -> Result<String> {
    let inner = format!(
        "dir={}; name={}; dest=\"$dir/$name\"; if [ ! -e \"$dest\" ]; then printf '%s\\n' \"$name\"; exit 0; fi; stem=${{name%.*}}; ext=; case \"$name\" in *.*) ext=.${{name##*.}}; stem=${{name%.*}};; esac; n=1; while [ \"$n\" -lt 10000 ]; do cand=\"$stem ($n)$ext\"; if [ ! -e \"$dir/$cand\" ]; then printf '%s\\n' \"$cand\"; exit 0; fi; n=$((n+1)); done; exit 1",
        shell_single_quote(remote_dir),
        shell_single_quote(name),
    );
    let out = sudo_sh(session, &inner, mode).await?;
    let chosen = out.trim();
    if chosen.is_empty() {
        bail!("could not find a free remote name for {name}");
    }
    Ok(chosen.to_string())
}

/// List a directory via sudo (SFTP `read_dir` cannot elevate).
pub async fn list_dir(
    session: &client::Handle<ClientHandler>,
    path: &str,
    mode: &SudoMode,
) -> Result<Vec<RemoteEntry>> {
    let path = path.trim().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let inner = format!(
        "path={}; [ -d \"$path\" ] || {{ printf '%s\\n' 'not a directory' >&2; exit 1; }}; find \"$path\" -mindepth 1 -maxdepth 1 2>/dev/null | while IFS= read -r full; do [ -z \"$full\" ] && continue; name=${{full##*/}}; if [ -d \"$full\" ]; then kind=d; else kind=f; fi; if [ -f \"$full\" ]; then sz=$(wc -c <\"$full\" 2>/dev/null | tr -d ' \\n' || echo 0); else sz=0; fi; safe=$(printf '%s' \"$name\" | tr '\\t\\n' '  '); printf '%s\\t%s\\t%s\\n' \"$kind\" \"$sz\" \"$safe\"; done",
        shell_single_quote(path)
    );
    let stdout = sudo_sh(session, &inner, mode).await?;
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let kind = parts.next().unwrap_or("");
        let size: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let name = parts.next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let is_dir = kind == "d";
        let full = join_remote(path, &name);
        rows.push(RemoteEntry {
            name,
            path: full,
            is_dir,
            size,
            mtime: None,
        });
    }
    Ok(rows)
}

/// Stat a path via sudo for the Files address bar.
pub async fn resolve_path(
    session: &client::Handle<ClientHandler>,
    path: &str,
    mode: &SudoMode,
) -> Result<ResolvedPath, String> {
    let path = path.trim().trim_matches('"').trim();
    if path.is_empty() {
        return Err("Path is empty".into());
    }
    let inner = format!(
        "path={}\n\
if [ -d \"$path\" ]; then printf 'd\\n%s\\n' \"$path\"; exit 0; fi\n\
if [ -e \"$path\" ]; then\n\
  size=$(wc -c < \"$path\" 2>/dev/null || printf '0')\n\
  printf 'f\\n%s\\n%s\\n' \"$path\" \"$size\"\n\
  exit 0\n\
fi\n\
printf '%s\\n' 'Path does not exist' >&2\n\
exit 1",
        shell_single_quote(path)
    );
    let stdout = sudo_sh(session, &inner, mode)
        .await
        .map_err(|err| format!("{err:#}"))?;
    let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    let kind = lines
        .next()
        .ok_or_else(|| "Path does not exist".to_string())?;
    let resolved = lines
        .next()
        .ok_or_else(|| "Path does not exist".to_string())?
        .to_string();
    match kind {
        "d" => Ok(ResolvedPath::Dir(resolved)),
        "f" => {
            let size = lines
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let name = resolved
                .rsplit('/')
                .next()
                .unwrap_or(resolved.as_str())
                .to_string();
            Ok(ResolvedPath::File(RemoteEntry {
                name,
                path: resolved,
                is_dir: false,
                size,
                mtime: None,
            }))
        }
        _ => Err("Path does not exist".into()),
    }
}

/// True when an error message looks like a sudo password rejection (re-prompt).
pub fn looks_like_sudo_auth_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("sorry, try again")
        || lower.contains("incorrect password")
        || lower.contains("a password is required")
        || lower.contains("no password was provided")
        || (lower.contains("authentication failed") && lower.contains("sudo"))
}

/// True when a transfer failure is likely fixable with sudo staging.
pub fn looks_like_permission_denied(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("permission denied")
        || lower.contains("permissiondenied")
        || lower.contains("status code: 3") // SFTP SSH_FX_PERMISSION_DENIED
        || lower.contains("ssh_fx_permission_denied")
}
