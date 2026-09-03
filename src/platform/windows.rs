use std::path::PathBuf;
use std::sync::LazyLock;

/// Cached default shell (Zed-style path scan; no `where.exe` subprocess).
static DEFAULT_SHELL: LazyLock<String> = LazyLock::new(detect_default_shell);

pub fn native_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Loom")
}

pub fn native_default_shell() -> String {
    DEFAULT_SHELL.clone()
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
        // Explorer is GUI — do not use CREATE_NO_WINDOW.
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
    if ret <= 32 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn font_file_exists(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

/// Zed `gpui_util::get_powershell` — scan common install locations, then PATH.
fn detect_default_shell() -> String {
    let probes: [fn() -> Option<PathBuf>; 11] = [
        || find_pwsh_in_programfiles(false, false),
        || find_pwsh_in_programfiles(true, false),
        || find_pwsh_in_msix(false),
        || find_pwsh_in_programfiles(false, true),
        || find_pwsh_in_msix(true),
        || find_pwsh_in_programfiles(true, true),
        || find_pwsh_in_scoop(),
        || find_pwsh_in_dotnet_tools(),
        || which_global("pwsh.exe"),
        || which_global("powershell.exe"),
        || find_windows_powershell(),
    ];

    if let Some(path) = probes.into_iter().find_map(|f| f()) {
        return path.to_string_lossy().trim().to_owned();
    }

    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(system_root)
        .join("System32\\cmd.exe")
        .to_string_lossy()
        .into_owned()
}

fn find_pwsh_in_programfiles(find_alternate: bool, find_preview: bool) -> Option<PathBuf> {
    #[cfg(target_pointer_width = "64")]
    let env_var = if find_alternate {
        "ProgramFiles(x86)"
    } else {
        "ProgramFiles"
    };
    #[cfg(target_pointer_width = "32")]
    let env_var = if find_alternate {
        "ProgramW6432"
    } else {
        "ProgramFiles"
    };

    let install_base_dir = PathBuf::from(std::env::var_os(env_var)?).join("PowerShell");
    install_base_dir
        .read_dir()
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| matches!(entry.file_type(), Ok(ft) if ft.is_dir()))
        .filter_map(|entry| {
            let dir_name = entry.file_name();
            let dir_name = dir_name.to_string_lossy();
            let version = if find_preview {
                let dash_index = dir_name.find('-')?;
                if &dir_name[dash_index + 1..] != "preview" {
                    return None;
                }
                dir_name[..dash_index].parse::<u32>().ok()?
            } else {
                dir_name.parse::<u32>().ok()?
            };
            let exe_path = entry.path().join("pwsh.exe");
            exe_path.is_file().then_some((version, exe_path))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

fn find_pwsh_in_msix(find_preview: bool) -> Option<PathBuf> {
    let msix_app_dir =
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Microsoft\\WindowsApps");
    let package_family_name = if find_preview {
        "Microsoft.PowerShellPreview_8wekyb3d8bbwe"
    } else {
        "Microsoft.PowerShell_8wekyb3d8bbwe"
    };
    let pwsh_exe = msix_app_dir.join(package_family_name).join("pwsh.exe");
    pwsh_exe.is_file().then_some(pwsh_exe)
}

fn find_pwsh_in_scoop() -> Option<PathBuf> {
    let pwsh_exe = PathBuf::from(std::env::var_os("USERPROFILE")?).join("scoop\\shims\\pwsh.exe");
    pwsh_exe.is_file().then_some(pwsh_exe)
}

fn find_pwsh_in_dotnet_tools() -> Option<PathBuf> {
    let pwsh_exe =
        PathBuf::from(std::env::var_os("USERPROFILE")?).join(".dotnet\\tools\\pwsh.exe");
    pwsh_exe.is_file().then_some(pwsh_exe)
}

fn find_windows_powershell() -> Option<PathBuf> {
    let system_root = PathBuf::from(std::env::var_os("SystemRoot")?);
    let powershell = system_root.join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    powershell.is_file().then_some(powershell)
}

fn which_global(name: &str) -> Option<PathBuf> {
    which::which_global(name).ok()
}
