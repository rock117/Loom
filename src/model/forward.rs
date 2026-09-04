//! Persisted SSH port-forward rules (Profile template). See docs/PORT_FORWARD.md.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Forward mode. Stage 1 only exposes Local in the UI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortForwardKind {
    #[default]
    Local,
    // Remote / Socks reserved for later stages.
}

/// One port-forward rule stored on an SSH Profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortForwardRule {
    pub id: Uuid,
    #[serde(default)]
    pub kind: PortForwardKind,
    /// Local listen host (default `127.0.0.1`).
    pub bind_host: String,
    pub bind_port: u16,
    /// Host as seen from the SSH server (e.g. `127.0.0.1` = remote loopback).
    pub target_host: String,
    pub target_port: u16,
    /// Optional display name (e.g. `Postgres`).
    #[serde(default)]
    pub name: String,
    /// Auto-start when the SSH session connects.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl PortForwardRule {
    pub fn new_local(
        bind_port: u16,
        target_host: impl Into<String>,
        target_port: u16,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: PortForwardKind::Local,
            bind_host: "127.0.0.1".into(),
            bind_port,
            target_host: target_host.into(),
            target_port,
            name: String::new(),
            enabled: true,
        }
    }

    pub fn label(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            format!(
                "{}:{} → {}:{}",
                self.bind_host, self.bind_port, self.target_host, self.target_port
            )
        } else {
            name.to_string()
        }
    }

    pub fn endpoint_line(&self) -> String {
        format!(
            "{}:{} → {}:{}",
            self.bind_host, self.bind_port, self.target_host, self.target_port
        )
    }

    /// OpenSSH forward flag only (`-L host:port:host:port`, later `-R` / `-D`).
    pub fn open_ssh_flag(&self) -> String {
        open_ssh_forward_flag(
            self.kind,
            &self.bind_host,
            self.bind_port,
            &self.target_host,
            self.target_port,
        )
    }

    /// Fresh id for Profile duplicate.
    pub fn duplicate(&self) -> Self {
        Self {
            id: Uuid::new_v4(),
            ..self.clone()
        }
    }
}

/// OpenSSH `-L` / `-R` / `-D` flag for one forward.
pub fn open_ssh_forward_flag(
    kind: PortForwardKind,
    bind_host: &str,
    bind_port: u16,
    target_host: &str,
    target_port: u16,
) -> String {
    let bind = bind_host.trim();
    match kind {
        PortForwardKind::Local => format!(
            "-L {bind}:{bind_port}:{}:{target_port}",
            target_host.trim()
        ),
        // Remote / Socks reserved — keep generating sensible flags when kinds land.
    }
}

/// Quote a path/arg for paste into a typical shell (spaces / quotes).
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`' | '!'))
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Full `ssh … user@host` line ready to paste into another terminal.
pub fn format_open_ssh_command(
    user: &str,
    host: &str,
    port: u16,
    identity_file: Option<&Path>,
    forward_flags: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let mut parts: Vec<String> = vec!["ssh".into()];
    for flag in forward_flags {
        let f = flag.as_ref().trim();
        if !f.is_empty() {
            parts.push(f.to_string());
        }
    }
    if let Some(path) = identity_file {
        parts.push("-i".into());
        parts.push(shell_quote(&path.display().to_string()));
    }
    if port != 22 {
        parts.push("-p".into());
        parts.push(port.to_string());
    }
    let user = user.trim();
    let host = host.trim();
    if user.is_empty() {
        parts.push(host.to_string());
    } else {
        parts.push(format!("{user}@{host}"));
    }
    parts.join(" ")
}
