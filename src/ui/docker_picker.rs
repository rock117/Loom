//! Overlay: pick Docker host (Local | SSH Profile) → Name + container → Save / Save & Open.
//!
//! - **Local** — `docker ps` on this machine; saves a Local profile (`docker exec …`).
//! - **SSH** — pick an existing SSH profile, list containers via
//!   [`docker::list_running_containers_ssh`] on a background thread; saves a
//!   Docker-over-SSH profile (`new_ssh_docker_profile`). Password auth reuses the
//!   host profile keyring; missing password emits [`DockerPickerEvent::NeedSshPassword`].
//!
//! See `docs/DOCKER_SESSION.md`.

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::{Profile, ProfileKind, SshAuth};
use crate::session::credentials;
use crate::session::docker::{self, ContainerInfo};
use crate::session::ssh::SshAuthMaterial;
use crate::shared::theme;
use crate::ui::rename_edit::{RenameEdit, typed_text_from_keystroke};
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum DockerPickerEvent {
    Close,
    Saved {
        profile_id: Uuid,
        connect: bool,
    },
    /// Password auth required for listing containers on this SSH host profile.
    NeedSshPassword {
        profile_id: Uuid,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostMode {
    Local,
    Ssh,
}

#[derive(Clone)]
enum ContainerList {
    Loading,
    Ready(Vec<ContainerInfo>),
    Error(String),
}

#[derive(Clone)]
struct SshProfileRow {
    id: Uuid,
    name: String,
    summary: String,
}

pub struct DockerPicker {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    mode: HostMode,
    containers: ContainerList,
    selected_container: Option<String>,
    ssh_profiles: Vec<SshProfileRow>,
    selected_ssh: Option<Uuid>,
    name: RenameEdit,
    name_touched: bool,
    error: Option<String>,
    selecting: bool,
    name_bounds: Option<Bounds<Pixels>>,
    _caret_blink: Option<Task<()>>,
    _load_task: Option<Task<()>>,
}

impl DockerPicker {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let mut picker = Self {
            store,
            focus_handle: cx.focus_handle(),
            mode: HostMode::Local,
            containers: ContainerList::Loading,
            selected_container: None,
            ssh_profiles: Vec::new(),
            selected_ssh: None,
            name: field_edit(""),
            name_touched: false,
            error: None,
            selecting: false,
            name_bounds: None,
            _caret_blink: None,
            _load_task: None,
        };
        picker.reload_ssh_profiles(cx);
        picker.refresh_local(cx);
        picker.start_caret_blink(cx);
        picker
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.mode = HostMode::Local;
        self.selected_container = None;
        self.selected_ssh = None;
        self.name = field_edit("");
        self.name_touched = false;
        self.error = None;
        self.selecting = false;
        self.reload_ssh_profiles(cx);
        self.refresh_local(cx);
        self.start_caret_blink(cx);
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        self.name.caret_visible = true;
        self._caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.name.caret_visible = !this.name.caret_visible;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn reload_ssh_profiles(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let mut rows = Vec::new();
        collect_ssh_profiles(&store.workspace, &mut rows);
        rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.ssh_profiles = rows;
    }

    fn refresh_local(&mut self, cx: &mut Context<Self>) {
        self.containers = ContainerList::Loading;
        self.selected_container = None;
        self.error = None;
        cx.notify();

        let (tx, rx) = flume::bounded(1);
        let _ = std::thread::Builder::new()
            .name("loom-docker-ps".into())
            .spawn(move || {
                let result = docker::list_running_containers();
                let _ = tx.send(result);
            });

        self._load_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.apply_list_result(result, cx);
            })
            .ok();
        }));
    }

    /// List containers on the selected SSH host (background thread).
    ///
    /// When `password` is `None`, resolves auth like TabManager (keyring / key file).
    /// Missing password emits [`DockerPickerEvent::NeedSshPassword`].
    fn refresh_ssh(&mut self, password: Option<String>, cx: &mut Context<Self>) {
        let Some(profile_id) = self.selected_ssh else {
            self.containers = ContainerList::Ready(Vec::new());
            self.selected_container = None;
            self.error = None;
            cx.notify();
            return;
        };

        let Some(profile) = self
            .store
            .read(cx)
            .workspace
            .find_profile(profile_id)
            .cloned()
        else {
            self.containers = ContainerList::Error("SSH profile not found".into());
            self.selected_container = None;
            cx.notify();
            return;
        };

        let ProfileKind::Ssh {
            host, port, user, ..
        } = profile.kind.clone()
        else {
            self.containers = ContainerList::Error("Not an SSH profile".into());
            self.selected_container = None;
            cx.notify();
            return;
        };

        let auth = match resolve_ssh_auth(&profile, password) {
            Ok(Some(auth)) => auth,
            Ok(None) => {
                self.containers = ContainerList::Loading;
                self.selected_container = None;
                self.error = None;
                cx.notify();
                cx.emit(DockerPickerEvent::NeedSshPassword { profile_id });
                return;
            }
            Err(err) => {
                self.containers = ContainerList::Error(format!("{err:#}"));
                self.selected_container = None;
                cx.notify();
                return;
            }
        };

        self.containers = ContainerList::Loading;
        self.selected_container = None;
        self.error = None;
        cx.notify();

        let (tx, rx) = flume::bounded(1);
        let _ = std::thread::Builder::new()
            .name("loom-docker-ps-ssh".into())
            .spawn(move || {
                let result = docker::list_running_containers_ssh(&host, port, &user, auth);
                let _ = tx.send(result);
            });

        self._load_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.apply_list_result(result, cx);
            })
            .ok();
        }));
    }

    /// After [`DockerPickerEvent::NeedSshPassword`], workspace submits the password here.
    pub fn continue_ssh_list(
        &mut self,
        profile_id: Uuid,
        password: String,
        cx: &mut Context<Self>,
    ) {
        self.mode = HostMode::Ssh;
        self.selected_ssh = Some(profile_id);
        self.error = None;
        self.refresh_ssh(Some(password), cx);
    }

    fn apply_list_result(
        &mut self,
        result: Result<Result<Vec<ContainerInfo>, anyhow::Error>, flume::RecvError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(Ok(list)) => {
                if let Some(first) = list.first() {
                    self.apply_container_default_name(&first.display_label(), cx);
                    self.selected_container = Some(first.id.clone());
                } else {
                    self.selected_container = None;
                }
                self.containers = ContainerList::Ready(list);
            }
            Ok(Err(err)) => {
                self.containers = ContainerList::Error(format!("{err:#}"));
                self.selected_container = None;
            }
            Err(_) => {
                self.containers = ContainerList::Error("List cancelled".into());
                self.selected_container = None;
            }
        }
        cx.notify();
    }

    fn select_ssh_profile(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selected_ssh = Some(id);
        self.name_touched = false;
        self.error = None;
        self.refresh_ssh(None, cx);
    }

    fn refresh_current(&mut self, cx: &mut Context<Self>) {
        self.name_touched = false;
        match self.mode {
            HostMode::Local => self.refresh_local(cx),
            HostMode::Ssh => {
                self.reload_ssh_profiles(cx);
                self.refresh_ssh(None, cx);
            }
        }
    }

    fn apply_container_default_name(&mut self, label: &str, cx: &App) {
        if self.name_touched {
            return;
        }
        let name = self.unique_profile_name(label, cx);
        self.name = field_edit(name);
    }

    fn select_container(&mut self, id: String, cx: &mut Context<Self>) {
        let label = match &self.containers {
            ContainerList::Ready(list) => list
                .iter()
                .find(|c| c.id == id || c.short_id() == id)
                .map(|c| c.display_label())
                .unwrap_or_else(|| id.clone()),
            _ => id.clone(),
        };
        self.apply_container_default_name(&label, cx);
        self.selected_container = Some(id);
        self.error = None;
        cx.notify();
    }

    fn unique_profile_name(&self, base: &str, cx: &App) -> String {
        let existing = self.store.read(cx).workspace.all_profile_names();
        if !existing.iter().any(|n| n == base) {
            return base.to_string();
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base} {n}");
            if !existing.iter().any(|e| e == &candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn resolve_save_name(&self, container_label: &str, cx: &App) -> String {
        let typed = self.name.text.trim();
        let base = if typed.is_empty() {
            container_label
        } else {
            typed
        };
        self.unique_profile_name(base, cx)
    }

    fn save(&mut self, connect: bool, cx: &mut Context<Self>) {
        let ContainerList::Ready(ref list) = self.containers else {
            self.error = Some("Wait for the container list to load.".into());
            cx.notify();
            return;
        };
        let Some(ref id) = self.selected_container else {
            self.error = Some("Select a container".into());
            cx.notify();
            return;
        };
        let Some(row) = list
            .iter()
            .find(|c| &c.id == id || c.short_id() == id)
        else {
            self.error = Some("Select a container".into());
            cx.notify();
            return;
        };
        let name = self.resolve_save_name(&row.display_label(), cx);
        if name.is_empty() {
            self.error = Some("Name is required".into());
            cx.notify();
            return;
        }

        let profile = match self.mode {
            HostMode::Local => docker::new_profile(name, &row.id),
            HostMode::Ssh => {
                let Some(host_id) = self.selected_ssh else {
                    self.error = Some("Select an SSH profile".into());
                    cx.notify();
                    return;
                };
                let Some(host_profile) = self
                    .store
                    .read(cx)
                    .workspace
                    .find_profile(host_id)
                    .cloned()
                else {
                    self.error = Some("SSH profile not found".into());
                    cx.notify();
                    return;
                };
                let ProfileKind::Ssh {
                    host,
                    port,
                    user,
                    auth,
                    ..
                } = host_profile.kind
                else {
                    self.error = Some("Not an SSH profile".into());
                    cx.notify();
                    return;
                };
                docker::new_ssh_docker_profile(name, host, port, user, auth, &row.id)
            }
        };

        let host_ssh_id = self.selected_ssh.filter(|_| self.mode == HostMode::Ssh);
        let profile_id = self.store.update(cx, |s, cx| {
            let target = s.insert_target();
            s.place_profile(profile, target, cx)
        });

        if let Some(host_id) = host_ssh_id {
            if let Ok(Some(pw)) = credentials::get_password(host_id) {
                let _ = credentials::set_password(profile_id, &pw);
            }
        }

        cx.emit(DockerPickerEvent::Saved {
            profile_id,
            connect,
        });
    }

    fn can_save(&self) -> bool {
        let list_ready =
            matches!(&self.containers, ContainerList::Ready(_)) && self.selected_container.is_some();
        match self.mode {
            HostMode::Local => list_ready,
            HostMode::Ssh => list_ready && self.selected_ssh.is_some(),
        }
    }

    fn handle_edit_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let chord = mods.control || mods.platform;
        let shift = mods.shift;

        if chord && key.eq_ignore_ascii_case("a") {
            self.name.select_all();
            self.name_touched = true;
            return true;
        }
        if chord && key.eq_ignore_ascii_case("c") {
            let text = if self.name.has_selection() {
                self.name.selected_text()
            } else {
                self.name.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return true;
        }
        if chord && key.eq_ignore_ascii_case("x") {
            let text = if self.name.has_selection() {
                self.name.selected_text()
            } else {
                self.name.text.clone()
            };
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            if self.name.has_selection() {
                self.name.delete_selection();
            } else {
                self.name.text.clear();
                self.name.cursor = 0;
                self.name.anchor = 0;
            }
            self.name_touched = true;
            return true;
        }
        if (chord && key.eq_ignore_ascii_case("v"))
            || (mods.shift && key.eq_ignore_ascii_case("insert"))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                let cleaned = text.replace('\r', "").replace('\n', "");
                if !cleaned.is_empty() {
                    self.name.insert(&cleaned);
                    self.name_touched = true;
                }
            }
            return true;
        }
        if key == "backspace" {
            self.name.backspace();
            self.name_touched = true;
            return true;
        }
        if key == "delete" {
            self.name.delete_forward();
            self.name_touched = true;
            return true;
        }
        if key == "left" {
            self.name.move_left(shift);
            return true;
        }
        if key == "right" {
            self.name.move_right(shift);
            return true;
        }
        if key == "home" {
            self.name.move_home(shift);
            return true;
        }
        if key == "end" {
            self.name.move_end(shift);
            return true;
        }
        if chord {
            return false;
        }
        if let Some(cleaned) = typed_text_from_keystroke(&event.keystroke) {
            self.name.insert(&cleaned);
            self.name_touched = true;
            return true;
        }
        false
    }

    fn index_at_pointer(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.name_bounds else {
            return self.name.char_len();
        };
        let pad: f32 = theme::SPACE_2;
        let local_x: f32 = (position.x - bounds.origin.x).into();
        char_index_at_x(&self.name, local_x - pad)
    }

    fn begin_mouse_select(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let extend = event.modifiers.shift;
        if event.click_count >= 2 {
            self.name.select_all();
            self.selecting = false;
        } else {
            let idx = self.index_at_pointer(event.position);
            self.name.set_caret(idx, extend);
            self.selecting = true;
        }
        self.name.caret_visible = true;
        cx.notify();
    }

    fn update_mouse_select(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let idx = self.index_at_pointer(position);
        self.name.set_caret(idx, true);
        self.name.caret_visible = true;
        cx.notify();
    }

    fn end_mouse_select(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
    }

    fn char_to_utf16(text: &str, char_idx: usize) -> usize {
        text.chars().take(char_idx).map(|c| c.len_utf16()).sum()
    }

    fn utf16_to_char(text: &str, utf16_idx: usize) -> usize {
        let mut u = 0;
        for (i, c) in text.chars().enumerate() {
            if u >= utf16_idx {
                return i;
            }
            u += c.len_utf16();
        }
        text.chars().count()
    }
}

