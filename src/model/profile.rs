use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::forward::PortForwardRule;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SshAuth {
    /// Password from OS credential store when `remember` is true.
    Password {
        #[serde(default = "default_true")]
        remember: bool,
    },
    PrivateKey {
        path: PathBuf,
    },
}

fn default_true() -> bool {
    true
}

/// One leaf in a saved Tab snapshot (see `docs/PROFILE_TAB_LAYOUT.md`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PaneSnapshot {
    /// Working directory for this pane (host or remote path). Empty → use profile default cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Persisted split tree: structure only — ratios are always 0.5 on restore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SavedPaneLayout {
    Leaf {
        /// Index into [`Profile::panes`].
        pane: u32,
    },
    Split {
        axis: SavedSplitAxis,
        first: Box<SavedPaneLayout>,
        second: Box<SavedPaneLayout>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SavedSplitAxis {
    /// Left | Right
    Horizontal,
    /// Top / Bottom
    Vertical,
}

impl SavedPaneLayout {
    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProfileKind {
    Local {
        shell: Option<String>,
        cwd: Option<PathBuf>,
        #[serde(default)]
        env: Vec<(String, String)>,
        /// Extra argv after `shell` (e.g. WSL: `["-d", "Ubuntu"]`).
        #[serde(default)]
        args: Vec<String>,
    },
    Ssh {
        host: String,
        port: u16,
        user: String,
        auth: SshAuth,
        /// When set, the session PTY runs `docker exec -it` into this container
        /// on the SSH host (Docker-over-SSH profiles). Older workspace files omit it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        docker_container: Option<String>,
        /// Default remote (or container) cwd when no per-pane snapshot cwd is set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
}

impl ProfileKind {
    pub fn local_default() -> Self {
        Self::Local {
            shell: None,
            cwd: None,
            env: Vec::new(),
            args: Vec::new(),
        }
    }

    /// Profile-level default cwd (host path for Local, remote string for SSH).
    pub fn common_cwd(&self) -> Option<String> {
        match self {
            Self::Local { cwd, .. } => cwd.as_ref().map(|p| p.display().to_string()),
            Self::Ssh { cwd, .. } => cwd.clone(),
        }
    }

    pub fn set_common_cwd(&mut self, cwd: Option<String>) {
        match self {
            Self::Local { cwd: stored, .. } => {
                *stored = cwd.filter(|s| !s.trim().is_empty()).map(PathBuf::from);
            }
            Self::Ssh { cwd: stored, .. } => {
                *stored = cwd.filter(|s| !s.trim().is_empty());
            }
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Local {
                shell, cwd, args, ..
            } => {
                if self.is_docker_local() {
                    // Prefer container id after `exec` (+ flags); fall back to generic label.
                    let id = args
                        .iter()
                        .position(|a| a == "exec")
                        .and_then(|i| {
                            args[i + 1..]
                                .iter()
                                .find(|a| !a.starts_with('-'))
                                .map(|s| s.as_str())
                        })
                        .unwrap_or("container");
                    let short = if id.len() > 12 { &id[..12] } else { id };
                    return format!("docker · {short}");
                }
                let shell = shell.as_deref().unwrap_or("default shell");
                let argv = if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", args.join(" "))
                };
                match cwd {
                    Some(c) => format!("local · {shell}{argv} · {}", c.display()),
                    None => format!("local · {shell}{argv}"),
                }
            }
            Self::Ssh {
                host,
                port,
                user,
                docker_container,
                ..
            } => {
                if let Some(id) = docker_container.as_deref() {
                    let id = id.trim();
                    let short = if id.len() > 12 { &id[..12] } else { id };
                    format!("docker · {short} @ {user}@{host}:{port}")
                } else {
                    format!("ssh · {user}@{host}:{port}")
                }
            }
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    /// Profiles with argv (e.g. WSL) spawn off the UI thread and preflight checks.
    pub fn local_has_args(&self) -> bool {
        matches!(self, Self::Local { args, .. } if !args.is_empty())
    }

    pub fn is_wsl_local(&self) -> bool {
        match self {
            Self::Local { shell, args, .. } => {
                is_wsl_shell(shell.as_deref().unwrap_or(""))
                    || args.iter().any(|a| a == "-d" || a.starts_with("--distribution"))
            }
            _ => false,
        }
    }

    /// Local profile that runs `docker exec …` (ephemeral Docker tabs).
    pub fn is_docker_local(&self) -> bool {
        match self {
            Self::Local { shell, args, .. } => {
                is_docker_shell(shell.as_deref().unwrap_or(""))
                    && args.iter().any(|a| a == "exec")
            }
            _ => false,
        }
    }

    /// SSH profile that opens `docker exec` on the remote host.
    pub fn is_docker_ssh(&self) -> bool {
        matches!(
            self,
            Self::Ssh {
                docker_container: Some(id),
                ..
            } if !id.trim().is_empty()
        )
    }

    pub fn is_docker(&self) -> bool {
        self.is_docker_local() || self.is_docker_ssh()
    }

    /// Container id for Docker-over-SSH profiles.
    pub fn docker_ssh_container_id(&self) -> Option<&str> {
        match self {
            Self::Ssh {
                docker_container: Some(id),
                ..
            } if !id.trim().is_empty() => Some(id.trim()),
            _ => None,
        }
    }

    /// WSL profiles are Windows-only in the UI (still persist in workspace.json).
    pub fn visible_in_sidebar(&self) -> bool {
        if self.is_wsl_local() {
            cfg!(windows)
        } else {
            true
        }
    }
}

fn is_wsl_shell(shell: &str) -> bool {
    PathBuf::from(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("wsl"))
}

fn is_docker_shell(shell: &str) -> bool {
    PathBuf::from(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("docker"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    /// Connection settings (product name: connection).
    pub kind: ProfileKind,
    /// SSH Local forwards (ignored for Local profiles). Empty by default for older files.
    #[serde(default)]
    pub forwards: Vec<PortForwardRule>,
    /// Saved tab split tree; paired with [`Self::panes`]. See PROFILE_TAB_LAYOUT.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<SavedPaneLayout>,
    /// Saved tab pane snapshots; paired with [`Self::layout`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<PaneSnapshot>>,
}

impl Profile {
    pub fn new_local(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: ProfileKind::local_default(),
            forwards: Vec::new(),
            layout: None,
            panes: None,
        }
    }

    /// Local profile that launches `wsl.exe -d <distro>`.
    pub fn new_wsl(name: impl Into<String>, distro: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: ProfileKind::Local {
                shell: Some("wsl.exe".into()),
                cwd: None,
                env: Vec::new(),
                args: vec!["-d".into(), distro.to_string()],
            },
            forwards: Vec::new(),
            layout: None,
            panes: None,
        }
    }

    pub fn new_ssh(name: impl Into<String>, host: String, port: u16, user: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: ProfileKind::Ssh {
                host,
                port,
                user,
                auth: SshAuth::Password { remember: true },
                docker_container: None,
                cwd: None,
            },
            forwards: Vec::new(),
            layout: None,
            panes: None,
        }
    }

    pub fn duplicate(&self) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: format!("{} (copy)", self.name),
            kind: self.kind.clone(),
            forwards: self.forwards.iter().map(|f| f.duplicate()).collect(),
            layout: self.layout.clone(),
            panes: self.panes.clone(),
        }
    }

    pub fn ssh_forwards(&self) -> &[PortForwardRule] {
        if self.kind.is_local() {
            &[]
        } else {
            &self.forwards
        }
    }

    /// Valid saved tab snapshot, if present.
    pub fn saved_tab(&self) -> Option<(&SavedPaneLayout, &[PaneSnapshot])> {
        let layout = self.layout.as_ref()?;
        let panes = self.panes.as_deref()?;
        if panes.is_empty() || layout.leaf_count() != panes.len() {
            return None;
        }
        Some((layout, panes))
    }

    /// Pane count for settings UI (1 when no snapshot).
    pub fn saved_pane_count(&self) -> usize {
        self.saved_tab()
            .map(|(_, panes)| panes.len())
            .unwrap_or(1)
    }

    pub fn clear_saved_tab(&mut self) {
        self.layout = None;
        self.panes = None;
    }

    /// Store a tab snapshot. Single pane → clear layout/panes (caller updates common cwd).
    pub fn set_saved_tab(&mut self, layout: SavedPaneLayout, panes: Vec<PaneSnapshot>) {
        if panes.len() <= 1 {
            self.clear_saved_tab();
            return;
        }
        if layout.leaf_count() != panes.len() {
            self.clear_saved_tab();
            return;
        }
        self.layout = Some(layout);
        self.panes = Some(panes);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Failed => "failed",
        }
    }
}
