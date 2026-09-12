//! Container filesystem via `docker exec` (list) and `docker cp` (transfer).
//!
//! Must not run on the UI thread. Windows: [`crate::platform::new_command`].

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::docker::{container_id_from_args, docker_program};
use super::sftp::{RemoteEntry, TransferCancel, TransferOutcome, TransferProgress};
use crate::model::ProfileKind;

/// Resolve container id from a Docker Local or Docker-over-SSH profile kind.
pub fn container_id_from_kind(kind: &ProfileKind) -> Option<&str> {
    if let Some(id) = kind.docker_ssh_container_id() {
        return Some(id);
    }
    match kind {
        ProfileKind::Local { args, .. } if kind.is_docker_local() => {
            container_id_from_args(args)
        }
        _ => None,
    }
}

fn docker_cmd() -> Command {
    crate::platform::new_command(docker_program())
}

/// Normalize to an absolute Unix path inside the container.
pub fn normalize_path(path: &str) -> String {
    let t = path.trim();
    if t.is_empty() || t == "." {
        return "/".into();
    }
    let mut out = String::new();
    if !t.starts_with('/') {
        out.push('/');
    }
    for part in t.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if let Some(i) = out.rfind('/') {
                out.truncate(i);
            }
            if out.is_empty() {
                out.push('/');
            }
            continue;
        }
        if out == "/" {
            out.push_str(part);
        } else {
            out.push('/');
            out.push_str(part);
        }
    }
    if out.is_empty() {
        "/".into()
    } else {
        out
    }
}

pub fn parent_path(path: &str) -> Option<String> {
    let p = normalize_path(path);
    if p == "/" {
        return None;
    }
    match p.rfind('/') {
        Some(0) => Some("/".into()),
        Some(i) => Some(p[..i].to_string()),
        None => None,
    }
}

pub fn join_child(parent: &str, name: &str) -> String {
    let name = name.trim().trim_start_matches('/');
    let p = normalize_path(parent);
    if p == "/" {
        format!("/{name}")
    } else {
        format!("{p}/{name}")
    }
}

/// `$HOME` inside the container, or `/`.
pub fn home_dir(container: &str) -> Result<String> {
    let output = docker_cmd()
        .args(["exec", container, "printenv", "HOME"])
        .output()
        .context("docker exec printenv HOME")?;
    if !output.status.success() {
        return Ok("/".into());
    }
    let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if home.is_empty() {
        Ok("/".into())
    } else {
        Ok(normalize_path(&home))
    }
}