fn collect_ssh_profiles(ws: &crate::model::WorkspaceFile, out: &mut Vec<SshProfileRow>) {
    for p in &ws.profiles {
        if matches!(p.kind, crate::model::ProfileKind::Ssh { .. }) {
            out.push(SshProfileRow {
                id: p.id,
                name: p.name.clone(),
                summary: p.kind.summary(),
            });
        }
    }
    for g in &ws.groups {
        collect_ssh_in_group(g, out);
    }
}

fn collect_ssh_in_group(g: &crate::model::Group, out: &mut Vec<SshProfileRow>) {
    for p in &g.profiles {
        if matches!(p.kind, crate::model::ProfileKind::Ssh { .. }) {
            out.push(SshProfileRow {
                id: p.id,
                name: p.name.clone(),
                summary: p.kind.summary(),
            });
        }
    }
    for c in &g.children {
        collect_ssh_in_group(c, out);
    }
}

/// Same priority as TabManager: prompt override → OS keyring / private key path.
fn resolve_ssh_auth(
    profile: &Profile,
    password_override: Option<String>,
) -> anyhow::Result<Option<SshAuthMaterial>> {
    let ProfileKind::Ssh { auth, .. } = &profile.kind else {
        anyhow::bail!("not an SSH profile");
    };
    match auth {
        SshAuth::Password { .. } => {
            if let Some(p) = password_override {
                return Ok(Some(SshAuthMaterial::Password(p)));
            }
            Ok(credentials::get_password(profile.id)?.map(SshAuthMaterial::Password))
        }
        SshAuth::PrivateKey { path } => Ok(Some(SshAuthMaterial::PrivateKey {
            path: path.clone(),
            passphrase: None,
        })),
    }
}

