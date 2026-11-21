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

    /// Prefer **name** for `docker exec` / profile storage so recreate-with-same-name
    /// still connects; fall back to full id when unnamed.
    pub fn exec_ref(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            self.id.trim()
        } else {
            name
        }
    }
}

/// Result of matching a stored ref against a live `docker ps` list.
#[derive(Debug, Clone)]
pub struct ResolvedContainer {
    /// Value to pass to `docker exec` / store in the profile (name if any).
    pub exec_ref: String,
    pub id: String,
    pub name: String,
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

/// Match `query` (name, full id, or short id) against a `docker ps` list.
pub fn resolve_running_ref(query: &str, list: &[ContainerInfo]) -> Option<ResolvedContainer> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let row = list
        .iter()
        .find(|c| !c.name.is_empty() && c.name == q)
        .or_else(|| list.iter().find(|c| c.id == q))
        .or_else(|| {
            list.iter().find(|c| {
                let short = c.short_id();
                c.id.starts_with(q) || q == short || (q.len() >= 12 && short.starts_with(q))
            })
        })?;
    Some(ResolvedContainer {
        exec_ref: row.exec_ref().to_string(),
        id: row.id.clone(),
        name: row.name.clone(),
    })
}

/// Local: `docker ps` then resolve name → id (background thread only).
pub fn resolve_local_running(query: &str) -> Result<ResolvedContainer> {
    let list = list_running_containers()?;
    resolve_running_ref(query, &list).ok_or_else(|| {
        anyhow::anyhow!(
            "Container `{query}` is not running (or was recreated with a new id). Re-pick it from Docker."
        )
    })
}

/// Remote: `docker ps` over SSH then resolve (background thread only).
pub fn resolve_ssh_running(
    query: &str,
    host: &str,
    port: u16,
    user: &str,
    auth: SshAuthMaterial,
) -> Result<ResolvedContainer> {
    let list = list_running_containers_ssh(host, port, user, auth)?;
    resolve_running_ref(query, &list).ok_or_else(|| {
        anyhow::anyhow!(
            "Container `{query}` is not running on the SSH host (or was recreated). Re-pick it from Docker."
        )
    })
}

/// Rewrite `docker exec -it <ref> …` argv slot to `new_ref`.
pub fn set_exec_container_ref(args: &mut [String], new_ref: &str) -> bool {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "exec" {
            i += 1;
            while i < args.len() && args[i].starts_with('-') {
                if matches!(
                    args[i].as_str(),
                    "-e" | "-u" | "-w" | "--env" | "--user" | "--workdir"
                ) {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if let Some(slot) = args.get_mut(i) {
                *slot = new_ref.to_string();
                return true;
            }
            return false;
        }
        i += 1;
    }
    false
}

/// Ensure Docker engine is up; resolve container by name/id and rewrite `args`.
/// Call only from a background thread.
pub fn prepare_local_docker_exec(shell: &str, args: &mut Vec<String>) -> Result<Option<ResolvedContainer>> {
    if args.is_empty() || !is_docker_executable(shell) {
        return Ok(None);
    }
    let Some(query) = container_id_from_args(args).map(str::to_string) else {
        return Ok(None);
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

    let resolved = resolve_local_running(&query)?;
    set_exec_container_ref(args, &resolved.exec_ref);
    Ok(Some(resolved))
}

/// Before spawning a Local profile that is `docker exec …`.
pub fn preflight_local_spawn(shell: &str, args: &[String]) -> Result<()> {
    if args.is_empty() || !is_docker_executable(shell) {
        return Ok(());
    }
    let mut owned = args.to_vec();
    prepare_local_docker_exec(shell, &mut owned)?;
    Ok(())
}

/// Fast `docker inspect` running check (background thread only — not UI).
#[allow(dead_code)]
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
        layout: None,
        panes: None,
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
            cwd: None,
            jump: None,
        },
        forwards: Vec::new(),
        layout: None,
        panes: None,
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
const MAX_DOCKER_ENV: usize = 40;
const MAX_DOCKER_NETWORKS: usize = 8;

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
    #[serde(rename = "Env", default)]
    env: Vec<String>,
    #[serde(rename = "Cmd", default)]
    cmd: Option<serde_json::Value>,
    #[serde(rename = "Entrypoint", default)]
    entrypoint: Option<serde_json::Value>,
    #[serde(rename = "WorkingDir", default)]
    working_dir: String,
    #[serde(rename = "User", default)]
    user: String,
}

