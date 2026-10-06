//! Application logging (`docs/LOGGING.md`).
//!
//! Priority: env (`LOOM_LOG` / `LOOM_LOG_DIR`) > Settings > built-in defaults.
//! Logger is installed once and can be reconfigured from Settings without restart.

use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::model::workspace::LoggingSettings;
use crate::shared::paths;

static LOGGER: LoomLogger = LoomLogger {
    state: Mutex::new(LoggerState::empty()),
};
static INSTALLED: OnceLock<()> = OnceLock::new();
static QUIT_TRACE: AtomicBool = AtomicBool::new(false);

struct LoggerState {
    enabled: bool,
    level: LevelFilter,
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    also_stderr: bool,
}

impl LoggerState {
    const fn empty() -> Self {
        Self {
            enabled: false,
            level: LevelFilter::Off,
            path: PathBuf::new(),
            max_bytes: 0,
            file: None,
            also_stderr: false,
        }
    }
}

struct LoomLogger {
    state: Mutex<LoggerState>,
}

impl Log for LoomLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        state.enabled && metadata.level() <= state.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let line = format_record(record);
        if state.also_stderr {
            let _ = writeln!(io::stderr(), "{line}");
        }
        if let Some(file) = state.file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
            maybe_rotate(&mut state);
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(file) = state.file.as_mut() {
                let _ = file.flush();
            }
        }
        let _ = io::stderr().flush();
    }
}

fn format_record(record: &Record) -> String {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    format!(
        "{ts} {level:<5} [{target}] {args}",
        level = record.level(),
        target = record.target(),
        args = record.args()
    )
}

fn maybe_rotate(state: &mut LoggerState) {
    if state.max_bytes == 0 {
        return;
    }
    let Ok(meta) = fs::metadata(&state.path) else {
        return;
    };
    if meta.len() < state.max_bytes {
        return;
    }
    let old = state.path.with_extension("log.old");
    let _ = state.file.take();
    let _ = fs::remove_file(&old);
    let _ = fs::rename(&state.path, &old);
    state.file = open_log_file(&state.path).ok();
}

fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

fn effective_dir(settings: &LoggingSettings) -> PathBuf {
    if let Ok(dir) = std::env::var("LOOM_LOG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    settings
        .dir
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned()
        .unwrap_or_else(paths::logs_dir)
}

fn effective_level(settings: &LoggingSettings) -> LevelFilter {
    if let Ok(raw) = std::env::var("LOOM_LOG") {
        if let Some(filter) = parse_level_filter(raw.trim()) {
            return filter;
        }
    }
    settings.level.to_filter()
}

fn parse_level_filter(raw: &str) -> Option<LevelFilter> {
    // Support plain level or RUST_LOG-style `info,module=debug` (take global token).
    let global = raw.split(',').next()?.trim();
    if global.contains('=') {
        return None;
    }
    match global.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn apply_config(state: &mut LoggerState, settings: &LoggingSettings) {
    let also_stderr = io::stderr().is_terminal();
    let enabled = settings.enabled;
    let level = effective_level(settings);
    let dir = effective_dir(settings);
    let path = dir.join("Loom.log");
    let max_bytes = settings.max_bytes.max(64 * 1024);

    state.enabled = enabled;
    state.level = level;
    state.path = path.clone();
    state.max_bytes = max_bytes;
    state.also_stderr = also_stderr;
    state.file = None;

    if enabled {
        match open_log_file(&path) {
            Ok(file) => state.file = Some(file),
            Err(err) => {
                // Keep going with stderr if possible.
                let _ = writeln!(
                    io::stderr(),
                    "loom: could not open log file {}: {err}",
                    path.display()
                );
                state.also_stderr = true;
            }
        }
    }

    log::set_max_level(if enabled { level } else { LevelFilter::Off });
}

/// Install the global logger (once) and apply Settings + env.
pub fn init(settings: &LoggingSettings) {
    QUIT_TRACE.store(quit_trace_env_enabled(), Ordering::Relaxed);
    if let Ok(mut state) = LOGGER.state.lock() {
        apply_config(&mut state, settings);
    }
    let _ = INSTALLED.get_or_init(|| {
        let _ = log::set_logger(&LOGGER);
    });
    log::info!(
        target: "loom",
        "logger ready path={} level={} enabled={}",
        current_log_path().display(),
        effective_level(settings),
        settings.enabled
    );
}

/// Hot-reload from Settings (env still wins for level/dir).
pub fn reconfigure(settings: &LoggingSettings) {
    if let Ok(mut state) = LOGGER.state.lock() {
        apply_config(&mut state, settings);
    }
    log::info!(
        target: "loom",
        "logger reconfigured path={} level={}",
        current_log_path().display(),
        effective_level(settings)
    );
}

pub fn current_log_path() -> PathBuf {
    LOGGER
        .state
        .lock()
        .map(|s| {
            if s.path.as_os_str().is_empty() {
                paths::log_file_path()
            } else {
                s.path.clone()
            }
        })
        .unwrap_or_else(|_| paths::log_file_path())
}

pub fn current_log_dir() -> PathBuf {
    current_log_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(paths::logs_dir)
}

fn quit_trace_env_enabled() -> bool {
    matches!(
        std::env::var("LOOM_QUIT_TRACE").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

/// Stage log for window-close diagnosis (`docs/WINDOW_CLOSE_HANG.md`).
/// Enabled when `LOOM_QUIT_TRACE=1`, or always at `debug` via normal filter.
pub fn quit_trace(stage: &str) {
    if QUIT_TRACE.load(Ordering::Relaxed) {
        // Force a line even if level is warn+ (append + stderr).
        force_line("loom::quit", Level::Info, stage);
        return;
    }
    log::debug!(target: "loom::quit", "{stage}");
}

fn force_line(target: &str, level: Level, msg: &str) {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("{ts} {level:<5} [{target}] {msg}");
    if let Ok(mut state) = LOGGER.state.lock() {
        if state.also_stderr || !state.enabled {
            let _ = writeln!(io::stderr(), "{line}");
        }
        if state.file.is_none() && state.enabled {
            state.file = open_log_file(&state.path).ok();
        }
        if let Some(file) = state.file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    } else {
        let _ = writeln!(io::stderr(), "{line}");
    }
}

/// Ensure log file exists (for Open / Reveal before first write).
pub fn ensure_log_file() -> io::Result<PathBuf> {
    let path = current_log_path();
    if !path.exists() {
        let _ = open_log_file(&path)?;
    }
    Ok(path)
}
