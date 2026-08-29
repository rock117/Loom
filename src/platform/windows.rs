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
