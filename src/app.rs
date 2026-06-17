//! Application state and the logic tying the sidecar + player to the UI.

use crate::models::{PlaylistMeta, Track};
use crate::player::Player;
use crate::api::Api;
use crate::keys::{Action, Keymap};
use anyhow::Result;
use ratatui::widgets::ListState;
use serde_json::json;
use std::collections::HashMap;

/// How far into a track "previous" restarts it instead of going back a track.
const PREV_RESTART_SECS: f64 = 3.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Search,
    Library,
    Liked,
    Playlists,
    Queue,
    Account,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Home,
        Tab::Search,
        Tab::Library,
        Tab::Liked,
        Tab::Playlists,
        Tab::Queue,
        Tab::Account,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Search => "Search",
            Tab::Library => "Library",
            Tab::Liked => "Liked",
            Tab::Playlists => "Playlists",
            Tab::Queue => "Queue",
            Tab::Account => "Account",
        }
    }
    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap()
    }
}

/// What a pending sidecar request id should populate when it returns.
enum Pending {
    Search,
    Library,
    Liked,
    Playlists,
    OpenPlaylist,
    Home,
    Radio,
    Account,
    Artist,
    Album,
    Lyrics,
    Rate,
    Suggestions,
    Autoplay,
}

/// A fetched lyrics view (modal overlay).
pub struct LyricsView {
    pub title: String,
    pub lines: Vec<String>,
    pub source: String,
    pub scroll: u16,
}

/// An entry in a browse page: either a playable track or a link to drill into.
pub enum BrowseItem {
    Track(Track),
    Artist { name: String, id: String },
    Album { title: String, subtitle: String, id: String },
    Playlist { title: String, id: String },
}

/// One screen in the browse navigation stack (search results, an artist page,
/// an album, …). Pushing drills in; Esc pops.
pub struct BrowsePage {
    pub title: String,
    pub items: Vec<BrowseItem>,
    pub state: ListState,
    /// Whether playing a track here starts a radio (loose songs: search/artist)
    /// vs. playing the page in order (an album or playlist).
    pub radio_on_play: bool,
}

pub struct App {
    pub api: Api,
    pub player: Player,

    pub active: Tab,
    pub should_quit: bool,
    pub status: String,

    // Search
    pub search_query: String,
    pub search_cursor: usize, // char index within search_query
    pub searching_input: bool,
    // Live suggestions while typing: (text, is_from_history).
    pub search_suggestions: Vec<(String, bool)>,
    pub suggestion_sel: Option<usize>,

    // Browse navigation stack (search results → artist → album → …).
    pub browse: Vec<BrowsePage>,

    // Lyrics overlay.
    pub lyrics: Option<LyricsView>,
    lyrics_title: String,

    // Library/Liked/History sort toggle.
    sort_by_artist: bool,

    // Endless autoplay: video id we last requested a radio continuation from.
    autoplay_seed: Option<String>,

    keymap: Keymap,

    // Library / Liked / History
    pub library: Vec<Track>,
    pub library_state: ListState,
    pub liked: Vec<Track>,
    pub liked_state: ListState,
    pub home: Vec<Track>,
    pub home_state: ListState,

    // Playlists
    pub playlists: Vec<PlaylistMeta>,
    pub playlists_state: ListState,
    pub open_playlist: Option<(String, Vec<Track>)>,
    pub open_playlist_state: ListState,

    // Queue / playback
    pub queue: Vec<Track>,
    pub queue_state: ListState,
    pub current: Option<usize>,

    // Volume mirrors the system output volume (0–100).
    pub volume: f64,
    volsync: crate::sysvol::Watcher,
    // Account tab data.
    pub account_source: String,
    pub account_name: Option<String>,
    pub account_email: Option<String>,
    pub account_handle: Option<String>,

    // Multi-account switching.
    config_dir: std::path::PathBuf,
    /// Signed-in accounts available to switch between (cached at login).
    pub accounts: Vec<crate::api::AccountInfo>,
    /// The currently connected `X-Goog-AuthUser` index.
    pub authuser: u32,
    /// When `Some`, the account-picker overlay is open at this selection.
    pub account_picker: Option<usize>,

    pending: HashMap<u64, Pending>,
    last_eof: u64,
}

