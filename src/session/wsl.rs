//! WSL helpers: list distros and preflight before spawning `wsl.exe -d …`.
//!
//! Listing / validation must not run on the UI thread (see `docs/HARD_PROBLEMS.md`).

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Distro name from Local profile args (`-d` / `--distribution`).
pub fn distro_from_args(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" | "--distribution" => return args.get(i + 1).map(String::as_str),
            flag if flag.starts_with("--distribution=") => {
                return Some(flag.trim_start_matches("--distribution="));
            }
            _ => i += 1,
        }
    }
    None
}

pub fn is_wsl_executable(shell: &str) -> bool {
    Path::new(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("wsl"))
}

/// Installed distro names (`wsl --list --quiet`). Empty Vec if none.
pub fn list_distros() -> Result<Vec<String>> {
    #[cfg(not(windows))]
    {
        bail!("WSL is only available on Windows");
    }
    #[cfg(windows)]
    {
        list_distros_windows()
    }
}

#[cfg(windows)]
fn list_distros_windows() -> Result<Vec<String>> {
    let output = crate::platform::new_command("wsl.exe")
        .args(["--list", "--quiet"])
        .output()
        .context(
            "Could not run wsl.exe. Install WSL from Microsoft Store or enable the Windows feature.",
        )?;

    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr);
        let stdout = decode_wsl_output(&output.stdout);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("wsl --list failed");
        bail!("{detail}");
    }

    let text = decode_wsl_output(&output.stdout);
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    Ok(names)
}

/// Run before spawning a Local profile that uses argv (esp. WSL).
///
/// Failures become pane `Failed` status only — never panic.
pub fn preflight_local_spawn(shell: &str, args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    if !is_wsl_executable(shell) {
        crate::session::docker::preflight_local_spawn(shell, args)?;
        return Ok(());
    }
    let Some(distro) = distro_from_args(args) else {
        return Ok(());
    };

    let installed = list_distros().with_context(|| {
        format!("Cannot verify WSL distro `{distro}` (is WSL installed and working?)")
    })?;

    if installed.is_empty() {
        bail!(
            "No WSL distributions are installed. Install one, or remove the `{distro}` profile."
        );
    }

    let found = installed
        .iter()
        .any(|name| name.eq_ignore_ascii_case(distro));
    if !found {
        bail!(
            "WSL distribution `{distro}` is not installed (uninstalled or renamed). \
             Other sessions are unaffected — remove or edit this profile, then try again."
        );
    }

    // `wsl -l` still lists distros whose ext4.vhdx was deleted; probe start.
    probe_distro(distro)?;
    Ok(())
}

/// Start the distro briefly to catch broken VHDX / MountDisk failures before PTY attach.
fn probe_distro(distro: &str) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = distro;
        Ok(())
    }
    #[cfg(windows)]
    {
        probe_distro_windows(distro)
    }
}

#[cfg(windows)]
fn probe_distro_windows(distro: &str) -> Result<()> {
    let output = crate::platform::new_command("wsl.exe")
        .args(["-d", distro, "--", "true"])
        .output()
        .with_context(|| format!("Could not probe WSL distro `{distro}`"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = decode_wsl_output(&output.stderr);
    let stdout = decode_wsl_output(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("WSL probe failed");

    bail!("{}", friendly_wsl_failure(distro, detail));
}

/// Map common WSL HCS / MountDisk errors to pane-local guidance.
pub fn friendly_wsl_failure(distro: &str, detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("error_file_not_found")
        || lower.contains("mountdisk")
        || lower.contains("mountvhd")
        || lower.contains("ext4.vhdx")
        || (lower.contains("找不到") && lower.contains("磁盘"))
    {
        return format!(
            "WSL distro `{distro}` is registered but its disk (ext4.vhdx) is missing or moved. \
             Other Loom sessions are unaffected. In PowerShell: \
             wsl --shutdown ; wsl --unregister {distro} ; \
             then reinstall the distro from the Store / `wsl --install -d {distro}`. \
             Detail: {detail}"
        );
    }
    format!("WSL distro `{distro}` failed to start: {detail}")
}

/// Friendly message when `shell` with args cannot be resolved (no default fallback).
pub fn missing_shell_message(shell: &str, args: &[String]) -> String {
    if is_wsl_executable(shell) {
        "wsl.exe was not found. Install WSL or fix PATH — other Loom sessions are unaffected."
            .into()
    } else if args.is_empty() {
        format!("Shell `{shell}` was not found")
    } else {
        format!(
            "Shell `{shell}` was not found (args: {}). Other sessions are unaffected.",
            args.join(" ")
        )
    }
}

/// Decode `wsl.exe` stdout/stderr (often UTF-16LE on Windows).
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let (bom, rest) = if bytes.starts_with(&[0xFF, 0xFE]) {
        (true, &bytes[2..])
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        // UTF-16BE — rare for wsl; fall through to lossy UTF-8
        return String::from_utf8_lossy(bytes).into_owned();
    } else {
        (false, bytes)
    };

    if bom || looks_like_utf16_le(rest) {
        return decode_utf16_le(rest);
    }
    String::from_utf8_lossy(bytes)
        .trim_matches('\0')
        .to_string()
}

fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }
    let sample = bytes.len().min(32);
    let nul_high = bytes[..sample]
        .chunks_exact(2)
        .filter(|c| c[1] == 0)
        .count();
    nul_high * 2 >= sample / 2
}

fn decode_utf16_le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_matches('\0')
        .to_string()
}

/// Used by the New WSL wizard: list with a wall-clock budget on the caller side.
#[allow(dead_code)]
pub const LIST_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distro_flag_parsing() {
        assert_eq!(
            distro_from_args(&["-d".into(), "Ubuntu".into()]),
            Some("Ubuntu")
        );
        assert_eq!(
            distro_from_args(&["--distribution".into(), "Debian".into()]),
            Some("Debian")
        );
        assert_eq!(
            distro_from_args(&["--distribution=Alpine".into()]),
            Some("Alpine")
        );
        assert_eq!(distro_from_args(&["--exec".into(), "bash".into()]), None);
    }

    #[test]
    fn decode_utf16_le_ascii() {
        let raw: Vec<u8> = "Ubuntu\0"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_wsl_output(&raw).trim(), "Ubuntu");
    }

    #[test]
    fn friendly_missing_vhdx() {
        let msg = friendly_wsl_failure(
            "Ubuntu",
            "MountDisk/HCS/ERROR_FILE_NOT_FOUND ext4.vhdx",
        );
        assert!(msg.contains("ext4.vhdx"));
        assert!(msg.contains("wsl --unregister"));
        assert!(msg.contains("Other Loom sessions"));
    }
}
