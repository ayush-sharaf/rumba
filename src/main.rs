//! rumba — a terminal UI music player for YouTube Music.

mod api;
mod app;
mod auth;
mod keys;
mod log;
mod models;
mod player;
mod sysvol;
mod ui;

use anyhow::{Context, Result};
use api::{Api, Auth};
use app::App;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use player::Player;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("rumba")
}

fn check_dependencies() -> Result<()> {
    for (bin, hint) in [("mpv", "brew install mpv"), ("yt-dlp", "brew install yt-dlp")] {
        if which(bin).is_none() {
            anyhow::bail!("required dependency `{bin}` not found on PATH (install: {hint})");
        }
    }
    Ok(())
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(bin))
        .find(|p| p.is_file())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    if has("-h") || has("--help") {
        print_help();
        return Ok(());
    }

    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    log::init(&dir.join("rumba.log"));
    log::log("rumba started");

    // `--browser NAME` forces a specific browser for cookie extraction.
    let browser = args
        .iter()
        .position(|a| a == "--browser")
        .and_then(|i| args.get(i + 1))
        .cloned();

    // Explicit re-login: always re-detect and verbosely report.
    if has("--login") || args.first().map(String::as_str) == Some("login") {
        if refresh_cookies(&dir, browser.as_deref())?.is_some() {
            return Ok(());
        }
        return interactive_login(&dir, browser.as_deref());
    }

    check_dependencies()?;

    // Normal startup: silently refresh the browser session so cookies stay
    // current. Fall back to a cached cookie if the browser can't be read, and
    // only prompt interactively if there's no session at all.
    match refresh_cookies(&dir, browser.as_deref())? {
        Some(_) => {}
        None if auth::cookie_path(&dir).exists() => {}
        None => interactive_login(&dir, browser.as_deref())?,
    }

    run_tui(&dir)
}

/// Non-interactive: detect a logged-in browser session and overwrite the cached
/// cookies. Returns the browser name if one was found.
fn refresh_cookies(dir: &Path, browser: Option<&str>) -> Result<Option<String>> {
    if let Some((b, cookie)) = auth::detect_session(browser) {
        std::fs::write(auth::cookie_path(dir), cookie)?;
        let _ = std::fs::write(dir.join("source.txt"), &b);
        println!("✓ Connected to YouTube Music via {b}.");
        Ok(Some(b))
    } else {
        Ok(None)
    }
}

/// No session found: open music.youtube.com, wait for the user to sign in,
/// then retry detection.
fn interactive_login(dir: &Path, browser: Option<&str>) -> Result<()> {
    println!("\nNo signed-in session found. Opening music.youtube.com — please log in");
    println!("there, then come back here.");
    open_url("https://music.youtube.com");
    print!("\nPress Enter once you're logged in… ");
    let _ = stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;

    if refresh_cookies(dir, browser)?.is_some() {
        Ok(())
    } else {
        anyhow::bail!(
            "still no YouTube Music session found.\n\
             • Make sure you're logged into music.youtube.com\n\
             • Try a specific browser:  rumba --login --browser firefox\n\
             • Firefox is the most reliable; Chrome may prompt for Keychain access."
        )
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd = "";
    if !cmd.is_empty() {
        let _ = std::process::Command::new(cmd).arg(url).status();
    }
}

fn run_tui(dir: &Path) -> Result<()> {
    let cookie = std::fs::read_to_string(auth::cookie_path(dir))
        .context("could not read saved session; run `rumba --login`")?;
    let api = Api::spawn(Auth::Cookie(cookie))?;

    let source = std::fs::read_to_string(dir.join("source.txt"))
        .unwrap_or_else(|_| "browser".into())
        .trim()
        .to_string();

    let keymap = keys::Keymap::load(dir);

    let socket = format!("/tmp/rumba-mpv-{}.sock", std::process::id());
    // Pass the browser session to mpv's yt-dlp for high-bitrate streams.
    let player_browser = (source != "browser").then_some(source.as_str());
    let player = Player::spawn("mpv", &socket, player_browser)?;

    // Restore the terminal even if we panic mid-render.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;

    let mut app = App::new(api, player, source, keymap);

    // Ask the terminal to report modifier+key combos (e.g. Cmd+Backspace) via
    // the Kitty keyboard protocol. No-op on terminals that don't support it.
    let enhanced = matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    );
    if enhanced {
        let _ = execute!(
            stdout(),
            event::PushKeyboardEnhancementFlags(
                event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        );
    }

    let backend = ratatui::backend::CrosstermBackend::new(stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;

    let res = event_loop(&mut terminal, &mut app);

    let _ = app.player.stop();
    if enhanced {
        let _ = execute!(stdout(), event::PopKeyboardEnhancementFlags);
    }
    restore_terminal()?;
    let _ = std::fs::remove_file(&socket);
    res
}

fn event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, app))?;
        app.poll_sidecar();
        app.poll_playback();

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key)?;
                }
            }
        }
    }
    Ok(())
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn print_help() {
    println!(
        r#"rumba — YouTube Music in your terminal

USAGE:
    rumba                      Launch (connects via your browser on first run)
    rumba --login              Re-connect using your browser session
    rumba --login --browser firefox
                               Force a specific browser (chrome/brave/edge/safari…)
    rumba --help               Show this help

On first launch rumba reads your logged-in YouTube Music session from your
browser. If you're not signed in, it opens music.youtube.com so you can.
Firefox needs no extra permission; Chrome may show a one-time Keychain prompt.

KEYS (in app):
    1-7 / Tab           Switch tabs (Home, Search, Library, Liked, Playlists, Queue, Account)
    j/k or ↑/↓          Move selection
    Enter               Play a song → endless radio of related songs;
                        in an album/playlist plays it in order; or drill in
    Esc                 Back (browse) / close lyrics / close playlist
    a                   Add selected track to the queue
    r                   Start a radio/autoplay queue from the selection
    y                   Show lyrics for the selected/now-playing track
    L / D               Like / dislike the selected track
    s                   Sort the list (toggle title ↔ artist)
    Space               Play / pause
    n / p               Next / previous track
    ←/→ or h/l          Seek -5s / +5s
    +/-                 Volume up / down
    /                   Search (songs, artists, albums, playlists)
    q / Ctrl-C          Quit
"#
    );
}
