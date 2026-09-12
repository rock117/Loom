//! Local / remote Docker helpers: list containers and build exec spawns.
//!
//! Listing / preflight must not run on the UI thread (see `docs/HARD_PROBLEMS.md`).
//! Windows: use [`crate::platform::new_command`] so `docker.exe` does not flash a console.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

use crate::model::{Profile, ProfileKind, SshAuth};
use crate::session::ssh::{SshAuthMaterial, exec_once_blocking};

/// One row from `docker ps` (running containers for the picker).
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
}

impl ContainerInfo {
    pub fn short_id(&self) -> &str {
        let id = self.id.trim();
        if id.len() >= 12 {
            &id[..12]
        } else {
            id
        }
    }

    pub fn display_label(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            self.short_id().to_string()
        } else {
            name.to_string()
        }
    }
}

#[derive(Debug, Deserialize)]
struct DockerPsJson {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names", default)]
    names: String,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "Status", default)]
    status: String,
}

pub fn is_docker_executable(shell: &str) -> bool {
    Path::new(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("docker"))
}

/// Program name for Local profile `shell` (`docker` / `docker.exe`).
pub fn docker_program() -> &'static str {
    if cfg!(windows) {
        "docker.exe"
    } else {
        "docker"
    }
}

fn exec_shell_fragment(_container_id: &str) -> String {
    // Prefer bash when present; keep as a single remote argv for Local and SSH exec.
    "command -v bash >/dev/null 2>&1 && exec bash || exec sh".into()
}

/// Interactive shell argv for Local `docker exec -it`.
pub fn exec_profile_kind(container_id: &str) -> ProfileKind {
    let id = container_id.trim().to_string();
    ProfileKind::Local {
        shell: Some(docker_program().into()),
        cwd: None,
        env: Vec::new(),
        args: vec![
            "exec".into(),
            "-it".into(),
            id.clone(),
            "sh".into(),
            "-c".into(),
            exec_shell_fragment(&id),
        ],
    }
}

/// Remote command line for SSH `request_exec` after PTY (`docker exec -it …`).
pub fn exec_remote_command(container_id: &str) -> String {
    let id = container_id.trim();
    let inner = exec_shell_fragment(id);
    format!(
        "docker exec -it {id} sh -c {}",
        crate::session::ssh::shell_single_quote(&inner)
    )
}

/// Container id from Local docker exec argv (`docker exec -it <id> …`).
pub fn container_id_from_args(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "exec" {
            // skip flags until container id
            i += 1;
            while i < args.len() && args[i].starts_with('-') {
                // -e VAR=val / -u user take a value
                if matches!(args[i].as_str(), "-e" | "-u" | "-w" | "--env" | "--user" | "--workdir")
                {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            return args.get(i).map(String::as_str);
        }
        i += 1;
    }
    None
}

fn parse_ps_json_lines(text: &str) -> Result<Vec<ContainerInfo>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: DockerPsJson = serde_json::from_str(line)
            .with_context(|| format!("parse docker ps JSON: {line}"))?;
        let name = row
            .names
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        rows.push(ContainerInfo {
            id: row.id,
            name,
            image: row.image,
            status: row.status,
        });
    }
    Ok(rows)
}

/// Running containers via local `docker ps` (JSON lines). Empty Vec if none.
pub fn list_running_containers() -> Result<Vec<ContainerInfo>> {
    let program = docker_program();
    let output = crate::platform::new_command(program)
        .args(["ps", "--format", "{{json .}}"])
        .output()
        .with_context(|| {
            format!(
                "Could not run `{program}`. Is Docker Desktop / the Docker CLI installed and on PATH?"
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("docker ps failed");
        bail!("{detail}");
    }

    parse_ps_json_lines(&String::from_utf8_lossy(&output.stdout))
}

/// Running containers on a remote host via SSH `docker ps`.
pub fn list_running_containers_ssh(
    host: &str,
    port: u16,
    user: &str,
    auth: SshAuthMaterial,
) -> Result<Vec<ContainerInfo>> {
    let stdout = exec_once_blocking(
        host,
        port,
        user,
        auth,
        "docker ps --format '{{json .}}'",
    )
    .map_err(|err| {
        let msg = format!("{err:#}");
        if msg.to_lowercase().contains("docker")
            || msg.contains("not found")
            || msg.contains("No such file")
        {
            anyhow::anyhow!(
                "{msg}\nIs Docker installed on the remote host and available to this user?"
            )
        } else {
            err
        }
    })?;
    parse_ps_json_lines(&stdout)
}

/// Before spawning a Local profile that is `docker exec …`.
pub fn preflight_local_spawn(shell: &str, args: &[String]) -> Result<()> {
    if args.is_empty() || !is_docker_executable(shell) {
        return Ok(());
    }
    let Some(id) = container_id_from_args(args) else {
        return Ok(());
    };

    // Engine reachable?
    let mut info = crate::platform::new_command(shell);
    info.args(["info", "--format", "{{.ServerVersion}}"]);
    match info.output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr.trim();
            if msg.is_empty() {
                bail!(
                    "Docker engine is not running. Start Docker Desktop (or the daemon) and try again."
                );
            }
            bail!("{msg}");
        }
        Err(err) => {
            bail!("Could not run `{shell}`: {err}. Is Docker installed?");
        }
    }

    // Container still present / running?
    if !container_is_running(id)? {
        bail!("Container `{id}` is not running.");
    }
    Ok(())
}

