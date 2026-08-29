use std::collections::HashMap;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use crate::shared::theme;
use crate::terminal::TerminalView;
use crate::ui::pane_layout::{PaneLayout, SplitAxis};
use crate::ui::tab_manager::TabManager;

struct SashDrag {
    split_id: Uuid,
    axis: SplitAxis,
}

struct PaneRender {
    terminal: Option<Entity<TerminalView>>,
    status_message: String,
}

pub struct TerminalPane {
    pub tabs: Entity<TabManager>,
    sash_drag: Option<SashDrag>,
    split_bounds: HashMap<Uuid, Bounds<Pixels>>,
    _observe_tabs: Subscription,
}

impl TerminalPane {
    pub fn new(tabs: Entity<TabManager>, cx: &mut Context<Self>) -> Self {
        let _observe_tabs = cx.observe(&tabs, |_this, _tabs, cx| cx.notify());
        Self {
            tabs,
            sash_drag: None,
            split_bounds: HashMap::new(),
            _observe_tabs,
        }
    }
}

impl Render for TerminalPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_id = self.tabs.read(cx).active;
        let tab_snap = active_id.and_then(|id| {
            self.tabs.read(cx).tabs.iter().find(|t| t.id == id).map(|t| {
                let panes: HashMap<Uuid, PaneRender> = t
                    .panes
                    .iter()
                    .map(|(pid, p)| {
                        (
                            *pid,
                            PaneRender {
                                terminal: p.terminal.clone(),
                                status_message: p.status_message.clone(),
                            },
                        )
                    })
                    .collect();
                (t.layout.clone(), panes, t.focused, t.zoomed)
            })
        });
        let view = cx.entity();
        let tabs = self.tabs.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::BG)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.sash_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let Some(drag) = this.sash_drag.as_ref() else {
                    return;
                };
                let Some(bounds) = this.split_bounds.get(&drag.split_id).copied() else {
                    return;
                };
                let ratio = match drag.axis {
                    SplitAxis::Horizontal => {
                        let w: f32 = bounds.size.width.into();
                        if w <= 0.0 {
                            return;
                        }
                        let x: f32 = (event.position.x - bounds.origin.x).into();
                        x / w
                    }
                    SplitAxis::Vertical => {
                        let h: f32 = bounds.size.height.into();
                        if h <= 0.0 {
                            return;
                        }
                        let y: f32 = (event.position.y - bounds.origin.y).into();
                        y / h
                    }
                };
                let split_id = drag.split_id;
                this.tabs
                    .update(cx, |m, cx| m.set_split_ratio(split_id, ratio, cx));
            }))
            .child(match tab_snap {
                None => empty_state().into_any_element(),
                Some((layout, panes, focused, zoomed)) => {
                    // Zoom: show one leaf full-bleed; keep split tree for restore.
                    if let Some(z) = zoomed.filter(|z| panes.contains_key(z)) {
                        // Full-bleed like Zed zoom — no multi-pane chrome.
                        render_pane(z, panes.get(&z), true, false, &tabs, cx)
                    } else {
                        let multi_pane = layout.leaf_count() > 1;
                        render_layout(&layout, &panes, focused, multi_pane, &view, &tabs, cx)
                    }
                }
            })
    }
}

fn empty_state() -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .flex_col()
        .gap(px(theme::SPACE_2))
        .child(
            div()
                .text_lg()
                .text_color(theme::TEXT)
                .child("No session open"),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme::TEXT_MUTED)
                .child("Open a profile from the left, or press Ctrl+T"),
        )
}