impl App {
    pub fn new(
        api: Api,
        player: Player,
        account_source: String,
        keymap: Keymap,
        config_dir: std::path::PathBuf,
        authuser: u32,
    ) -> Self {
        let volsync = crate::sysvol::Watcher::spawn();
        let volume = volsync.get().unwrap_or(100.0);
        let accounts = std::fs::read_to_string(config_dir.join("accounts.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut app = Self {
            api,
            player,
            active: Tab::Home,
            should_quit: false,
            status: "Loading… press ? for help, / to search, q to quit".into(),
            search_query: String::new(),
            search_cursor: 0,
            searching_input: false,
            search_suggestions: Vec::new(),
            suggestion_sel: None,
            browse: Vec::new(),
            lyrics: None,
            lyrics_title: String::new(),
            sort_by_artist: false,
            autoplay_seed: None,
            keymap,
            library: Vec::new(),
            library_state: ListState::default(),
            liked: Vec::new(),
            liked_state: ListState::default(),
            home: Vec::new(),
            home_state: ListState::default(),
            playlists: Vec::new(),
            playlists_state: ListState::default(),
            open_playlist: None,
            open_playlist_state: ListState::default(),
            queue: Vec::new(),
            queue_state: ListState::default(),
            current: None,
            volume,
            volsync,
            account_source,
            account_name: None,
            account_email: None,
            account_handle: None,
            config_dir,
            accounts,
            authuser,
            account_picker: None,
            pending: HashMap::new(),
            last_eof: 0,
        };
        // Kick off initial loads.
        app.refresh_home();
        app.refresh_library();
        app.refresh_liked();
        app.refresh_playlists();
        app.refresh_account();
        app
    }

    // ---- data loading ---------------------------------------------------- //
    fn send(&mut self, method: &str, params: serde_json::Value, kind: Pending) {
        match self.api.request(method, params) {
            Ok(id) => {
                self.pending.insert(id, kind);
            }
            Err(e) => self.status = format!("api error: {e}"),
        }
    }

    pub fn refresh_home(&mut self) {
        self.send("home", json!({}), Pending::Home);
    }
    pub fn refresh_library(&mut self) {
        self.send("library_songs", json!({}), Pending::Library);
    }
    pub fn refresh_liked(&mut self) {
        self.send("liked_songs", json!({}), Pending::Liked);
    }
    pub fn refresh_playlists(&mut self) {
        self.send("library_playlists", json!({}), Pending::Playlists);
    }
    pub fn refresh_account(&mut self) {
        self.send("account", json!({}), Pending::Account);
    }
    pub fn do_search(&mut self) {
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_suggestions.clear();
        self.suggestion_sel = None;
        self.status = format!("Searching “{q}”…");
        self.send("search", json!({ "query": q, "filter": "songs" }), Pending::Search);
    }

    /// Fetch live suggestions for the current query (predictions + history).
    fn fetch_suggestions(&mut self) {
        self.suggestion_sel = None;
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            self.search_suggestions.clear();
            return;
        }
        self.send("suggestions", json!({ "query": q }), Pending::Suggestions);
    }