impl Focusable for DockerPicker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DockerPickerEvent> for DockerPicker {}

impl EntityInputHandler for DockerPicker {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let utf16: Vec<u16> = self.name.text.encode_utf16().collect();
        let start = range.start.min(utf16.len());
        let end = range.end.min(utf16.len());
        *adjusted_range = Some(start..end);
        String::from_utf16(&utf16[start..end]).ok()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let (lo, hi) = self.name.sel_range();
        let start = Self::char_to_utf16(&self.name.text, lo);
        let end = Self::char_to_utf16(&self.name.text, hi);
        Some(UTF16Selection {
            range: start..end,
            reversed: self.name.cursor < self.name.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_range = range.is_some();
        if let Some(r) = range {
            let lo = Self::utf16_to_char(&self.name.text, r.start);
            let hi = Self::utf16_to_char(&self.name.text, r.end);
            self.name.anchor = lo;
            self.name.cursor = hi;
        }
        let cleaned = text.replace('\r', "").replace('\n', "");
        if !cleaned.is_empty() || had_range {
            if cleaned.is_empty() {
                self.name.delete_selection();
            } else {
                self.name.insert(&cleaned);
            }
            self.name_touched = true;
            self.name.caret_visible = true;
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.name_bounds
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (lo, _) = self.name.sel_range();
        let _ = point;
        Some(Self::char_to_utf16(&self.name.text, lo))
    }
}

impl Render for DockerPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;
        let containers = self.containers.clone();
        let selected_c = self.selected_container.clone();
        let selected_ssh = self.selected_ssh;
        let ssh_profiles = self.ssh_profiles.clone();
        let err = self.error.clone();
        let name_edit = self.name.clone();
        let view = cx.entity();
        let can_save = self.can_save();
        let show_name_and_containers =
            mode == HostMode::Local || (mode == HostMode::Ssh && selected_ssh.is_some());

        div()
            .id("docker-picker-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(DockerPickerEvent::Close)),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.update_mouse_select(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.end_mouse_select(cx);
                }),
            )
            .child(
                div()
                    .id("docker-picker-card")
                    .w(px(480.0))
                    .max_h(px(600.0))
                    .p_5()
                    .rounded(px(8.0))
                    .bg(theme::PANEL_BG)
                    .border_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .track_focus(&self.focus_handle)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.focus_handle.focus(window);
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        if key == "escape" {
                            cx.emit(DockerPickerEvent::Close);
                            cx.stop_propagation();
                            return;
                        }
                        if key == "enter" && this.can_save() {
                            this.save(true, cx);
                            cx.stop_propagation();
                            return;
                        }
                        if this.handle_edit_key(event, cx) {
                            this.name.caret_visible = true;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::TEXT)
                                    .child("New Docker Profile"),
                            )
                            .child(
                                div()
                                    .id("docker-refresh")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                                    .cursor_pointer()
                                    .child("Refresh")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_current(cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(
                                "Pick a host and container, set the profile name. Opens docker exec -it.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(mode_tab("Local", mode == HostMode::Local, cx, |this, _, cx| {
                                this.mode = HostMode::Local;
                                this.error = None;
                                this.name_touched = false;
                                this.refresh_local(cx);
                            }))
                            .child(mode_tab(
                                "SSH Profile",
                                mode == HostMode::Ssh,
                                cx,
                                |this, _, cx| {
                                    this.mode = HostMode::Ssh;
                                    this.error = None;
                                    this.reload_ssh_profiles(cx);
                                    if this.selected_ssh.is_some() {
                                        this.name_touched = false;
                                        this.refresh_ssh(None, cx);
                                    } else {
                                        this.containers = ContainerList::Ready(Vec::new());
                                        this.selected_container = None;
                                        cx.notify();
                                    }
                                },
                            )),
                    )
                    .when(mode == HostMode::Ssh, |d| {
                        d.child(ssh_profiles_body(ssh_profiles, selected_ssh, cx))
                    })
                    .when(show_name_and_containers, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("Name"),
                                )
                                .child(
                                    div()
                                        .id("docker-name-field")
                                        .relative()
                                        .w_full()
                                        .px(px(theme::SPACE_2))
                                        .py(px(theme::SPACE_1))
                                        .rounded(px(theme::RADIUS_SM))
                                        .bg(theme::ELEVATED)
                                        .border_1()
                                        .border_color(theme::ACCENT)
                                        .cursor_text()
                                        .overflow_hidden()
                                        .child({
                                            let view = view.clone();
                                            let view_paint = view.clone();
                                            canvas(
                                                move |bounds, _, cx| {
                                                    view.update(cx, |this, _| {
                                                        this.name_bounds = Some(bounds);
                                                    });
                                                    bounds
                                                },
                                                move |bounds, _, window, cx| {
                                                    let focus =
                                                        view_paint.read(cx).focus_handle.clone();
                                                    window.handle_input(
                                                        &focus,
                                                        ElementInputHandler::new(
                                                            bounds,
                                                            view_paint.clone(),
                                                        ),
                                                        cx,
                                                    );
                                                },
                                            )
                                            .absolute()
                                            .size_full()
                                        })
                                        .child(name_edit.into_element_bare())
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(
                                                |this, event: &MouseDownEvent, window, cx| {
                                                    this.begin_mouse_select(event, window, cx);
                                                    cx.stop_propagation();
                                                },
                                            ),
                                        ),
                                ),
                        )
                        .child(containers_body(
                            if mode == HostMode::Local {
                                "Running containers (this machine)"
                            } else {
                                "Running containers (via SSH)"
                            },
                            containers,
                            selected_c,
                            cx,
                        ))
                    })
                    .when_some(err, |d, msg| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::DANGER)
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(form_btn("Cancel", false, true, cx, |_, _, cx| {
                                cx.emit(DockerPickerEvent::Close);
                            }))
                            .child(form_btn("Save", false, can_save, cx, move |this, _, cx| {
                                if can_save {
                                    this.save(false, cx);
                                }
                            }))
                            .child(form_btn(
                                "Save & Open",
                                true,
                                can_save,
                                cx,
                                move |this, _, cx| {
                                    if can_save {
                                        this.save(true, cx);
                                    }
                                },
                            )),
                    ),
            )
    }
}

