//! Persisted SSH port-forward rules (Profile template). See docs/PORT_FORWARD.md.

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

    /// Fresh id for Profile duplicate.
    pub fn duplicate(&self) -> Self {
        Self {
            id: Uuid::new_v4(),
            ..self.clone()
        }
    }
}
