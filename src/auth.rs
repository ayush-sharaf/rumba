//! Browser-cookie based authentication.
//!
//! We read the user's logged-in YouTube Music session straight from their
//! browser's cookie store (via `rookie`) and hand the resulting `Cookie:`
//! header to `ytmapi-rs`. No copy-paste, no Google Cloud project.

use anyhow::{anyhow, Result};
use rookie::common::enums::Cookie;
use std::path::{Path, PathBuf};

/// Presence of either proves a real signed-in session (SAPISIDHASH is derived
/// from these on each request).
const AUTH_COOKIES: [&str; 2] = ["__Secure-3PAPISID", "SAPISID"];

/// Browsers we probe, in order. Firefox first: it needs no Keychain access.
pub const BROWSERS: [&str; 8] = [
    "firefox", "chrome", "brave", "edge", "chromium", "opera", "arc", "safari",
];

pub fn cookie_path(dir: &Path) -> PathBuf {
    dir.join("cookies.txt")
}

fn read_browser(browser: &str, domains: Option<Vec<String>>) -> Result<Vec<Cookie>> {
    let res = match browser {
        "firefox" => rookie::firefox(domains),
        "chrome" => rookie::chrome(domains),
        "brave" => rookie::brave(domains),
        "edge" => rookie::edge(domains),
        "chromium" => rookie::chromium(domains),
        "opera" => rookie::opera(domains),
        "arc" => rookie::arc(domains),
        // Safari cookie access is macOS-only in `rookie`; on other platforms
        // this falls through to the catch-all below.
        #[cfg(target_os = "macos")]
        "safari" => rookie::safari(domains),
        other => return Err(anyhow!("unsupported browser on this platform: {other}")),
    };
    res.map_err(|e| anyhow!("{e:?}"))
}

/// Extract a `Cookie:` header for `browser` if it holds a YT Music session.
fn extract(browser: &str) -> Result<Option<String>> {
    let cookies = read_browser(browser, Some(vec!["youtube.com".to_string()]))?;
    let mut pairs = Vec::new();
    let mut has_auth = false;
    for c in &cookies {
        if c.domain.contains("youtube.com") {
            pairs.push(format!("{}={}", c.name, c.value));
            if AUTH_COOKIES.contains(&c.name.as_str()) {
                has_auth = true;
            }
        }
    }
    if has_auth {
        Ok(Some(pairs.join("; ")))
    } else {
        Ok(None)
    }
}

/// Scan browsers (or just `browser`) for a logged-in session.
/// Returns `(browser_name, cookie_header)`.
pub fn detect_session(browser: Option<&str>) -> Option<(String, String)> {
    let list: Vec<&str> = match browser {
        Some(b) => vec![b],
        None => BROWSERS.to_vec(),
    };
    for b in list {
        if let Ok(Some(cookie)) = extract(b) {
            return Some((b.to_string(), cookie));
        }
    }
    None
}
