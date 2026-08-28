//! Event bridge: alacritty `EventListener` → channel → `TerminalView`.
//!
//! Reply-class events (`PtyWrite`, `ColorRequest`, `TextAreaSizeRequest`, clipboard)
//! are queued (not applied inside the listener) so `TerminalView` can handle them
//! **in order** after `process_bytes` releases the term lock.

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::vte::ansi::Rgb;
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// Events relevant to the GPUI terminal view.
pub enum TerminalEvent {
    Wakeup,
    Bell,
    Title(String),
    ClipboardStore(String),
    ClipboardLoad(Arc<dyn Fn(&str) -> String + Send + Sync>),
    Exit,
    /// Emulator must write these bytes to the PTY (DSR, etc.).
    PtyWrite(String),
    ColorRequest(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync>),
    TextAreaSizeRequest(Arc<dyn Fn(WindowSize) -> String + Send + Sync>),
}

/// Forwards alacritty events onto a channel for ordered handling on the view.
pub struct GpuiEventProxy {
    tx: Sender<TerminalEvent>,
}

impl GpuiEventProxy {
    pub fn new(tx: Sender<TerminalEvent>) -> Self {
        Self { tx }
    }

    fn send(&self, event: TerminalEvent) {
        let _ = self.tx.send(event);
    }
}

impl EventListener for GpuiEventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => self.send(TerminalEvent::Wakeup),
            Event::Bell => self.send(TerminalEvent::Bell),
            Event::Title(title) => self.send(TerminalEvent::Title(title)),
            Event::ClipboardStore(_ty, data) => self.send(TerminalEvent::ClipboardStore(data)),
            Event::ClipboardLoad(_ty, format) => self.send(TerminalEvent::ClipboardLoad(format)),
            Event::Exit => self.send(TerminalEvent::Exit),
            Event::MouseCursorDirty => {}
            Event::PtyWrite(data) => self.send(TerminalEvent::PtyWrite(data)),
            Event::ColorRequest(index, format) => {
                self.send(TerminalEvent::ColorRequest(index, format));
            }
            Event::TextAreaSizeRequest(format) => {
                self.send(TerminalEvent::TextAreaSizeRequest(format));
            }
            Event::CursorBlinkingChange => {}
            Event::ResetTitle => self.send(TerminalEvent::Title(String::new())),
            Event::ChildExit(_code) => self.send(TerminalEvent::Exit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn pty_write_is_queued() {
        let (tx, rx) = channel();
        let proxy = GpuiEventProxy::new(tx);
        proxy.send_event(Event::PtyWrite("\x1b[1;1R".into()));
        match rx.recv().unwrap() {
            TerminalEvent::PtyWrite(s) => assert_eq!(s, "\x1b[1;1R"),
            _ => panic!("expected PtyWrite"),
        }
    }

    #[test]
    fn wakeup_event() {
        let (tx, rx) = channel();
        let proxy = GpuiEventProxy::new(tx);
        proxy.send_event(Event::Wakeup);
        assert!(matches!(rx.recv().unwrap(), TerminalEvent::Wakeup));
    }
}
