# rumba

A terminal-UI music player for **YouTube Music**, built with [ratatui](https://ratatui.rs).
Search, browse artists/albums/playlists, play your library, see lyrics, and more —
all from the terminal. **Pure Rust, single binary** (no Python).

```
┌ rumba ───────────────────────────────────────────────────────────────────────┐
│ 1 Home  2 Search  3 Library  4 Liked  5 Playlists  6 Queue  7 Account           │
└────────────────────────────────────────────────────────────────────────────────┘
┌ Results for "daft punk"  (Enter: open · Esc: back) ───────────────────────────┐
│ ▶ ♪ Get Lucky                      Daft Punk                            6:09    │
│     Instant Crush                  Daft Punk                            5:37    │
│   🎤 Daft Punk                                                  ▸ artist        │
│   💿 Random Access Memories        Daft Punk · 2013             ▸ album         │
│   🎵 This Is Daft Punk                                          ▸ playlist      │
└────────────────────────────────────────────────────────────────────────────────┘
┌ Now Playing ───────────────────────────────────────────────────────────────────┐
│ ▶  Get Lucky  —  Daft Punk                                          🔊  80%       │
│ ███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  2:14 / 6:09                          │
└────────────────────────────────────────────────────────────────────────────────┘
```

## Why a CLI?

No browser engine means a fraction of the resources. Measured on an Apple Silicon
Mac (June 2026):

| | rumba | YouTube Music in a browser tab |
| --- | --- | --- |
| RAM | **~25 MB** UI (+ 65–103 MB `mpv`) | **402–517 MB** for the tab alone |
| Bandwidth | **~112 MB/hr** @ 256 kbps audio | same for equal quality · **0.5–3 GB/hr if a music video plays** |
| Disk | **8.7 MB** binary | full browser / 150–250 MB+ Electron app |

rumba is **always audio-only** — it never fetches the video stream (which runs
0.5–3 GB/hr), and there are no ads, images, JavaScript, or telemetry along for the
ride. See **[docs/performance.md](docs/performance.md)** for the full methodology,
measurements, and sources.

## How it works

- **UI** — `ratatui` + `crossterm`.
- **Account data** — [`ytmapi-rs`](https://docs.rs/ytmapi-rs), a native async Rust
  client for YouTube Music's internal API: home recommendations, search (with live
  suggestions), artist/album/playlist browsing, library, liked, lyrics, ratings,
  radio.
- **Auth** — your logged-in YouTube Music session is read straight from your
  browser's cookies via the [`rookie`](https://crates.io/crates/rookie) crate.
  No Google Cloud project, no copy-paste. It's refreshed automatically on every
  launch, so it stays current.
- **Playback** — [`mpv`](https://mpv.io) runs headless, driven over its JSON IPC
  socket. mpv resolves stream URLs itself via its bundled `yt-dlp` hook (passed
  your browser session so it fetches the account's **highest-bitrate** audio,
  ~256 kbps opus), and auto-advances to the next queued track on completion.
- **Volume** — mirrors the system output volume (macOS, via `osascript`), so it
  stays in sync even when you change it outside rumba.

## Requirements

- macOS or Linux, a Rust toolchain (`cargo`)
- `mpv`, `yt-dlp`, and `ffmpeg` on your `PATH`

```sh
brew install rustup-init yt-dlp mpv ffmpeg && rustup default stable
```

No Python and no `pip` packages are needed.

## Build & install

```sh
cargo build --release
ln -sf "$PWD/target/release/rumba" /opt/homebrew/bin/rumba   # or anywhere on PATH
```

## Run

Just run it. On first launch (and every launch) rumba finds your logged-in
YouTube Music session in your browser and connects automatically:

```sh
rumba
```

If you're not signed in anywhere, it opens `music.youtube.com` so you can log in,
then continues. The session cookie is cached in `~/.config/rumba/`.

- **Firefox** works with no extra permission (most reliable).
- **Chrome / Brave / Edge / Arc** may show a one-time macOS **Keychain** prompt — click *Allow*.
- **Safari** needs your terminal to have **Full Disk Access**
  (System Settings → Privacy & Security → Full Disk Access).

Force a specific browser or re-connect later:

```sh
rumba --login --browser firefox   # chrome / brave / edge / chromium / opera / arc / safari
rumba --login                      # refresh from the auto-detected browser
rumba --switch-account             # pick which signed-in Google account to use
rumba --help
```

## Keys

| Key | Action |
| --- | --- |
| `1`–`7` / `Tab` / `Shift-Tab` | Switch tabs: Home, Search, Library, Liked, Playlists, Queue, Account |
| `/` | Search — live suggestions (incl. your search history) as you type; `↑`/`↓` pick one |
| `j`/`k` or `↑`/`↓` | Move selection |
| `Enter` | Play a song → **endless radio** of related songs; an album/playlist plays in order; or **drill into** an artist/album/playlist |
| `Esc` | Back (browse stack) · close lyrics · close an opened playlist |
| `a` | Add the selected track to the queue |
| `r` | Start a radio / autoplay queue from the selection |
| `y` | Show lyrics for the selected / now-playing track |
| `L` / `D` | Like / dislike the selected track |
| `s` | Sort the list (toggle title ↔ artist) |
| `c` | Switch Google account (when multiple are signed in) |
| `d` | Download the selected track (mp3) to `~/Music/rumba/` |
| `Space` | Play / pause |
| `n` / `p` | Next / previous track |
| `←`/`→` or `h`/`l` | Seek −5s / +5s |
| `+` / `-` | Volume up / down (system volume) |
| `q` / `Ctrl-C` | Quit |

In the **search box**: `←`/`→` move the cursor, `Option+←/→` by word, `Ctrl-U` /
`Cmd-Backspace` clear the line, `Option-Backspace` / `Ctrl-W` delete a word.

## Configuration

Optional `~/.config/rumba/config.toml` remaps any normal-mode key. Built-in keys
remain the defaults; only what you set is overridden:

```toml
[keymap]
quit = "x"
play_pause = ["space", "p"]   # one or many keys per action
download = "w"
search = "/"
```

Action names: `quit`, `next_tab`, `prev_tab`, `tab1`…`tab7`, `search`, `up`,
`down`, `activate`, `back`, `enqueue`, `radio`, `lyrics`, `like`, `dislike`,
`sort`, `download`, `switch_account`, `play_pause`, `next`, `prev`, `seek_fwd`,
`seek_back`, `vol_up`, `vol_down`. Key names: a single character (`q`, `+`), or `space`,
`tab`, `backtab`, `enter`, `esc`, `up`/`down`/`left`/`right`, and `ctrl-<key>`.

## Files & environment

Everything lives in `~/.config/rumba/`:

| File | Purpose |
| --- | --- |
| `cookies.txt` | cached browser session cookie |
| `source.txt` | which browser the session came from |
| `account.txt` | chosen Google account index (`X-Goog-AuthUser`) when several are signed in |
| `accounts.json` | cached list of signed-in accounts (for the in-app `c` switcher) |
| `config.toml` | optional key bindings (see above) |
| `rumba.log` | event / error log |

| Var | Default | Purpose |
| --- | --- | --- |
| `RUMBA_CONFIG_DIR` | `~/.config/rumba` | config / credentials location |

## Notes & limits

- YouTube Music has no official public API; rumba relies on `ytmapi-rs` + `yt-dlp`,
  which can break when YouTube changes things. Keep yt-dlp current
  (`brew upgrade yt-dlp`) if playback ever fails.
- **Multiple Google accounts:** browsers share one set of YouTube cookies across
  every signed-in account, so by default rumba connects to your *default* account —
  which may not be the one your music lives on (a work account with no YouTube
  channel, say, shows none of your playlists). rumba prompts you to pick at login;
  switch anytime with **`c`** in-app or `rumba --switch-account`.
- The **Home** tab shows your personalized recommendations (the music.youtube.com
  home feed). **Library / Liked depend on an active account** — if they show empty,
  first check you're on the right account (above). For accounts with little or no
  YouTube Music activity, YouTube also serves a "single-column" layout these
  endpoints can't parse. Home, search, artist/album browsing, playlists, lyrics,
  and playback work regardless.
- **Subscription/Premium status** isn't exposed by the internal API, so the
  Account tab can't show it.
- **OS media keys** aren't supported on macOS (a non-bundled CLI can't capture
  them without a full `.app`); on Linux this would be MPRIS/D-Bus.
- Respect YouTube's Terms of Service.

## Project layout

```
src/
  main.rs      entry point, terminal setup, event loop, login
  app.rs       application state + playback / navigation / browse stack
  ui.rs        ratatui rendering (tabs, browse, lyrics, now-playing)
  api.rs       native ytmapi-rs client (async worker on a Tokio thread)
  auth.rs      browser-cookie extraction via rookie
  keys.rs      configurable key bindings (config.toml)
  player.rs    mpv IPC control
  sysvol.rs    system output-volume read/write + watcher
  log.rs       file logger
  models.rs    shared data types
```
