//! Terminal subsystem: alacritty grid + GPUI paint (MIT path; not Zed GPL).
//!
//! Adapted from gpui-terminal (MIT OR Apache-2.0) with Loom PTY lifecycle and
//! `PtyWrite` writeback so shells that query cursor position (e.g. PowerShell) do not stall.

pub mod gpui_emu;
pub mod session;

pub use gpui_emu::{ColorPalette, TerminalConfig, TerminalView, TerminalViewEvent};
