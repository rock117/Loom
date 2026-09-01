use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::profile::Profile;
use crate::session::local_proxy::LocalProxyMode;

/// Sibling slot in a workspace or group folder (profile and group share one order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "t", content = "id", rename_all = "snake_case")]
pub enum OrderKey {
    Profile(Uuid),
    Group(Uuid),
}

/// Workspace root (`/`: files + directories).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    /// Root-level profiles (files under `/`).
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Root-level groups (directories under `/`).
    pub groups: Vec<Group>,
    /// Display / move order of root profiles and groups (interleaved).
    #[serde(default)]
    pub order: Vec<OrderKey>,
}

impl WorkspaceFile {
    pub fn default_workspace() -> Self {
        let profile_name = if cfg!(windows) {
            "PowerShell"
        } else {
            "Shell"
        };
        let profile = Profile::new_local(profile_name);
        let id = profile.id;
        Self {
            version: 2,
            // Default Local shell sits at root — no forced group.
            profiles: vec![profile],
            groups: Vec::new(),
            order: vec![OrderKey::Profile(id)],
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
    /// Flatten for sidebar using interleaved `order` (respecting collapsed).
    pub fn sidebar_entries(&self) -> Vec<SidebarEntry> {
        let mut out = Vec::new();
        self.append_ordered_entries(&self.order, &self.profiles, &self.groups, 0, &mut out);
        out
    }

    fn append_ordered_entries(
        &self,
        order: &[OrderKey],
        profiles: &[Profile],
        groups: &[Group],
        depth: u32,
        out: &mut Vec<SidebarEntry>,
    ) {
        for key in order {
            match key {
                OrderKey::Profile(id) => {
                    if let Some(p) = profiles.iter().find(|p| p.id == *id) {
                        out.push(SidebarEntry::Profile {
                            id: p.id,
                            name: p.name.clone(),
                            depth,
                            is_local: p.kind.is_local(),
                        });
                    }
                }
                OrderKey::Group(id) => {
                    if let Some(g) = groups.iter().find(|g| g.id == *id) {
                        g.append_sidebar_entries(depth, out);
                    }
                }
            }
        }
    }

    /// Repair `order` lists after load or structural edits (preserves relative order).
    pub fn sync_orders(&mut self) {
        sync_order_list(&mut self.order, &self.profiles, &self.groups);
        for g in &mut self.groups {
            g.sync_orders_recursive();
        }
    }

    /// Whether a profile can move up / down among its ordered siblings (profiles + groups).
    pub fn profile_can_move(&self, id: Uuid) -> (bool, bool) {
        self.order_key_can_move(OrderKey::Profile(id))
    }

    /// Swap a profile with the previous/next sibling in the unified order.
    pub fn move_profile_by(&mut self, id: Uuid, delta: isize) -> bool {
        self.move_order_key(OrderKey::Profile(id), delta)
    }

    /// Whether a group can move up / down among its ordered siblings.
    pub fn group_can_move(&self, id: Uuid) -> (bool, bool) {
        self.order_key_can_move(OrderKey::Group(id))
    }

    /// Swap a group with the previous/next sibling in the unified order.
    pub fn move_group_by(&mut self, id: Uuid, delta: isize) -> bool {
        self.move_order_key(OrderKey::Group(id), delta)
    }

    fn order_key_can_move(&self, key: OrderKey) -> (bool, bool) {
        if let Some(i) = self.order.iter().position(|k| *k == key) {
            return (i > 0, i + 1 < self.order.len());
        }
        for g in &self.groups {
            if let Some(bounds) = g.order_key_can_move(key) {
                return bounds;
            }
        }
        (false, false)
    }

    fn move_order_key(&mut self, key: OrderKey, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        if move_key_in_order(&mut self.order, key, delta) {
            return true;
        }
        for g in &mut self.groups {
            if g.move_order_key_recursive(key, delta) {
                return true;
            }
        }
        false
    }

    /// Remove `key` from every order list.
    pub fn remove_order_key(&mut self, key: OrderKey) {
        self.order.retain(|k| *k != key);
        for g in &mut self.groups {
            g.remove_order_key_recursive(key);
        }
    }

    /// Place `key` immediately before `before` in the order list that contains `before`.
    /// If `before` is missing, appends to root order.
    pub fn insert_order_key_before(&mut self, key: OrderKey, before: OrderKey) {
        self.remove_order_key(key);
        if insert_before_in_order(&mut self.order, key, before) {
            return;
        }
        for g in &mut self.groups {
            if g.insert_order_key_before_recursive(key, before) {
                return;
            }
        }
        self.order.push(key);
    }

    /// Place `key` immediately after `after` in the order list that contains `after`.
    pub fn insert_order_key_after(&mut self, key: OrderKey, after: OrderKey) {
        self.remove_order_key(key);
        if insert_after_in_order(&mut self.order, key, after) {
            return;
        }
        for g in &mut self.groups {
            if g.insert_order_key_after_recursive(key, after) {
                return;
            }
        }
        self.order.push(key);
    }
}

fn sync_order_list(order: &mut Vec<OrderKey>, profiles: &[Profile], groups: &[Group]) {
    order.retain(|k| match k {
        OrderKey::Profile(id) => profiles.iter().any(|p| p.id == *id),
        OrderKey::Group(id) => groups.iter().any(|g| g.id == *id),
    });
    for p in profiles {
        let key = OrderKey::Profile(p.id);
        if !order.iter().any(|k| *k == key) {
            order.push(key);
        }
    }
    for g in groups {
        let key = OrderKey::Group(g.id);
        if !order.iter().any(|k| *k == key) {
            order.push(key);
        }
    }
}

fn move_key_in_order(order: &mut [OrderKey], key: OrderKey, delta: isize) -> bool {
    let Some(i) = order.iter().position(|k| *k == key) else {
        return false;
    };
    let j = i as isize + delta;
    if j < 0 || j as usize >= order.len() {
        return false;
    }
    order.swap(i, j as usize);
    true
}

fn insert_before_in_order(order: &mut Vec<OrderKey>, key: OrderKey, before: OrderKey) -> bool {
    let Some(pos) = order.iter().position(|k| *k == before) else {
        return false;
    };
    order.insert(pos, key);
    true
}

fn insert_after_in_order(order: &mut Vec<OrderKey>, key: OrderKey, after: OrderKey) -> bool {
    let Some(pos) = order.iter().position(|k| *k == after) else {
        return false;
    };
    order.insert(pos + 1, key);
    true
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
    /// Display / move order of this folder's profiles and child groups.
    #[serde(default)]
    pub order: Vec<OrderKey>,
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
            order: Vec::new(),
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
        for key in &self.order {
            match key {
                OrderKey::Profile(id) => {
                    if let Some(p) = self.profiles.iter().find(|p| p.id == *id) {
                        out.push(SidebarEntry::Profile {
                            id: p.id,
                            name: p.name.clone(),
                            depth: depth + 1,
                            is_local: p.kind.is_local(),
                        });
                    }
                }
                OrderKey::Group(id) => {
                    if let Some(c) = self.children.iter().find(|c| c.id == *id) {
                        c.append_sidebar_entries(depth + 1, out);
                    }
                }
            }
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

    fn sync_orders_recursive(&mut self) {
        sync_order_list(&mut self.order, &self.profiles, &self.children);
        for c in &mut self.children {
            c.sync_orders_recursive();
        }
    }

    fn order_key_can_move(&self, key: OrderKey) -> Option<(bool, bool)> {
        if let Some(i) = self.order.iter().position(|k| *k == key) {
            return Some((i > 0, i + 1 < self.order.len()));
        }
        for c in &self.children {
            if let Some(bounds) = c.order_key_can_move(key) {
                return Some(bounds);
            }
        }
        None
    }

    fn move_order_key_recursive(&mut self, key: OrderKey, delta: isize) -> bool {
        if move_key_in_order(&mut self.order, key, delta) {
            return true;
        }
        for c in &mut self.children {
            if c.move_order_key_recursive(key, delta) {
                return true;
            }
        }
        false
    }

    fn remove_order_key_recursive(&mut self, key: OrderKey) {
        self.order.retain(|k| *k != key);
        for c in &mut self.children {
            c.remove_order_key_recursive(key);
        }
    }

    fn insert_order_key_before_recursive(&mut self, key: OrderKey, before: OrderKey) -> bool {
        if insert_before_in_order(&mut self.order, key, before) {
            return true;
        }
        for c in &mut self.children {
            if c.insert_order_key_before_recursive(key, before) {
                return true;
            }
        }
        false
    }

    fn insert_order_key_after_recursive(&mut self, key: OrderKey, after: OrderKey) -> bool {
        if insert_after_in_order(&mut self.order, key, after) {
            return true;
        }
        for c in &mut self.children {
            if c.insert_order_key_after_recursive(key, after) {
                return true;
            }
        }
        false
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
            order: Vec::new(),
        };
        let mut child = Group::new("child");
        let p = Profile::new_local("inner");
        let pid = p.id;
        child.profiles.push(p);
        child.order.push(OrderKey::Profile(pid));
        let mut parent = Group::new("parent");
        let child_id = child.id;
        parent.children.push(child);
        parent.order.push(OrderKey::Group(child_id));
        ws.groups.push(parent);
        ws.order.push(OrderKey::Group(ws.groups[0].id));

        assert_eq!(ws.group_id_for_profile(pid), Some(child_id));
        assert!(ws.find_profile(pid).is_some());
        assert!(ws.find_group(child_id).is_some());
    }

    #[test]
    fn root_profile_can_move_past_group() {
        let mut ws = WorkspaceFile::default_workspace();
        let pid = ws.profiles[0].id;
        let g = Group::new("G");
        let gid = g.id;
        ws.groups.push(g);
        ws.sync_orders();
        // order: profile, group
        assert_eq!(ws.order.len(), 2);
        assert!(ws.move_profile_by(pid, 1));
        assert_eq!(ws.order, vec![OrderKey::Group(gid), OrderKey::Profile(pid)]);
        let entries = ws.sidebar_entries();
        assert!(matches!(entries[0], SidebarEntry::Group { id, .. } if id == gid));
        assert!(matches!(entries[1], SidebarEntry::Profile { id, .. } if id == pid));
    }
}