/// Fast `docker inspect` running check (background thread only — not UI).
pub fn container_is_running(container_id: &str) -> Result<bool> {
    let id = container_id.trim();
    if id.is_empty() {
        return Ok(false);
    }
    let program = docker_program();
    let mut inspect = crate::platform::new_command(program);
    inspect.args(["inspect", "-f", "{{.State.Running}}", id]);
    let output = inspect
        .output()
        .with_context(|| format!("inspect container `{id}`"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let running = String::from_utf8_lossy(&output.stdout);
    Ok(running.trim().eq_ignore_ascii_case("true"))
}

/// Build a sidebar Profile for local `docker exec` (same shape as WSL Local profiles).
pub fn new_profile(name: impl Into<String>, container_id: &str) -> Profile {
    Profile {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        kind: exec_profile_kind(container_id),
        forwards: Vec::new(),
    }
}

/// Docker-over-SSH profile: same host credentials, PTY runs `docker exec -it`.
pub fn new_ssh_docker_profile(
    name: impl Into<String>,
    host: String,
    port: u16,
    user: String,
    auth: SshAuth,
    container_id: &str,
) -> Profile {
    Profile {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        kind: ProfileKind::Ssh {
            host,
            port,
            user,
            auth,
            docker_container: Some(container_id.trim().to_string()),
        },
        forwards: Vec::new(),
    }
}

/// Soft probe used by the picker before listing (optional fast fail).
#[allow(dead_code)]
pub fn docker_engine_ok() -> Result<()> {
    let program = docker_program();
    let output = crate::platform::new_command(program)
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .context("run docker version")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{}",
            if stderr.trim().is_empty() {
                "Docker engine is not available"
            } else {
                stderr.trim()
            }
        );
    }
    Ok(())
}

const MAX_DOCKER_PORTS: usize = 32;
const MAX_DOCKER_VOLUMES: usize = 24;

#[derive(Debug, Deserialize)]
struct InspectJson {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Config")]
    config: Option<InspectConfig>,
    #[serde(rename = "State")]
    state: Option<InspectState>,
    #[serde(rename = "NetworkSettings")]
    network: Option<InspectNetwork>,
    #[serde(rename = "Mounts", default)]
    mounts: Vec<InspectMount>,
}

#[derive(Debug, Deserialize)]
struct InspectConfig {
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "Hostname", default)]
    hostname: String,
}

#[derive(Debug, Deserialize)]
struct InspectState {
    #[serde(rename = "Status", default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct InspectNetwork {
    /// `null` binding = published but unbound / not mapped yet.
    #[serde(rename = "Ports", default)]
    ports: std::collections::BTreeMap<String, Option<Vec<InspectPortBinding>>>,
}

#[derive(Debug, Deserialize)]
struct InspectPortBinding {
    #[serde(rename = "HostIp", default)]
    host_ip: String,
    #[serde(rename = "HostPort", default)]
    host_port: String,
}

#[derive(Debug, Deserialize)]
struct InspectMount {
    #[serde(rename = "Type", default)]
    type_: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Source", default)]
    source: String,
    #[serde(rename = "Destination", default)]
    destination: String,
    #[serde(rename = "RW", default = "default_true")]
    rw: bool,
}

fn default_true() -> bool {
    true
}