#[derive(Debug, Deserialize)]
struct InspectState {
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "StartedAt", default)]
    started_at: String,
    #[serde(rename = "RestartCount", default)]
    restart_count: u32,
}

#[derive(Debug, Deserialize)]
struct InspectNetwork {
    /// `null` binding = published but unbound / not mapped yet.
    #[serde(rename = "Ports", default)]
    ports: std::collections::BTreeMap<String, Option<Vec<InspectPortBinding>>>,
    #[serde(rename = "Networks", default)]
    networks: std::collections::BTreeMap<String, InspectEndpoint>,
}

#[derive(Debug, Deserialize)]
struct InspectEndpoint {
    #[serde(rename = "IPAddress", default)]
    ip: String,
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

fn json_string_list(v: &Option<serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        Some(serde_json::Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn join_cmd_parts(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| {
            if p.is_empty() || p.contains(char::is_whitespace) {
                format!("\"{p}\"")
            } else {
                p.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn uptime_from_started_at(started_at: &str) -> u64 {
    let s = started_at.trim();
    if s.is_empty() || s.starts_with("0001-01-01") {
        return 0;
    }
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) else {
        return 0;
    };
    let now = chrono::Utc::now();
    let secs = (now - dt.with_timezone(&chrono::Utc)).num_seconds();
    secs.max(0) as u64
}

/// Parse `docker stats --no-stream` one-liner into CPU % + mem used/limit bytes.
pub fn parse_stats_line(line: &str) -> Option<(f32, u64, u64)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (cpu_s, mem_s) = line
        .split_once('\t')
        .or_else(|| line.split_once("  "))
        .unwrap_or((line, ""));
    let cpu = cpu_s.trim().trim_end_matches('%').parse::<f32>().ok()?;
    let mem_s = mem_s.trim();
    if mem_s.is_empty() {
        return Some((cpu, 0, 0));
    }
    let (used_s, total_s) = mem_s.split_once('/')?;
    let used = parse_docker_size(used_s.trim())?;
    let total = parse_docker_size(total_s.trim())?;
    Some((cpu, used, total))
}

fn parse_docker_size(s: &str) -> Option<u64> {
    let s = s.trim().replace(' ', "");
    if s.is_empty() {
        return None;
    }
    let (num_s, unit) = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(i, _)| (&s[..i], s[i..].to_ascii_lowercase()))
        .unwrap_or((s.as_str(), String::new()));
    let n: f64 = num_s.parse().ok()?;
    let mult = match unit.as_str() {
        "" | "b" => 1.0,
        "kb" | "kib" => 1024.0,
        "mb" | "mib" => 1024.0 * 1024.0,
        "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// Apply a `docker stats` sample onto an existing container snapshot (best-effort).
pub fn apply_stats_sample(snap: &mut crate::session::host_info::HostSnapshot, line: &str) {
    if let Some((cpu, used, total)) = parse_stats_line(line) {
        snap.cpu_usage_pct = Some(cpu);
        if total > 0 {
            snap.mem_used = used;
            snap.mem_total = total;
        }
    }
}

/// Build an Info-tab snapshot from `docker inspect --format '{{json .}}'` output.
pub fn host_snapshot_from_inspect(
    json: &str,
    container_fallback: &str,
    via_ssh: bool,
) -> Result<crate::session::host_info::HostSnapshot> {
    use crate::session::host_info::{
        DockerNetworkMap, DockerPortMap, DockerVolumeMap, HostSnapshot,
    };

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
    let config = inspect.config.as_ref();
    let image = config.map(|c| c.image.clone()).unwrap_or_default();
    let hostname = config.map(|c| c.hostname.clone()).unwrap_or_default();
    let workdir = config.map(|c| c.working_dir.clone()).unwrap_or_default();
    let user = config.map(|c| c.user.clone()).unwrap_or_default();
    let mut env = config.map(|c| c.env.clone()).unwrap_or_default();
    env.truncate(MAX_DOCKER_ENV);
    let entrypoint_v = config.and_then(|c| c.entrypoint.clone());
    let cmd_v = config.and_then(|c| c.cmd.clone());
    let docker_command = {
        let mut parts = json_string_list(&entrypoint_v);
        parts.extend(json_string_list(&cmd_v));
        join_cmd_parts(&parts)
    };

    let state = inspect.state.as_ref();
    let status = state.map(|s| s.status.clone()).unwrap_or_default();
    let restart_count = state.map(|s| s.restart_count).unwrap_or(0);
    let uptime_secs = state
        .map(|s| uptime_from_started_at(&s.started_at))
        .unwrap_or(0);

    let id = inspect.id.trim();
    let short_id = if id.len() >= 12 { &id[..12] } else { id };

    let mut docker_ports = Vec::new();
    let mut docker_networks = Vec::new();
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

        for (net_name, ep) in &net.networks {
            let ip = ep.ip.trim().to_string();
            if net_name.is_empty() && ip.is_empty() {
                continue;
            }
            docker_networks.push(DockerNetworkMap {
                name: net_name.clone(),
                ip,
            });
            if docker_networks.len() >= MAX_DOCKER_NETWORKS {
                break;
            }
        }
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
        uptime_secs,
        is_docker: true,
        docker_ports,
        docker_volumes,
        docker_status: status,
        docker_networks,
        docker_env: env,
        docker_command,
        docker_workdir: workdir,
        docker_user: user,
        docker_restart_count: restart_count,
    })
}

/// Map a container path to a host path using bind mounts (longest destination wins).
///
/// Named volumes / tmpfs are skipped — `source` is not a useful host filesystem path
/// for “Copy host path”. Local Docker and Docker-over-SSH both use the same mapping
/// (SSH: path is on the remote Docker host).
pub fn host_path_for_container_path(
    container_path: &str,
    mounts: &[crate::session::host_info::DockerVolumeMap],
) -> Option<String> {
    let cpath = normalize_container_mount_path(container_path);
    if cpath.is_empty() {
        return None;
    }

    let mut best: Option<(&crate::session::host_info::DockerVolumeMap, usize)> = None;
    for mount in mounts {
        if !is_mappable_host_bind(mount) {
            continue;
        }
        let dest = normalize_container_mount_path(&mount.destination);
        if dest.is_empty() {
            continue;
        }
        let matched = if dest == "/" {
            cpath.starts_with('/')
        } else {
            cpath == dest || cpath.starts_with(&(dest.clone() + "/"))
        };
        if !matched {
            continue;
        }
        let len = dest.len();
        if best.is_none_or(|(_, best_len)| len >= best_len) {
            best = Some((mount, len));
        }
    }

    let (mount, dest_len) = best?;
    let rest = if cpath.len() <= dest_len {
        ""
    } else {
        cpath[dest_len..].trim_start_matches('/')
    };
    Some(join_host_mount_path(&mount.source, rest))
}

fn is_mappable_host_bind(mount: &crate::session::host_info::DockerVolumeMap) -> bool {
    let source = mount.source.trim();
    let dest = mount.destination.trim();
    if source.is_empty() || dest.is_empty() {
        return false;
    }
    if mount.kind.eq_ignore_ascii_case("tmpfs") || mount.kind.eq_ignore_ascii_case("volume") {
        return false;
    }
    // Prefer explicit bind; also accept path-like sources when Type is missing/other.
    mount.kind.eq_ignore_ascii_case("bind") || looks_like_host_fs_path(source)
}

fn looks_like_host_fs_path(source: &str) -> bool {
    let s = source.trim();
    if s.starts_with('/') || s.starts_with('\\') {
        return true;
    }
    // Windows `C:\…` / `C:/…`
    let bytes = s.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn normalize_container_mount_path(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() {
        return String::new();
    }
    // Reuse Files normalization (collapse `..`, ensure absolute).
    crate::session::docker_fs::normalize_path(p)
}

fn join_host_mount_path(source: &str, rest: &str) -> String {
    let source = source.trim();
    if rest.is_empty() {
        return source.to_string();
    }
    let windows = source.contains('\\')
        || (source.len() >= 2 && source.as_bytes()[1] == b':');
    if windows {
        let base = source.trim_end_matches(['\\', '/']);
        format!("{base}\\{}", rest.replace('/', "\\"))
    } else {
        let base = source.trim_end_matches('/');
        format!("{base}/{rest}")
    }
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
            "Config": {
                "Image": "nginx:latest",
                "Hostname": "web",
                "Env": ["PATH=/usr/bin", "NGINX=1"],
                "Cmd": ["nginx", "-g", "daemon off;"],
                "Entrypoint": ["/docker-entrypoint.sh"],
                "WorkingDir": "/usr/share/nginx/html",
                "User": "nginx"
            },
            "State": {"Status": "running", "StartedAt": "2020-01-01T00:00:00Z", "RestartCount": 2},
            "NetworkSettings": {
                "Ports": {
                    "80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}],
                    "443/tcp": null
                },
                "Networks": {
                    "bridge": {"IPAddress": "172.17.0.2"}
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
        assert_eq!(snap.docker_status, "running");
        assert!(snap.uptime_secs > 0);
        assert_eq!(snap.docker_restart_count, 2);
        assert_eq!(snap.docker_networks.len(), 1);
        assert_eq!(snap.docker_networks[0].name, "bridge");
        assert_eq!(snap.docker_networks[0].ip, "172.17.0.2");
        assert_eq!(snap.docker_env.len(), 2);
        assert!(snap.docker_command.contains("nginx"));
        assert_eq!(snap.docker_workdir, "/usr/share/nginx/html");
        assert_eq!(snap.docker_user, "nginx");
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

    #[test]
    fn host_path_maps_bind_longest_prefix() {
        use crate::session::host_info::DockerVolumeMap;
        let mounts = vec![
            DockerVolumeMap {
                kind: "bind".into(),
                source: r"C:\work".into(),
                destination: "/app".into(),
                read_only: false,
            },
            DockerVolumeMap {
                kind: "bind".into(),
                source: r"C:\work\src".into(),
                destination: "/app/src".into(),
                read_only: false,
            },
            DockerVolumeMap {
                kind: "volume".into(),
                source: "pgdata".into(),
                destination: "/data".into(),
                read_only: false,
            },
        ];
        assert_eq!(
            host_path_for_container_path("/app/src/main.rs", &mounts).as_deref(),
            Some(r"C:\work\src\main.rs")
        );
        assert_eq!(
            host_path_for_container_path("/app/README.md", &mounts).as_deref(),
            Some(r"C:\work\README.md")
        );
        assert_eq!(host_path_for_container_path("/data/x", &mounts), None);
        assert_eq!(host_path_for_container_path("/other", &mounts), None);
    }

    #[test]
    fn parse_stats_mem_and_cpu() {
        let (cpu, used, total) =
            parse_stats_line("1.23%\t41.02MiB / 7.653GiB").expect("stats line");
        assert!((cpu - 1.23).abs() < 0.01);
        assert!(used > 40 * 1024 * 1024);
        assert!(total > 7 * 1024 * 1024 * 1024);
    }

    #[test]
    fn resolve_prefers_name_over_stale_id() {
        let list = vec![ContainerInfo {
            id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            name: "web".into(),
            image: "nginx".into(),
            status: "Up".into(),
        }];
        let by_name = resolve_running_ref("web", &list).unwrap();
        assert_eq!(by_name.exec_ref, "web");
        let by_old_id = resolve_running_ref(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            &list,
        );
        assert!(by_old_id.is_none());
        let by_short = resolve_running_ref("aaaaaaaaaaaa", &list).unwrap();
        assert_eq!(by_short.exec_ref, "web");
    }

    #[test]
    fn set_exec_ref_rewrites_argv() {
        let mut args = vec![
            "exec".into(),
            "-it".into(),
            "oldid".into(),
            "sh".into(),
            "-c".into(),
            "exec bash".into(),
        ];
        assert!(set_exec_container_ref(&mut args, "web"));
        assert_eq!(args[2], "web");
    }
}
