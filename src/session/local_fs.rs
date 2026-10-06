//! Local filesystem helpers for the Context Files browser (non-SFTP).
//!
//! Uses `std::fs` (not `tokio::fs`) so callers can run on GPUI's executor
//! without a Tokio reactor.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::sftp::RemoteEntry;

/// List a local directory as [`RemoteEntry`] rows (unsorted; UI applies sort).
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
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        out.push(RemoteEntry {
            name,
            path: full.to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_file() { meta.len() } else { 0 },
            mtime,
        });
    }
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

/// `~` or `~/…` / `~\…` (current user only). `~other` is left for the caller.
pub fn is_tilde_path(path: &str) -> bool {
    let trimmed = path.trim().trim_matches('"').trim();
    let Some(rest) = trimmed.strip_prefix('~') else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
}

/// Expand a leading `~` using `home`. Non-tilde paths are returned trimmed, quotes stripped.
pub fn expand_tilde(path: &str, home: &str) -> String {
    let trimmed = path.trim().trim_matches('"').trim();
    let Some(rest) = trimmed.strip_prefix('~') else {
        return trimmed.to_string();
    };
    if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
        return trimmed.to_string();
    }
    join_under_home(home, rest.trim_start_matches(['/', '\\']))
}

/// Windows path with at least one `%NAME%` variable (`%USERPROFILE%`, `%USERPROFILE%\Documents`).
pub fn is_windows_env_path(path: &str) -> bool {
    let trimmed = path.trim().trim_matches('"').trim();
    env_var_span(trimmed).is_some()
}

/// Expand `%NAME%` from the process environment. Unset names are an error.
pub fn expand_windows_env(path: &str) -> Result<String, String> {
    expand_windows_env_with(path, |name| std::env::var(name).ok())
}

fn expand_windows_env_with(
    path: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut s = path.trim().trim_matches('"').trim().to_string();
    for _ in 0..8 {
        let Some((start, end, name)) = env_var_span(&s) else {
            return Ok(s);
        };
        let name = name.to_string();
        let Some(value) = lookup(&name) else {
            return Err(format!("Environment variable {name} is not set"));
        };
        s.replace_range(start..end, &value);
    }
    Err("Too many environment variables in path".into())
}

/// `(start, end_exclusive, name)` of the first `%NAME%`.
fn env_var_span(s: &str) -> Option<(usize, usize, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(rel) = s[i + 1..].find('%') {
                let name = &s[i + 1..i + 1 + rel];
                if is_env_name(name) {
                    return Some((i, i + 1 + rel + 1, name));
                }
            }
        }
        i += s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    None
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn join_under_home(home: &str, tail: &str) -> String {
    let windows = home.contains('\\')
        || home.as_bytes().get(1) == Some(&b':');
    if !windows {
        let home = home.trim_end_matches('/');
        if tail.is_empty() {
            return if home.is_empty() {
                "/".into()
            } else {
                home.to_string()
            };
        }
        let tail = tail.replace('\\', "/");
        if home.is_empty() || home == "/" {
            format!("/{tail}")
        } else {
            format!("{home}/{tail}")
        }
    } else {
        let mut path = PathBuf::from(home.trim_end_matches(['\\', '/']));
        for part in tail.split(['/', '\\']) {
            if !part.is_empty() {
                path.push(part);
            }
        }
        path.to_string_lossy().into_owned()
    }
}

/// Validate a typed/pasted path for the Files browser: must exist and be a directory.
/// Strips whitespace and surrounding quotes (Explorer-style paste).
pub fn resolve_existing_dir(path: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return Err("Path is empty".into());
    }
    let p = PathBuf::from(trimmed);
    match std::fs::metadata(&p) {
        Ok(meta) if meta.is_dir() => {
            let display = match p.canonicalize() {
                Ok(c) => {
                    let s = c.to_string_lossy().into_owned();
                    s.strip_prefix(r"\\?\")
                        .unwrap_or(&s)
                        .to_string()
                }
                Err(_) => trimmed.to_string(),
            };
            Ok(display)
        }
        Ok(_) => Err("Path is a file, not a directory".into()),
        Err(_) => Err("Path does not exist".into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_unix_home() {
        assert!(is_tilde_path("~/src"));
        assert!(is_tilde_path("~"));
        assert!(is_tilde_path(r"~\src"));
        assert!(!is_tilde_path("~other"));
        assert!(!is_tilde_path("/tmp"));
        assert_eq!(expand_tilde("~/src", "/home/me"), "/home/me/src");
        assert_eq!(expand_tilde("~", "/home/me"), "/home/me");
        assert_eq!(expand_tilde("  \"~/a/b\" ", "/root"), "/root/a/b");
        assert_eq!(expand_tilde("~other/x", "/home/me"), "~other/x");
        assert_eq!(expand_tilde("/abs", "/home/me"), "/abs");
        assert_eq!(expand_tilde("~/x", "/"), "/x");
        assert_eq!(expand_tilde("~/x", "/home/me/"), "/home/me/x");
    }

    #[test]
    fn expand_tilde_windows_home() {
        let home = r"C:\Users\me";
        let mut expect = PathBuf::from(home);
        expect.push("docs");
        assert_eq!(expand_tilde(r"~\docs", home), expect.to_string_lossy());
        let mut nested = PathBuf::from(home);
        nested.push("a");
        nested.push("b");
        assert_eq!(expand_tilde("~/a/b", home), nested.to_string_lossy());
        assert_eq!(
            expand_tilde("~", home),
            PathBuf::from(home).to_string_lossy()
        );
    }

    #[test]
    fn expand_windows_userprofile() {
        let lookup = |name: &str| {
            if name.eq_ignore_ascii_case("USERPROFILE") {
                Some(r"C:\Users\me".into())
            } else {
                None
            }
        };
        assert!(is_windows_env_path(r"%USERPROFILE%\Documents"));
        assert!(is_windows_env_path("%USERPROFILE%"));
        assert!(!is_windows_env_path(r"C:\Users\me"));
        assert!(!is_windows_env_path("100%"));
        assert_eq!(
            expand_windows_env_with("%USERPROFILE%", lookup).unwrap(),
            r"C:\Users\me"
        );
        assert_eq!(
            expand_windows_env_with(r"%USERPROFILE%\Documents", lookup).unwrap(),
            r"C:\Users\me\Documents"
        );
        assert_eq!(
            expand_windows_env_with(r"  %USERPROFILE%/Desktop  ", lookup).unwrap(),
            r"C:\Users\me/Desktop"
        );
        assert!(expand_windows_env_with("%NOT_A_REAL_VAR%\\x", lookup).is_err());
    }
}