fn mode_tab(
    label: &'static str,
    active: bool,
    cx: &mut Context<DockerPicker>,
    on_click: impl Fn(&mut DockerPicker, &mut Window, &mut Context<DockerPicker>) + 'static,
) -> impl IntoElement {
    let id = SharedString::from(format!("docker-mode-{label}"));
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded(px(4.0))
        .text_xs()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .bg(if active {
            theme::ELEVATED
        } else {
            theme::PANEL_BG
        })
        .text_color(if active {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        })
        .border_1()
        .border_color(if active {
            theme::ACCENT
        } else {
            theme::BORDER_SUBTLE
        })
        .cursor_pointer()
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            on_click(this, window, cx);
            cx.stop_propagation();
        }))
}

fn containers_body(
    label: &'static str,
    list: ContainerList,
    selected: Option<String>,
    cx: &mut Context<DockerPicker>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .text_xs()
                .text_color(theme::TEXT_MUTED)
                .child(label),
        )
        .child(
            div()
                .id("docker-container-list")
                .flex_1()
                .min_h(px(160.0))
                .max_h(px(240.0))
                .overflow_y_scroll()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme::BORDER_SUBTLE)
                .bg(theme::BG)
                .p_1()
                .child(match &list {
                    ContainerList::Loading => div()
                        .p_3()
                        .text_sm()
                        .text_color(theme::TEXT_MUTED)
                        .child("Loading…")
                        .into_any_element(),
                    ContainerList::Error(err) => div()
                        .p_3()
                        .text_sm()
                        .text_color(theme::DANGER)
                        .child(err.clone())
                        .into_any_element(),
                    ContainerList::Ready(rows) if rows.is_empty() => div()
                        .p_3()
                        .text_sm()
                        .text_color(theme::TEXT_MUTED)
                        .child("No running containers. Start one, then Refresh.")
                        .into_any_element(),
                    ContainerList::Ready(rows) => div()
                        .flex()
                        .flex_col()
                        .children(rows.iter().map(|c| {
                            let id = c.id.clone();
                            let id_click = id.clone();
                            let is_sel = selected
                                .as_ref()
                                .is_some_and(|s| s == &c.id || s == c.short_id());
                            let title = c.display_label();
                            let detail = format!("{} · {}", c.short_id(), c.image);
                            let status = c.status.clone();
                            div()
                                .id(SharedString::from(format!("docker-c-{id}")))
                                .px_2()
                                .py_1()
                                .rounded(px(3.0))
                                .cursor_pointer()
                                .bg(if is_sel {
                                    theme::SELECTION
                                } else {
                                    theme::BG
                                })
                                .hover(|s| {
                                    if is_sel {
                                        s
                                    } else {
                                        s.bg(theme::HOVER)
                                    }
                                })
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::TEXT)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(detail),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(status),
                                )
                                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                    this.select_container(id_click.clone(), cx);
                                    if event.click_count() >= 2 {
                                        this.save(true, cx);
                                    }
                                    cx.stop_propagation();
                                }))
                        }))
                        .into_any_element(),
                }),
        )
}

