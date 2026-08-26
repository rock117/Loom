use std::path::PathBuf;

pub fn loom_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Loom")
}

pub fn workspace_path() -> PathBuf {
    loom_dir().join("workspace.json")
}

pub fn ui_state_path() -> PathBuf {
    loom_dir().join("ui_state.json")
}

pub fn settings_path() -> PathBuf {
    loom_dir().join("settings.json")
}

pub fn known_hosts_path() -> PathBuf {
    loom_dir().join("known_hosts.json")
}
