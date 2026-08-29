//! In-house GPUI terminal view, adapted from [gpui-terminal](https://github.com/zortax/gpui-terminal)
//! (MIT OR Apache-2.0) with Loom fixes (notably `Event::PtyWrite` → PTY writeback).

mod colors;
mod event;
mod input;
mod osc;
mod render;
mod terminal;
mod view;

pub use colors::ColorPalette;
pub use view::{TerminalConfig, TerminalView, TerminalViewEvent};