    /// Drain API responses and apply them. Called each UI tick.
    pub fn poll_sidecar(&mut self) {
        for resp in self.api.drain() {
            let Some(kind) = self.pending.remove(&resp.id) else { continue };
            if let Some(err) = resp.error {
                // A failed lyrics lookup (e.g. none published for the track)
                // shouldn't read as an error — show a friendly overlay instead.
                if matches!(kind, Pending::Lyrics) {
                    self.lyrics = Some(LyricsView {
                        title: self.lyrics_title.clone(),
                        lines: vec!["Lyrics not found for this track.".to_string()],
                        source: String::new(),
                        scroll: 0,
                    });
                    self.status = "Lyrics".into();
                } else {
                    self.status = format!("error: {err}");
                }
                continue;
            }
            let Some(result) = resp.result else { continue };
            match kind {
                Pending::Search => {
                    let mut items: Vec<BrowseItem> = Vec::new();
                    let songs = parse_songs(&result);
                    let n_songs = songs.len();
                    items.extend(songs.into_iter().map(BrowseItem::Track));
                    let mut n_art = 0;
                    if let Some(arr) = result.get("artists").and_then(|v| v.as_array()) {
                        for a in arr {
                            n_art += 1;
                            items.push(BrowseItem::Artist {
                                name: jstr(a, "name"),
                                id: jstr(a, "id"),
                            });
                        }
                    }
                    let mut n_alb = 0;
                    if let Some(arr) = result.get("albums").and_then(|v| v.as_array()) {
                        for a in arr {
                            n_alb += 1;
                            let year = jstr(a, "year");
                            let artist = jstr(a, "artist");
                            items.push(BrowseItem::Album {
                                title: jstr(a, "title"),
                                subtitle: format!("{artist} · {year}"),
                                id: jstr(a, "id"),
                            });
                        }
                    }
                    let mut n_pl = 0;
                    if let Some(arr) = result.get("playlists").and_then(|v| v.as_array()) {
                        for p in arr {
                            n_pl += 1;
                            items.push(BrowseItem::Playlist {
                                title: jstr(p, "title"),
                                id: jstr(p, "id"),
                            });
                        }
                    }
                    let title = format!("Results for “{}”", self.search_query.trim());
                    self.browse = vec![new_page(title, items, true)];
                    self.status = format!(
                        "{n_songs} songs · {n_art} artists · {n_alb} albums · {n_pl} playlists"
                    );
                }
                Pending::Artist => {
                    let name = jstr(&result, "name");
                    let mut items: Vec<BrowseItem> = parse_songs(&result)
                        .into_iter()
                        .map(BrowseItem::Track)
                        .collect();
                    if let Some(arr) = result.get("albums").and_then(|v| v.as_array()) {
                        for a in arr {
                            items.push(BrowseItem::Album {
                                title: jstr(a, "title"),
                                subtitle: jstr(a, "year"),
                                id: jstr(a, "id"),
                            });
                        }
                    }
                    self.browse.push(new_page(name, items, true));
                }
                Pending::Album => {
                    let title = result
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Album")
                        .to_string();
                    let items = parse_songs(&result)
                        .into_iter()
                        .map(BrowseItem::Track)
                        .collect();
                    // Albums/playlists play in order, not as a radio.
                    self.browse.push(new_page(title, items, false));
                }
                Pending::Library => {
                    self.library = parse_songs(&result);
                    select_first(&mut self.library_state, self.library.len());
                }
                Pending::Liked => {
                    self.liked = parse_songs(&result);
                    select_first(&mut self.liked_state, self.liked.len());
                }
                Pending::Playlists => {
                    self.playlists = serde_json::from_value(
                        result.get("playlists").cloned().unwrap_or_default(),
                    )
                    .unwrap_or_default();
                    select_first(&mut self.playlists_state, self.playlists.len());
                }
                Pending::OpenPlaylist => {
                    let title = result
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Playlist")
                        .to_string();
                    let songs = parse_songs(&result);
                    select_first(&mut self.open_playlist_state, songs.len());
                    self.status = format!("Opened “{title}” ({} tracks)", songs.len());
                    self.open_playlist = Some((title, songs));
                }
                Pending::Home => {
                    self.home = parse_songs(&result);
                    select_first(&mut self.home_state, self.home.len());
                    if self.status.starts_with("Loading") {
                        self.status = "Ready".into();
                    }
                }
                Pending::Radio => {
                    let songs = parse_songs(&result);
                    if songs.is_empty() {
                        self.status = "No radio tracks found".into();
                    } else {
                        self.status = format!("📻 Radio · {} tracks", songs.len());
                        self.set_queue(songs, 0);
                    }
                }
                Pending::Autoplay => {
                    // Append related tracks we don't already have, keeping it endless.
                    let existing: std::collections::HashSet<String> = self
                        .queue
                        .iter()
                        .filter_map(|t| t.video_id.clone())
                        .collect();
                    let mut added = 0;
                    for s in parse_songs(&result) {
                        if let Some(id) = &s.video_id {
                            if !existing.contains(id) {
                                self.queue.push(s);
                                added += 1;
                            }
                        }
                    }
                    if added > 0 {
                        self.status = format!("＋{added} related songs queued");
                    }
                }
                Pending::Account => {
                    let s = |k: &str| {
                        result.get(k).and_then(|v| v.as_str()).map(String::from)
                    };
                    self.account_name = s("name");
                    self.account_email = s("email");
                    self.account_handle = s("handle");
                }
                Pending::Lyrics => {
                    let text = jstr(&result, "lyrics");
                    let lines = if text.trim().is_empty() {
                        vec!["No lyrics available for this track.".to_string()]
                    } else {
                        text.lines().map(String::from).collect()
                    };
                    self.lyrics = Some(LyricsView {
                        title: self.lyrics_title.clone(),
                        lines,
                        source: jstr(&result, "source"),
                        scroll: 0,
                    });
                    self.status = "Lyrics".into();
                }
                Pending::Rate => {}
                Pending::Suggestions => {
                    // Ignore stale replies once the user has left the search box.
                    if self.searching_input {
                        let mut out = Vec::new();
                        if let Some(arr) = result.get("suggestions").and_then(|v| v.as_array()) {
                            for s in arr {
                                out.push((
                                    jstr(s, "text"),
                                    s.get("history").and_then(|v| v.as_bool()).unwrap_or(false),
                                ));
                            }
                        }
                        self.search_suggestions = out;
                    }
                }
            }
        }
    }

    // ---- playback -------------------------------------------------------- //
    pub fn now_playing(&self) -> Option<&Track> {
        self.current.and_then(|i| self.queue.get(i))
    }

    /// The currently playing track's video id, if any (owned to avoid borrow
    /// conflicts when rendering lists that also need `&mut` selection state).
    pub fn current_video_id(&self) -> Option<String> {
        self.now_playing().and_then(|t| t.video_id.clone())
    }

