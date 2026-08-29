//! OS-specific helpers. Windows is fully implemented; other targets use stubs/defaults.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
compile_error!("Loom currently supports Windows, macOS, and Linux only");

use std::path::PathBuf;

/// Application config directory (`%APPDATA%/Loom`, etc.).
pub fn config_dir() -> PathBuf {
    native_config_dir()
}

/// Default interactive shell executable (absolute path preferred on Windows).
pub fn default_shell() -> String {
    native_default_shell()
}

/// Preferred monospace UI/terminal font family for this platform.
pub fn monospace_font_family() -> &'static str {
    native_monospace_font_family()
}

/// Resolve a configured shell or fall back to [`default_shell`].
pub fn resolve_shell(configured: Option<&str>) -> String {
    if let Some(shell) = configured {
        if !shell.is_empty() {
            return shell.to_string();
        }
    }
    default_shell()
}

/// Reveal a file or directory in the OS file manager (Explorer / Finder / file manager).
pub fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    native_reveal_in_file_manager(path)
}

/// Live cwd of a local process (Zed-style), used when OSC hooks are missing or stale.
///
/// Returns `None` if the pid is gone, inaccessible, or the OS cannot report cwd.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );
    sys.process(pid)
        .and_then(|p| p.cwd().map(|p| p.to_path_buf()))
}
