use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::profile::Profile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    pub groups: Vec<Group>,
}

impl WorkspaceFile {
    pub fn default_workspace() -> Self {
        let profile_name = if cfg!(windows) {
            "PowerShell"
        } else {
            "Shell"
        };
        Self {
            version: 1,
            groups: vec![Group {
                id: Uuid::new_v4(),
                name: "Local".into(),
                collapsed: false,
                profiles: vec![Profile::new_local(profile_name)],
            }],
        }
    }

    pub fn find_profile(&self, id: Uuid) -> Option<&Profile> {
        self.groups
            .iter()
            .flat_map(|g| g.profiles.iter())
            .find(|p| p.id == id)
    }

    pub fn find_profile_mut(&mut self, id: Uuid) -> Option<&mut Profile> {
        self.groups
            .iter_mut()
            .flat_map(|g| g.profiles.iter_mut())
            .find(|p| p.id == id)
    }

    pub fn find_group_mut(&mut self, id: Uuid) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    pub fn remove_profile(&mut self, id: Uuid) -> Option<Profile> {
        for group in &mut self.groups {
            if let Some(pos) = group.profiles.iter().position(|p| p.id == id) {
                return Some(group.profiles.remove(pos));
            }
        }
        None
    }

    pub fn group_id_for_profile(&self, profile_id: Uuid) -> Option<Uuid> {
        self.groups
            .iter()
            .find(|g| g.profiles.iter().any(|p| p.id == profile_id))
            .map(|g| g.id)
    }

    pub fn first_local_profile_id(&self) -> Option<Uuid> {
        self.groups
            .iter()
            .flat_map(|g| g.profiles.iter())
            .find(|p| p.kind.is_local())
            .map(|p| p.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
    pub profiles: Vec<Profile>,
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            collapsed: false,
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenTabRef {
    pub profile_id: Uuid,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiStateFile {
    pub version: u32,
    pub sidebar_width: f32,
    /// When false, the profiles sidebar is hidden (Zed-style toggle).
    #[serde(default = "default_sidebar_visible")]
    pub sidebar_visible: bool,
    /// Right context panel width (snippets / files / info).
    #[serde(default = "default_context_panel_width")]
    pub context_panel_width: f32,
    /// When false, the context panel is hidden.
    #[serde(default = "default_context_panel_visible")]
    pub context_panel_visible: bool,
    /// Share of Files tab height for the file list (rest is Transfers).
    #[serde(default = "default_context_files_list_ratio")]
    pub context_files_list_ratio: f32,
    pub font_size: f32,
    pub open_tabs: Vec<OpenTabRef>,
    pub active_tab_index: usize,
}

fn default_sidebar_visible() -> bool {
    true
}

fn default_context_panel_width() -> f32 {
    280.0
}

fn default_context_panel_visible() -> bool {
    false
}

fn default_context_files_list_ratio() -> f32 {
    0.72
}

impl Default for UiStateFile {
    fn default() -> Self {
        Self {
            version: 1,
            sidebar_width: 240.0,
            sidebar_visible: true,
            context_panel_width: 280.0,
            context_panel_visible: false,
            context_files_list_ratio: 0.72,
            font_size: 14.0,
            open_tabs: Vec::new(),
            active_tab_index: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsFile {
    pub version: u32,
    pub default_shell: Option<String>,
    pub font_family: String,
    pub font_size: f32,
    /// Left gutter with absolute scrollback line numbers (1 = oldest in buffer).
    #[serde(default = "default_show_line_numbers")]
    pub show_line_numbers: bool,
}

fn default_show_line_numbers() -> bool {
    true
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            version: 1,
            default_shell: None,
            font_family: crate::platform::monospace_font_family().into(),
            font_size: 14.0,
            show_line_numbers: true,
        }
    }
}