/// List one directory level. Paths in entries are absolute Unix paths.
pub fn list_dir(container: &str, path: &str) -> Result<Vec<RemoteEntry>> {
    let path = normalize_path(path);
    // kind \t size \t name — names must not contain tabs/newlines.
    let script = r#"
path=$1
[ -d "$path" ] || exit 1
find "$path" -mindepth 1 -maxdepth 1 2>/dev/null | while IFS= read -r full; do
  [ -z "$full" ] && continue
  name=${full##*/}
  if [ -d "$full" ]; then kind=d; else kind=f; fi
  if [ -f "$full" ]; then
    sz=$(wc -c <"$full" 2>/dev/null | tr -d ' \n' || echo 0)
  else
    sz=0
  fi
  safe=$(printf '%s' "$name" | tr '\t\n' '  ')
  printf '%s\t%s\t%s\n' "$kind" "$sz" "$safe"
done
"#;
    let output = docker_cmd()
        .args(["exec", container, "sh", "-c", script, "--", &path])
        .output()
        .with_context(|| format!("docker exec list `{path}`"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let msg = err.trim();
        bail!(
            "{}",
            if msg.is_empty() {
                format!("Cannot list `{path}` in container")
            } else {
                msg.to_string()
            }
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
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

/// Check that `path` exists and is a directory inside the container.
pub fn resolve_existing_dir(container: &str, path: &str) -> Result<String, String> {
    let path = normalize_path(path);
    let output = docker_cmd()
        .args([
            "exec",
            container,
            "sh",
            "-c",
            r#"path=$1; [ -d "$path" ] || exit 1; printf '%s\n' "$path""#,
            "--",
            &path,
        ])
        .output()
        .map_err(|e| format!("docker exec: {e}"))?;
    if !output.status.success() {
        return Err("Path does not exist or is not a directory".into());
    }
    Ok(path)
}

pub fn create_dir(container: &str, path: &str) -> Result<()> {
    let path = normalize_path(path);
    let status = docker_cmd()
        .args(["exec", container, "mkdir", "-p", &path])
        .status()
        .context("docker exec mkdir")?;
    if !status.success() {
        bail!("mkdir failed: {path}");
    }
    Ok(())
}

pub fn remove_path(container: &str, path: &str, is_dir: bool) -> Result<()> {
    let path = normalize_path(path);
    if path == "/" {
        bail!("Refusing to remove /");
    }
    let mut cmd = docker_cmd();
    if is_dir {
        cmd.args(["exec", container, "rm", "-rf", &path]);
    } else {
        cmd.args(["exec", container, "rm", "-f", &path]);
    }
    let status = cmd.status().context("docker exec rm")?;
    if !status.success() {
        bail!("remove failed: {path}");
    }
    Ok(())
}

pub fn rename(container: &str, from: &str, to: &str) -> Result<()> {
    let from = normalize_path(from);
    let to = normalize_path(to);
    let status = docker_cmd()
        .args(["exec", container, "mv", &from, &to])
        .status()
        .context("docker exec mv")?;
    if !status.success() {
        bail!("rename failed: {from} → {to}");
    }
    Ok(())
}

pub fn chmod(container: &str, path: &str, mode: u32) -> Result<()> {
    let path = normalize_path(path);
    let mode_s = format!("{mode:o}");
    let status = docker_cmd()
        .args(["exec", container, "chmod", &mode_s, &path])
        .status()
        .context("docker exec chmod")?;
    if !status.success() {
        bail!("chmod failed: {path}");
    }
    Ok(())
}

/// Best-effort Info tab snapshot (container summary + ports / volumes).
pub fn container_snapshot(container: &str) -> Result<crate::session::host_info::HostSnapshot> {
    let output = docker_cmd()
        .args(["inspect", "--format", "{{json .}}", container])
        .output()
        .context("docker inspect")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("{}", err.trim());
    }
    let json = String::from_utf8_lossy(&output.stdout);
    super::docker::host_snapshot_from_inspect(&json, container, false)
}

fn docker_cp_spec(container: &str, container_path: &str) -> String {
    let p = normalize_path(container_path);
    format!("{container}:{p}")
}

/// `docker cp container:remote → local` (file or directory).
pub fn copy_from_container(
    container: &str,
    remote: &str,
    local: &Path,
    id: uuid::Uuid,
    progress: flume::Sender<TransferProgress>,
    cancel: TransferCancel,
) -> Result<TransferOutcome> {
    if let Some(parent) = local.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let src = docker_cp_spec(container, remote);
    let mut child = docker_cmd()
        .arg("cp")
        .arg(&src)
        .arg(local)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn docker cp (download)")?;
    wait_child_cancellable(&mut child, &cancel, id, &progress)?;
    let bytes = dir_or_file_size(local).unwrap_or(0);
    let files = if local.is_dir() {
        count_files(local).unwrap_or(1)
    } else {
        1
    };
    let _ = progress.send(TransferProgress {
        id,
        done: bytes,
        total: Some(bytes),
        files_done: Some(files),
        files_total: Some(files),
        overall_done: Some(bytes),
        overall_total: Some(bytes),
        label: None,
        local_path: Some(local.to_path_buf()),
    });
    Ok(TransferOutcome {
        files,
        bytes,
        saved_as: None,
    })
}

/// `docker cp local → container:remote` (file or directory).
pub fn copy_to_container(
    container: &str,
    local: &Path,
    remote: &str,
    id: uuid::Uuid,
    progress: flume::Sender<TransferProgress>,
    cancel: TransferCancel,
) -> Result<TransferOutcome> {
    if !local.exists() {
        bail!("Local path missing: {}", local.display());
    }
    let dest = docker_cp_spec(container, remote);
    let mut child = docker_cmd()
        .arg("cp")
        .arg(local)
        .arg(&dest)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn docker cp (upload)")?;
    wait_child_cancellable(&mut child, &cancel, id, &progress)?;
    let bytes = dir_or_file_size(local).unwrap_or(0);
    let files = if local.is_dir() {
        count_files(local).unwrap_or(1)
    } else {
        1
    };
    let _ = progress.send(TransferProgress {
        id,
        done: bytes,
        total: Some(bytes),
        files_done: Some(files),
        files_total: Some(files),
        overall_done: Some(bytes),
        overall_total: Some(bytes),
        label: None,
        local_path: None,
    });
    Ok(TransferOutcome {
        files,
        bytes,
        saved_as: None,
    })
}

fn wait_child_cancellable(
    child: &mut Child,
    cancel: &TransferCancel,
    id: uuid::Uuid,
    progress: &flume::Sender<TransferProgress>,
) -> Result<()> {
    let stderr = child.stderr.take();
    let err_thread = stderr.map(|err| {
        thread::spawn(move || {
            let mut buf = String::new();
            let mut r = BufReader::new(err);
            let _ = r.read_line(&mut buf);
            buf
        })
    });

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled");
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let err_msg = err_thread
                    .and_then(|t| t.join().ok())
                    .unwrap_or_default();
                if !status.success() {
                    let msg = err_msg.trim();
                    bail!(
                        "{}",
                        if msg.is_empty() {
                            "docker cp failed"
                        } else {
                            msg
                        }
                    );
                }
                return Ok(());
            }
            Ok(None) => {
                let _ = progress.send(TransferProgress {
                    id,
                    done: 0,
                    total: None,
                    files_done: None,
                    files_total: None,
                    overall_done: None,
                    overall_total: None,
                    label: None,
                    local_path: None,
                });
                thread::sleep(Duration::from_millis(200));
            }
            Err(e) => bail!("wait docker cp: {e}"),
        }
    }
}

fn dir_or_file_size(path: &Path) -> Result<u64> {
    let meta = std::fs::metadata(path)?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    let mut total = 0u64;
    fn walk(p: &Path, total: &mut u64) -> Result<()> {
        if p.is_file() {
            *total += std::fs::metadata(p)?.len();
            return Ok(());
        }
        if p.is_dir() {
            for e in std::fs::read_dir(p)? {
                walk(&e?.path(), total)?;
            }
        }
        Ok(())
    }
    walk(path, &mut total)?;
    Ok(total)
}

fn count_files(path: &Path) -> Result<u32> {
    if path.is_file() {
        return Ok(1);
    }
    let mut n = 0u32;
    fn walk(p: &Path, n: &mut u32) -> Result<()> {
        if p.is_file() {
            *n = n.saturating_add(1);
            return Ok(());
        }
        if p.is_dir() {
            for e in std::fs::read_dir(p)? {
                walk(&e?.path(), n)?;
            }
        }
        Ok(())
    }
    walk(path, &mut n)?;
    Ok(n.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_parent() {
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
        assert_eq!(parent_path("/"), None);
        assert_eq!(parent_path("/a"), Some("/".into()));
        assert_eq!(parent_path("/a/b"), Some("/a".into()));
        assert_eq!(join_child("/", "x"), "/x");
        assert_eq!(join_child("/a", "b"), "/a/b");
    }
}
