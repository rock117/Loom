//! Main terminal view component for GPUI.
//!
//! This module provides [`TerminalView`], the primary component for embedding terminals
//! in GPUI applications. It manages:
//!
//! - **I/O Streams**: Accepts arbitrary [`Read`]/[`Write`]
//!   streams, allowing integration with any PTY implementation
//! - **Event Handling**: Keyboard and mouse input, with configurable callbacks
//! - **Rendering**: Efficient canvas-based rendering via [`TerminalRenderer`]
//! - **Configuration**: Font, colors, dimensions, and padding via [`TerminalConfig`]
//!
//! # Architecture
//!
//! The terminal uses a push-based async I/O architecture:
//!
//! 1. A background thread reads bytes from the PTY stdout in 4KB chunks
//! 2. Bytes are sent through a [flume](https://docs.rs/flume) channel to an async task
//! 3. The async task processes bytes through the VTE parser and calls `cx.notify()`
//! 4. GPUI repaints the terminal with the updated grid
//!
//! This approach ensures the terminal only wakes when data arrives, avoiding polling.
//!
//! # Thread Safety
//!
//! - [`TerminalView`] itself is not `Send` (it contains GPUI handles)
//! - The stdin writer is wrapped in `Arc<parking_lot::Mutex<>>` for thread-safe writes
//! - Callbacks ([`ResizeCallback`], [`KeyHandler`]) must be `Send + Sync`
//!
//! # Example
//!
//! ```ignore
//! use gpui::{Context, Edges, px};
//! use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};
//!
//! // In a GPUI window context:
//! let terminal = cx.new(|cx| {
//!     TerminalView::new(pty_writer, pty_reader, TerminalConfig::default(), cx)
//!         .with_resize_callback(move |cols, rows| {
//!             // Notify PTY of new dimensions
//!         })
//!         .with_exit_callback(|_, cx| {
//!             cx.quit();
//!         })
//! });
//!
//! // Focus the terminal to receive keyboard input
//! terminal.read(cx).focus_handle().focus(window);
//! ```

mod context_menu;
mod find;

pub use context_menu::TerminalViewEvent;

use super::colors::ColorPalette;
use super::event::{GpuiEventProxy, TerminalEvent};
use super::hyperlink;
use super::input::keystroke_to_bytes;
use super::render::TerminalRenderer;
use super::terminal::TerminalState;
use crate::platform;
use crate::shared::theme;
use gpui::prelude::FluentBuilder;
use gpui::{Edges, *};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

/// Configuration for terminal creation and runtime updates.
///
/// This struct defines the terminal's appearance and behavior, including
/// grid dimensions, font settings, scrollback buffer, and color scheme.
///
/// # Default Values
///
/// | Field | Default |
/// |-------|---------|
/// | `cols` | 80 |
/// | `rows` | 24 |
/// | `font_family` | "monospace" |
/// | `font_size` | 14px |
/// | `scrollback` | 10000 |
/// | `line_height_multiplier` | 1.2 |
/// | `padding` | 0px all sides |
/// | `colors` | Default palette |
///
/// # Example
///
/// ```ignore
/// use gpui::{Edges, px};
/// use gpui_terminal::{ColorPalette, TerminalConfig};
///
/// let config = TerminalConfig {
///     cols: 120,
///     rows: 40,
///     font_family: "JetBrains Mono".into(),
///     font_size: px(13.0),
///     scrollback: 50000,
///     line_height_multiplier: 1.1,
///     padding: Edges::all(px(10.0)),
///     colors: ColorPalette::builder()
///         .background(0x1a, 0x1a, 0x1a)
///         .foreground(0xe0, 0xe0, 0xe0)
///         .build(),
/// };
/// ```
///
/// # Runtime Updates
///
/// Configuration can be updated at runtime via [`TerminalView::update_config`].
/// This is useful for implementing features like dynamic font sizing:
///
/// ```ignore
/// terminal.update(cx, |terminal, cx| {
///     let mut config = terminal.config().clone();
///     config.font_size += px(1.0);
///     terminal.update_config(config, cx);
/// });
/// ```
#[derive(Clone, Debug)]
pub struct TerminalConfig {
    /// Number of columns (character width) in the terminal
    pub cols: usize,

    /// Number of rows (lines) in the terminal
    pub rows: usize,

    /// Font family name (e.g., "Fira Code", "JetBrains Mono")
    pub font_family: String,

    /// Font size in pixels
    pub font_size: Pixels,

    /// Maximum number of scrollback lines to keep in history
    pub scrollback: usize,

    /// Multiplier for line height to accommodate tall glyphs (e.g., nerd fonts)
    /// Default is 1.2 (20% extra height)
    pub line_height_multiplier: f32,

    /// Padding around the terminal content (top, right, bottom, left)
    /// The padding area renders with the terminal's background color
    pub padding: Edges<Pixels>,

    /// Paint a left gutter with absolute scrollback line numbers (1 = oldest).
    pub show_line_numbers: bool,

    /// Color palette for terminal colors (16 ANSI colors, 256 extended colors,
    /// foreground, background, and cursor colors)
    pub colors: ColorPalette,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            font_family: "monospace".into(),
            font_size: px(14.0),
            scrollback: 10000,
            line_height_multiplier: 1.2,
            padding: Edges::all(px(0.0)),
            show_line_numbers: true,
            colors: ColorPalette::default(),
        }
    }
}

/// Callback type for PTY resize notifications.
///
/// This callback is invoked when the terminal grid dimensions change,
/// typically due to window resizing. The callback receives the new
/// column and row counts.
///
/// # Arguments
///
/// * `cols` - New number of columns (characters wide)
/// * `rows` - New number of rows (lines tall)
///
/// # Thread Safety
///
/// This callback must be `Send + Sync` as it may be called from the render thread.
///
/// # Example
///
/// ```ignore
/// use portable_pty::PtySize;
///
/// let pty = Arc::new(Mutex::new(pty_master));
/// let pty_clone = pty.clone();
///
/// terminal.with_resize_callback(move |cols, rows| {
///     pty_clone.lock().resize(PtySize {
///         cols: cols as u16,
///         rows: rows as u16,
///         pixel_width: 0,
///         pixel_height: 0,
///     }).ok();
/// });
/// ```
pub type ResizeCallback = Box<dyn Fn(usize, usize) + Send + Sync>;

/// Callback type for key event interception.
///
/// This callback is invoked before the terminal processes a key event,
/// allowing you to intercept and handle specific key combinations.
///
/// # Arguments
///
/// * `event` - The key down event from GPUI
///
/// # Returns
///
/// * `true` - Consume the event (terminal will not process it)
/// * `false` - Let the terminal handle the event normally
///
/// # Thread Safety
///
/// This callback must be `Send + Sync`.
///
/// # Example
///
/// ```ignore
/// terminal.with_key_handler(|event| {
///     let keystroke = &event.keystroke;
///
///     // Intercept Ctrl++ for font size increase
///     if keystroke.modifiers.control && (keystroke.key == "+" || keystroke.key == "=") {
///         // Handle font size increase
///         return true; // Consume the event
///     }
///
///     // Intercept Ctrl+- for font size decrease
///     if keystroke.modifiers.control && keystroke.key == "-" {
///         // Handle font size decrease
///         return true;
///     }
///
///     false // Let terminal handle all other keys
/// });
/// ```
pub type KeyHandler = Box<dyn Fn(&KeyDownEvent) -> bool + Send + Sync>;