fn ssh_profiles_body(
    profiles: Vec<SshProfileRow>,
    selected: Option<Uuid>,
    cx: &mut Context<DockerPicker>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .text_xs()
                .text_color(theme::TEXT_MUTED)
                .child("SSH Profile (host)"),
        )
        .child(
            div()
                .id("docker-ssh-list")
                .flex_1()
                .min_h(px(120.0))
                .max_h(px(200.0))
                .overflow_y_scroll()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme::BORDER_SUBTLE)
                .bg(theme::BG)
                .p_1()
                .child(if profiles.is_empty() {
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(theme::TEXT_MUTED)
                        .child(
                            "No SSH profiles yet. Create one from the sidebar, then return here.",
                        )
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .children(profiles.into_iter().map(|p| {
                            let id = p.id;
                            let is_sel = selected == Some(id);
                            div()
                                .id(SharedString::from(format!("docker-ssh-{id}")))
                                .px_2()
                                .py_1()
                                .rounded(px(3.0))
                                .cursor_pointer()
                                .bg(if is_sel {
                                    theme::SELECTION
                                } else {
                                    theme::BG
                                })
                                .hover(|s| {
                                    if is_sel {
                                        s
                                    } else {
                                        s.bg(theme::HOVER)
                                    }
                                })
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::TEXT)
                                        .child(p.name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child(p.summary),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_ssh_profile(id, cx);
                                    cx.stop_propagation();
                                }))
                        }))
                        .into_any_element()
                }),
        )
}

