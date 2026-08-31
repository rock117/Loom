//! Application-level event bus (GPUI entity events, not a global EventBus).

use gpui::*;
use uuid::Uuid;

/// Lifecycle / persistence facts emitted by [`AppBus`].
#[derive(Clone, Debug)]
pub enum AppBusEvent {
    /// App is about to exit — Persistence flushes then `cx.quit()`.
    WillQuit,
    /// Session/UI state should hit disk (debounced).
    PersistRequested,
    /// Bound Local profile cwd changed in memory; disk write is debounced / on quit.
    BoundLocalCwdChanged {
        profile_id: Uuid,
        path: std::path::PathBuf,
    },
}

/// Empty emitter entity for [`AppBusEvent`].
pub struct AppBus;

impl EventEmitter<AppBusEvent> for AppBus {}

impl AppBus {
    pub fn emit(this: &Entity<Self>, event: AppBusEvent, cx: &mut App) {
        this.update(cx, |_, cx| cx.emit(event));
    }
}
