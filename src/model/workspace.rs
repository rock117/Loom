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
    pub font_size: f32,
    pub open_tabs: Vec<OpenTabRef>,
    pub active_tab_index: usize,
}

impl Default for UiStateFile {
    fn default() -> Self {
        Self {
            version: 1,
            sidebar_width: 240.0,
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
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            version: 1,
            default_shell: None,
            font_family: if cfg!(windows) {
                "Cascadia Mono".into()
            } else {
                "monospace".into()
            },
            font_size: 14.0,
        }
    }
}