    fn set_queue(&mut self, tracks: Vec<Track>, start: usize) {
        self.queue = tracks;
        self.autoplay_seed = None; // fresh queue → allow a new continuation
        select_first(&mut self.queue_state, self.queue.len());
        self.play_index(start);
    }

    fn play_index(&mut self, i: usize) {
        let Some(track) = self.queue.get(i) else { return };
        let Some(vid) = track.video_id.clone() else {
            self.status = "Track has no playable id".into();
            return;
        };
        let label = format!("{} — {}", track.title, track.artist);
        self.current = Some(i);
        self.queue_state.select(Some(i));
        match self.player.load(&vid) {
            Ok(_) => self.status = format!("▶ {label}"),
            Err(e) => self.status = format!("play error: {e}"),
        }
        // Keep the queue endless: top it up with related songs near the end.
        self.maybe_extend_queue();
    }

    /// Start an autoplay radio from a single track (the YouTube-Music behaviour
    /// of "play this song → keep playing related songs").
    fn play_radio_from(&mut self, video_id: String) {
        self.status = "Starting radio…".into();
        self.send("radio", json!({ "video_id": video_id }), Pending::Radio);
    }

    /// When the playing track is near the end of the queue, fetch a radio seeded
    /// from it and append the related tracks, so playback never stops.
    fn maybe_extend_queue(&mut self) {
        let Some(i) = self.current else { return };
        if i + 2 < self.queue.len() {
            return; // plenty queued ahead
        }
        let Some(id) = self.queue.get(i).and_then(|t| t.video_id.clone()) else { return };
        if self.autoplay_seed.as_deref() == Some(id.as_str()) {
            return; // already requested a continuation from this track
        }
        self.autoplay_seed = Some(id.clone());
        self.send("radio", json!({ "video_id": id }), Pending::Autoplay);
    }

    fn next_track(&mut self) {
        if let Some(i) = self.current {
            if i + 1 < self.queue.len() {
                self.play_index(i + 1);
            } else {
                self.status = "End of queue".into();
            }
        }
    }

    fn prev_track(&mut self) {
        let Some(i) = self.current else { return };
        // Past the grace window — or already at the first track — restart the
        // current song instead of jumping back, matching common players.
        if i == 0 || self.player.state().time_pos > PREV_RESTART_SECS {
            if self.player.restart().is_ok() {
                self.status = "Restarting track".into();
            }
            return;
        }
        self.play_index(i - 1);
    }

    /// Open the account-switcher overlay, selecting the current account.
    fn open_account_picker(&mut self) {
        if self.accounts.len() < 2 {
            self.status = "Only one account is signed in (run `rumba --login` to refresh)".into();
            return;
        }
        let sel = self
            .accounts
            .iter()
            .position(|a| a.authuser == self.authuser)
            .unwrap_or(0);
        self.account_picker = Some(sel);
    }

    /// Reconnect the API to a different Google account, persist the choice, and
    /// reload every tab's data from the new account.
    fn switch_account(&mut self, authuser: u32) -> Result<()> {
        let cookie = std::fs::read_to_string(crate::auth::cookie_path(&self.config_dir))
            .map_err(|e| anyhow::anyhow!("could not read session: {e}"))?;
        let api = Api::spawn(crate::api::Auth::Cookie(cookie), authuser)?;
        self.api = api;
        self.authuser = authuser;
        let _ = std::fs::write(self.config_dir.join("account.txt"), authuser.to_string());

        // Stop playback and drop everything tied to the old account.
        let _ = self.player.stop();
        self.pending.clear();
        self.queue.clear();
        self.current = None;
        self.browse.clear();
        self.open_playlist = None;
        self.library.clear();
        self.liked.clear();
        self.home.clear();
        self.playlists.clear();
        self.autoplay_seed = None;
        self.account_name = None;
        self.account_email = None;
        self.account_handle = None;

        self.refresh_home();
        self.refresh_library();
        self.refresh_liked();
        self.refresh_playlists();
        self.refresh_account();
        self.status = "Switched account — reloading…".into();
        Ok(())
    }

    /// Auto-advance when mpv reports the current track finished.
    pub fn poll_playback(&mut self) {
        let st = self.player.state();
        if st.eof_count != self.last_eof {
            self.last_eof = st.eof_count;
            self.next_track();
        }
        // Pick up volume changes made outside rumba (system settings, media keys).
        if let Some(v) = self.volsync.get() {
            self.volume = v;
        }
    }

    /// Adjust the system output volume by `delta` (percentage points).
    fn adjust_volume(&mut self, delta: f64) {
        self.volume = (self.volume + delta).clamp(0.0, 100.0);
        self.volsync.set(self.volume);
        self.status = format!("Volume {}%", self.volume.round() as i64);
    }

