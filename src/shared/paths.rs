use std::path::PathBuf;

use crate::platform;

pub fn loom_dir() -> PathBuf {
    platform::config_dir()
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

pub fn logs_dir() -> PathBuf {
    platform::logs_dir()
}

pub fn log_file_path() -> PathBuf {
    logs_dir().join("Loom.log")
}

pub fn snippets_path() -> PathBuf {
    loom_dir().join("snippets.json")
}

pub fn known_hosts_path() -> PathBuf {
    loom_dir().join("known_hosts.json")
}
