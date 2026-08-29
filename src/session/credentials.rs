//! OS credential helpers for SSH passwords (Windows Credential Manager via `keyring`).

use anyhow::{Context, Result};
use keyring::Entry;
use uuid::Uuid;

fn entry(profile_id: Uuid) -> Result<Entry> {
    Entry::new("loom", &format!("ssh/{profile_id}"))
        .context("open credential entry")
}

pub fn get_password(profile_id: Uuid) -> Result<Option<String>> {
    match entry(profile_id)?.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).context("read stored SSH password"),
    }
}

pub fn set_password(profile_id: Uuid, password: &str) -> Result<()> {
    entry(profile_id)?
        .set_password(password)
        .context("store SSH password")
}

pub fn delete_password(profile_id: Uuid) -> Result<()> {
    match entry(profile_id)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context("delete SSH password"),
    }
}