    // ---- selection helpers ---------------------------------------------- //
    /// The track list + selection state backing the currently active tab.
    fn active_list(&mut self) -> Option<(&Vec<Track>, &mut ListState)> {
        match self.active {
            Tab::Home => Some((&self.home, &mut self.home_state)),
            Tab::Search => None,
            Tab::Library => Some((&self.library, &mut self.library_state)),
            Tab::Liked => Some((&self.liked, &mut self.liked_state)),
            Tab::Queue => Some((&self.queue, &mut self.queue_state)),
            Tab::Playlists => {
                if let Some((_, songs)) = &self.open_playlist {
                    Some((songs, &mut self.open_playlist_state))
                } else {
                    None
                }
            }
            Tab::Account => None,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        // Playlists tab with no playlist open navigates the playlist list.
        if self.active == Tab::Playlists && self.open_playlist.is_none() {
            move_state(&mut self.playlists_state, self.playlists.len(), delta);
            return;
        }
        if let Some((list, state)) = self.active_list() {
            let len = list.len();
            move_state(state, len, delta);
        }
    }

    fn selected_index(&mut self) -> Option<usize> {
        if self.active == Tab::Playlists && self.open_playlist.is_none() {
            return self.playlists_state.selected();
        }
        self.active_list().and_then(|(_, s)| s.selected())
    }

    /// Clone the active tab's track list (for building a queue context).
    fn active_tracks(&self) -> Vec<Track> {
        match self.active {
            Tab::Home => self.home.clone(),
            Tab::Search => Vec::new(),
            Tab::Library => self.library.clone(),
            Tab::Liked => self.liked.clone(),
            Tab::Queue => self.queue.clone(),
            Tab::Playlists => self
                .open_playlist
                .as_ref()
                .map(|(_, s)| s.clone())
                .unwrap_or_default(),
            Tab::Account => Vec::new(),
        }
    }

    fn selected_track(&mut self) -> Option<Track> {
        let i = self.selected_index()?;
        self.active_tracks().get(i).cloned()
    }

    // ---- key handling ---------------------------------------------------- //
    fn activate(&mut self) {
        // Playlists list: open the selected playlist's tracks.
        if self.active == Tab::Playlists && self.open_playlist.is_none() {
            if let Some(i) = self.playlists_state.selected() {
                if let Some(pl) = self.playlists.get(i) {
                    if let Some(id) = pl.playlist_id.clone() {
                        self.status = format!("Opening “{}”…", pl.title);
                        self.send("playlist", json!({ "playlist_id": id }), Pending::OpenPlaylist);
                    }
                }
            }
            return;
        }
        // Queue tab: jump straight to the chosen track.
        if self.active == Tab::Queue {
            if let Some(i) = self.queue_state.selected() {
                self.play_index(i);
            }
            return;
        }
        // Opened playlist: play it in order.
        if self.active == Tab::Playlists {
            if let Some(i) = self.selected_index() {
                let tracks = self.active_tracks();
                if !tracks.is_empty() {
                    self.set_queue(tracks, i);
                }
            }
            return;
        }
        // Home / Library / Liked: a loose song → start a radio from it
        // (plays the song, then auto-continues with related tracks).
        if let Some(id) = self.selected_track().and_then(|t| t.video_id) {
            self.play_radio_from(id);
        }
    }

    fn enqueue_selected(&mut self) {
        if let Some(t) = self.selected_track() {
            let title = t.title.clone();
            self.queue.push(t);
            if self.current.is_none() {
                self.play_index(self.queue.len() - 1);
            }
            self.status = format!("Queued “{title}”");
        }
    }

    fn start_radio(&mut self) {
        let seed = self
            .browse_selected_track()
            .or_else(|| self.selected_track())
            .or_else(|| self.now_playing().cloned());
        if let Some(id) = seed.and_then(|t| t.video_id) {
            self.status = "Building radio…".into();
            self.send("radio", json!({ "video_id": id }), Pending::Radio);
        }
    }

    // ---- browse navigation stack ---------------------------------------- //
    fn browse_move(&mut self, delta: isize) {
        if let Some(page) = self.browse.last_mut() {
            move_state(&mut page.state, page.items.len(), delta);
        }
    }

    /// Track currently selected in the top browse page (if it is a track).
    fn browse_selected_track(&self) -> Option<Track> {
        let page = self.browse.last()?;
        match page.items.get(page.state.selected()?)? {
            BrowseItem::Track(t) => Some(t.clone()),
            _ => None,
        }
    }

    fn enqueue_browse_selected(&mut self) {
        if let Some(t) = self.browse_selected_track() {
            let title = t.title.clone();
            self.queue.push(t);
            if self.current.is_none() {
                self.play_index(self.queue.len() - 1);
            }
            self.status = format!("Queued “{title}”");
        }
    }

    fn browse_activate(&mut self) {
        enum Act {
            Radio(String),
            Play(Vec<Track>, usize),
            Artist(String, String),
            Album(String, String),
            Playlist(String),
        }
        let act = {
            let Some(page) = self.browse.last() else { return };
            let Some(i) = page.state.selected() else { return };
            let Some(item) = page.items.get(i) else { return };
            match item {
                // Loose songs (search/artist) → radio; album/playlist → in order.
                BrowseItem::Track(t) if page.radio_on_play => match &t.video_id {
                    Some(id) => Act::Radio(id.clone()),
                    None => return,
                },
                BrowseItem::Track(_) => {
                    let tracks: Vec<Track> = page
                        .items
                        .iter()
                        .filter_map(|it| match it {
                            BrowseItem::Track(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect();
                    let pos = page.items[..=i]
                        .iter()
                        .filter(|it| matches!(it, BrowseItem::Track(_)))
                        .count()
                        .saturating_sub(1);
                    Act::Play(tracks, pos)
                }
                BrowseItem::Artist { id, name } => Act::Artist(id.clone(), name.clone()),
                BrowseItem::Album { id, title, .. } => Act::Album(id.clone(), title.clone()),
                BrowseItem::Playlist { id, .. } => Act::Playlist(id.clone()),
            }
        };
        match act {
            Act::Radio(id) => self.play_radio_from(id),
            Act::Play(tracks, pos) => self.set_queue(tracks, pos),
            Act::Artist(id, name) => {
                self.status = format!("Opening {name}…");
                self.send("artist", json!({ "channel_id": id }), Pending::Artist);
            }
            Act::Album(id, title) => {
                self.status = format!("Opening {title}…");
                self.send("album", json!({ "album_id": id }), Pending::Album);
            }
            Act::Playlist(id) => {
                self.send("playlist", json!({ "playlist_id": id }), Pending::Album);
            }
        }
    }

    // ---- lyrics & rating ------------------------------------------------ //
    /// The track to act on: browse selection, then tab selection, then now-playing.
    fn target_track(&mut self) -> Option<Track> {
        self.browse_selected_track()
            .or_else(|| self.selected_track())
            .or_else(|| self.now_playing().cloned())
    }

    fn show_lyrics(&mut self) {
        if self.lyrics.is_some() {
            self.lyrics = None;
            return;
        }
        let Some(track) = self.target_track() else {
            self.status = "No track selected".into();
            return;
        };
        let Some(id) = track.video_id.clone() else { return };
        self.lyrics_title = format!("{} — {}", track.title, track.artist);
        self.status = "Fetching lyrics…".into();
        self.send("lyrics", json!({ "video_id": id }), Pending::Lyrics);
    }

    /// Sort the active Library/Liked/History list, toggling title ↔ artist.
    fn sort_active(&mut self) {
        self.sort_by_artist = !self.sort_by_artist;
        let by_artist = self.sort_by_artist;
        let list = match self.active {
            Tab::Library => &mut self.library,
            Tab::Liked => &mut self.liked,
            Tab::Home => &mut self.home,
            _ => return,
        };
        if by_artist {
            list.sort_by(|a, b| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()));
        } else {
            list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        }
        self.status = format!(
            "Sorted by {}",
            if by_artist { "artist" } else { "title" }
        );
    }

    /// Download the selected/now-playing track's audio via yt-dlp (background).
    fn download_current(&mut self) {
        let Some(track) = self.target_track() else {
            self.status = "No track selected".into();
            return;
        };
        let Some(id) = track.video_id.clone() else { return };
        let dir = dirs::audio_dir()
            .or_else(dirs::download_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rumba");
        let _ = std::fs::create_dir_all(&dir);
        let label = format!("{} — {}", track.title, track.artist);
        self.status = format!("⬇ Downloading “{}” → {}", track.title, dir.display());
        crate::log::log(format!("download start: {label}"));
        std::thread::spawn(move || {
            let url = format!("https://music.youtube.com/watch?v={id}");
            let res = std::process::Command::new("yt-dlp")
                .args(["-x", "--audio-format", "mp3", "--embed-thumbnail", "--add-metadata", "-o"])
                .arg(dir.join("%(artist)s - %(title)s.%(ext)s"))
                .arg(&url)
                .output();
            match res {
                Ok(o) if o.status.success() => crate::log::log(format!("download done: {label}")),
                Ok(o) => crate::log::log(format!(
                    "download failed: {label}: {}",
                    String::from_utf8_lossy(&o.stderr).chars().rev().take(200).collect::<String>()
                )),
                Err(e) => crate::log::log(format!("download error: {label}: {e}")),
            }
        });
    }

    fn rate_current(&mut self, status: &str) {
        let Some(id) = self.target_track().and_then(|t| t.video_id) else { return };
        self.send("rate", json!({ "video_id": id, "status": status }), Pending::Rate);
        self.status = match status {
            "like" => "♥ Liked",
            "dislike" => "Disliked",
            _ => "Rating cleared",
        }
        .into();
    }

    // ---- search input line editing -------------------------------------- //
    fn insert_char(&mut self, c: char) {
        let mut v: Vec<char> = self.search_query.chars().collect();
        let i = self.search_cursor.min(v.len());
        v.insert(i, c);
        self.search_cursor = i + 1;
        self.search_query = v.into_iter().collect();
    }

    fn delete_char_before(&mut self) {
        if self.search_cursor == 0 {
            return;
        }
        let mut v: Vec<char> = self.search_query.chars().collect();
        let i = self.search_cursor - 1;
        if i < v.len() {
            v.remove(i);
        }
        self.search_cursor = i;
        self.search_query = v.into_iter().collect();
    }

    fn delete_word_before(&mut self) {
        let start = word_left(&self.search_query, self.search_cursor);
        let mut v: Vec<char> = self.search_query.chars().collect();
        let end = self.search_cursor.min(v.len());
        v.drain(start..end);
        self.search_cursor = start;
        self.search_query = v.into_iter().collect();
    }

    fn delete_to_start(&mut self) {
        let v: Vec<char> = self.search_query.chars().collect();
        let i = self.search_cursor.min(v.len());
        self.search_query = v[i..].iter().collect();
        self.search_cursor = 0;
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Text entry mode for the search box swallows most keys.
        if self.searching_input {
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let cmd = key.modifiers.contains(KeyModifiers::SUPER);
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let len = self.search_query.chars().count();
            let query_before = self.search_query.clone();
            match key.code {
                KeyCode::Enter => {
                    // If a suggestion is highlighted, search that instead.
                    if let Some(i) = self.suggestion_sel {
                        if let Some((text, _)) = self.search_suggestions.get(i) {
                            self.search_query = text.clone();
                        }
                    }
                    self.searching_input = false;
                    self.do_search();
                }
                KeyCode::Esc => {
                    self.searching_input = false;
                    self.search_suggestions.clear();
                }
                // Move through the live suggestions.
                KeyCode::Down => {
                    if !self.search_suggestions.is_empty() {
                        let n = self.search_suggestions.len();
                        self.suggestion_sel = Some(match self.suggestion_sel {
                            Some(i) => (i + 1).min(n - 1),
                            None => 0,
                        });
                    }
                }
                KeyCode::Up => {
                    self.suggestion_sel = match self.suggestion_sel {
                        Some(0) | None => None,
                        Some(i) => Some(i - 1),
                    };
                }
                // Cursor movement (Option/Alt = by word). Terminal.app emits
                // Option+Left/Right as Esc-b / Esc-f, so accept those too.
                KeyCode::Left if alt => self.search_cursor = word_left(&self.search_query, self.search_cursor),
                KeyCode::Right if alt => self.search_cursor = word_right(&self.search_query, self.search_cursor),
                KeyCode::Char('b') if alt => self.search_cursor = word_left(&self.search_query, self.search_cursor),
                KeyCode::Char('f') if alt => self.search_cursor = word_right(&self.search_query, self.search_cursor),
                KeyCode::Left => self.search_cursor = self.search_cursor.saturating_sub(1),
                KeyCode::Right => self.search_cursor = (self.search_cursor + 1).min(len),
                KeyCode::Home => self.search_cursor = 0,
                KeyCode::End => self.search_cursor = len,
                KeyCode::Char('a') if ctrl => self.search_cursor = 0,
                KeyCode::Char('e') if ctrl => self.search_cursor = len,
                // Deletion. Cmd+Backspace / Ctrl+U clear the whole line before the
                // cursor; Option+Backspace / Ctrl+W delete the previous word.
                KeyCode::Char('u') if ctrl => self.delete_to_start(),
                KeyCode::Char('w') if ctrl => self.delete_word_before(),
                KeyCode::Backspace if cmd => self.delete_to_start(),
                KeyCode::Backspace if alt => self.delete_word_before(),
                KeyCode::Backspace => self.delete_char_before(),
                KeyCode::Char(c) if !ctrl && !cmd => self.insert_char(c),
                _ => {}
            }
            // Refresh suggestions whenever the query text changed.
            if self.search_query != query_before {
                self.fetch_suggestions();
            }
            return Ok(());
        }

        // Lyrics overlay: scroll / close, plus quit & play-pause passthrough.
        if self.lyrics.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('y') => self.lyrics = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(lv) = self.lyrics.as_mut() {
                        lv.scroll = lv.scroll.saturating_add(1);
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(lv) = self.lyrics.as_mut() {
                        lv.scroll = lv.scroll.saturating_sub(1);
                    }
                }
                KeyCode::Char(' ') => self.player.toggle_pause()?,
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            }
            return Ok(());
        }

        // Account picker overlay: move / select / cancel.
        if let Some(sel) = self.account_picker {
            match key.code {
                KeyCode::Esc | KeyCode::Char('c') => self.account_picker = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    if !self.accounts.is_empty() {
                        self.account_picker = Some((sel + 1).min(self.accounts.len() - 1));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.account_picker = Some(sel.saturating_sub(1));
                }
                KeyCode::Enter => {
                    if let Some(acct) = self.accounts.get(sel) {
                        let authuser = acct.authuser;
                        self.account_picker = None;
                        if authuser != self.authuser {
                            self.switch_account(authuser)?;
                        }
                    }
                }
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            }
            return Ok(());
        }

        // Normal mode: dispatch through the (configurable) keymap.
        if let Some(action) = self.keymap.action(&key) {
            self.do_action(action)?;
        }
        Ok(())
    }

    /// Perform a bound action. Several are browse-stack-aware.
    fn do_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NextTab => {
                self.browse.clear();
                self.cycle_tab(1);
            }
            Action::PrevTab => {
                self.browse.clear();
                self.cycle_tab(-1);
            }
            Action::Tab(idx) => {
                if idx < Tab::ALL.len() {
                    self.browse.clear();
                    self.active = Tab::ALL[idx];
                }
            }
            Action::Search => {
                self.browse.clear();
                self.active = Tab::Search;
                self.searching_input = true;
                self.search_cursor = self.search_query.chars().count();
                self.search_suggestions.clear();
                self.suggestion_sel = None;
            }
            Action::Down => {
                if self.browse.is_empty() {
                    self.move_selection(1);
                } else {
                    self.browse_move(1);
                }
            }
            Action::Up => {
                if self.browse.is_empty() {
                    self.move_selection(-1);
                } else {
                    self.browse_move(-1);
                }
            }
            Action::Activate => {
                if self.browse.is_empty() {
                    self.activate();
                } else {
                    self.browse_activate();
                }
            }
            Action::Back => {
                if !self.browse.is_empty() {
                    self.browse.pop();
                } else if self.active == Tab::Playlists {
                    self.open_playlist = None;
                }
            }
            Action::Enqueue => {
                if self.browse.is_empty() {
                    self.enqueue_selected();
                } else {
                    self.enqueue_browse_selected();
                }
            }
            Action::Radio => self.start_radio(),
            Action::Lyrics => self.show_lyrics(),
            Action::Like => self.rate_current("like"),
            Action::Dislike => self.rate_current("dislike"),
            Action::Sort => self.sort_active(),
            Action::Download => self.download_current(),
            Action::SwitchAccount => self.open_account_picker(),
            Action::PlayPause => self.player.toggle_pause()?,
            Action::Next => self.next_track(),
            Action::Prev => self.prev_track(),
            Action::SeekFwd => self.player.seek_relative(5.0)?,
            Action::SeekBack => self.player.seek_relative(-5.0)?,
            Action::VolUp => self.adjust_volume(5.0),
            Action::VolDown => self.adjust_volume(-5.0),
        }
        Ok(())
    }

