use gpui::*;
use uuid::Uuid;

use crate::model::{
    Group, OpenTabRef, Profile, ProfileKind, SettingsFile, UiStateFile, WorkspaceFile,
    load_settings, load_ui_state, load_workspace, save_settings, save_ui_state, save_workspace,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Group(Uuid),
    Profile(Uuid),
}

pub struct WorkspaceStore {
    pub workspace: WorkspaceFile,
    pub ui_state: UiStateFile,
    pub settings: SettingsFile,
    pub selection: Selection,
    pub search: String,
    pub rename_buffer: Option<String>,
    dirty: bool,
}

impl WorkspaceStore {
    pub fn load(_cx: &mut Context<Self>) -> Self {
        let workspace = load_workspace();
        let ui_state = load_ui_state();
        let settings = load_settings();
        Self {
            workspace,
            ui_state,
            settings,
            selection: Selection::None,
            search: String::new(),
            rename_buffer: None,
            dirty: false,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn persist_now(&mut self) {
        if let Err(error) = save_workspace(&self.workspace) {
            eprintln!("loom: failed to save workspace: {error:#}");
        }
        if let Err(error) = save_ui_state(&self.ui_state) {
            eprintln!("loom: failed to save ui state: {error:#}");
        }
        if let Err(error) = save_settings(&self.settings) {
            eprintln!("loom: failed to save settings: {error:#}");
        }
        self.dirty = false;
    }

    pub fn persist_if_dirty(&mut self) {
        if self.dirty {
            self.persist_now();
        }
    }

    pub fn set_search(&mut self, q: String, cx: &mut Context<Self>) {
        self.search = q;
        cx.notify();
    }

    pub fn select_group(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selection = Selection::Group(id);
        self.rename_buffer = None;
        cx.notify();
    }

    pub fn select_profile(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selection = Selection::Profile(id);
        self.rename_buffer = None;
        cx.notify();
    }

    pub fn toggle_group(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(g) = self.workspace.find_group_mut(id) {
            g.collapsed = !g.collapsed;
            self.mark_dirty();
            cx.notify();
        }
    }

    pub fn add_group(&mut self, cx: &mut Context<Self>) {
        let name = unique_name(
            "Group",
            &self
                .workspace
                .groups
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>(),
        );
        let group = Group::new(name);
        let id = group.id;
        self.workspace.groups.push(group);
        self.selection = Selection::Group(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn add_local_profile(&mut self, cx: &mut Context<Self>) -> Option<Uuid> {
        let group_id = match self.selection {
            Selection::Group(id) => id,
            Selection::Profile(pid) => self.workspace.group_id_for_profile(pid)?,
            Selection::None => self.workspace.groups.first().map(|g| g.id)?,
        };
        let names: Vec<String> = self
            .workspace
            .groups
            .iter()
            .flat_map(|g| g.profiles.iter().map(|p| p.name.clone()))
            .collect();
        let name = unique_name("Shell", &names);
        let profile = Profile::new_local(name);
        let id = profile.id;
        if let Some(g) = self.workspace.find_group_mut(group_id) {
            g.profiles.push(profile);
            g.collapsed = false;
        }
        self.selection = Selection::Profile(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        Some(id)
    }

    pub fn duplicate_profile(&mut self, id: Uuid, cx: &mut Context<Self>) -> Option<Uuid> {
        let group_id = self.workspace.group_id_for_profile(id)?;
        let dup = self.workspace.find_profile(id)?.duplicate();
        let new_id = dup.id;
        if let Some(g) = self.workspace.find_group_mut(group_id) {
            if let Some(pos) = g.profiles.iter().position(|p| p.id == id) {
                g.profiles.insert(pos + 1, dup);
            } else {
                g.profiles.push(dup);
            }
        }
        self.selection = Selection::Profile(new_id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        Some(new_id)
    }

    pub fn delete_selection(&mut self, cx: &mut Context<Self>) {
        match self.selection {
            Selection::Profile(id) => {
                self.workspace.remove_profile(id);
                self.selection = Selection::None;
                self.mark_dirty();
                self.persist_now();
                cx.notify();
            }
            Selection::Group(id) => {
                self.workspace.groups.retain(|g| g.id != id);
                if self.workspace.groups.is_empty() {
                    self.workspace = WorkspaceFile::default_workspace();
                }
                self.selection = Selection::None;
                self.mark_dirty();
                self.persist_now();
                cx.notify();
            }
            Selection::None => {}
        }
    }

    pub fn begin_rename(&mut self, cx: &mut Context<Self>) {
        let name = match self.selection {
            Selection::Profile(id) => self.workspace.find_profile(id).map(|p| p.name.clone()),
            Selection::Group(id) => self
                .workspace
                .groups
                .iter()
                .find(|g| g.id == id)
                .map(|g| g.name.clone()),
            Selection::None => None,
        };
        if let Some(name) = name {
            self.rename_buffer = Some(name);
            cx.notify();
        }
    }

    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.rename_buffer.take() else {
            return;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        match self.selection {
            Selection::Profile(id) => {
                if let Some(p) = self.workspace.find_profile_mut(id) {
                    p.name = name;
                }
            }
            Selection::Group(id) => {
                if let Some(g) = self.workspace.find_group_mut(id) {
                    g.name = name;
                }
            }
            Selection::None => {}
        }
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn push_rename_char(&mut self, ch: char, cx: &mut Context<Self>) {
        if let Some(buf) = &mut self.rename_buffer {
            if !ch.is_control() {
                buf.push(ch);
                cx.notify();
            }
        }
    }

    pub fn pop_rename_char(&mut self, cx: &mut Context<Self>) {
        if let Some(buf) = &mut self.rename_buffer {
            buf.pop();
            cx.notify();
        }
    }

    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename_buffer = None;
        cx.notify();
    }

    pub fn move_profile_to_group(&mut self, profile_id: Uuid, target_group: Uuid, cx: &mut Context<Self>) {
        let Some(profile) = self.workspace.remove_profile(profile_id) else {
            return;
        };
        if let Some(g) = self.workspace.find_group_mut(target_group) {
            g.profiles.push(profile);
            g.collapsed = false;
        }
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn filtered_groups(&self) -> Vec<(Uuid, String, bool, Vec<Profile>)> {
        let q = self.search.trim().to_lowercase();
        self.workspace
            .groups
            .iter()
            .filter_map(|g| {
                let profiles: Vec<Profile> = g
                    .profiles
                    .iter()
                    .filter(|p| {
                        if q.is_empty() {
                            return true;
                        }
                        p.name.to_lowercase().contains(&q)
                            || p.kind.search_haystack().to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect();
                if q.is_empty() || !profiles.is_empty() || g.name.to_lowercase().contains(&q) {
                    let collapsed = if !q.is_empty() { false } else { g.collapsed };
                    Some((g.id, g.name.clone(), collapsed, profiles))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn sync_open_tabs(&mut self, tabs: &[(Uuid, Option<String>)], active: usize) {
        self.ui_state.open_tabs = tabs
            .iter()
            .map(|(pid, title)| OpenTabRef {
                profile_id: *pid,
                title: title.clone(),
            })
            .collect();
        self.ui_state.active_tab_index = active;
        self.mark_dirty();
    }

    pub fn default_local_profile_id(&self) -> Option<Uuid> {
        self.workspace.first_local_profile_id()
    }

    pub fn profile_kind(&self, id: Uuid) -> Option<&ProfileKind> {
        self.workspace.find_profile(id).map(|p| &p.kind)
    }

    pub fn replace_workspace(&mut self, workspace: WorkspaceFile, cx: &mut Context<Self>) {
        self.workspace = workspace;
        self.selection = Selection::None;
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }
}

fn unique_name(prefix: &str, existing: &[String]) -> String {
    let mut n = 1;
    loop {
        let candidate = format!("{prefix} {n}");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}
