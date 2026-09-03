//! Background subprocess helpers (Zed-style: no console flash on Windows GUI apps).

/// Build a [`std::process::Command`] for background helpers (git probe, GPU info, etc.).
///
/// On Windows the parent is a GUI process; without `CREATE_NO_WINDOW`, console
/// helpers (`powershell`, `nvidia-smi`, …) briefly flash a CMD window.
#[cfg(windows)]
pub fn new_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
pub fn new_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    std::process::Command::new(program)
}