    fn cycle_tab(&mut self, delta: isize) {
        let n = Tab::ALL.len() as isize;
        let cur = self.active.index() as isize;
        let next = ((cur + delta) % n + n) % n;
        self.active = Tab::ALL[next as usize];
    }
}

// ---- free helpers -------------------------------------------------------- //
fn parse_songs(result: &serde_json::Value) -> Vec<Track> {
    serde_json::from_value(result.get("songs").cloned().unwrap_or_default()).unwrap_or_default()
}

/// Index of the start of the word at/just before `cursor` (skip spaces, then
/// the run of non-space chars) — mirrors Option+Left in a macOS text field.
fn word_left(s: &str, cursor: usize) -> usize {
    let v: Vec<char> = s.chars().collect();
    let mut i = cursor.min(v.len());
    while i > 0 && v[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !v[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Index just past the next word — mirrors Option+Right.
fn word_right(s: &str, cursor: usize) -> usize {
    let v: Vec<char> = s.chars().collect();
    let mut i = cursor.min(v.len());
    while i < v.len() && v[i].is_whitespace() {
        i += 1;
    }
    while i < v.len() && !v[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Read a string field from a JSON object (empty string if missing).
fn jstr(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Build a BrowsePage with the first item selected.
fn new_page(title: String, items: Vec<BrowseItem>, radio_on_play: bool) -> BrowsePage {
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(0));
    }
    BrowsePage {
        title,
        items,
        state,
        radio_on_play,
    }
}

fn select_first(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
    } else if state.selected().is_none() {
        state.select(Some(0));
    }
}

fn move_state(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, len as isize - 1);
    state.select(Some(next as usize));
}
