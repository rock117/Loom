//! Local Docker helpers: list running containers and build `docker exec` spawns.
//!
//! Listing / preflight must not run on the UI thread (see `docs/HARD_PROBLEMS.md`).
//! Windows: use [`crate::platform::new_command`] so `docker.exe` does not flash a console.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

use crate::model::ProfileKind;

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
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Status")]
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

/// `ProfileKind::Local` that runs interactive exec (bash if present, else sh).
pub fn exec_profile_kind(container_id: &str) -> ProfileKind {
    let id = container_id.trim().to_string();
    ProfileKind::Local {
        shell: Some(docker_program().into()),
        cwd: None,
        env: Vec::new(),
        args: vec![
            "exec".into(),
            "-it".into(),
            id,
            "sh".into(),
            "-c".into(),
            "command -v bash >/dev/null 2>&1 && exec bash || exec sh".into(),
        ],
    }
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

/// Running containers via `docker ps` (JSON lines). Empty Vec if none.
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

    let text = String::from_utf8_lossy(&output.stdout);
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
    let mut inspect = crate::platform::new_command(shell);
    inspect.args(["inspect", "-f", "{{.State.Running}}", id]);
    let output = inspect
        .output()
        .with_context(|| format!("inspect container `{id}`"))?;
    if !output.status.success() {
        bail!("Container `{id}` not found. Refresh the list and try again.");
    }
    let running = String::from_utf8_lossy(&output.stdout);
    if !running.trim().eq_ignore_ascii_case("true") {
        bail!("Container `{id}` is not running.");
    }
    Ok(())
}

/// Build a sidebar Profile for local `docker exec` (same shape as WSL Local profiles).
pub fn new_profile(name: impl Into<String>, container_id: &str) -> crate::model::Profile {
    crate::model::Profile {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        kind: exec_profile_kind(container_id),
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
}
