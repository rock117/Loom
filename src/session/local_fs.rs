//! Local filesystem helpers for the Context Files browser (non-SFTP).
//!
//! Uses `std::fs` (not `tokio::fs`) so callers can run on GPUI's executor
//! without a Tokio reactor.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::sftp::RemoteEntry;

/// List a local directory as [`RemoteEntry`] rows (sorted dirs-first, then name).
pub fn list_dir(path: &Path) -> Result<Vec<RemoteEntry>> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))?;
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let full = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        out.push(RemoteEntry {
            name,
            path: full.to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_file() { meta.len() } else { 0 },
        });
    }
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(out)
}

pub fn parent_path(path: &str) -> Option<String> {
    let p = Path::new(path);
    let parent = p.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    if parent == p {
        return None;
    }
    Some(parent.to_string_lossy().into_owned())
}

pub fn create_dir(path: &Path) -> Result<()> {
    std::fs::create_dir(path).with_context(|| format!("mkdir {}", path.display()))
}

pub fn remove_path(path: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        std::fs::remove_dir_all(path).with_context(|| format!("remove_dir {}", path.display()))
    } else {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
    }
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to)
        .with_context(|| format!("rename {} → {}", from.display(), to.display()))
}

/// Apply permission bits. On Windows only the write/readonly bit is honored.
pub fn chmod(path: &Path, mode: u32) -> Result<()> {
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(mode & 0o7777);
    }
    #[cfg(not(unix))]
    {
        // Approximate: no owner-write bit → readonly.
        perms.set_readonly(mode & 0o222 == 0);
    }
    std::fs::set_permissions(path, perms).with_context(|| format!("chmod {}", path.display()))
}

pub fn join_child(parent: &str, name: &str) -> PathBuf {
    Path::new(parent).join(name)
}

pub fn parse_mode(text: &str) -> Result<u32> {
    let t = text.trim();
    if t.is_empty() {
        bail!("mode required");
    }
    let mode = if t.starts_with("0o") || t.starts_with("0O") {
        u32::from_str_radix(&t[2..], 8)
    } else if t.starts_with('0') && t.chars().all(|c| matches!(c, '0'..='7')) {
        u32::from_str_radix(t, 8)
    } else if t.chars().all(|c| matches!(c, '0'..='7')) {
        u32::from_str_radix(t, 8)
    } else {
        bail!("expected octal mode like 755");
    }
    .map_err(|_| anyhow::anyhow!("invalid octal mode"))?;
    if mode > 0o7777 {
        bail!("mode out of range");
    }
    Ok(mode)
}
