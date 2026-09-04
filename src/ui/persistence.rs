//! Central persistence listener for [`AppBusEvent`]s.

use std::time::Duration;

use gpui::*;

use crate::ui::app_bus::{AppBus, AppBusEvent};
use crate::ui::workspace_store::WorkspaceStore;
use crate::ui::workspace_view::WorkspaceView;

const PERSIST_DEBOUNCE: Duration = Duration::from_millis(300);

pub struct Persistence {
    store: Entity<WorkspaceStore>,
    workspace: WeakEntity<WorkspaceView>,
    debounce_generation: u64,
    _debounce: Option<Task<()>>,
    /// After a successful WillQuit flush; allows window close to proceed.
    pub flushed_for_quit: bool,
    _subscription: Subscription,
}

impl Persistence {
    pub fn new(
        app_bus: Entity<AppBus>,
        store: Entity<WorkspaceStore>,
        workspace: WeakEntity<WorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _subscription = cx.subscribe(&app_bus, |this, _bus, event: &AppBusEvent, cx| {
            match event {
                AppBusEvent::WillQuit => this.on_will_quit(cx),
                AppBusEvent::PersistRequested => this.schedule_debounce(cx),
                AppBusEvent::BoundLocalCwdChanged { profile_id, path } => {
                    let profile_id = *profile_id;
                    let path = path.clone();
                    this.store.update(cx, |s, cx| {
                        s.update_local_profile_cwd(profile_id, path, cx);
                    });
                    this.schedule_debounce(cx);
                }
                AppBusEvent::SplitPane { .. } => {}
                AppBusEvent::DuplicateActiveTab => {}
                AppBusEvent::Toast(_) => {}
            }
        });

        Self {
            store,
            workspace,
            debounce_generation: 0,
            _debounce: None,
            flushed_for_quit: false,
            _subscription,
        }
    }

    pub fn allow_window_close(&self) -> bool {
        self.flushed_for_quit
    }

    /// Immediate full flush (Ctrl+S, WillQuit, debounce fire).
    pub fn flush_now(&mut self, cx: &mut Context<Self>) {
        self.debounce_generation = self.debounce_generation.wrapping_add(1);
        self._debounce = None;
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |view, cx| view.flush_persist(cx));
        } else {
            self.store.update(cx, |s, _| s.persist_if_dirty());
        }
    }

    fn on_will_quit(&mut self, cx: &mut Context<Self>) {
        if self.flushed_for_quit {
            return;
        }
        self.flush_now(cx);
        self.flushed_for_quit = true;
        cx.quit();
    }

    fn schedule_debounce(&mut self, cx: &mut Context<Self>) {
        if self.flushed_for_quit {
            return;
        }
        self.debounce_generation = self.debounce_generation.wrapping_add(1);
        let generation = self.debounce_generation;
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PERSIST_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.debounce_generation != generation || this.flushed_for_quit {
                    return;
                }
                this.flush_now(cx);
            })
            .ok();
        }));
    }
}
