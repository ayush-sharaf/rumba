//! Minimal append-only file logger (no external deps).
//!
//! `init` opens the log once; `log!`-style calls go to `~/.config/rumba/rumba.log`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

pub fn init(path: &Path) {
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = FILE.set(Mutex::new(f));
    }
}

/// Append a timestamped line to the log (no-op if logging isn't initialised).
pub fn log(msg: impl AsRef<str>) {
    if let Some(m) = FILE.get() {
        if let Ok(mut f) = m.lock() {
            let _ = writeln!(f, "[{}] {}", epoch_secs(), msg.as_ref());
        }
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