/// Callback for terminal bell events.
///
/// This callback is invoked when the terminal bell is triggered (BEL character,
/// ASCII 0x07), allowing you to play a sound or show a visual indicator.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
///
/// # Example
///
/// ```ignore
/// terminal.with_bell_callback(|window, cx| {
///     // Option 1: Visual bell (flash the window or show an indicator)
///     // Option 2: Play a sound
///     // Option 3: Notify the user via system notification
/// });
/// ```
pub type BellCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>)>;

/// Callback for terminal title changes.
///
/// This callback is invoked when the terminal title changes via escape sequences
/// (OSC 0, OSC 2), allowing you to update the window or tab title.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
/// * `title` - The new title string
///
/// # Example
///
/// ```ignore
/// terminal.with_title_callback(|window, cx, title| {
///     // Update the window title
///     // Or update a tab label in a tabbed interface
///     println!("Terminal title changed to: {}", title);
/// });
/// ```
pub type TitleCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>, &str)>;

/// Callback for clipboard store requests.
///
/// This callback is invoked when the terminal wants to store data to the clipboard
/// via OSC 52 escape sequence. Applications like tmux and vim can use this to
/// copy text to the system clipboard.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
/// * `text` - The text to store in the clipboard
///
/// # Example
///
/// ```ignore
/// use gpui_terminal::Clipboard;
///
/// terminal.with_clipboard_store_callback(|window, cx, text| {
///     if let Ok(mut clipboard) = Clipboard::new() {
///         clipboard.copy(text).ok();
///     }
/// });
/// ```
pub type ClipboardStoreCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>, &str)>;

/// Callback for terminal exit events.
///
/// This callback is invoked when the terminal process exits (e.g., shell exits,
/// process terminates). This is detected when the PTY reader reaches EOF.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
///
/// # Example
///
/// ```ignore
/// terminal.with_exit_callback(|window, cx| {
///     // Option 1: Quit the application
///     cx.quit();
///
///     // Option 2: Close this terminal tab/pane
///     // terminal_manager.close_terminal(terminal_id);
///
///     // Option 3: Show an exit message
///     // show_notification("Terminal exited");
/// });
/// ```
pub type ExitCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>)>;

/// The main terminal view component for GPUI applications.
///
/// `TerminalView` is a GPUI entity that implements the [`Render`] trait,
/// providing a complete terminal emulator that can be embedded in any GPUI application.
///
/// # Responsibilities
///
/// - **Terminal State**: Manages the grid, cursor, and colors via [`TerminalState`]
/// - **I/O Streams**: Reads from PTY stdout and writes to PTY stdin
/// - **Event Handling**: Processes keyboard, mouse, and resize events
/// - **Rendering**: Paints text, backgrounds, and cursor via [`TerminalRenderer`]
/// - **Callbacks**: Dispatches events to user-provided callbacks
///
/// # Creating a Terminal
///
/// Use [`TerminalView::new`] within a GPUI entity context:
///
/// ```ignore
/// let terminal = cx.new(|cx| {
///     TerminalView::new(writer, reader, config, cx)
///         .with_resize_callback(resize_callback)
///         .with_exit_callback(|_, cx| cx.quit())
/// });
/// ```
///
/// # Focus
///
/// The terminal must be focused to receive keyboard input:
///
/// ```ignore
/// terminal.read(cx).focus_handle().focus(window);
/// ```
///
/// # Callbacks
///
/// Configure behavior through builder methods:
///
/// - [`with_resize_callback`](Self::with_resize_callback) - PTY size changes
/// - [`with_exit_callback`](Self::with_exit_callback) - Process exit
/// - [`with_key_handler`](Self::with_key_handler) - Key event interception
/// - [`with_bell_callback`](Self::with_bell_callback) - Terminal bell
/// - [`with_title_callback`](Self::with_title_callback) - Title changes
/// - [`with_clipboard_store_callback`](Self::with_clipboard_store_callback) - Clipboard writes
///
/// # Thread Safety
///
/// `TerminalView` is not `Send` as it contains GPUI handles. The stdin writer
/// is internally wrapped in `Arc<parking_lot::Mutex<>>` for safe concurrent access.
pub struct TerminalView {
    /// The terminal state managing the grid and VTE parser
    state: TerminalState,

    /// The renderer for drawing terminal content
    renderer: TerminalRenderer,

    /// Focus handle for keyboard event handling
    focus_handle: FocusHandle,

    /// Writer for sending input to the terminal process
    stdin_writer: Arc<parking_lot::Mutex<Box<dyn Write + Send>>>,

    /// Receiver for terminal events from the event proxy
    event_rx: mpsc::Receiver<TerminalEvent>,

    /// Configuration used to create this terminal
    config: TerminalConfig,

    /// Async task that reads bytes and notifies the view (push-based)
    #[allow(dead_code)]
    _reader_task: Task<()>,

    /// Callback to notify the PTY about size changes
    resize_callback: Option<Arc<ResizeCallback>>,

    /// Optional callback to intercept key events before terminal processing
    key_handler: Option<Arc<KeyHandler>>,

    /// Callback for terminal bell events
    bell_callback: Option<BellCallback>,

    /// Callback for terminal title changes
    title_callback: Option<TitleCallback>,

    /// Callback for clipboard store requests
    clipboard_store_callback: Option<ClipboardStoreCallback>,

    /// Callback for terminal exit events
    exit_callback: Option<ExitCallback>,

    /// True while the left button is dragging a selection.
    selecting: bool,

    /// True once the pointer moved to a different cell during this drag.
    /// Click-without-drag must not leave a 1-cell “fake cursor” highlight.
    selection_dragged: bool,

    /// Cell where the current selection press started.
    selection_anchor: Option<(alacritty_terminal::index::Point, alacritty_terminal::index::Side)>,

    /// True while Ctrl (or macOS Cmd) is held — enable clickable URL hover.
    hyperlink_mods: bool,
    /// Pointer is over a URL while `hyperlink_mods` is active.
    hover_hyperlink: bool,
    /// Grid line + exclusive column range of the hovered URL (for paint).
    hover_url_span: Option<(alacritty_terminal::index::Line, usize, usize)>,

    /// Last painted terminal bounds (window space) for hit-testing.
    last_bounds: Bounds<Pixels>,

    /// Active scrollbar thumb drag (window Y → display offset).
    scrollbar_drag: Option<ScrollbarDrag>,

    /// Ctrl+F find bar over scrollback / viewport.
    find: Option<find::FindState>,

    /// Keeps the find-bar caret blink loop alive while find is open.
    _find_caret_blink: Option<Task<()>>,

    /// Right-click context menu anchor (window coordinates).
    context_menu: Option<Point<Pixels>>,

    /// Best-known working directory for Copy Path / Reveal.
    /// Updated by OSC 7/9;9 and (for local shells) process cwd refresh.
    working_directory: Option<std::path::PathBuf>,

    /// Local shell PID for Zed-style cwd refresh; `None` for SSH.
    shell_pid: Option<u32>,

    /// IME marked (composing) text as UTF-16 length range into a virtual buffer.
    /// When `Some`, Windows routes keys through TranslateMessage / IME composition.
    ime_marked: Option<(String, std::ops::Range<usize>)>,

    /// False after PTY/SSH EOF or a broken stdin write — keys must not pretend to work.
    session_alive: bool,
}