/// Build an Info-tab snapshot from `docker inspect --format '{{json .}}'` output.
pub fn host_snapshot_from_inspect(
    json: &str,
    container_fallback: &str,
    via_ssh: bool,
) -> Result<crate::session::host_info::HostSnapshot> {
    use crate::session::host_info::{DockerPortMap, DockerVolumeMap, HostSnapshot};

    let raw = json.trim();
    // Some clients wrap as a one-element array.
    let inspect: InspectJson = if raw.starts_with('[') {
        let list: Vec<InspectJson> =
            serde_json::from_str(raw).context("parse docker inspect JSON array")?;
        list.into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("docker inspect returned an empty array"))?
    } else {
        serde_json::from_str(raw).context("parse docker inspect JSON")?
    };

    let name = inspect
        .name
        .trim()
        .trim_start_matches('/')
        .to_string();
    let name = if name.is_empty() {
        container_fallback.trim().to_string()
    } else {
        name
    };
    let image = inspect
        .config
        .as_ref()
        .map(|c| c.image.clone())
        .unwrap_or_default();
    let status = inspect
        .state
        .as_ref()
        .map(|s| s.status.clone())
        .unwrap_or_default();
    let hostname = inspect
        .config
        .as_ref()
        .map(|c| c.hostname.clone())
        .unwrap_or_default();
    let id = inspect.id.trim();
    let short_id = if id.len() >= 12 { &id[..12] } else { id };

    let mut docker_ports = Vec::new();
    if let Some(net) = inspect.network.as_ref() {
        for (container_port, bindings) in &net.ports {
            match bindings {
                None => {
                    docker_ports.push(DockerPortMap {
                        host: String::new(),
                        container: container_port.clone(),
                    });
                }
                Some(list) if list.is_empty() => {
                    docker_ports.push(DockerPortMap {
                        host: String::new(),
                        container: container_port.clone(),
                    });
                }
                Some(list) => {
                    for b in list {
                        let ip = if b.host_ip.is_empty() {
                            "0.0.0.0"
                        } else {
                            b.host_ip.as_str()
                        };
                        let host = if b.host_port.is_empty() {
                            String::new()
                        } else {
                            format!("{ip}:{}", b.host_port)
                        };
                        docker_ports.push(DockerPortMap {
                            host,
                            container: container_port.clone(),
                        });
                    }
                }
            }
            if docker_ports.len() >= MAX_DOCKER_PORTS {
                break;
            }
        }
        docker_ports.sort_by(|a, b| {
            a.container
                .cmp(&b.container)
                .then_with(|| a.host.cmp(&b.host))
        });
        docker_ports.truncate(MAX_DOCKER_PORTS);
    }

    let mut docker_volumes = Vec::new();
    for m in inspect.mounts {
        let kind = if m.type_.is_empty() {
            "mount".into()
        } else {
            m.type_.clone()
        };
        let source = if kind.eq_ignore_ascii_case("volume") && !m.name.is_empty() {
            m.name
        } else if kind.eq_ignore_ascii_case("tmpfs") {
            String::new()
        } else {
            m.source
        };
        if m.destination.is_empty() && source.is_empty() {
            continue;
        }
        docker_volumes.push(DockerVolumeMap {
            kind,
            source,
            destination: m.destination,
            read_only: !m.rw,
        });
        if docker_volumes.len() >= MAX_DOCKER_VOLUMES {
            break;
        }
    }

    let os = if via_ssh {
        format!("Docker · {status} (SSH host)")
    } else {
        format!("Docker · {status}")
    };
    let title = if hostname.is_empty() {
        name.clone()
    } else {
        hostname
    };

    Ok(HostSnapshot {
        hostname: title,
        os,
        kernel: if short_id.is_empty() {
            format!("container {container_fallback}")
        } else {
            format!("container {short_id}")
        },
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
        is_docker: true,
        docker_ports,
        docker_volumes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_container_id_from_exec_args() {
        let args = vec![
            "exec".into(),
            "-it".into(),
            "abc123deadbeef".into(),
            "sh".into(),
            "-c".into(),
            "exec bash".into(),
        ];
        assert_eq!(container_id_from_args(&args), Some("abc123deadbeef"));
    }

    #[test]
    fn is_docker_shell_name() {
        assert!(is_docker_executable("docker"));
        assert!(is_docker_executable("docker.exe"));
        assert!(!is_docker_executable("wsl.exe"));
    }

    #[test]
    fn remote_exec_command_quotes_inner() {
        let cmd = exec_remote_command("abc");
        assert!(cmd.starts_with("docker exec -it abc sh -c "));
        assert!(cmd.contains("bash"));
    }

    #[test]
    fn inspect_json_ports_and_mounts() {
        let json = r#"{
            "Id": "abcdef1234567890",
            "Name": "/web",
            "Config": {"Image": "nginx:latest", "Hostname": "web"},
            "State": {"Status": "running"},
            "NetworkSettings": {
                "Ports": {
                    "80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}],
                    "443/tcp": null
                }
            },
            "Mounts": [
                {"Type": "bind", "Source": "/data", "Destination": "/app", "RW": true},
                {"Type": "volume", "Name": "pgdata", "Source": "/var/lib/docker/volumes/pgdata/_data", "Destination": "/var/lib/postgresql/data", "RW": false}
            ]
        }"#;
        let snap = host_snapshot_from_inspect(json, "abcdef", false).unwrap();
        assert!(snap.is_docker);
        assert_eq!(snap.hostname, "web");
        assert_eq!(snap.cpu_model, "nginx:latest");
        assert_eq!(snap.docker_ports.len(), 2);
        let p80 = snap
            .docker_ports
            .iter()
            .find(|p| p.container == "80/tcp")
            .expect("80/tcp");
        assert_eq!(p80.host, "0.0.0.0:8080");
        let p443 = snap
            .docker_ports
            .iter()
            .find(|p| p.container == "443/tcp")
            .expect("443/tcp");
        assert!(p443.host.is_empty());
        assert_eq!(snap.docker_volumes.len(), 2);
        assert_eq!(snap.docker_volumes[0].kind, "bind");
        assert_eq!(snap.docker_volumes[0].source, "/data");
        assert_eq!(snap.docker_volumes[1].kind, "volume");
        assert_eq!(snap.docker_volumes[1].source, "pgdata");
        assert!(snap.docker_volumes[1].read_only);
    }
}
