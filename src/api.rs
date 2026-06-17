//! Native YouTube Music API client backed by `ytmapi-rs`.
//!
//! This replaces the old Python sidecar. It is a drop-in for it: same
//! `request(method, params) -> id` / `drain() -> Vec<RawResponse>` interface,
//! emitting the same JSON `Value` shapes the UI already parses — so the rest of
//! the app is unchanged. Internally it runs a Tokio runtime on a background
//! thread that owns a `YtMusic` client and answers jobs over channels.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use ytmapi_rs::auth::{AuthToken, LoggedIn};
use ytmapi_rs::common::{
    AlbumID, ArtistChannelID, LikeStatus, PlaylistID, SuggestionType, Thumbnail, VideoID, YoutubeID,
};
use ytmapi_rs::parse::{
    HistoryItem, ParseFrom, PlaylistItem, ProcessedResult, SearchResultPlaylist,
    SearchResultSong, TableListSong, WatchPlaylistTrack,
};
use ytmapi_rs::query::{PostMethod, PostQuery, Query};
use ytmapi_rs::YtMusic;

/// Minimal custom query for YouTube's `account/account_menu` endpoint, which
/// `ytmapi-rs` doesn't wrap. We only need the raw JSON (via `json_query`), so
/// the `ParseFrom` impl is a never-called stub.
struct AccountMenuQuery;

impl<A: AuthToken> Query<A> for AccountMenuQuery {
    type Output = ();
    type Method = PostMethod;
}

impl PostQuery for AccountMenuQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "account/account_menu"
    }
}

impl ParseFrom<AccountMenuQuery> for () {
    fn parse_from(_: ProcessedResult<AccountMenuQuery>) -> ytmapi_rs::Result<Self> {
        Ok(())
    }
}

/// Custom query for the personalized home feed (`FEmusic_home`) — the
/// recommendations you see on music.youtube.com. ytmapi-rs doesn't wrap it, so
/// we fetch raw JSON via `json_query` and pull the playable songs out ourselves.
struct HomeQuery;

impl<A: AuthToken> Query<A> for HomeQuery {
    type Output = ();
    type Method = PostMethod;
}

impl PostQuery for HomeQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([("browseId".to_string(), json!("FEmusic_home"))])
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<HomeQuery> for () {
    fn parse_from(_: ProcessedResult<HomeQuery>) -> ytmapi_rs::Result<Self> {
        Ok(())
    }
}

/// Raw query for the library playlists grid (`FEmusic_liked_playlists`).
/// ytmapi-rs wraps this as `get_library_playlists`, but its parser uses hard
/// `?`s on every field (track count, subtitle runs, …) and fails the *entire*
/// list if a single tile differs — which a freshly created or empty playlist
/// often does, blanking the page. We fetch raw JSON and parse tolerantly.
struct LibraryPlaylistsQuery;

impl<A: AuthToken> Query<A> for LibraryPlaylistsQuery {
    type Output = ();
    type Method = PostMethod;
}

impl PostQuery for LibraryPlaylistsQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([("browseId".to_string(), json!("FEmusic_liked_playlists"))])
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<LibraryPlaylistsQuery> for () {
    fn parse_from(_: ProcessedResult<LibraryPlaylistsQuery>) -> ytmapi_rs::Result<Self> {
        Ok(())
    }
}

/// Walk a library-playlists response and collect playlist tiles. Each playlist
/// is a `musicTwoRowItemRenderer` whose navigation browse id starts with "VL".
/// This skips the "New playlist" tile (it navigates elsewhere) and tolerates
/// missing subtitle/track-count fields.
fn collect_library_playlists(
    v: &Value,
    out: &mut Vec<Value>,
    seen: &mut std::collections::HashSet<String>,
) {
    match v {
        Value::Object(o) => {
            if let Some(item) = o.get("musicTwoRowItemRenderer") {
                if let Some(pl) = parse_library_playlist_tile(item) {
                    if let Some(id) = pl.get("playlist_id").and_then(Value::as_str) {
                        if seen.insert(id.to_string()) {
                            out.push(pl);
                        }
                    }
                }
            }
            for val in o.values() {
                collect_library_playlists(val, out, seen);
            }
        }
        Value::Array(a) => {
            for val in a {
                collect_library_playlists(val, out, seen);
            }
        }
        _ => {}
    }
}

