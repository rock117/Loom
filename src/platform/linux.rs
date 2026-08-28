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
