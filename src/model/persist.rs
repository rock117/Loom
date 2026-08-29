use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::snippets::SnippetsFile;
use super::workspace::{SettingsFile, UiStateFile, WorkspaceFile};
use crate::shared::paths;

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(value))
}

fn write_json<T: serde::Serialize>(path: &PathBuf, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn load_workspace() -> WorkspaceFile {
    let path = paths::workspace_path();
    match read_json::<WorkspaceFile>(&path) {
        Ok(Some(ws)) if !ws.groups.is_empty() => ws,
        Ok(Some(_)) | Ok(None) => WorkspaceFile::default_workspace(),
        Err(err) => {
            eprintln!("loom: workspace load failed ({err}); using defaults");
            let bak = path.with_extension("json.bak");
            if let Err(copy_error) = fs::copy(&path, &bak) {
                eprintln!("loom: failed to backup corrupt workspace: {copy_error}");
            }
            WorkspaceFile::default_workspace()
        }
    }
}

pub fn save_workspace(ws: &WorkspaceFile) -> Result<()> {
    write_json(&paths::workspace_path(), ws)
}

pub fn load_ui_state() -> UiStateFile {
    read_json(&paths::ui_state_path())
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn save_ui_state(state: &UiStateFile) -> Result<()> {
    write_json(&paths::ui_state_path(), state)
}

pub fn load_settings() -> SettingsFile {
    read_json(&paths::settings_path())
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn save_settings(settings: &SettingsFile) -> Result<()> {
    write_json(&paths::settings_path(), settings)
}

pub fn load_snippets() -> SnippetsFile {
    read_json(&paths::snippets_path())
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn save_snippets(file: &SnippetsFile) -> Result<()> {
    write_json(&paths::snippets_path(), file)
}

pub fn export_workspace_to(path: &PathBuf, ws: &WorkspaceFile) -> Result<()> {
    write_json(path, ws)
}

pub fn import_workspace_from(path: &PathBuf) -> Result<WorkspaceFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let ws: WorkspaceFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if ws.groups.is_empty() {
        anyhow::bail!("imported workspace has no groups");
    }
    Ok(ws)
}
