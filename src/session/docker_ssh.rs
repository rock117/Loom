//! Docker Files over an existing SSH session: browse via `docker exec`,
//! stage transfers with remote `docker cp` under `/tmp` (SFTP lane finishes the copy).

use anyhow::{Context, Result, bail};
use russh::client;
use russh::ChannelMsg;

use super::docker_fs::{join_child, normalize_path};
use super::sftp::RemoteEntry;
use super::ssh::{ClientHandler, shell_single_quote};

/// Run a non-interactive remote command; return stdout on exit 0.
pub async fn remote_exec(
    session: &client::Handle<ClientHandler>,
    command: &str,
) -> Result<String> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("open docker-ssh exec channel")?;
    channel
        .exec(true, command)
        .await
        .context("exec docker-ssh command")?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { ref data }) => stdout.extend_from_slice(data),
            Some(ChannelMsg::ExtendedData { ref data, .. }) => stderr.extend_from_slice(data),
            Some(ChannelMsg::ExitStatus { exit_status }) => status = Some(exit_status),
            Some(ChannelMsg::Eof) | None => break,
            _ => {}
        }
    }
    if status != Some(0) {
        let err = String::from_utf8_lossy(&stderr);
        let out = String::from_utf8_lossy(&stdout);
        let detail = [err.trim(), out.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("remote docker command failed");
        bail!("{detail}");
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

pub async fn home_dir(
    session: &client::Handle<ClientHandler>,
    container: &str,
) -> Result<String> {
    let cmd = format!(
        "docker exec {} sh -c {}",
        shell_single_quote(container.trim()),
        shell_single_quote("printenv HOME || echo /")
    );
    let out = remote_exec(session, &cmd).await?;
    let home = out.trim();
    if home.is_empty() {
        Ok("/".into())
    } else {
        Ok(normalize_path(home))
    }
}

pub async fn list_dir(
    session: &client::Handle<ClientHandler>,
    container: &str,
    path: &str,
) -> Result<Vec<RemoteEntry>> {
    let path = normalize_path(path);
    let inner = format!(
        "path={}; [ -d \"$path\" ] || exit 1; find \"$path\" -mindepth 1 -maxdepth 1 2>/dev/null | while IFS= read -r full; do [ -z \"$full\" ] && continue; name=${{full##*/}}; if [ -d \"$full\" ]; then kind=d; else kind=f; fi; if [ -f \"$full\" ]; then sz=$(wc -c <\"$full\" 2>/dev/null | tr -d ' \\n' || echo 0); else sz=0; fi; safe=$(printf '%s' \"$name\" | tr '\\t\\n' '  '); printf '%s\\t%s\\t%s\\n' \"$kind\" \"$sz\" \"$safe\"; done",
        shell_single_quote(&path)
    );
    let cmd = format!(
        "docker exec {} sh -c {}",
        shell_single_quote(container.trim()),
        shell_single_quote(&inner)
    );
    let stdout = remote_exec(session, &cmd).await?;
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
        let full = join_child(&path, &name);
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

pub async fn resolve_existing_dir(
    session: &client::Handle<ClientHandler>,
    container: &str,
    path: &str,
) -> Result<String, String> {
    let path = normalize_path(path);
    let inner = format!(
        "path={}; [ -d \"$path\" ] || exit 1; printf '%s\\n' \"$path\"",
        shell_single_quote(&path)
    );
    let cmd = format!(
        "docker exec {} sh -c {}",
        shell_single_quote(container.trim()),
        shell_single_quote(&inner)
    );
    match remote_exec(session, &cmd).await {
        Ok(_) => Ok(path),
        Err(_) => Err("Path does not exist or is not a directory".into()),
    }
}

pub async fn create_dir(
    session: &client::Handle<ClientHandler>,
    container: &str,
    path: &str,
) -> Result<()> {
    let path = normalize_path(path);
    let cmd = format!(
        "docker exec {} mkdir -p {}",
        shell_single_quote(container.trim()),
        shell_single_quote(&path)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}

pub async fn remove_path(
    session: &client::Handle<ClientHandler>,
    container: &str,
    path: &str,
    is_dir: bool,
) -> Result<()> {
    let path = normalize_path(path);
    if path == "/" {
        bail!("Refusing to remove /");
    }
    let flag = if is_dir { "-rf" } else { "-f" };
    let cmd = format!(
        "docker exec {} rm {} {}",
        shell_single_quote(container.trim()),
        flag,
        shell_single_quote(&path)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}

pub async fn rename(
    session: &client::Handle<ClientHandler>,
    container: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let from = normalize_path(from);
    let to = normalize_path(to);
    let cmd = format!(
        "docker exec {} mv {} {}",
        shell_single_quote(container.trim()),
        shell_single_quote(&from),
        shell_single_quote(&to)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}

pub async fn chmod(
    session: &client::Handle<ClientHandler>,
    container: &str,
    path: &str,
    mode: u32,
) -> Result<()> {
    let path = normalize_path(path);
    let mode_s = format!("{mode:o}");
    let cmd = format!(
        "docker exec {} chmod {} {}",
        shell_single_quote(container.trim()),
        mode_s,
        shell_single_quote(&path)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}

pub async fn container_snapshot(
    session: &client::Handle<ClientHandler>,
    container: &str,
) -> Result<crate::session::host_info::HostSnapshot> {
    let format = "{{.Name}}\t{{.Config.Image}}\t{{.State.Status}}\t{{.Config.Hostname}}";
    let cmd = format!(
        "docker inspect -f {} {}",
        shell_single_quote(format),
        shell_single_quote(container.trim())
    );
    let line = remote_exec(session, &cmd).await?;
    let mut parts = line.trim().split('\t');
    let name = parts
        .next()
        .unwrap_or(container)
        .trim_start_matches('/')
        .to_string();
    let image = parts.next().unwrap_or("").to_string();
    let status = parts.next().unwrap_or("").to_string();
    let hostname = parts.next().unwrap_or("").to_string();
    Ok(crate::session::host_info::HostSnapshot {
        hostname: if hostname.is_empty() {
            name.clone()
        } else {
            hostname
        },
        os: format!("Docker · {status} (SSH host)"),
        kernel: format!("container {container}"),
        cpu_model: image,
        cpu_cores: 0,
        cpu_usage_pct: None,
        mem_used: 0,
        mem_total: 0,
        disks: Vec::new(),
        gpus: Vec::new(),
        listening: Vec::new(),
        load: Some(name),
        uptime_secs: 0,
    })
}

pub async fn mktemp_dir(session: &client::Handle<ClientHandler>) -> Result<String> {
    let out = remote_exec(session, "mktemp -d /tmp/loom-docker-cp.XXXXXX").await?;
    let dir = out.trim().to_string();
    if dir.is_empty() {
        bail!("mktemp failed");
    }
    Ok(dir)
}

pub async fn rm_rf(session: &client::Handle<ClientHandler>, path: &str) -> Result<()> {
    let _ = remote_exec(session, &format!("rm -rf {}", shell_single_quote(path))).await;
    Ok(())
}

/// `docker cp container:remote → host_path` on the SSH machine.
pub async fn docker_cp_out(
    session: &client::Handle<ClientHandler>,
    container: &str,
    remote: &str,
    host_path: &str,
) -> Result<()> {
    let remote = normalize_path(remote);
    let cmd = format!(
        "docker cp {}:{} {}",
        shell_single_quote(container.trim()),
        shell_single_quote(&remote),
        shell_single_quote(host_path)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}

/// `docker cp host_path → container:remote_dir` on the SSH machine.
pub async fn docker_cp_in(
    session: &client::Handle<ClientHandler>,
    container: &str,
    host_path: &str,
    remote_dir: &str,
) -> Result<()> {
    let remote_dir = normalize_path(remote_dir);
    let cmd = format!(
        "docker cp {} {}:{}",
        shell_single_quote(host_path),
        shell_single_quote(container.trim()),
        shell_single_quote(&remote_dir)
    );
    remote_exec(session, &cmd).await?;
    Ok(())
}
