use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;

pub fn write_debug_log(path: &Path, message: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            f,
            "{} {}",
            Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            message
        );
        let _ = f.flush();
    }
}

// ── log::Log integration ─────────────────────────────────────────────────────

/// A [`log::Log`] implementation that appends records to the debug log file.
///
/// Only records whose target starts with `"disk_tree"` are written; all other
/// crates are silenced so third-party noise doesn't pollute the log.
struct DebugFileLogger {
    path: Mutex<PathBuf>,
}

impl log::Log for DebugFileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().starts_with("disk_tree")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(path) = self.path.lock() {
            write_debug_log(
                &path,
                &format!("[{}] {}: {}", record.level(), record.target(), record.args()),
            );
        }
    }

    fn flush(&self) {}
}

/// Install a global logger that writes `disk_tree::*` debug records to `path`.
///
/// Call once at application startup, right after the debug log path is known.
/// Silently does nothing if a logger is already registered.
pub fn init_debug_logger(path: PathBuf) {
    let logger = Box::new(DebugFileLogger { path: Mutex::new(path) });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