struct ScrollbarDrag {
    /// Pointer Y within the track when the drag started.
    pointer_y_in_thumb: Pixels,
}

struct ScrollMetrics {
    display_offset: usize,
    history: usize,
    screen_lines: usize,
}

struct ScrollbarGeometry {
    track: Bounds<Pixels>,
    thumb_y: Pixels,
    thumb_h: Pixels,
}

/// Alacritty-style wheel multiplier (lines per notch / scaled pixel delta).
const SCROLL_MULTIPLIER: f32 = 3.0;
const SCROLLBAR_WIDTH: f32 = 10.0;
const SCROLLBAR_MIN_THUMB: f32 = 24.0;
const LINE_NUMBER_PAD: f32 = 6.0;

impl TerminalView {
    /// Create a new terminal with provided I/O streams.
    ///
    /// This method initializes a new terminal emulator with the given stdin writer
    /// and stdout reader. It spawns a background task to read from stdout and
    /// process incoming bytes through the VTE parser.
    ///
    /// # Arguments
    ///
    /// * `stdin_writer` - Writer for sending input bytes to the terminal process
    /// * `stdout_reader` - Reader for receiving output bytes from the terminal process
    /// * `config` - Terminal configuration (dimensions, font, etc.)
    /// * `cx` - GPUI context for this view
    ///
    /// # Returns
    ///
    /// A new `TerminalView` instance ready to be rendered.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // In a GPUI window context:
    /// let terminal = cx.new(|cx| {
    ///     TerminalView::new(stdin_writer, stdout_reader, TerminalConfig::default(), cx)
    /// });
    /// ```
    pub fn new<W, R>(
        stdin_writer: W,
        stdout_reader: R,
        config: TerminalConfig,
        cx: &mut Context<Self>,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        // Create event channel for terminal events
        let (event_tx, event_rx) = mpsc::channel();

        // Clone event_tx for the reader task to send Exit event when PTY closes
        let exit_event_tx = event_tx.clone();

        // Wrap stdin writer in Arc<Mutex> for thread-safe access
        let stdin_writer = Arc::new(parking_lot::Mutex::new(
            Box::new(stdin_writer) as Box<dyn Write + Send>
        ));

        // Events are queued so reply writebacks stay ordered after process_bytes.
        let event_proxy = GpuiEventProxy::new(event_tx);

        // Create terminal state
        let state = TerminalState::new_with_scrollback(
            config.cols,
            config.rows,
            config.scrollback,
            event_proxy,
        );

        // Create renderer with font settings and color palette
        let renderer = TerminalRenderer::new(
            config.font_family.clone(),
            config.font_size,
            config.line_height_multiplier,
            config.colors.clone(),
        );

        // Create focus handle
        let focus_handle = cx.focus_handle();

        // Create async channel for bytes (push-based notification)
        // Using flume instead of smol::channel because flume is executor-agnostic
        // and properly wakes GPUI's async executor when data arrives
        let (bytes_tx, bytes_rx) = flume::unbounded::<Vec<u8>>();

        // Spawn background thread to read from stdout
        // This thread sends bytes through the async channel
        thread::spawn(move || {
            Self::read_stdout_blocking(stdout_reader, bytes_tx);
        });

        // Spawn async task that awaits on the channel and notifies the view
        // This is push-based: the task blocks until bytes arrive, then immediately notifies
        let reader_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                // Wait for bytes from the background reader (blocks until data arrives)
                match bytes_rx.recv_async().await {
                    Ok(bytes) => {
                        // Process bytes and notify the view
                        let result = this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            view.state.process_bytes(&bytes);
                            if let Some(cwd) = view.state.take_cwd_update() {
                                view.working_directory = Some(cwd.clone());
                                cx.emit(TerminalViewEvent::WorkingDirectoryChanged(cwd));
                            }
                            view.dispatch_pending_events(None, cx);
                            cx.notify();
                        });
                        if result.is_err() {
                            // View was dropped, exit
                            break;
                        }
                    }
                    Err(_) => {
                        // Channel closed - PTY has finished, send Exit event
                        let _ = exit_event_tx.send(TerminalEvent::Exit);
                        // Notify view to process the Exit event
                        let _ = this.update(cx, |_view, cx: &mut Context<Self>| {
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        });

        Self {
            state,
            renderer,
            focus_handle,
            stdin_writer,
            event_rx,
            config,
            _reader_task: reader_task,
            resize_callback: None,
            key_handler: None,
            bell_callback: None,
            title_callback: None,
            clipboard_store_callback: None,
            exit_callback: None,
            selecting: false,
            selection_dragged: false,
            selection_anchor: None,
            hyperlink_mods: false,
            hover_hyperlink: false,
            hover_url_span: None,
            last_bounds: Bounds::default(),
            scrollbar_drag: None,
            find: None,
            _find_caret_blink: None,
            context_menu: None,
            working_directory: None,
            shell_pid: None,
            ime_marked: None,
            session_alive: true,
        }
    }

    /// Attach the local shell PID so Copy Path / Reveal can refresh cwd from the process.
    pub fn with_shell_pid(mut self, pid: Option<u32>) -> Self {
        self.shell_pid = pid;
        self
    }

