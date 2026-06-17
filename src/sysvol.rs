//! System output-volume control.
//!
//! rumba's volume mirrors the OS output volume (so it matches what the rest of
//! the machine is doing) rather than mpv's private software volume. On macOS we
//! read/write it via `osascript`; other platforms fall back to no-ops and the
//! player keeps its own 0–100 value.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// Background watcher that keeps the system volume current even when it's
/// changed outside rumba (system settings, hardware keys). Polls once a second
/// on its own thread and exposes the latest value via an atomic.
pub struct Watcher {
    vol: Arc<AtomicI64>, // rounded percent, or -1 if unknown/unsupported
}

impl Watcher {
    pub fn spawn() -> Self {
        let initial = get().map(|v| v.round() as i64).unwrap_or(-1);
        let vol = Arc::new(AtomicI64::new(initial));
        let shared = Arc::clone(&vol);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            if let Some(cur) = get() {
                shared.store(cur.round() as i64, Ordering::Relaxed);
            }
        });
        Self { vol }
    }

    /// Latest known system volume (0–100), or None if unsupported.
    pub fn get(&self) -> Option<f64> {
        let v = self.vol.load(Ordering::Relaxed);
        (v >= 0).then_some(v as f64)
    }

    /// Set the system volume and reflect it immediately (don't wait for the poll).
    pub fn set(&self, v: f64) {
        set(v);
        self.vol.store(v.clamp(0.0, 100.0).round() as i64, Ordering::Relaxed);
    }
}

/// Read the current system output volume (0–100), if available.
pub fn get() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("osascript")
            .args(["-e", "output volume of (get volume settings)"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        return s.trim().parse::<f64>().ok();
    }
    #[allow(unreachable_code)]
    None
}

/// Set the system output volume (0–100).
pub fn set(vol: f64) {
    let v = vol.clamp(0.0, 100.0).round() as i64;
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("set volume output volume {v}")])
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = v;
    }
}
