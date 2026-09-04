use gpui::*;
use uuid::Uuid;

use crate::model::{
    Group, OpenTabRef, OrderKey, Profile, ProfileKind, SettingsFile, UiStateFile, WorkspaceFile,
    load_settings, load_ui_state, load_workspace, save_settings, save_ui_state, save_workspace,
};
use crate::ui::rename_edit::RenameEdit;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Group(Uuid),
    Profile(Uuid),
}

/// Where to place a newly created profile or group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertTarget {
    /// Workspace root (`/`).
    Root,
    /// Inside this group.
    Group(Uuid),
}

pub struct WorkspaceStore {
    pub workspace: WorkspaceFile,
    pub ui_state: UiStateFile,
    pub settings: SettingsFile,
    pub selection: Selection,
    pub rename: Option<RenameEdit>,
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
            rename: None,
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

    /// New* placement from current selection (Unix `/` rules).
    pub fn insert_target(&self) -> InsertTarget {
        match self.selection {
            Selection::Group(id) => InsertTarget::Group(id),
            Selection::Profile(pid) => match self.workspace.group_id_for_profile(pid) {
                Some(gid) => InsertTarget::Group(gid),
                None => InsertTarget::Root,
            },
            Selection::None => InsertTarget::Root,
        }
    }

    pub fn select_group(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.selection != Selection::Group(id) {
            self.rename = None;
        }
        self.selection = Selection::Group(id);
        cx.notify();
    }

