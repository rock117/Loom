//! Overlay: pick Docker host mode (Local | SSH Profile) → container → open exec Tab.
//!
//! Phase 1: Local lists `docker ps` and opens exec. SSH Profile mode is shown but
//! remote listing is deferred (see `docs/DOCKER_SESSION.md`).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::session::docker::{self, ContainerInfo};
use crate::shared::theme;
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum DockerPickerEvent {
    Close,
    /// Open ephemeral local `docker exec` for this container.
    OpenLocal {
        container_id: String,
        label: String,
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
            _load_task: None,
        };
        picker.reload_ssh_profiles(cx);
        picker.refresh_local(cx);
        picker
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.mode = HostMode::Local;
        self.selected_container = None;
        self.selected_ssh = None;
        self.reload_ssh_profiles(cx);
        self.refresh_local(cx);
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
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
                match result {
                    Ok(Ok(list)) => {
                        this.selected_container = list.first().map(|c| c.id.clone());
                        this.containers = ContainerList::Ready(list);
                    }
                    Ok(Err(err)) => {
                        this.containers = ContainerList::Error(format!("{err:#}"));
                        this.selected_container = None;
                    }
                    Err(_) => {
                        this.containers = ContainerList::Error("List cancelled".into());
                        this.selected_container = None;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn confirm_open(&mut self, cx: &mut Context<Self>) {
        if self.mode != HostMode::Local {
            return;
        }
        let ContainerList::Ready(ref list) = self.containers else {
            return;
        };
        let Some(ref id) = self.selected_container else {
            return;
        };
        let Some(row) = list
            .iter()
            .find(|c| &c.id == id || c.short_id() == id)
        else {
            return;
        };
        cx.emit(DockerPickerEvent::OpenLocal {
            container_id: row.id.clone(),
            label: row.display_label(),
        });
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

impl Focusable for DockerPicker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DockerPickerEvent> for DockerPicker {}

impl Render for DockerPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;
        let containers = self.containers.clone();
        let selected_c = self.selected_container.clone();
        let selected_ssh = self.selected_ssh;
        let ssh_profiles = self.ssh_profiles.clone();
        let can_open = mode == HostMode::Local
            && matches!(&containers, ContainerList::Ready(_))
            && selected_c.is_some();

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
            .child(
                div()
                    .id("docker-picker-card")
                    .w(px(480.0))
                    .max_h(px(560.0))
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
                        if key == "enter" {
                            this.confirm_open(cx);
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
                                    .child("Open Docker container"),
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
                                        if this.mode == HostMode::Local {
                                            this.refresh_local(cx);
                                        } else {
                                            this.reload_ssh_profiles(cx);
                                            cx.notify();
                                        }
                                        cx.stop_propagation();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(
                                "Choose a host, then a running container. Opens an interactive shell (docker exec).",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(mode_tab("Local", mode == HostMode::Local, cx, |this, _, cx| {
                                this.mode = HostMode::Local;
                                cx.notify();
                            }))
                            .child(mode_tab(
                                "SSH Profile",
                                mode == HostMode::Ssh,
                                cx,
                                |this, _, cx| {
                                    this.mode = HostMode::Ssh;
                                    this.reload_ssh_profiles(cx);
                                    cx.notify();
                                },
                            )),
                    )
                    .child(match mode {
                        HostMode::Local => local_body(containers, selected_c, cx),
                        HostMode::Ssh => ssh_body(ssh_profiles, selected_ssh, cx),
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_1()
                            .child(form_btn("Cancel", false, true, cx, |_, _, cx| {
                                cx.emit(DockerPickerEvent::Close);
                            }))
                            .child(form_btn("Open", true, can_open, cx, |this, _, cx| {
                                this.confirm_open(cx);
                            })),
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

fn local_body(
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
                .child("Running containers (this machine)"),
        )
        .child(
            div()
                .id("docker-container-list")
                .flex_1()
                .min_h(px(180.0))
                .max_h(px(280.0))
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
                                    this.selected_container = Some(id_click.clone());
                                    if event.click_count() >= 2 {
                                        this.confirm_open(cx);
                                    }
                                    cx.notify();
                                    cx.stop_propagation();
                                }))
                        }))
                        .into_any_element(),
                }),
        )
}

fn ssh_body(
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
                                    this.selected_ssh = Some(id);
                                    cx.notify();
                                    cx.stop_propagation();
                                }))
                        }))
                        .into_any_element()
                }),
        )
        .child(
            div()
                .mt_1()
                .p_3()
                .rounded(px(4.0))
                .bg(theme::ELEVATED)
                .border_1()
                .border_color(theme::BORDER_SUBTLE)
                .text_xs()
                .text_color(theme::TEXT_MUTED)
                .child(
                    "Remote Docker over SSH is planned next: pick a profile here, then list containers on that host. Not available in this build yet.",
                ),
        )
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
        .when(!primary, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT)
                .hover(|s| s.bg(theme::HOVER))
        })
        .child(label)
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, window, cx| {
                on_click(this, window, cx);
                cx.stop_propagation();
            }))
        })
}
