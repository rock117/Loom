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
                host, port, user, ..
            } => format!("ssh · {user}@{host}:{port}"),
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
    pub kind: ProfileKind,
    /// SSH Local forwards (ignored for Local profiles). Empty by default for older files.
    #[serde(default)]
    pub forwards: Vec<PortForwardRule>,
}

impl Profile {
    pub fn new_local(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: ProfileKind::local_default(),
            forwards: Vec::new(),
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
            },
            forwards: Vec::new(),
        }
    }

    pub fn duplicate(&self) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: format!("{} (copy)", self.name),
            kind: self.kind.clone(),
            forwards: self.forwards.iter().map(|f| f.duplicate()).collect(),
        }
    }

    pub fn ssh_forwards(&self) -> &[PortForwardRule] {
        if self.kind.is_local() {
            &[]
        } else {
            &self.forwards
        }
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
