use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::profile::Profile;
use crate::session::local_proxy::LocalProxyMode;

/// Workspace root (`/`: files + directories).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    /// Root-level profiles (files under `/`).
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Root-level groups (directories under `/`).
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
            version: 2,
            // Default Local shell sits at root — no forced group.
            profiles: vec![Profile::new_local(profile_name)],
            groups: Vec::new(),
        }
    }

    pub fn find_profile(&self, id: Uuid) -> Option<&Profile> {
        if let Some(p) = self.profiles.iter().find(|p| p.id == id) {
            return Some(p);
        }
        for g in &self.groups {
            if let Some(p) = g.find_profile(id) {
                return Some(p);
            }
        }
        None
    }

    pub fn find_profile_mut(&mut self, id: Uuid) -> Option<&mut Profile> {
        if let Some(p) = self.profiles.iter_mut().find(|p| p.id == id) {
            return Some(p);
        }
        for g in &mut self.groups {
            if let Some(p) = g.find_profile_mut(id) {
                return Some(p);
            }
        }
        None
    }

    pub fn find_group(&self, id: Uuid) -> Option<&Group> {
        for g in &self.groups {
            if g.id == id {
                return Some(g);
            }
            if let Some(found) = g.find_group(id) {
                return Some(found);
            }
        }
        None
    }

    pub fn find_group_mut(&mut self, id: Uuid) -> Option<&mut Group> {
        Self::find_group_mut_slice(&mut self.groups, id)
    }

    fn find_group_mut_slice(groups: &mut [Group], id: Uuid) -> Option<&mut Group> {
        for i in 0..groups.len() {
            if groups[i].id == id {
                return Some(&mut groups[i]);
            }
        }
        for g in groups.iter_mut() {
            if let Some(found) = Self::find_group_mut_slice(&mut g.children, id) {
                return Some(found);
            }
        }
        None
    }

    pub fn remove_profile(&mut self, id: Uuid) -> Option<Profile> {
        if let Some(pos) = self.profiles.iter().position(|p| p.id == id) {
            return Some(self.profiles.remove(pos));
        }
        for g in &mut self.groups {
            if let Some(p) = g.remove_profile(id) {
                return Some(p);
            }
        }
        None
    }

    /// Parent group of a profile, or `None` if the profile is at workspace root.
    pub fn group_id_for_profile(&self, profile_id: Uuid) -> Option<Uuid> {
        if self.profiles.iter().any(|p| p.id == profile_id) {
            return None;
        }
        for g in &self.groups {
            if let Some(id) = g.group_id_for_profile(profile_id) {
                return Some(id);
            }
        }
        None
    }

    pub fn first_local_profile_id(&self) -> Option<Uuid> {
        if let Some(p) = self.profiles.iter().find(|p| p.kind.is_local()) {
            return Some(p.id);
        }
        for g in &self.groups {
            if let Some(id) = g.first_local_profile_id() {
                return Some(id);
            }
        }
        None
    }

    /// All profile names in the tree (for unique_name).
    pub fn all_profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();
        for g in &self.groups {
            g.collect_profile_names(&mut names);
        }
        names
    }

    /// Flatten groups for simple UIs: (id, name, depth, collapsed).
    /// Skips children of collapsed groups (sidebar-style).
    pub fn walk_groups(&self) -> Vec<(Uuid, String, u32, bool)> {
        let mut out = Vec::new();
        for g in &self.groups {
            g.walk(0, &mut out, false);
        }
        out
    }

    /// All groups regardless of collapsed (for Save / Move pickers).
    pub fn walk_groups_all(&self) -> Vec<(Uuid, String, u32, bool)> {
        let mut out = Vec::new();
        for g in &self.groups {
            g.walk(0, &mut out, true);
        }
        out
    }

    /// Collect every profile id under a group (recursive) for credential cleanup.
    pub fn profile_ids_in_group(&self, group_id: Uuid) -> Vec<Uuid> {
        match self.find_group(group_id) {
            Some(g) => {
                let mut ids = Vec::new();
                g.collect_profile_ids(&mut ids);
                ids
            }
            None => Vec::new(),
        }
    }

    pub fn remove_group(&mut self, id: Uuid) -> Option<Group> {
        if let Some(pos) = self.groups.iter().position(|g| g.id == id) {
            return Some(self.groups.remove(pos));
        }
        for g in &mut self.groups {
            if let Some(removed) = g.remove_child_group(id) {
                return Some(removed);
            }
        }
        None
    }
    /// Flatten for sidebar: root profiles, then groups (respecting collapsed).
    pub fn sidebar_entries(&self) -> Vec<SidebarEntry> {
        let mut out = Vec::new();
        for p in &self.profiles {
            out.push(SidebarEntry::Profile {
                id: p.id,
                name: p.name.clone(),
                depth: 0,
                is_local: p.kind.is_local(),
            });
        }
        for g in &self.groups {
            g.append_sidebar_entries(0, &mut out);
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Nested groups (subdirectories).
    #[serde(default)]
    pub children: Vec<Group>,
}

/// One row in the profiles sidebar tree.
#[derive(Debug, Clone)]
pub enum SidebarEntry {
    Group {
        id: Uuid,
        name: String,
        depth: u32,
        collapsed: bool,
    },
    Profile {
        id: Uuid,
        name: String,
        depth: u32,
        is_local: bool,
    },
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            collapsed: false,
            profiles: Vec::new(),
            children: Vec::new(),
        }
    }

    fn append_sidebar_entries(&self, depth: u32, out: &mut Vec<SidebarEntry>) {
        out.push(SidebarEntry::Group {
            id: self.id,
            name: self.name.clone(),
            depth,
            collapsed: self.collapsed,
        });
        if self.collapsed {
            return;
        }
        for p in &self.profiles {
            out.push(SidebarEntry::Profile {
                id: p.id,
                name: p.name.clone(),
                depth: depth + 1,
                is_local: p.kind.is_local(),
            });
        }
        for c in &self.children {
            c.append_sidebar_entries(depth + 1, out);
        }
    }

    fn find_profile(&self, id: Uuid) -> Option<&Profile> {
        if let Some(p) = self.profiles.iter().find(|p| p.id == id) {
            return Some(p);
        }
        for c in &self.children {
            if let Some(p) = c.find_profile(id) {
                return Some(p);
            }
        }
        None
    }

    fn find_profile_mut(&mut self, id: Uuid) -> Option<&mut Profile> {
        if let Some(p) = self.profiles.iter_mut().find(|p| p.id == id) {
            return Some(p);
        }
        for c in &mut self.children {
            if let Some(p) = c.find_profile_mut(id) {
                return Some(p);
            }
        }
        None
    }

    fn find_group(&self, id: Uuid) -> Option<&Group> {
        for c in &self.children {
            if c.id == id {
                return Some(c);
            }
            if let Some(found) = c.find_group(id) {
                return Some(found);
            }
        }
        None
    }

    fn remove_profile(&mut self, id: Uuid) -> Option<Profile> {
        if let Some(pos) = self.profiles.iter().position(|p| p.id == id) {
            return Some(self.profiles.remove(pos));
        }
        for c in &mut self.children {
            if let Some(p) = c.remove_profile(id) {
                return Some(p);
            }
        }
        None
    }

    fn remove_child_group(&mut self, id: Uuid) -> Option<Group> {
        if let Some(pos) = self.children.iter().position(|g| g.id == id) {
            return Some(self.children.remove(pos));
        }
        for c in &mut self.children {
            if let Some(g) = c.remove_child_group(id) {
                return Some(g);
            }
        }
        None
    }

    /// Returns the id of the group that directly contains `profile_id`.
    fn group_id_for_profile(&self, profile_id: Uuid) -> Option<Uuid> {
        if self.profiles.iter().any(|p| p.id == profile_id) {
            return Some(self.id);
        }
        for c in &self.children {
            if let Some(id) = c.group_id_for_profile(profile_id) {
                return Some(id);
            }
        }
        None
    }

    fn first_local_profile_id(&self) -> Option<Uuid> {
        if let Some(p) = self.profiles.iter().find(|p| p.kind.is_local()) {
            return Some(p.id);
        }
        for c in &self.children {
            if let Some(id) = c.first_local_profile_id() {
                return Some(id);
            }
        }
        None
    }

    fn collect_profile_names(&self, out: &mut Vec<String>) {
        out.extend(self.profiles.iter().map(|p| p.name.clone()));
        for c in &self.children {
            c.collect_profile_names(out);
        }
    }

    fn collect_profile_ids(&self, out: &mut Vec<Uuid>) {
        out.extend(self.profiles.iter().map(|p| p.id));
        for c in &self.children {
            c.collect_profile_ids(out);
        }
    }

    fn walk(&self, depth: u32, out: &mut Vec<(Uuid, String, u32, bool)>, include_collapsed: bool) {
        out.push((self.id, self.name.clone(), depth, self.collapsed));
        if include_collapsed || !self.collapsed {
            for c in &self.children {
                c.walk(depth + 1, out, include_collapsed);
            }
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
    /// Off / Auto (env + OS) / Manual URL — Local shells only.
    #[serde(default)]
    pub local_proxy_mode: LocalProxyMode,
    /// Used when [`local_proxy_mode`] is Manual.
    #[serde(default)]
    pub local_proxy_url: Option<String>,
    /// Optional `NO_PROXY` when Auto or Manual.
    #[serde(default)]
    pub local_proxy_no_proxy: Option<String>,
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
            local_proxy_mode: LocalProxyMode::Off,
            local_proxy_url: None,
            local_proxy_no_proxy: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_profile_has_no_group() {
        let mut ws = WorkspaceFile::default_workspace();
        let id = ws.profiles[0].id;
        assert!(ws.group_id_for_profile(id).is_none());
        assert!(ws.find_profile(id).is_some());
    }

    #[test]
    fn nested_group_profile() {
        let mut ws = WorkspaceFile {
            version: 2,
            profiles: Vec::new(),
            groups: Vec::new(),
        };
        let mut child = Group::new("child");
        let p = Profile::new_local("inner");
        let pid = p.id;
        child.profiles.push(p);
        let mut parent = Group::new("parent");
        let child_id = child.id;
        parent.children.push(child);
        ws.groups.push(parent);

        assert_eq!(ws.group_id_for_profile(pid), Some(child_id));
        assert!(ws.find_profile(pid).is_some());
        assert!(ws.find_group(child_id).is_some());
    }
}