fn render_layout(
    layout: &PaneLayout,
    panes: &HashMap<Uuid, PaneRender>,
    focused: Uuid,
    multi_pane: bool,
    view: &Entity<TerminalPane>,
    tabs: &Entity<TabManager>,
    cx: &mut Context<TerminalPane>,
) -> AnyElement {
    match layout {
        PaneLayout::Leaf(id) => {
            render_pane(*id, panes.get(id), *id == focused, multi_pane, tabs, cx)
        }
        PaneLayout::Split {
            id: split_id,
            axis,
            ratio,
            first,
            second,
        } => {
            let split_id = *split_id;
            let axis = *axis;
            let ratio = PaneLayout::clamp_ratio(*ratio);
            let view_measure = view.clone();

            let mut row = div()
                .relative()
                .flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(
                    canvas(
                        move |bounds, _, cx| {
                            view_measure.update(cx, |this, _| {
                                this.split_bounds.insert(split_id, bounds);
                            });
                            bounds
                        },
                        |_bounds, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                );

            row = match axis {
                SplitAxis::Horizontal => row.flex_row(),
                SplitAxis::Vertical => row.flex_col(),
            };

            let first_slot = match axis {
                SplitAxis::Horizontal => div().w(relative(ratio)).h_full(),
                SplitAxis::Vertical => div().h(relative(ratio)).w_full(),
            };
            let second_slot = match axis {
                SplitAxis::Horizontal => div().w(relative(1.0 - ratio)).h_full(),
                SplitAxis::Vertical => div().h(relative(1.0 - ratio)).w_full(),
            };

            let sash = match axis {
                SplitAxis::Horizontal => div()
                    .id(SharedString::from(format!("sash-{split_id}")))
                    .w(px(4.0))
                    .h_full()
                    .flex_shrink_0()
                    .cursor(CursorStyle::ResizeColumn)
                    .bg(theme::BORDER)
                    .hover(|s| s.bg(theme::ACCENT)),
                SplitAxis::Vertical => div()
                    .id(SharedString::from(format!("sash-{split_id}")))
                    .h(px(4.0))
                    .w_full()
                    .flex_shrink_0()
                    .cursor(CursorStyle::ResizeRow)
                    .bg(theme::BORDER)
                    .hover(|s| s.bg(theme::ACCENT)),
            }
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.sash_drag = Some(SashDrag { split_id, axis });
                    cx.notify();
                }),
            );

            row.child(
                first_slot
                    .flex()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(render_layout(
                        first, panes, focused, multi_pane, view, tabs, cx,
                    )),
            )
            .child(sash)
            .child(
                second_slot
                    .flex()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(render_layout(
                        second, panes, focused, multi_pane, view, tabs, cx,
                    )),
            )
            .into_any_element()
        }
    }
}

fn render_pane(
    id: Uuid,
    pane: Option<&PaneRender>,
    is_focused: bool,
    multi_pane: bool,
    tabs: &Entity<TabManager>,
    cx: &mut Context<TerminalPane>,
) -> AnyElement {
    let Some(pane) = pane else {
        return div()
            .flex_1()
            .size_full()
            .bg(theme::BG)
            .into_any_element();
    };
    let msg = pane.status_message.clone();
    // Only chrome panes when there is more than one (Zed-like focus ring).
    let show_border = multi_pane;
    let border = if is_focused {
        theme::ACCENT
    } else {
        theme::BORDER_SUBTLE
    };

    let focus_overlay = (multi_pane && !is_focused).then(|| {
        let tabs = tabs.clone();
        div()
            .id(SharedString::from(format!("pane-focus-{id}")))
            .absolute()
            .inset_0()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    tabs.update(cx, |m, cx| m.focus_pane(id, window, cx));
                }),
            )
    });

    match &pane.terminal {
        Some(entity) => div()
            .id(SharedString::from(format!("pane-{id}")))
            .relative()
            .flex_1()
            .size_full()
            .min_h_0()
            .when(show_border, |d| d.border_1().border_color(border))
            .child(entity.clone())
            .when_some(focus_overlay, |d, overlay| d.child(overlay))
            .into_any_element(),
        None => div()
            .id(SharedString::from(format!("pane-{id}")))
            .relative()
            .flex_1()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .p(px(theme::SPACE_4))
            .when(show_border, |d| d.border_1().border_color(border))
            .text_sm()
            .text_color(theme::TEXT_MUTED)
            .child(msg)
            .when_some(focus_overlay, |d, overlay| d.child(overlay))
            .into_any_element(),
    }
}
