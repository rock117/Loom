//! Trust-on-first-use host key store (`known_hosts.json`).

use std::collections::HashMap;
use std::fmt;
use std::fs;

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::shared::paths;

/// Shared by [`check_and_record`] and [`trust_host`] (one JSON file, read-modify-write).
static KNOWN_HOSTS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownHostsFile {
    version: u32,
    /// host:port → fingerprint string from russh (`SHA256:…`)
    hosts: HashMap<String, String>,
}

/// Offered host key does not match the previously trusted fingerprint.
#[derive(Debug, Clone)]
pub struct HostKeyMismatch {
    pub host: String,
    pub port: u16,
    pub expected: String,
    pub offered: String,
}

impl HostKeyMismatch {
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl fmt::Display for HostKeyMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Host key changed for {} (trust the new key or remove the entry)",
            self.endpoint()
        )
    }
}

impl std::error::Error for HostKeyMismatch {}

fn host_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

fn load() -> KnownHostsFile {
    let path = paths::known_hosts_path();
    if !path.exists() {
        return KnownHostsFile {
            version: 1,
            hosts: HashMap::new(),
        };
    }
    match fs::read_to_string(&path).and_then(|raw| {
        serde_json::from_str(&raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("loom: known_hosts load failed ({err}); starting empty");
            KnownHostsFile {
                version: 1,
                hosts: HashMap::new(),
            }
        }
    }
}

fn save(file: &KnownHostsFile) -> Result<()> {
    let path = paths::known_hosts_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(file)?;
    fs::write(&path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Verify server key fingerprint. Unknown hosts are stored (TOFU). Mismatch fails with
/// [`HostKeyMismatch`] (downcast from the returned `anyhow::Error`).
///
/// Serialized: every SSH connection (shell, transfer, forward probe) verifies its host key
/// from its own thread, and this is a read-modify-write of one JSON file.
pub fn check_and_record(host: &str, port: u16, fingerprint: &str) -> Result<()> {
    let _guard = KNOWN_HOSTS_LOCK.lock();
    let key = host_key(host, port);
    let mut file = load();
    match file.hosts.get(&key) {
        Some(known) if known == fingerprint => Ok(()),
        Some(known) => Err(anyhow::Error::new(HostKeyMismatch {
            host: host.to_string(),
            port,
            expected: known.clone(),
            offered: fingerprint.to_string(),
        })),
        None => {
            file.hosts.insert(key, fingerprint.to_string());
            save(&file)?;
            Ok(())
        }
    }
}

/// Replace or insert the trusted fingerprint for `host:port` (after user confirmation).
pub fn trust_host(host: &str, port: u16, fingerprint: &str) -> Result<()> {
    let _guard = KNOWN_HOSTS_LOCK.lock();
    let key = host_key(host, port);
    let fp = fingerprint.trim();
    if fp.is_empty() {
        bail!("empty host key fingerprint");
    }
    let mut file = load();
    file.hosts.insert(key, fp.to_string());
    save(&file)
}
