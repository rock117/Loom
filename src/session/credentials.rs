//! OS credential helpers for SSH passwords.
//!
//! On Windows this uses Credential Manager (`keyring` feature `windows-native`).
//! Without that feature, keyring falls back to an in-memory mock that does not
//! persist across `Entry` instances — passwords would appear to never stick.

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

/// True when this profile needs an interactive password prompt.
pub fn needs_password_prompt(profile_id: Uuid) -> bool {
    match get_password(profile_id) {
        Ok(None) => true,
        Ok(Some(_)) => false,
        Err(err) => {
            eprintln!("loom: keyring read failed ({err:#}); prompting for password");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_persists_across_entry_handles() {
        let id = Uuid::new_v4();
        set_password(id, "secret-test").expect("set");
        assert_eq!(get_password(id).unwrap().as_deref(), Some("secret-test"));
        // Second get constructs a new Entry — must still hit the OS store.
        assert_eq!(get_password(id).unwrap().as_deref(), Some("secret-test"));
        delete_password(id).unwrap();
        assert_eq!(get_password(id).unwrap(), None);
    }
}
