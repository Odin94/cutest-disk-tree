use std::io::Write;
use std::path::Path;

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