/// Extract a single playlist tile, or `None` if it isn't a real, openable
/// playlist (the create-new tile, or special shelves we don't list).
fn parse_library_playlist_tile(item: &Value) -> Option<Value> {
    let browse_id = item
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)?;
    // Library playlist browse ids are prefixed "VL"; the "New playlist" tile
    // and other shelves are not.
    if !browse_id.starts_with("VL") {
        return None;
    }
    let title = item
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if title.is_empty()
        || title.eq_ignore_ascii_case("liked music")
        || title.eq_ignore_ascii_case("episodes for later")
    {
        return None;
    }
    // Track count lives in the subtitle runs (e.g. "12 songs"); optional.
    let count = item
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .and_then(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .find(|t| t.chars().any(|c| c.is_ascii_digit()))
        })
        .map(|t| t.to_string());
    Some(json!({ "playlist_id": browse_id, "title": title, "count": count }))
}

/// Find the first `videoId` anywhere under a node.
fn find_video_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => {
            if let Some(id) = o.get("videoId").and_then(Value::as_str) {
                return Some(id.to_string());
            }
            o.values().find_map(find_video_id)
        }
        Value::Array(a) => a.iter().find_map(find_video_id),
        _ => None,
    }
}

/// Walk the home feed and collect playable songs from its shelves.
fn collect_home_songs(v: &Value, out: &mut Vec<Value>, seen: &mut std::collections::HashSet<String>) {
    match v {
        Value::Object(o) => {
            if let Some(item) = o.get("musicResponsiveListItemRenderer") {
                if let Some(vid) = find_video_id(item) {
                    let flex = |i: usize| {
                        item.pointer(&format!(
                            "/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text"
                        ))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                    };
                    let title = flex(0);
                    if !title.is_empty() && seen.insert(vid.clone()) {
                        out.push(song_value(&vid, &title, flex(1), String::new(), "", None));
                    }
                }
            }
            for val in o.values() {
                collect_home_songs(val, out, seen);
            }
        }
        Value::Array(a) => {
            for val in a {
                collect_home_songs(val, out, seen);
            }
        }
        _ => {}
    }
}

/// A parsed response destined for the UI loop (mirrors the old sidecar type).
pub struct RawResponse {
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<String>,
}

struct Job {
    id: u64,
    method: String,
    params: Value,
}

/// How to authenticate the underlying client.
pub enum Auth {
    /// Raw `Cookie:` header string (from the user's browser session).
    Cookie(String),
}

pub struct Api {
    tx: tokio::sync::mpsc::UnboundedSender<Job>,
    rx: Receiver<RawResponse>,
    next_id: AtomicU64,
}

impl Api {
    pub fn spawn(auth: Auth) -> Result<Self> {
        let (tx, job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (resp_tx, resp_rx) = mpsc::channel::<RawResponse>();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };
            rt.block_on(async move {
                match auth {
                    Auth::Cookie(cookie) => match YtMusic::from_cookie(cookie).await {
                        Ok(yt) => serve(yt, job_rx, resp_tx).await,
                        Err(e) => fail_all(job_rx, resp_tx, format!("auth failed: {e}")).await,
                    },
                }
            });
        });

        Ok(Self {
            tx,
            rx: resp_rx,
            next_id: AtomicU64::new(1),
        })
    }

    /// Queue a request; returns the id so the caller can route the reply.
    pub fn request(&self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.tx
            .send(Job {
                id,
                method: method.into(),
                params,
            })
            .map_err(|_| anyhow!("api worker is gone"))?;
        Ok(id)
    }

    /// Drain all responses currently available (non-blocking).
    pub fn drain(&self) -> Vec<RawResponse> {
        self.rx.try_iter().collect()
    }
}

