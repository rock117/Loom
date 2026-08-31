use std::path::PathBuf;

pub fn native_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("loom")
}

pub fn native_default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
}

pub fn native_monospace_font_family() -> &'static str {
    "DejaVu Sans Mono"
}

pub fn native_reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    std::process::Command::new("xdg-open")
        .arg(path.as_os_str())
        .spawn()?;
    Ok(())
}

pub fn native_open_url(url: &str) -> std::io::Result<()> {
    use std::process::Stdio;
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