    /// Set a callback to be invoked when the terminal is resized.
    ///
    /// This callback should resize the underlying PTY to match the new dimensions.
    /// The callback receives (cols, rows) as arguments.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with (cols, rows) on resize
    pub fn with_resize_callback(
        mut self,
        callback: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Self {
        self.resize_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Set a callback to intercept key events before terminal processing.
    ///
    /// The callback receives the key event and should return `true` to consume
    /// the event (prevent the terminal from processing it), or `false` to allow
    /// normal terminal processing.
    ///
    /// # Arguments
    ///
    /// * `handler` - A function that receives key events and returns whether to consume them
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_key_handler(|event| {
    ///     // Handle Ctrl++ to increase font size
    ///     if event.keystroke.modifiers.control && event.keystroke.key == "+" {
    ///         // Handle the event
    ///         return true; // Consume the event
    ///     }
    ///     false // Let terminal handle it
    /// })
    /// ```
    pub fn with_key_handler(
        mut self,
        handler: impl Fn(&KeyDownEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.key_handler = Some(Arc::new(Box::new(handler)));
        self
    }

    /// Set a callback to be invoked when the terminal bell is triggered.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to play a sound or show a visual indicator.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the bell is triggered
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_bell_callback(|window, cx| {
    ///     // Play a sound or flash the screen
    /// })
    /// ```
    pub fn with_bell_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>) + 'static,
    ) -> Self {
        self.bell_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal title changes.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// along with the new title string.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with the new title
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_title_callback(|window, cx, title| {
    ///     // Update window title or tab title
    /// })
    /// ```
    pub fn with_title_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>, &str) + 'static,
    ) -> Self {
        self.title_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal wants to store data to the clipboard.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// along with the text to store. This is typically triggered by OSC 52 escape sequences.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with the text to store
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_clipboard_store_callback(|window, cx, text| {
    ///     // Store text to system clipboard
    /// })
    /// ```
    pub fn with_clipboard_store_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>, &str) + 'static,
    ) -> Self {
        self.clipboard_store_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal process exits.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to close the terminal view or show an exit message.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the process exits
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_exit_callback(|window, cx| {
    ///     // Close the terminal tab or show exit message
    /// })
    /// ```
    pub fn with_exit_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>) + 'static,
    ) -> Self {
        self.exit_callback = Some(Box::new(callback));
        self
    }

    /// Background thread that reads from stdout.
    ///
    /// This function runs in a background thread, continuously reading bytes
    /// from the stdout reader and sending them through the async channel.
    /// The async channel allows the main async task to be woken up immediately
    /// when data arrives (push-based).
    fn read_stdout_blocking<R: Read + Send + 'static>(
        mut stdout_reader: R,
        bytes_tx: flume::Sender<Vec<u8>>,
    ) {
        let mut buffer = [0u8; 4096];

        loop {
            match stdout_reader.read(&mut buffer) {
                Ok(0) => {
                    // EOF - channel will be dropped, signaling completion
                    break;
                }
                Ok(n) => {
                    // Send bytes to the async task
                    let bytes = buffer[..n].to_vec();
                    if bytes_tx.send(bytes).is_err() {
                        break; // Channel closed
                    }
                }
                Err(_) => {
                    // Read error
                    break;
                }
            }
        }
    }

    /// Handle keyboard input events.
    ///
    /// Converts GPUI keystrokes to terminal escape sequences and writes them
    /// to the stdin writer. If a key handler is set and returns true, the event
    /// is consumed and not sent to the terminal.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Check if key handler wants to consume this event
        if let Some(ref handler) = self.key_handler
            && handler(event)
        {
            return; // Event consumed by handler
        }

        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        if self.context_menu.is_some() && key == "escape" {
            self.close_context_menu(cx);
            return;
        }

        // Ctrl+F opens find (do not forward 0x06 to the PTY).
        if mods.control
            && !mods.alt
            && !mods.shift
            && key.eq_ignore_ascii_case("f")
        {
            self.open_find(window, cx);
            return;
        }

        if self.is_find_open() {
            if key == "escape" {
                self.close_find(window, cx);
                return;
            }
            if key == "f3" {
                if mods.shift {
                    self.find_prev(cx);
                } else {
                    self.find_next(cx);
                }
                return;
            }
        }

        if self.is_paste_keystroke(&event.keystroke) {
            self.paste_from_clipboard(cx);
            return;
        }

        // With an active selection, Ctrl+C copies (Windows Terminal style) instead of SIGINT.
        if mods.control
            && !mods.alt
            && key.eq_ignore_ascii_case("c")
            && self.copy_selection_to_clipboard(cx)
        {
            return;
        }
        // Ctrl+Shift+C always copies when possible.
        if mods.control && mods.shift && key.eq_ignore_ascii_case("c") {
            self.copy_selection_to_clipboard(cx);
            return;
        }

        // Scrollback navigation (Zed / Windows Terminal style).
        if self.handle_scroll_key(&event.keystroke, cx) {
            cx.stop_propagation();
            return;
        }

        // Zed model: KeyDown only handles escape/control sequences. Printable
        // text (and IME commit) comes from InputHandler. When handled, stop
        // propagation so Windows does not also TranslateMessage (avoids dupes).
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke, self.state.mode()) {
            // Enter/Esc end any stuck IME mark so the next KeyDown is delivered.
            if matches!(key, "enter" | "escape") {
                self.ime_marked = None;
            }
            self.state.with_term_mut(|term| {
                term.selection = None;
                term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
            });
            self.write_to_pty(&bytes, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn handle_scroll_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        use alacritty_terminal::grid::Scroll;

        let key = keystroke.key.as_str();
        let mods = &keystroke.modifiers;
        // Shift+… scrolls history; plain PageUp/Down stay available to the shell.
        if !mods.shift || mods.control || mods.alt || mods.platform {
            return false;
        }

        let scroll = match key {
            "pageup" => Scroll::PageUp,
            "pagedown" => Scroll::PageDown,
            "up" => Scroll::Delta(1),
            "down" => Scroll::Delta(-1),
            "home" => Scroll::Top,
            "end" => Scroll::Bottom,
            _ => return false,
        };
        self.scroll_display(scroll, cx);
        true
    }

    fn is_paste_keystroke(&self, keystroke: &Keystroke) -> bool {
        let key = keystroke.key.as_str();
        let mods = &keystroke.modifiers;
        // Windows Terminal-style: Ctrl+Shift+V; also Ctrl+V and Shift+Insert.
        (mods.control && key.eq_ignore_ascii_case("v"))
            || (mods.shift && key.eq_ignore_ascii_case("insert"))
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if !text.is_empty() {
                self.paste_text(&text, cx);
            }
        }
    }

    /// Handle mouse down events.
    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        cx.notify();

        if event.button == MouseButton::Right {
            cx.emit(crate::terminal::TerminalViewEvent::FocusRequested);
            self.open_context_menu(event.position, cx);
            cx.stop_propagation();
            return;
        }

        if event.button != MouseButton::Left {
            return;
        }

        self.close_context_menu(cx);

        if self.handle_scrollbar_mouse_down(event.position, cx) {
            return;
        }

        // Ctrl+click (Cmd on macOS): open URL under cursor in the browser.
        if event.modifiers.control || event.modifiers.platform {
            if self.open_url_at(event.position, cx) {
                cx.stop_propagation();
                return;
            }
        }

        let Some((point, side)) = self.cell_at(event.position) else {
            return;
        };

        use alacritty_terminal::selection::{Selection, SelectionType};
        self.selecting = true;
        self.selection_dragged = false;
        self.selection_anchor = Some((point, side));
        self.state.with_term_mut(|term| {
            term.selection = Some(Selection::new(SelectionType::Simple, point, side));
        });
        cx.notify();
    }

    fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        if self.scrollbar_drag.take().is_some() {
            cx.notify();
            return;
        }

        if !self.selecting {
            return;
        }
        self.selecting = false;
        self.selection_anchor = None;

        // Plain click (no drag): clear selection — don't leave a blue cell that
        // looks like a second cursor. Real shell cursor stays at the prompt.
        if !self.selection_dragged {
            self.selection_dragged = false;
            self.state.with_term_mut(|term| term.selection = None);
            cx.notify();
            return;
        }
        self.selection_dragged = false;

        if let Some((point, side)) = self.cell_at(event.position) {
            self.state.with_term_mut(|term| {
                if let Some(selection) = term.selection.as_mut() {
                    selection.update(point, side);
                    selection.include_all();
                }
            });
        } else {
            self.state.with_term_mut(|term| {
                if let Some(selection) = term.selection.as_mut() {
                    selection.include_all();
                }
            });
        }

        // Auto-copy on select (Windows console style).
        self.copy_selection_to_clipboard(cx);
        cx.notify();
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on = event.modifiers.control || event.modifiers.platform;
        if self.hyperlink_mods == on {
            return;
        }
        self.hyperlink_mods = on;
        if !on {
            self.hover_hyperlink = false;
            self.hover_url_span = None;
        }
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_drag.is_some() {
            self.update_scrollbar_drag(event.position.y, cx);
            return;
        }

        if !self.selecting {
            if self.hyperlink_mods {
                let span = self
                    .cell_at(event.position)
                    .and_then(|(point, _)| self.url_col_span_at(point));
                let over = span.is_some();
                if over != self.hover_hyperlink || span != self.hover_url_span {
                    self.hover_hyperlink = over;
                    self.hover_url_span = span;
                    cx.notify();
                }
            }
            return;
        }
        let Some((point, side)) = self.cell_at(event.position) else {
            return;
        };
        if self
            .selection_anchor
            .is_some_and(|(anchor, anchor_side)| anchor != point || anchor_side != side)
        {
            self.selection_dragged = true;
        }
        self.state.with_term_mut(|term| {
            if let Some(selection) = term.selection.as_mut() {
                selection.update(point, side);
            }
        });
        cx.notify();
    }

    fn cell_at(
        &self,
        position: Point<Pixels>,
    ) -> Option<(alacritty_terminal::index::Point, alacritty_terminal::index::Side)> {
        use alacritty_terminal::index::{Column, Point as AlacPoint, Side};
        use alacritty_terminal::term::viewport_to_point;

        let cell_w: f32 = self.renderer.cell_width.into();
        let cell_h: f32 = self.renderer.cell_height.into();
        if cell_w <= 0.0 || cell_h <= 0.0 {
            return None;
        }

        let gutter = self.line_number_gutter_width();
        let origin_x = self.last_bounds.origin.x + self.config.padding.left + px(gutter);
        let origin_y = self.last_bounds.origin.y + self.config.padding.top;
        let rel_x: f32 = (position.x - origin_x).into();
        let rel_y: f32 = (position.y - origin_y).into();
        if rel_x < 0.0 || rel_y < 0.0 {
            return None;
        }

        let (cols, rows) = (self.state.cols().max(1), self.state.rows().max(1));
        let col = ((rel_x / cell_w) as usize).min(cols.saturating_sub(1));
        let row = ((rel_y / cell_h) as usize).min(rows.saturating_sub(1));
        let side = if (rel_x % cell_w) < cell_w * 0.5 {
            Side::Left
        } else {
            Side::Right
        };

        let display_offset = self
            .state
            .with_term(|term| term.grid().display_offset());
        let point = viewport_to_point(display_offset, AlacPoint::new(row, Column(col)));
        Some((point, side))
    }

    /// Ctrl/Cmd+click: open http(s)/… URL under the cell, if any.
    ///
    /// Opens **synchronously on the UI thread** via [`platform::open_url`].
    /// GPUI's `cx.open_url` runs on a background executor, which on Windows
    /// often prevents the browser from taking foreground (taskbar flash only).
    fn open_url_at(&self, position: Point<Pixels>, _cx: &App) -> bool {
        let Some((point, _)) = self.cell_at(position) else {
            return false;
        };
        let Some(url) = self.url_at_point(point) else {
            return false;
        };
        if !hyperlink::is_safe_open_url(&url) {
            return false;
        }
        if let Err(err) = platform::open_url(&url) {
            eprintln!("loom: failed to open URL {url}: {err}");
        }
        // Consume the click either way so we don't start a selection mid-Ctrl.
        true
    }

    fn url_at_point(&self, point: alacritty_terminal::index::Point) -> Option<String> {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::Column;
        use alacritty_terminal::term::cell::Flags;

        self.state.with_term(|term| {
            let grid = term.grid();
            let cols = term.columns();
            if cols == 0 || point.column.0 >= cols {
                return None;
            }
            if point.line < grid.topmost_line() || point.line > grid.bottommost_line() {
                return None;
            }

            let mut text = String::new();
            let mut col_to_char: Vec<Option<usize>> = vec![None; cols];
            for col in 0..cols {
                let cell = &grid[alacritty_terminal::index::Point::new(point.line, Column(col))];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = if cell.c == '\0' { ' ' } else { cell.c };
                let idx = text.chars().count();
                text.push(ch);
                col_to_char[col] = Some(idx);
            }
            let char_idx = col_to_char.get(point.column.0).copied().flatten()?;
            hyperlink::url_covering_char(&text, char_idx)
        })
    }

    /// Column range (exclusive end) of the URL under `point`, if any.
    fn url_col_span_at(
        &self,
        point: alacritty_terminal::index::Point,
    ) -> Option<(alacritty_terminal::index::Line, usize, usize)> {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::Column;
        use alacritty_terminal::term::cell::Flags;

        self.state.with_term(|term| {
            let grid = term.grid();
            let cols = term.columns();
            if cols == 0 || point.column.0 >= cols {
                return None;
            }
            if point.line < grid.topmost_line() || point.line > grid.bottommost_line() {
                return None;
            }

            let mut text = String::new();
            let mut col_to_char: Vec<Option<usize>> = vec![None; cols];
            let mut char_to_col: Vec<usize> = Vec::new();
            for col in 0..cols {
                let cell = &grid[alacritty_terminal::index::Point::new(point.line, Column(col))];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = if cell.c == '\0' { ' ' } else { cell.c };
                let idx = text.chars().count();
                text.push(ch);
                col_to_char[col] = Some(idx);
                char_to_col.push(col);
            }
            let char_idx = col_to_char.get(point.column.0).copied().flatten()?;
            let (start_c, end_c) = hyperlink::url_char_spans(&text)
                .into_iter()
                .find(|(s, e)| char_idx >= *s && char_idx < *e)?;
            let start_col = *char_to_col.get(start_c)?;
            let end_col = char_to_col
                .get(end_c.saturating_sub(1))
                .map(|c| c + 1)
                .unwrap_or(cols)
                .min(cols);
            Some((point.line, start_col, end_col))
        })
    }

    /// Width in pixels of the optional left line-number gutter (0 when disabled).
    ///
    /// Uses `scrollback + screen rows` for digit width so this never locks the
    /// term mutex (safe to call while the paint path already holds `term_arc`).
    fn line_number_gutter_width(&self) -> f32 {
        if !self.config.show_line_numbers {
            return 0.0;
        }
        let cell_w: f32 = self.renderer.cell_width.into();
        if cell_w <= 0.0 {
            return 0.0;
        }
        let max_num = (self.config.scrollback + self.state.rows()).max(1);
        let digits = ((max_num as f32).log10().floor() as usize + 1).max(2);
        digits as f32 * cell_w + LINE_NUMBER_PAD * 2.0
    }

    fn copy_selection_to_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self
            .state
            .with_term(|term| term.selection_to_string())
            .filter(|s| !s.is_empty());
        let Some(text) = text else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// Handle scroll events — wheel / trackpad moves the scrollback viewport.
    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use alacritty_terminal::grid::Scroll;

        let line_height = self.renderer.cell_height.max(px(1.0));
        let pixel_delta = event.delta.pixel_delta(line_height);
        let dy: f32 = pixel_delta.y.into();
        if dy.abs() < f32::EPSILON {
            return;
        }

        // Positive wheel delta (up) → increase display_offset (older history).
        let lines = ((dy / f32::from(line_height)) * SCROLL_MULTIPLIER).round() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_display(Scroll::Delta(lines), cx);
    }

    fn scroll_display(
        &mut self,
        scroll: alacritty_terminal::grid::Scroll,
        cx: &mut Context<Self>,
    ) {
        let changed = self.state.with_term_mut(|term| {
            let before = term.grid().display_offset();
            term.scroll_display(scroll);
            term.grid().display_offset() != before
        });
        if changed {
            cx.notify();
        }
    }

    /// Scroll metrics for the overlay scrollbar. `None` when there is no history.
    fn scroll_metrics(&self) -> Option<ScrollMetrics> {
        use alacritty_terminal::grid::Dimensions;
        self.state.with_term(|term| {
            let history = term.history_size();
            if history == 0 {
                return None;
            }
            Some(ScrollMetrics {
                display_offset: term.grid().display_offset(),
                history,
                screen_lines: term.screen_lines().max(1),
            })
        })
    }

    fn scrollbar_geometry(&self, metrics: &ScrollMetrics) -> Option<ScrollbarGeometry> {
        let track = self.scrollbar_track_bounds();
        let track_h: f32 = track.size.height.into();
        if track_h <= 0.0 {
            return None;
        }

        let content = (metrics.history + metrics.screen_lines) as f32;
        let mut thumb_h = (metrics.screen_lines as f32 / content) * track_h;
        thumb_h = thumb_h.clamp(SCROLLBAR_MIN_THUMB.min(track_h), track_h);

        let max_offset = metrics.history as f32;
        // document offset from top: 0 at oldest, history at live edge
        let doc_offset = (metrics.history - metrics.display_offset) as f32;
        let travel = (track_h - thumb_h).max(0.0);
        let thumb_y = if max_offset <= 0.0 {
            0.0
        } else {
            (doc_offset / max_offset) * travel
        };

        Some(ScrollbarGeometry {
            track,
            thumb_y: px(thumb_y),
            thumb_h: px(thumb_h),
        })
    }

    fn scrollbar_paint_info(&self) -> Option<ScrollbarGeometry> {
        let metrics = self.scroll_metrics()?;
        self.scrollbar_geometry(&metrics)
    }

    fn scrollbar_track_bounds(&self) -> Bounds<Pixels> {
        let pad_top = self.config.padding.top;
        let pad_bottom = self.config.padding.bottom;
        Bounds {
            origin: Point {
                x: self.last_bounds.right() - px(SCROLLBAR_WIDTH),
                y: self.last_bounds.origin.y + pad_top,
            },
            size: Size {
                width: px(SCROLLBAR_WIDTH),
                height: (self.last_bounds.size.height - pad_top - pad_bottom).max(px(0.0)),
            },
        }
    }

    fn handle_scrollbar_mouse_down(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(metrics) = self.scroll_metrics() else {
            return false;
        };
        let Some(geo) = self.scrollbar_geometry(&metrics) else {
            return false;
        };
        if !geo.track.contains(&position) {
            return false;
        }

        let thumb_top = geo.track.origin.y + geo.thumb_y;
        let thumb_bottom = thumb_top + geo.thumb_h;
        if position.y >= thumb_top && position.y <= thumb_bottom {
            self.scrollbar_drag = Some(ScrollbarDrag {
                pointer_y_in_thumb: position.y - thumb_top,
            });
        } else {
            // Click in track: jump so thumb centers on the click.
            let thumb_h: f32 = geo.thumb_h.into();
            let track_h: f32 = geo.track.size.height.into();
            let click_y: f32 = (position.y - geo.track.origin.y).into();
            let target_thumb_y = (click_y - thumb_h * 0.5).clamp(0.0, (track_h - thumb_h).max(0.0));
            self.set_display_offset_from_thumb_y(target_thumb_y, &metrics, &geo, cx);
            self.scrollbar_drag = Some(ScrollbarDrag {
                pointer_y_in_thumb: px(thumb_h * 0.5),
            });
        }
        self.selecting = false;
        cx.notify();
        true
    }

    fn update_scrollbar_drag(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.scrollbar_drag.as_ref() else {
            return;
        };
        let pointer_in_thumb = drag.pointer_y_in_thumb;
        let Some(metrics) = self.scroll_metrics() else {
            return;
        };
        let Some(geo) = self.scrollbar_geometry(&metrics) else {
            return;
        };
        let thumb_h: f32 = geo.thumb_h.into();
        let track_h: f32 = geo.track.size.height.into();
        let y: f32 = (pointer_y - geo.track.origin.y - pointer_in_thumb).into();
        let target_thumb_y = y.clamp(0.0, (track_h - thumb_h).max(0.0));
        self.set_display_offset_from_thumb_y(target_thumb_y, &metrics, &geo, cx);
    }

    fn set_display_offset_from_thumb_y(
        &mut self,
        thumb_y: f32,
        metrics: &ScrollMetrics,
        geo: &ScrollbarGeometry,
        cx: &mut Context<Self>,
    ) {
        use alacritty_terminal::grid::Scroll;

        let thumb_h: f32 = geo.thumb_h.into();
        let track_h: f32 = geo.track.size.height.into();
        let travel = (track_h - thumb_h).max(0.0);
        let ratio = if travel <= 0.0 {
            1.0
        } else {
            (thumb_y / travel).clamp(0.0, 1.0)
        };
        // ratio 0 → top of history (display_offset = history)
        // ratio 1 → live edge (display_offset = 0)
        let target_offset = ((1.0 - ratio) * metrics.history as f32).round() as usize;
        let current = metrics.display_offset;
        let delta = target_offset as i32 - current as i32;
        if delta != 0 {
            self.scroll_display(Scroll::Delta(delta), cx);
        }
    }

    /// Process pending terminal events.
    ///
    /// Called after `process_bytes` (no window) and again during render (with window)
    /// so reply writebacks stay ordered and UI callbacks still run.
    fn process_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_pending_events(Some(window), cx);
    }

    fn dispatch_pending_events(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TerminalEvent::Wakeup => {}
                TerminalEvent::Bell => {
                    if let Some(callback) = self.bell_callback.as_ref() {
                        if let Some(window) = window.as_mut() {
                            callback(window, cx);
                        }
                    }
                }
                TerminalEvent::Title(title) => {
                    if let Some(callback) = self.title_callback.as_ref() {
                        if let Some(window) = window.as_mut() {
                            callback(window, cx, &title);
                        }
                    }
                }
                TerminalEvent::ClipboardStore(text) => {
                    if let Some(callback) = self.clipboard_store_callback.as_ref() {
                        if let Some(window) = window.as_mut() {
                            callback(window, cx, &text);
                        }
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                TerminalEvent::ClipboardLoad(format) => {
                    let payload = match cx.read_from_clipboard().and_then(|item| item.text()) {
                        Some(text) => format(&text),
                        None => format(""),
                    };
                    self.write_to_pty(payload.as_bytes(), cx);
                }
                TerminalEvent::PtyWrite(data) => {
                    self.write_to_pty(data.as_bytes(), cx);
                }
                TerminalEvent::ColorRequest(index, format) => {
                    let color = self
                        .state
                        .with_term(|term| term.colors()[index])
                        .unwrap_or_else(|| self.config.colors.rgb_at_index(index));
                    self.write_to_pty(format(color).as_bytes(), cx);
                }
                TerminalEvent::TextAreaSizeRequest(format) => {
                    let font_px: f32 = self.config.font_size.into();
                    let cell_h = font_px * self.config.line_height_multiplier;
                    let cell_w = font_px * 0.6;
                    let size = alacritty_terminal::event::WindowSize {
                        num_lines: self.state.rows() as u16,
                        num_cols: self.state.cols() as u16,
                        cell_width: cell_w.max(1.0) as u16,
                        cell_height: cell_h.max(1.0) as u16,
                    };
                    self.write_to_pty(format(size).as_bytes(), cx);
                }
                TerminalEvent::Exit => {
                    self.note_session_ended(cx);
                    if let Some(callback) = self.exit_callback.as_ref() {
                        if let Some(window) = window.as_mut() {
                            callback(window, cx);
                        }
                    }
                }
            }
        }
    }

    /// Mark the session dead once; emit [`TerminalViewEvent::SessionEnded`].
    fn note_session_ended(&mut self, cx: &mut Context<Self>) {
        if !self.session_alive {
            return;
        }
        self.session_alive = false;
        self.ime_marked = None;
        cx.emit(crate::terminal::TerminalViewEvent::SessionEnded);
        cx.notify();
    }

    pub fn session_alive(&self) -> bool {
        self.session_alive
    }

    fn write_to_pty(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if !self.session_alive || bytes.is_empty() {
            return;
        }
        let result = {
            let mut writer = self.stdin_writer.lock();
            let w = writer.write_all(bytes).and_then(|_| writer.flush());
            w
        };
        if result.is_err() {
            self.note_session_ended(cx);
        }
    }

    fn utf16_len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    /// Commit text from IME / WM_CHAR (not bracketed paste).
    fn insert_composed_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            self.ime_marked = None;
            return;
        }
        if !self.session_alive {
            self.ime_marked = None;
            return;
        }
        self.state.with_term_mut(|term| {
            term.selection = None;
            term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        });
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        self.write_to_pty(normalized.as_bytes(), cx);
        self.ime_marked = None;
        cx.notify();
    }

