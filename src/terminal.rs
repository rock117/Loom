//! Terminal subsystem scaffold (Zed-inspired pipeline; implementation grows over time).
//!
//! Target flow:
//!   PTY bytes → VT grid → GPUI TerminalElement paint → keystrokes → PTY write
//!
//! Today tabs still use `gpui-terminal` for rendering; session helpers here own
//! PTY lifecycle improvements shared with that path.

pub mod input;
pub mod session;

pub use session::TerminalSessionHandles;
