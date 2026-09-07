//! Overlay: pick an installed WSL distro → save as a Local profile (`wsl.exe -d …`).

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::model::Profile;
use crate::session::wsl;
use crate::shared::theme;
use crate::ui::workspace_store::WorkspaceStore;

#[derive(Clone, Debug)]
pub enum WslFormEvent {
    Close,
    Saved {
        profile_id: Uuid,
        connect: bool,
    },
}

#[derive(Clone)]
enum ListState {
    Loading,
    Ready(Vec<String>),
    Error(String),
}

pub struct WslForm {
    pub store: Entity<WorkspaceStore>,
    focus_handle: FocusHandle,
    list: ListState,
    selected: Option<String>,
    _load_task: Option<Task<()>>,
}

impl WslForm {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let mut form = Self {
            store,
            focus_handle: cx.focus_handle(),
            list: ListState::Loading,
            selected: None,
            _load_task: None,
        };
        form.refresh(cx);
        form
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.refresh(cx);
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.list = ListState::Loading;
        self.selected = None;
        cx.notify();

        let (tx, rx) = flume::bounded(1);
        let _ = std::thread::Builder::new()
            .name("loom-wsl-list".into())
            .spawn(move || {
                let result = wsl::list_distros();
                let _ = tx.send(result);
            });

        self._load_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.list = match result {
                    Ok(Ok(names)) if names.is_empty() => ListState::Error(
                        "No WSL distributions found. Install one from the Microsoft Store, then Refresh."
                            .into(),
                    ),
                    Ok(Ok(names)) => {
                        this.selected = names.first().cloned();
                        ListState::Ready(names)
                    }
                    Ok(Err(err)) => ListState::Error(format!("{err:#}")),
                    Err(_) => ListState::Error("WSL list cancelled".into()),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn unique_distro_profile_name(&self, distro: &str, cx: &App) -> String {
        let existing = self.store.read(cx).workspace.all_profile_names();
        if !existing.iter().any(|n| n == distro) {
            return distro.to_string();
        }
        let mut n = 2;
        loop {
            let candidate = format!("{distro} {n}");
            if !existing.iter().any(|e| e == &candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn save(&mut self, connect: bool, cx: &mut Context<Self>) {
        let Some(distro) = self.selected.clone() else {
            return;
        };
        let name = self.unique_distro_profile_name(&distro, cx);
        let profile = Profile::new_wsl(name, &distro);
        let id = self.store.update(cx, |s, cx| {
            let target = s.insert_target();
            s.place_profile(profile, target, cx)
        });
        cx.emit(WslFormEvent::Saved {
            profile_id: id,
            connect,
        });
    }
}

impl Focusable for WslForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<WslFormEvent> for WslForm {}

impl Render for WslForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.clone();
        let selected = self.selected.clone();
        let can_save = matches!(&list, ListState::Ready(_)) && selected.is_some();

        div()
            .id("wsl-form-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(WslFormEvent::Close)),
            )
            .child(
                div()
                    .id("wsl-form-card")
                    .w(px(400.0))
                    .max_h(px(480.0))
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
                        if event.keystroke.key == "escape" {
                            cx.emit(WslFormEvent::Close);
                            cx.stop_propagation();
                        } else if event.keystroke.key == "enter"
                            && matches!(&this.list, ListState::Ready(_))
                            && this.selected.is_some()
                        {
                            this.save(true, cx);
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
                                    .child("New WSL Profile"),
                            )
                            .child(
                                div()
                                    .id("wsl-refresh")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                                    .cursor_pointer()
                                    .child("Refresh")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh(cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child(
                                "Creates a Local profile that runs wsl.exe -d <distro>. \
                                 If the distro is later uninstalled, opening it only fails that tab.",
                            ),
                    )
                    .child(
                        div()
                            .id("wsl-distro-list")
                            .flex_1()
                            .min_h(px(160.0))
                            .max_h(px(280.0))
                            .overflow_y_scroll()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme::BORDER_SUBTLE)
                            .bg(theme::BG)
                            .p_1()
                            .child(match &list {
                                ListState::Loading => div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("Scanning installed distributions…")
                                    .into_any_element(),
                                ListState::Error(msg) => div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(theme::DANGER)
                                    .child(msg.clone())
                                    .into_any_element(),
                                ListState::Ready(names) => div()
                                    .flex()
                                    .flex_col()
                                    .gap_0()
                                    .children(names.iter().map(|name| {
                                        let name = name.clone();
                                        let is_sel = selected.as_ref() == Some(&name);
                                        let row_id = SharedString::from(format!("wsl-d-{name}"));
                                        div()
                                            .id(row_id)
                                            .px_3()
                                            .py_2()
                                            .rounded(px(4.0))
                                            .cursor_pointer()
                                            .bg(if is_sel {
                                                theme::SELECTION
                                            } else {
                                                hsla(0.0, 0.0, 0.0, 0.0)
                                            })
                                            .hover(|s| {
                                                if is_sel {
                                                    s
                                                } else {
                                                    s.bg(theme::HOVER)
                                                }
                                            })
                                            .text_sm()
                                            .text_color(theme::TEXT)
                                            .child(name.clone())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.selected = Some(name.clone());
                                                cx.notify();
                                                cx.stop_propagation();
                                            }))
                                    }))
                                    .into_any_element(),
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(form_btn("Cancel", false, cx, |_, _, cx| {
                                cx.emit(WslFormEvent::Close);
                            }))
                            .child(form_btn("Save", false, cx, move |this, _, cx| {
                                if can_save {
                                    this.save(false, cx);
                                }
                            }))
                            .child(form_btn("Save & Open", true, cx, move |this, _, cx| {
                                if can_save {
                                    this.save(true, cx);
                                }
                            })),
                    ),
            )
    }
}

fn form_btn(
    label: &'static str,
    primary: bool,
    cx: &mut Context<WslForm>,
    on_click: impl Fn(&mut WslForm, &mut Window, &mut Context<WslForm>) + 'static,
) -> impl IntoElement {
    let id = SharedString::from(format!("wsl-btn-{label}"));
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded(px(theme::RADIUS_SM))
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(theme::ACCENT)
                .text_color(rgb(0xffffff))
                .hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.bg(theme::ELEVATED)
                .text_color(theme::TEXT)
                .hover(|s| s.bg(theme::HOVER))
        })
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            on_click(this, window, cx);
            cx.stop_propagation();
        }))
}
