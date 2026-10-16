//! OS credential helpers for SSH passwords.
//!
//! On Windows this uses Credential Manager (`keyring` feature `windows-native`).
//! Without that feature, keyring falls back to an in-memory mock that does not
//! persist across `Entry` instances — passwords would appear to never stick.

use anyhow::{Context, Result};
use keyring::Entry;
use uuid::Uuid;

fn entry(profile_id: Uuid) -> Result<Entry> {
    // Windows Cred Manager target ≈ `{user}.{service}` — keep target user names
    // free of nested path prefixes so jump cannot collide with target.
    Entry::new("loom", &format!("ssh/{profile_id}"))
        .context("open credential entry")
}

fn jump_entry(profile_id: Uuid) -> Result<Entry> {
    Entry::new("loom", &format!("ssh-jump/{profile_id}"))
        .context("open jump credential entry")
}

/// Pre-fix key `ssh/{id}/jump` (same prefix as target on Windows; could clobber).
fn jump_entry_legacy(profile_id: Uuid) -> Result<Entry> {
    Entry::new("loom", &format!("ssh/{profile_id}/jump"))
        .context("open legacy jump credential entry")
}

fn read_password(entry: Entry, what: &str) -> Result<Option<String>> {
    match entry.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).context(format!("read stored {what} password")),
    }
}

fn delete_entry(entry: Entry, what: &str) -> Result<()> {
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context(format!("delete {what} password")),
    }
}

pub fn get_password(profile_id: Uuid) -> Result<Option<String>> {
    read_password(entry(profile_id)?, "SSH")
}

pub fn set_password(profile_id: Uuid, password: &str) -> Result<()> {
    entry(profile_id)?
        .set_password(password)
        .context("store SSH password")?;
    // Read-back: catch Windows store quirks before the UI continues.
    match get_password(profile_id)? {
        Some(p) if p == password => Ok(()),
        Some(_) => anyhow::bail!("stored SSH password did not match after write"),
        None => anyhow::bail!("stored SSH password missing after write"),
    }
}

pub fn delete_password(profile_id: Uuid) -> Result<()> {
    delete_entry(entry(profile_id)?, "SSH")
}

pub fn get_jump_password(profile_id: Uuid) -> Result<Option<String>> {
    if let Some(p) = read_password(jump_entry(profile_id)?, "jump")? {
        return Ok(Some(p));
    }
    // Migrate legacy nested key if present.
    let legacy = read_password(jump_entry_legacy(profile_id)?, "legacy jump")?;
    if let Some(p) = legacy {
        let _ = set_jump_password(profile_id, &p);
        let _ = delete_entry(jump_entry_legacy(profile_id)?, "legacy jump");
        return Ok(Some(p));
    }
    Ok(None)
}

pub fn set_jump_password(profile_id: Uuid, password: &str) -> Result<()> {
    jump_entry(profile_id)?
        .set_password(password)
        .context("store jump SSH password")?;
    let _ = delete_entry(jump_entry_legacy(profile_id)?, "legacy jump");
    match read_password(jump_entry(profile_id)?, "jump")? {
        Some(p) if p == password => Ok(()),
        Some(_) => anyhow::bail!("stored jump password did not match after write"),
        None => anyhow::bail!("stored jump password missing after write"),
    }
}

pub fn delete_jump_password(profile_id: Uuid) -> Result<()> {
    delete_entry(jump_entry(profile_id)?, "jump")?;
    delete_entry(jump_entry_legacy(profile_id)?, "legacy jump")
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

pub fn needs_jump_password_prompt(profile_id: Uuid) -> bool {
    match get_jump_password(profile_id) {
        Ok(None) => true,
        Ok(Some(_)) => false,
        Err(err) => {
            eprintln!("loom: jump keyring read failed ({err:#}); prompting for password");
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

    #[test]
    fn target_and_jump_passwords_do_not_clobber() {
        let id = Uuid::new_v4();
        // Form order: jump first, target last.
        set_jump_password(id, "jump-secret").expect("set jump");
        set_password(id, "target-secret").expect("set target");
        assert_eq!(
            get_password(id).unwrap().as_deref(),
            Some("target-secret"),
            "target must survive after jump set"
        );
        assert_eq!(
            get_jump_password(id).unwrap().as_deref(),
            Some("jump-secret")
        );
        // Opposite order still OK with distinct prefixes.
        set_password(id, "target-2").unwrap();
        set_jump_password(id, "jump-2").unwrap();
        assert_eq!(
            get_password(id).unwrap().as_deref(),
            Some("target-2"),
            "target must survive when jump is written after"
        );
        assert_eq!(get_jump_password(id).unwrap().as_deref(), Some("jump-2"));
        delete_password(id).unwrap();
        delete_jump_password(id).unwrap();
        assert_eq!(get_password(id).unwrap(), None);
        assert_eq!(get_jump_password(id).unwrap(), None);
    }
}