    /// Cursor cell bounds in window coordinates (for IME candidate window).
    fn cursor_bounds_window(&self) -> Option<Bounds<Pixels>> {
        use alacritty_terminal::term::point_to_viewport;
        let cell_w = self.renderer.cell_width;
        let cell_h = self.renderer.cell_height;
        if cell_w <= px(0.0) || cell_h <= px(0.0) {
            return None;
        }
        let gutter = self.line_number_gutter_width();
        let (col, row) = self.state.with_term(|term| {
            let grid = term.grid();
            let cursor = grid.cursor.point;
            let display_offset = grid.display_offset();
            let vp = point_to_viewport(display_offset, cursor)?;
            Some((cursor.column.0, vp.line))
        })?;
        let origin_x = self.last_bounds.origin.x + self.config.padding.left + px(gutter);
        let origin_y = self.last_bounds.origin.y + self.config.padding.top;
        Some(Bounds {
            origin: Point {
                x: origin_x + cell_w * (col as f32),
                y: origin_y + cell_h * (row as f32),
            },
            size: Size {
                width: cell_w,
                height: cell_h,
            },
        })
    }

    /// Insert text into the PTY (uses bracketed paste when the shell enables it).
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        use alacritty_terminal::grid::Scroll;
        use alacritty_terminal::term::TermMode;
        if !self.session_alive {
            return;
        }
        self.state
            .with_term_mut(|term| term.scroll_display(Scroll::Bottom));
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.state.mode().contains(TermMode::BRACKETED_PASTE) {
            let mut buf = Vec::with_capacity(normalized.len() + 16);
            buf.extend_from_slice(b"\x1b[200~");
            buf.extend(normalized.bytes().filter(|&b| b != 0x1b));
            buf.extend_from_slice(b"\x1b[201~");
            self.write_to_pty(&buf, cx);
        } else {
            self.write_to_pty(normalized.as_bytes(), cx);
        }
    }

    /// Get the current terminal dimensions.
    ///
    /// # Returns
    ///
    /// A tuple of (columns, rows).
    pub fn dimensions(&self) -> (usize, usize) {
        (self.state.cols(), self.state.rows())
    }

    /// Resize the terminal to new dimensions.
    ///
    /// This method should be called when the terminal view size changes.
    /// It updates the internal grid and notifies the terminal process of the new size.
    ///
    /// # Arguments
    ///
    /// * `cols` - New number of columns
    /// * `rows` - New number of rows
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.state.resize(cols, rows);
    }

    /// Get the current terminal configuration.
    ///
    /// # Returns
    ///
    /// A reference to the current configuration.
    pub fn config(&self) -> &TerminalConfig {
        &self.config
    }

    /// Get the focus handle for this terminal view.
    ///
    /// # Returns
    ///
    /// A reference to the focus handle.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Update the terminal configuration.
    ///
    /// This method updates the terminal's configuration, including font settings,
    /// padding, and color palette. Changes take effect on the next render.
    ///
    /// # Arguments
    ///
    /// * `config` - The new configuration to apply
    /// * `cx` - The context for triggering a repaint
    pub fn update_config(&mut self, config: TerminalConfig, cx: &mut Context<Self>) {
        // Update renderer with new font settings and palette
        self.renderer.font_family = config.font_family.clone();
        self.renderer.font_size = config.font_size;
        self.renderer.line_height_multiplier = config.line_height_multiplier;
        self.renderer.palette = config.colors.clone();

        // Store the new config
        self.config = config;

        // Trigger a repaint - cell dimensions will be recalculated via measure_cell()
        cx.notify();
    }

    /// Calculate terminal dimensions from pixel bounds and cell size.
    ///
    /// Helper method to determine how many columns and rows fit in the given bounds.
    #[allow(dead_code)]
    fn calculate_dimensions(&self, bounds: Bounds<Pixels>) -> (usize, usize) {
        let width_f32: f32 = bounds.size.width.into();
        let height_f32: f32 = bounds.size.height.into();
        let cell_width_f32: f32 = self.renderer.cell_width.into();
        let cell_height_f32: f32 = self.renderer.cell_height.into();

        let cols = ((width_f32 / cell_width_f32) as usize).max(1);
        let rows = ((height_f32 / cell_height_f32) as usize).max(1);
        (cols, rows)
    }
}

