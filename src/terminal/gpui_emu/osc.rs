//! Side-channel OSC observer (cwd via OSC 7 / OSC 9;9).
//!
//! `alacritty_terminal` does not surface OSC 7 on its event API, so we run a second
//! VTE parser on the same PTY byte stream. OSC itself is standard (shells / Windows
//! Terminal emit it); the gap is the Alacritty crate, not the protocol.
//!
//! For **local** shells Loom also refreshes cwd from the process (Zed-style via
//! `sysinfo`) when opening Copy Path / Reveal — OSC remains important for SSH.

use std::path::{Path, PathBuf};

use alacritty_terminal::vte::{Parser, Perform};

/// Observes shell-reported working directories without touching the grid.
pub struct OscSidecar {
    parser: Parser,
    handler: OscHandler,
}

impl OscSidecar {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            handler: OscHandler::default(),
        }
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.handler, bytes);
    }

    /// Take a newly reported cwd (clears the pending flag).
    pub fn take_cwd_update(&mut self) -> Option<PathBuf> {
        self.handler.pending.take()
    }

    pub fn current_cwd(&self) -> Option<&Path> {
        self.handler.cwd.as_deref()
    }
}

#[derive(Default)]
struct OscHandler {
    cwd: Option<PathBuf>,
    pending: Option<PathBuf>,
}

impl OscHandler {
    fn set_cwd(&mut self, path: PathBuf) {
        if self.cwd.as_ref() == Some(&path) {
            return;
        }
        self.cwd = Some(path.clone());
        self.pending = Some(path);
    }
}

impl Perform for OscHandler {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        // OSC 7 ; file://host/path
        if params[0] == b"7" {
            let uri = params.get(1).copied().unwrap_or(b"");
            if let Some(path) = parse_osc7_file_uri(uri) {
                self.set_cwd(path);
            }
            return;
        }
        // OSC 9 ; 9 ; C:\windows\path  (ConEmu / Windows Terminal)
        if params[0] == b"9" && params.get(1).is_some_and(|p| *p == b"9") {
            let raw = params.get(2).copied().unwrap_or(b"");
            if let Some(path) = parse_osc99_path(raw) {
                self.set_cwd(path);
            }
        }
    }
}

fn parse_osc99_path(raw: &[u8]) -> Option<PathBuf> {
    let s = std::str::from_utf8(raw).ok()?.trim();
    if s.is_empty() {
        return None;
    }
    // Strip optional quotes from PowerShell/WT forms.
    let s = s.trim_matches('"');
    Some(PathBuf::from(s))
}

fn parse_osc7_file_uri(raw: &[u8]) -> Option<PathBuf> {
    let s = std::str::from_utf8(raw).ok()?.trim();
    if s.is_empty() {
        return None;
    }
    let rest = s.strip_prefix("file://")?;
    // file:///C:/Users/...  or file://hostname/C:/...  or file:///home/...
    let path_part = if let Some(after_host) = rest.strip_prefix("//") {
        // file://// rare; treat as path
        after_host
    } else if rest.starts_with('/') {
        // file:///path or file:///C:/path — leading slash kept for Unix;
        // Windows drive: "/C:/Users" → "C:/Users"
        rest
    } else {
        // file://host/path
        rest.find('/').map(|i| &rest[i..])?
    };

    let decoded = percent_decode(path_part);
    let path = normalize_file_uri_path(&decoded)?;
    Some(path)
}

fn normalize_file_uri_path(path_part: &str) -> Option<PathBuf> {
    let p = path_part;
    // Windows drive in URI: /C:/Users/... or /C|/Users/...
    if let Some(rest) = p.strip_prefix('/') {
        if rest.len() >= 2 {
            let bytes = rest.as_bytes();
            if bytes[0].is_ascii_alphabetic() && (bytes[1] == b':' || bytes[1] == b'|') {
                let mut drive = String::new();
                drive.push(bytes[0] as char);
                drive.push(':');
                let tail = &rest[2..];
                let joined = if tail.is_empty() {
                    format!("{drive}/")
                } else if tail.starts_with('/') || tail.starts_with('\\') {
                    format!("{drive}{tail}")
                } else {
                    format!("{drive}/{tail}")
                };
                return Some(PathBuf::from(joined.replace('/', std::path::MAIN_SEPARATOR_STR)));
            }
        }
        // Unix absolute
        return Some(PathBuf::from(p));
    }
    Some(PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_unix_path() {
        let mut s = OscSidecar::new();
        s.advance(b"\x1b]7;file:///home/user/proj\x07");
        assert_eq!(
            s.take_cwd_update().as_deref(),
            Some(Path::new("/home/user/proj"))
        );
    }

    #[test]
    fn osc7_windows_drive() {
        let mut s = OscSidecar::new();
        s.advance(b"\x1b]7;file:///C:/Users/me/Loom\x07");
        let got = s.take_cwd_update().expect("cwd");
        let s = got.to_string_lossy();
        assert!(s.contains("Users"), "{s}");
        assert!(s.starts_with("C:"), "{s}");
    }

    #[test]
    fn osc99_windows_path() {
        let mut s = OscSidecar::new();
        s.advance(b"\x1b]9;9;C:\\rock\\coding\x07");
        let got = s.take_cwd_update().expect("cwd");
        assert!(got.to_string_lossy().contains("rock"));
    }
}