/// If the client couldn't be built, answer every job with the same error.
async fn fail_all(
    mut job_rx: tokio::sync::mpsc::UnboundedReceiver<Job>,
    resp_tx: mpsc::Sender<RawResponse>,
    err: String,
) {
    while let Some(job) = job_rx.recv().await {
        let _ = resp_tx.send(RawResponse {
            id: job.id,
            result: None,
            error: Some(err.clone()),
        });
    }
}

/// Main worker loop, generic over the (logged-in) auth token type.
async fn serve<A: LoggedIn>(
    yt: YtMusic<A>,
    mut job_rx: tokio::sync::mpsc::UnboundedReceiver<Job>,
    resp_tx: mpsc::Sender<RawResponse>,
) {
    while let Some(job) = job_rx.recv().await {
        let resp = match exec(&yt, &job.method, &job.params).await {
            Ok(v) => RawResponse {
                id: job.id,
                result: Some(v),
                error: None,
            },
            Err(e) => RawResponse {
                id: job.id,
                result: None,
                error: Some(format!("{e}")),
            },
        };
        if resp_tx.send(resp).is_err() {
            break;
        }
    }
}

async fn exec<A: LoggedIn>(yt: &YtMusic<A>, method: &str, params: &Value) -> Result<Value> {
    let str_param = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match method {
        "search" => {
            let q = str_param("query");
            // Each filtered search errors (rather than returning empty) when its
            // shelf is absent, so treat failures as "no results of that kind".
            let songs = yt.search_songs(q.as_str()).await.unwrap_or_default();
            let artists = yt.search_artists(q.as_str()).await.unwrap_or_default();
            let albums = yt.search_albums(q.as_str()).await.unwrap_or_default();
            let playlists = yt.search_playlists(q.as_str()).await.unwrap_or_default();
            Ok(json!({
                "songs": songs.iter().map(search_song_value).collect::<Vec<_>>(),
                "artists": artists.iter().map(|a| json!({
                    "name": a.artist, "id": a.browse_id.get_raw(),
                })).collect::<Vec<_>>(),
                "albums": albums.iter().map(|a| json!({
                    "title": a.title, "artist": a.artist,
                    "id": a.album_id.get_raw(), "year": a.year,
                })).collect::<Vec<_>>(),
                "playlists": playlists.iter().filter_map(|p| match p {
                    SearchResultPlaylist::Featured(f) =>
                        Some(json!({ "title": f.title, "id": f.playlist_id.get_raw() })),
                    SearchResultPlaylist::Community(c) =>
                        Some(json!({ "title": c.title, "id": c.playlist_id.get_raw() })),
                    _ => None,
                }).collect::<Vec<_>>(),
            }))
        }
        "artist" => {
            let art = yt
                .get_artist(ArtistChannelID::from_raw(str_param("channel_id")))
                .await?;
            let songs: Vec<Value> = art
                .top_releases
                .songs
                .map(|s| s.results)
                .unwrap_or_default()
                .iter()
                .map(|s| {
                    let artist = s
                        .artists
                        .iter()
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    song_value(s.video_id.get_raw(), &s.title, artist, s.album.name.clone(), "", None)
                })
                .collect();
            let albums: Vec<Value> = art
                .top_releases
                .albums
                .map(|a| a.results)
                .unwrap_or_default()
                .iter()
                .map(|a| json!({ "title": a.title, "id": a.album_id.get_raw(), "year": a.year }))
                .collect();
            Ok(json!({ "name": art.name, "songs": songs, "albums": albums }))
        }
        "album" => {
            let album = yt.get_album(AlbumID::from_raw(str_param("album_id"))).await?;
            let artist = album
                .artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let cover = thumb_url(&album.thumbnails);
            let songs: Vec<Value> = album
                .tracks
                .iter()
                .map(|t| {
                    song_value(
                        t.video_id.get_raw(),
                        &t.title,
                        artist.clone(),
                        album.title.clone(),
                        &t.duration,
                        cover.clone(),
                    )
                })
                .collect();
            Ok(json!({ "title": album.title, "songs": songs }))
        }
        // Personalized recommendations (music.youtube.com home feed).
        "home" => {
            let raw = yt.json_query::<HomeQuery>(HomeQuery).await?;
            let v = serde_json::to_value(&raw).unwrap_or(Value::Null);
            let mut songs = Vec::new();
            let mut seen = std::collections::HashSet::new();
            collect_home_songs(&v, &mut songs, &mut seen);
            Ok(json!({ "songs": songs }))
        }
        "suggestions" => {
            let q = str_param("query");
            let sugg = yt.get_search_suggestions(q.as_str()).await.unwrap_or_default();
            let items: Vec<Value> = sugg
                .iter()
                .map(|s| {
                    json!({
                        "text": s.get_text(),
                        "history": matches!(s.suggestion_type, SuggestionType::History),
                    })
                })
                .collect();
            Ok(json!({ "suggestions": items }))
        }
        // Library/liked browse can return YouTube's single-column "empty
        // library" layout for inactive accounts, which the API can't parse —
        // treat that as simply empty rather than surfacing a scary error.
        "library_songs" => match yt.get_library_songs().await {
            Ok(songs) => {
                Ok(json!({ "songs": songs.iter().map(tablelist_song_value).collect::<Vec<_>>() }))
            }
            Err(_) => Ok(json!({ "songs": [] })),
        },
        "liked_songs" => match yt.get_library_songs().await {
            Ok(songs) => {
                let liked: Vec<_> = songs
                    .iter()
                    .filter(|s| matches!(s.like_status, LikeStatus::Liked))
                    .map(tablelist_song_value)
                    .collect();
                Ok(json!({ "songs": liked }))
            }
            Err(_) => Ok(json!({ "songs": [] })),
        },
        "library_playlists" => {
            let raw = yt
                .json_query::<LibraryPlaylistsQuery>(LibraryPlaylistsQuery)
                .await?;
            let v = serde_json::to_value(&raw).unwrap_or(Value::Null);
            let mut playlists = Vec::new();
            let mut seen = std::collections::HashSet::new();
            collect_library_playlists(&v, &mut playlists, &mut seen);
            Ok(json!({ "playlists": playlists }))
        }
        "playlist" => {
            let id = str_param("playlist_id");
            let items = yt
                .get_playlist_tracks(PlaylistID::from_raw(id.clone()))
                .await?;
            let title = yt
                .get_playlist_details(PlaylistID::from_raw(id))
                .await
                .map(|d| d.title)
                .unwrap_or_default();
            Ok(json!({ "title": title, "songs": playlist_items_value(&items) }))
        }
        "radio" => {
            let tracks = yt
                .get_watch_playlist_from_video_id(VideoID::from_raw(str_param("video_id")))
                .await?;
            Ok(json!({ "songs": tracks.iter().map(watch_track_value).collect::<Vec<_>>() }))
        }
        "history" => {
            let mut songs: Vec<Value> = Vec::new();
            if let Ok(periods) = yt.get_history().await {
                for p in periods {
                    for it in p.items {
                        match it {
                            HistoryItem::Song(s) => {
                                let artist = s
                                    .artists
                                    .iter()
                                    .map(|a| a.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                songs.push(song_value(
                                    s.video_id.get_raw(),
                                    &s.title,
                                    artist,
                                    s.album.name.clone(),
                                    &s.duration,
                                    thumb_url(&s.thumbnails),
                                ));
                            }
                            HistoryItem::Video(v) => songs.push(song_value(
                                v.video_id.get_raw(),
                                &v.title,
                                v.channel_name.clone(),
                                String::new(),
                                &v.duration,
                                thumb_url(&v.thumbnails),
                            )),
                            _ => {}
                        }
                    }
                }
            }
            Ok(json!({ "songs": songs }))
        }
        "lyrics" => {
            let vid = VideoID::from_raw(str_param("video_id"));
            let lyrics_id = yt.get_lyrics_id(vid).await?;
            let lyrics = yt.get_lyrics(lyrics_id).await?;
            Ok(json!({ "lyrics": lyrics.lyrics, "source": lyrics.source }))
        }
        "rate" => {
            let status = match str_param("status").as_str() {
                "like" => LikeStatus::Liked,
                "dislike" => LikeStatus::Disliked,
                _ => LikeStatus::Indifferent,
            };
            yt.rate_song(VideoID::from_raw(str_param("video_id")), status)
                .await?;
            Ok(json!({ "ok": true }))
        }
        "account" => {
            let raw = yt.json_query::<AccountMenuQuery>(AccountMenuQuery).await?;
            // Json is #[serde(transparent)] over serde_json::Value.
            let v = serde_json::to_value(&raw).unwrap_or(Value::Null);
            let base = "/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer";
            let run = |key: &str| {
                v.pointer(&format!("{base}/{key}/runs/0/text"))
                    .and_then(Value::as_str)
                    .map(String::from)
            };
            let photo = v
                .pointer(&format!("{base}/accountPhoto/thumbnails"))
                .and_then(Value::as_array)
                .and_then(|a| a.last())
                .and_then(|t| t.get("url"))
                .and_then(Value::as_str)
                .map(String::from);
            Ok(json!({
                "name": run("accountName"),
                "email": run("email"),
                "handle": run("channelHandle"),
                "photo": photo,
            }))
        }
        other => Err(anyhow!("unknown method: {other}")),
    }
}

// --------------------------------------------------------------------------- //
// Mapping ytmapi-rs types -> the flat JSON the UI consumes
// --------------------------------------------------------------------------- //

/// Parse a `m:ss` / `h:mm:ss` duration string into seconds.
fn parse_dur(s: &str) -> u64 {
    let mut total = 0u64;
    for part in s.split(':') {
        match part.trim().parse::<u64>() {
            Ok(n) => total = total * 60 + n,
            Err(_) => return total,
        }
    }
    total
}

fn thumb_url(thumbs: &[Thumbnail]) -> Option<String> {
    thumbs.last().map(|t| t.url.clone())
}

fn song_value(
    video_id: &str,
    title: &str,
    artist: String,
    album: String,
    dur: &str,
    thumbnail: Option<String>,
) -> Value {
    json!({
        "video_id": video_id,
        "title": title,
        "artist": artist,
        "album": album,
        "duration": parse_dur(dur),
        "thumbnail": thumbnail,
    })
}

fn search_song_value(s: &SearchResultSong) -> Value {
    let album = s.album.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    song_value(
        s.video_id.get_raw(),
        &s.title,
        s.artist.clone(),
        album,
        &s.duration,
        thumb_url(&s.thumbnails),
    )
}

fn tablelist_song_value(s: &TableListSong) -> Value {
    let artist = s
        .artists
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    song_value(
        s.video_id.get_raw(),
        &s.title,
        artist,
        s.album.name.clone(),
        &s.duration,
        thumb_url(&s.thumbnails),
    )
}


fn watch_track_value(t: &WatchPlaylistTrack) -> Value {
    song_value(
        t.video_id.get_raw(),
        &t.title,
        t.author.clone(),
        String::new(),
        &t.duration,
        thumb_url(&t.thumbnails),
    )
}

fn playlist_items_value(items: &[PlaylistItem]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|it| match it {
            PlaylistItem::Song(s) => {
                let artist = s
                    .artists
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(song_value(
                    s.video_id.get_raw(),
                    &s.title,
                    artist,
                    s.album.name.clone(),
                    &s.duration,
                    thumb_url(&s.thumbnails),
                ))
            }
            PlaylistItem::Video(v) => Some(song_value(
                v.video_id.get_raw(),
                &v.title,
                v.channel_name.clone(),
                String::new(),
                &v.duration,
                thumb_url(&v.thumbnails),
            )),
            _ => None,
        })
        .collect()
}
