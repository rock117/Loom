use std::path::PathBuf;

pub fn native_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Loom")
}

pub fn native_default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
}

pub fn native_monospace_font_family() -> &'static str {
    "Menlo"
}

pub fn native_reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    std::process::Command::new("open").arg(path.as_os_str()).spawn()?;
    Ok(())
}
