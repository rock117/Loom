use std::path::PathBuf;

pub fn native_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Loom")
}

pub fn native_default_shell() -> String {
    for name in ["pwsh", "powershell", "cmd"] {
        if let Some(path) = resolve_executable(name) {
            return path;
        }
    }
    r"C:\Windows\System32\cmd.exe".to_string()
}

pub fn native_monospace_font_family() -> &'static str {
    if font_file_exists(r"C:\Windows\Fonts\CascadiaMono.ttf")
        || font_file_exists(r"C:\Windows\Fonts\CascadiaCode.ttf")
    {
        "Cascadia Mono"
    } else {
        "Consolas"
    }
}

pub fn native_reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if path.is_file() {
        // Single argument form: /select,C:\full\path
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()?;
    } else {
        std::process::Command::new("explorer")
            .arg(path.as_os_str())
            .spawn()?;
    }
    Ok(())
}

pub fn native_open_url(url: &str) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(dw_process_id: u32) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut core::ffi::c_void,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    // ASFW_ANY (-1): let the browser take focus after a user Ctrl+click.
    // Without this (or if open runs on a background thread), Windows often only
    // flashes the taskbar and keeps Loom in front — unlike Zed's sync open.
    const ASFW_ANY: u32 = u32::MAX;
    const SW_SHOWNORMAL: i32 = 1;

    let operation: Vec<u16> = OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let file: Vec<u16> = OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    let ret = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Per MSDN, values ≤ 32 indicate failure.
    if ret <= 32 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn font_file_exists(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

fn resolve_executable(name: &str) -> Option<String> {
    let output = std::process::Command::new("where")
        .arg(name)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}
