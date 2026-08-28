//! Session handle types for PTY teardown (shared by TabManager).

use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::{ChildKiller, MasterPty};

/// Owns the pieces needed to tear down a local PTY without blocking the UI.
pub struct TerminalSessionHandles {
    pub master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
}

impl TerminalSessionHandles {
    pub fn teardown(self) {
        crate::session::local::teardown_pty(Some(self.killer), Some(self.master));
    }
}
