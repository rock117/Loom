//! Trust-on-first-use host key store (`known_hosts.json`).

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::shared::paths;

#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownHostsFile {
    version: u32,
    /// host:port → base64-ish fingerprint string from russh
    hosts: HashMap<String, String>,
}

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

/// Verify server key fingerprint. Unknown hosts are stored (TOFU). Mismatch fails.
pub fn check_and_record(host: &str, port: u16, fingerprint: &str) -> Result<()> {
    let key = host_key(host, port);
    let mut file = load();
    match file.hosts.get(&key) {
        Some(known) if known == fingerprint => Ok(()),
        Some(_) => bail!(
            "HOST KEY CHANGED for {key}\n\
             Expected a different key. If the server was reinstalled, remove the entry from:\n\
             {}",
            paths::known_hosts_path().display()
        ),
        None => {
            file.hosts.insert(key, fingerprint.to_string());
            save(&file)?;
            Ok(())
        }
    }
}