    pub fn select_profile(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.selection != Selection::Profile(id) {
            self.rename = None;
        }
        self.selection = Selection::Profile(id);
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
        let sibling_names: Vec<String> = match self.insert_target() {
            InsertTarget::Root => self.workspace.groups.iter().map(|g| g.name.clone()).collect(),
            InsertTarget::Group(gid) => self
                .workspace
                .find_group(gid)
                .map(|g| g.children.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default(),
        };
        let name = unique_name("Group", &sibling_names);
        let group = Group::new(name);
        let id = group.id;
        match self.insert_target() {
            InsertTarget::Root => self.workspace.groups.push(group),
            InsertTarget::Group(gid) => {
                if let Some(parent) = self.workspace.find_group_mut(gid) {
                    parent.children.push(group);
                    parent.collapsed = false;
                } else {
                    self.workspace.groups.push(group);
                }
            }
        }
        self.workspace.sync_orders();
        self.selection = Selection::Group(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn add_local_profile(&mut self, cx: &mut Context<Self>) -> Option<Uuid> {
        let names = self.workspace.all_profile_names();
        let name = unique_name("Shell", &names);
        let profile = Profile::new_local(name);
        let id = profile.id;
        match self.insert_target() {
            InsertTarget::Root => self.workspace.profiles.push(profile),
            InsertTarget::Group(gid) => {
                if let Some(g) = self.workspace.find_group_mut(gid) {
                    g.profiles.push(profile);
                    g.collapsed = false;
                } else {
                    self.workspace.profiles.push(profile);
                }
            }
        }
        self.workspace.sync_orders();
        self.selection = Selection::Profile(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        Some(id)
    }

    pub fn add_ssh_profile(&mut self, profile: Profile, cx: &mut Context<Self>) -> bool {
        let id = profile.id;
        match self.insert_target() {
            InsertTarget::Root => self.workspace.profiles.push(profile),
            InsertTarget::Group(gid) => {
                if let Some(g) = self.workspace.find_group_mut(gid) {
                    g.profiles.push(profile);
                    g.collapsed = false;
                } else {
                    self.workspace.profiles.push(profile);
                }
            }
        }
        self.workspace.sync_orders();
        self.selection = Selection::Profile(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        true
    }

    /// Insert an existing profile into root or a group (Save to… / move).
    pub fn place_profile(
        &mut self,
        profile: Profile,
        target: InsertTarget,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let id = profile.id;
        match target {
            InsertTarget::Root => self.workspace.profiles.push(profile),
            InsertTarget::Group(gid) => {
                if let Some(g) = self.workspace.find_group_mut(gid) {
                    g.profiles.push(profile);
                    g.collapsed = false;
                } else {
                    self.workspace.profiles.push(profile);
                }
            }
        }
        self.workspace.sync_orders();
        self.selection = Selection::Profile(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        id
    }

    /// Update an existing SSH profile's connection fields (keeps id / group).
    pub fn update_ssh_profile(
        &mut self,
        id: Uuid,
        name: String,
        host: String,
        port: u16,
        user: String,
        auth: crate::model::SshAuth,
        forwards: Vec<crate::model::PortForwardRule>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(profile) = self.workspace.find_profile_mut(id) else {
            return false;
        };
        if !matches!(profile.kind, crate::model::ProfileKind::Ssh { .. }) {
            return false;
        }
        profile.name = name;
        profile.kind = crate::model::ProfileKind::Ssh {
            host,
            port,
            user,
            auth,
        };
        profile.forwards = forwards;
        self.selection = Selection::Profile(id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        true
    }

    pub fn duplicate_profile(&mut self, id: Uuid, cx: &mut Context<Self>) -> Option<Uuid> {
        let parent = self.workspace.group_id_for_profile(id);
        let dup = self.workspace.find_profile(id)?.duplicate();
        let new_id = dup.id;
        match parent {
            None => {
                if let Some(pos) = self.workspace.profiles.iter().position(|p| p.id == id) {
                    self.workspace.profiles.insert(pos + 1, dup);
                } else {
                    self.workspace.profiles.push(dup);
                }
            }
            Some(gid) => {
                if let Some(g) = self.workspace.find_group_mut(gid) {
                    if let Some(pos) = g.profiles.iter().position(|p| p.id == id) {
                        g.profiles.insert(pos + 1, dup);
                    } else {
                        g.profiles.push(dup);
                    }
                }
            }
        }
        self.workspace.sync_orders();
        self.workspace
            .insert_order_key_after(OrderKey::Profile(new_id), OrderKey::Profile(id));
        self.selection = Selection::Profile(new_id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
        Some(new_id)
    }

    pub fn delete_selection(&mut self, cx: &mut Context<Self>) {
        match self.selection {
            Selection::Profile(id) => {
                let _ = crate::session::credentials::delete_password(id);
                self.workspace.remove_profile(id);
                self.workspace.sync_orders();
                self.selection = Selection::None;
                self.mark_dirty();
                self.persist_now();
                cx.notify();
            }
            Selection::Group(id) => {
                for pid in self.workspace.profile_ids_in_group(id) {
                    let _ = crate::session::credentials::delete_password(pid);
                }
                self.workspace.remove_group(id);
                if self.workspace.groups.is_empty() && self.workspace.profiles.is_empty() {
                    self.workspace = WorkspaceFile::default_workspace();
                } else {
                    self.workspace.sync_orders();
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
            Selection::Group(id) => self.workspace.find_group(id).map(|g| g.name.clone()),
            Selection::None => None,
        };
        if let Some(name) = name {
            self.rename = Some(RenameEdit::new(name));
            cx.notify();
        }
    }

    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.rename.take() else {
            return;
        };
        let name = edit.text.trim().to_string();
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

    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename = None;
        cx.notify();
    }

    pub fn toggle_rename_caret(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.rename {
            edit.caret_visible = !edit.caret_visible;
            cx.notify();
        }
    }

    pub fn with_rename(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut RenameEdit)) {
        if let Some(edit) = &mut self.rename {
            f(edit);
            cx.notify();
        }
    }

    pub fn move_profile_to_group(
        &mut self,
        profile_id: Uuid,
        target_group: Uuid,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.workspace.remove_profile(profile_id) else {
            return;
        };
        if let Some(g) = self.workspace.find_group_mut(target_group) {
            g.profiles.push(profile);
            g.collapsed = false;
        } else {
            self.workspace.profiles.push(profile);
        }
        self.workspace.sync_orders();
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn move_profile_to_root(&mut self, profile_id: Uuid, cx: &mut Context<Self>) {
        let Some(profile) = self.workspace.remove_profile(profile_id) else {
            return;
        };
        self.workspace.profiles.push(profile);
        self.workspace.sync_orders();
        self.selection = Selection::Profile(profile_id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    /// Move a profile into the group of `before_profile`, inserting before it.
    /// If `before_profile` is at root, insert before it among root profiles.
    pub fn move_profile_before(
        &mut self,
        profile_id: Uuid,
        before_profile: Uuid,
        cx: &mut Context<Self>,
    ) {
        if profile_id == before_profile {
            return;
        }
        let parent = self.workspace.group_id_for_profile(before_profile);
        let Some(profile) = self.workspace.remove_profile(profile_id) else {
            return;
        };
        match parent {
            None => {
                let pos = self
                    .workspace
                    .profiles
                    .iter()
                    .position(|p| p.id == before_profile)
                    .unwrap_or(self.workspace.profiles.len());
                self.workspace.profiles.insert(pos, profile);
            }
            Some(gid) => {
                if let Some(g) = self.workspace.find_group_mut(gid) {
                    let pos = g
                        .profiles
                        .iter()
                        .position(|p| p.id == before_profile)
                        .unwrap_or(g.profiles.len());
                    g.profiles.insert(pos, profile);
                    g.collapsed = false;
                } else {
                    self.workspace.profiles.push(profile);
                }
            }
        }
        self.workspace.sync_orders();
        self.workspace.insert_order_key_before(
            OrderKey::Profile(profile_id),
            OrderKey::Profile(before_profile),
        );
        self.selection = Selection::Profile(profile_id);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    /// Place `dragged` group immediately before `before` in the shared sibling order.
    pub fn reorder_group_before(
        &mut self,
        dragged: Uuid,
        before: Uuid,
        cx: &mut Context<Self>,
    ) {
        if dragged == before {
            return;
        }
        self.workspace.sync_orders();
        self.workspace
            .insert_order_key_before(OrderKey::Group(dragged), OrderKey::Group(before));
        self.selection = Selection::Group(dragged);
        self.mark_dirty();
        self.persist_now();
        cx.notify();
    }

    pub fn move_profile_up(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.workspace.move_profile_by(id, -1) {
            self.selection = Selection::Profile(id);
            self.mark_dirty();
            self.persist_now();
            cx.notify();
        }
    }

    pub fn move_profile_down(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.workspace.move_profile_by(id, 1) {
            self.selection = Selection::Profile(id);
            self.mark_dirty();
            self.persist_now();
            cx.notify();
        }
    }

    pub fn move_group_up(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.workspace.move_group_by(id, -1) {
            self.selection = Selection::Group(id);
            self.mark_dirty();
            self.persist_now();
            cx.notify();
        }
    }

    pub fn move_group_down(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.workspace.move_group_by(id, 1) {
            self.selection = Selection::Group(id);
            self.mark_dirty();
            self.persist_now();
            cx.notify();
        }
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

    /// Remember last Local shell cwd on a Bound profile (reopen / reconnect).
    pub fn update_local_profile_cwd(
        &mut self,
        profile_id: Uuid,
        cwd: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.workspace.find_profile_mut(profile_id) else {
            return;
        };
        let ProfileKind::Local { cwd: stored, .. } = &mut profile.kind else {
            return;
        };
        if stored.as_ref() == Some(&cwd) {
            return;
        }
        *stored = Some(cwd);
        self.mark_dirty();
        cx.notify();
    }

    pub fn default_local_profile_id(&self) -> Option<Uuid> {
        self.workspace.first_local_profile_id()
    }

    pub fn profile_kind(&self, id: Uuid) -> Option<&ProfileKind> {
        self.workspace.find_profile(id).map(|p| &p.kind)
    }

    pub fn replace_workspace(&mut self, workspace: WorkspaceFile, cx: &mut Context<Self>) {
        self.workspace = workspace;
        self.workspace.sync_orders();
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
