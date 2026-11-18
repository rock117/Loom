//! Persisted SFTP transfer history (survives quit / pane close).
//!
//! Keyed by a stable **scope** (`profile:{uuid}` or `ssh:user@host:port`), not pane id.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const HISTORY_VERSION: u32 = 1;
/// Cap finished rows kept per scope (newest first).
pub const MAX_PER_SCOPE: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHistoryFile {
    pub version: u32,
    /// scope → rows (newest first).
    #[serde(default)]
    pub by_scope: HashMap<String, Vec<PersistedTransfer>>,
}

impl Default for TransferHistoryFile {
    fn default() -> Self {
        Self {
            version: HISTORY_VERSION,
            by_scope: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistedTransferDir {
    Download,
    Upload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistedTransferStatus {
    Done { files: Option<u32> },
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTransfer {
    pub id: Uuid,
    pub label: String,
    pub direction: PersistedTransferDir,
    pub status: PersistedTransferStatus,
    pub local_path: Option<String>,
    pub remote_path: Option<String>,
    pub is_dir: bool,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub elapsed_ms: Option<u64>,
    /// Unix seconds when the job finished (sort / display).
    #[serde(default)]
    pub finished_at: Option<u64>,
}

impl TransferHistoryFile {
    pub fn trim(&mut self) {
        for rows in self.by_scope.values_mut() {
            if rows.len() > MAX_PER_SCOPE {
                rows.truncate(MAX_PER_SCOPE);
            }
        }
        self.by_scope.retain(|_, rows| !rows.is_empty());
    }
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable history key for an SSH (or Docker-over-SSH) pane.
pub fn scope_key_from_parts(
    profile_id: Option<Uuid>,
    user: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
) -> Option<String> {
    if let Some(id) = profile_id {
        return Some(format!("profile:{id}"));
    }
    let user = user.filter(|s| !s.is_empty())?;
    let host = host.filter(|s| !s.is_empty())?;
    let port = port.unwrap_or(22);
    Some(format!(
        "ssh:{}@{}:{}",
        user,
        host.trim().to_ascii_lowercase(),
        port
    ))
}