fn paint_line_number_gutter(
    bounds: Bounds<Pixels>,
    term: &alacritty_terminal::Term<GpuiEventProxy>,
    pad: Edges<Pixels>,
    gutter: f32,
    font_family: &str,
    font_size: Pixels,
    cell_h: Pixels,
    line_height_multiplier: f32,
    window: &mut Window,
    cx: &mut App,
) {
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Point as AlacPoint};
    use alacritty_terminal::term::viewport_to_point;
    use gpui::{Font, FontFeatures, FontStyle, FontWeight, SharedString, TextRun};

    let history = term.history_size() as i32;
    let display_offset = term.grid().display_offset();
    let num_lines = term.screen_lines();
    let color = theme::TEXT_DISABLED;
    let draw_size = font_size * 0.85;
    let base_height = cell_h / line_height_multiplier;
    let vertical_offset = (cell_h - base_height) / 2.0;

    let sep_x = bounds.origin.x + pad.left + px(gutter) - px(1.0);
    window.paint_quad(quad(
        Bounds {
            origin: Point {
                x: sep_x,
                y: bounds.origin.y + pad.top,
            },
            size: Size {
                width: px(1.0),
                height: (bounds.size.height - pad.top - pad.bottom).max(px(0.0)),
            },
        },
        px(0.0),
        hsla(0.0, 0.0, 1.0, 0.06),
        Edges::default(),
        transparent_black(),
        Default::default(),
    ));

    for line_idx in 0..num_lines {
        let point = viewport_to_point(display_offset, AlacPoint::new(line_idx, Column(0)));
        let number = (point.line.0 + history + 1).max(1) as usize;
        let label = number.to_string();
        let text_run = TextRun {
            len: label.len(),
            font: Font {
                family: SharedString::from(font_family.to_string()),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped =
            window
                .text_system()
                .shape_line(SharedString::from(label), draw_size, &[text_run], None);
        let text_w: f32 = shaped.width.into();
        let x = bounds.origin.x + pad.left + px(gutter - LINE_NUMBER_PAD - text_w);
        let y = bounds.origin.y + pad.top + cell_h * (line_idx as f32) + vertical_offset;
        let _ = shaped.paint(Point { x, y }, cell_h, window, cx);
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, _) = self.ime_marked.as_ref()?;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let start = range.start.min(utf16.len());
        let end = range.end.min(utf16.len());
        *adjusted_range = Some(start..end);
        String::from_utf16(&utf16[start..end]).ok()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // Zed terminal: always report a caret at 0 when not in alt-screen; keeps
        // IME candidate positioning stable and avoids a “stuck composing” empty range.
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.ime_marked.as_ref().and_then(|(text, range)| {
            if text.is_empty() {
                None
            } else {
                Some(range.clone())
            }
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.ime_marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_composed_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = Self::utf16_len(new_text);
        let marked = new_selected_range.unwrap_or(0..len);
        if new_text.is_empty() {
            self.ime_marked = None;
        } else {
            self.ime_marked = Some((new_text.to_string(), marked));
        }
        // Preedit is not written to the PTY; candidacy UI is owned by the OS IME.
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds_window()
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending events
        self.process_events(window, cx);

        // Get terminal state and renderer for rendering
        let state_arc = self.state.term_arc();
        let resize_callback = self.resize_callback.clone();
        let padding = self.config.padding;
        let view = cx.entity();
        let view_paint = view.clone();

        div()
            .relative()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .track_focus(&self.focus_handle)
            .when(self.hyperlink_mods && self.hover_hyperlink, |d| {
                d.cursor(CursorStyle::PointingHand)
            })
            .when(!(self.hyperlink_mods && self.hover_hyperlink), |d| {
                d.cursor(CursorStyle::IBeam)
            })
            .on_key_down(cx.listener(Self::on_key_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        view.update(cx, |this, _cx| {
                            this.renderer.measure_cell(window);
                            this.last_bounds = bounds;
                        });
                        bounds
                    },
                    move |bounds, _, window, cx| {
                        use alacritty_terminal::grid::Dimensions;

                        // Register IME / text input handler while this terminal is focused.
                        let focus = view_paint.read(cx).focus_handle.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view_paint.clone()),
                            cx,
                        );

                        let measured_renderer = view_paint.read(cx).renderer.clone();
                        let line_gutter = px(view_paint.read(cx).line_number_gutter_width());

                        // Leave gutters for line numbers (left) and overlay scrollbar (right).
                        let scrollbar_gutter = px(SCROLLBAR_WIDTH + 2.0);
                        let available_width: f32 = (bounds.size.width
                            - padding.left
                            - padding.right
                            - line_gutter
                            - scrollbar_gutter)
                            .into();
                        let available_height: f32 =
                            (bounds.size.height - padding.top - padding.bottom).into();
                        let cell_width_f32: f32 = measured_renderer.cell_width.into();
                        let cell_height_f32: f32 = measured_renderer.cell_height.into();

                        let cols = ((available_width / cell_width_f32) as usize).max(1);
                        let rows = ((available_height / cell_height_f32) as usize).max(1);

                        struct TermSize {
                            cols: usize,
                            rows: usize,
                        }
                        impl Dimensions for TermSize {
                            fn total_lines(&self) -> usize {
                                self.rows
                            }
                            fn screen_lines(&self) -> usize {
                                self.rows
                            }
                            fn columns(&self) -> usize {
                                self.cols
                            }
                            fn last_column(&self) -> alacritty_terminal::index::Column {
                                alacritty_terminal::index::Column(self.cols.saturating_sub(1))
                            }
                            fn bottommost_line(&self) -> alacritty_terminal::index::Line {
                                alacritty_terminal::index::Line(self.rows as i32 - 1)
                            }
                            fn topmost_line(&self) -> alacritty_terminal::index::Line {
                                alacritty_terminal::index::Line(0)
                            }
                        }

                        // Gather paint config before / without nested term locks.
                        // `state_arc` is already held below — never call `with_term` here.
                        let show_line_numbers = view_paint.read(cx).config.show_line_numbers;
                        let base_pad = view_paint.read(cx).config.padding;
                        let line_gutter_w = view_paint.read(cx).line_number_gutter_width();
                        let line_nums = show_line_numbers.then(|| {
                            (
                                line_gutter_w,
                                base_pad,
                                view_paint.read(cx).renderer.font_family.clone(),
                                view_paint.read(cx).renderer.font_size,
                                view_paint.read(cx).renderer.cell_height,
                                view_paint.read(cx).renderer.line_height_multiplier,
                            )
                        });
                        let mut content_pad = base_pad;
                        content_pad.left = content_pad.left + px(line_gutter_w);

                        let mut term = state_arc.lock();
                        let current_cols = term.columns();
                        let current_rows = term.screen_lines();
                        if cols != current_cols || rows != current_rows {
                            if let Some(ref callback) = resize_callback {
                                callback(cols, rows);
                            }
                            term.resize(TermSize { cols, rows });
                            // Keep TerminalState.cols/rows aligned for hit-testing.
                            drop(term);
                            view_paint.update(cx, |this, _| {
                                this.state.set_dimensions(cols, rows);
                            });
                            term = state_arc.lock();
                        }

                        let hover_url_span = view_paint.read(cx).hover_url_span;
                        measured_renderer.paint(
                            bounds,
                            content_pad,
                            &term,
                            hover_url_span,
                            window,
                            cx,
                        );
                        if let Some((gutter, pad, family, font_size, cell_h, mult)) = line_nums {
                            paint_line_number_gutter(
                                bounds,
                                &term,
                                pad,
                                gutter,
                                &family,
                                font_size,
                                cell_h,
                                mult,
                                window,
                                cx,
                            );
                        }
                        drop(term);

                        let scrollbar = view_paint.read(cx).scrollbar_paint_info();
                        if let Some(geo) = scrollbar {
                            let track = geo.track;
                            window.paint_quad(quad(
                                track,
                                px(4.0),
                                hsla(0.60, 0.04, 0.14, 0.35),
                                Edges::default(),
                                transparent_black(),
                                Default::default(),
                            ));
                            let thumb = Bounds {
                                origin: Point {
                                    x: track.origin.x + px(1.0),
                                    y: track.origin.y + geo.thumb_y,
                                },
                                size: Size {
                                    width: track.size.width - px(2.0),
                                    height: geo.thumb_h,
                                },
                            };
                            window.paint_quad(quad(
                                thumb,
                                px(3.0),
                                hsla(0.60, 0.04, 0.55, 0.55),
                                Edges::default(),
                                transparent_black(),
                                Default::default(),
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .when_some(self.render_find_bar(cx), |d, bar| d.child(bar))
            .when_some(self.render_context_menu(cx), |d, menu| d.child(menu))
            .when(!self.session_alive, |d| {
                d.child(
                    div()
                        .id("term-disconnected-banner")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .px_3()
                        .py_2()
                        .bg(hsla(0.02, 0.45, 0.22, 0.92))
                        .border_b_1()
                        .border_color(hsla(0.02, 0.50, 0.40, 1.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xffffff))
                                .child(
                                    "Disconnected — click Reconnect in the status bar to restore this session.",
                                ),
                        ),
                )
            })
    }
}

// Tests are omitted due to macro expansion issues with the test attribute
// in this configuration. Integration tests can be added separately.
