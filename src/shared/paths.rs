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

pub fn known_hosts_path() -> PathBuf {
    loom_dir().join("known_hosts.json")
}
