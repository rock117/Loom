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
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Local { shell, cwd, .. } => {
                let shell = shell.as_deref().unwrap_or("default shell");
                match cwd {
                    Some(c) => format!("local · {shell} · {}", c.display()),
                    None => format!("local · {shell}"),
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