fn field_edit(text: impl Into<String>) -> RenameEdit {
    let mut edit = RenameEdit::new(text);
    edit.move_end(false);
    edit
}

fn char_index_at_x(edit: &RenameEdit, local_x: f32) -> usize {
    const AVG_CHAR_W: f32 = 7.4;
    if local_x <= 0.0 {
        return 0;
    }
    let idx = (local_x / AVG_CHAR_W).round() as usize;
    idx.min(edit.char_len())
}

fn form_btn(
    label: &'static str,
    primary: bool,
    enabled: bool,
    cx: &mut Context<DockerPicker>,
    on_click: impl Fn(&mut DockerPicker, &mut Window, &mut Context<DockerPicker>) + 'static,
) -> impl IntoElement {
    let id = SharedString::from(format!("docker-btn-{label}"));
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded(px(theme::RADIUS_SM))
        .text_sm()
        .when(enabled, |d| d.cursor_pointer())
        .when(primary && enabled, |d| {
            d.bg(theme::ACCENT)
                .text_color(rgb(0xffffff))
                .hover(|s| s.opacity(0.9))
        })
        .when(primary && !enabled, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT_MUTED)
                .opacity(0.55)
        })
        .when(!primary && enabled, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT)
                .hover(|s| s.bg(theme::HOVER))
        })
        .when(!primary && !enabled, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT_MUTED)
                .opacity(0.55)
        })
        .child(label)
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, window, cx| {
                on_click(this, window, cx);
                cx.stop_propagation();
            }))
        })
}
