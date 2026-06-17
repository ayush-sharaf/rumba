//! All ratatui rendering. Pure function of `App` state.

use crate::app::{App, BrowseItem, Tab};
use crate::models::{fmt_secs, Track};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};

const ACCENT: Color = Color::Rgb(255, 70, 70); // YouTube Music red

pub fn render(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tabs
            Constraint::Min(3),    // main content
            Constraint::Length(4), // now playing
            Constraint::Length(1), // status
        ])
        .split(f.area());

    render_tabs(f, app, chunks[0]);
    render_main(f, app, chunks[1]);
    render_now_playing(f, app, chunks[2]);
    render_status(f, app, chunks[3]);
}

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" {} {} ", i + 1, t.title())))
        .collect();
    let selected = Tab::ALL.iter().position(|t| *t == app.active).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " rumba ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn render_main(f: &mut Frame, app: &mut App, area: Rect) {
    // Lyrics overlay takes precedence over everything.
    if app.lyrics.is_some() {
        render_lyrics(f, app, area);
        return;
    }
    // The browse stack (search results / artist / album) overlays everything.
    if !app.browse.is_empty() {
        render_browse(f, app, area);
        return;
    }
    match app.active {
        Tab::Search => render_search(f, app, area),
        Tab::Account => render_account(f, app, area),
        Tab::Playlists if app.open_playlist.is_none() => render_playlist_list(f, app, area),
        _ => render_track_view(f, app, area),
    }
}

fn render_account(f: &mut Frame, app: &App, area: Rect) {
    let dash = "—".to_string();
    let name = app.account_name.clone().unwrap_or_else(|| dash.clone());
    let handle = app.account_handle.clone().unwrap_or_else(|| dash.clone());
    // YT exposes the email in some account_menu layouts; fall back to handle.
    let email = app
        .account_email
        .clone()
        .or_else(|| app.account_handle.clone())
        .unwrap_or_else(|| dash.clone());

    let header = Line::from(vec![
        Span::styled(name.clone(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("   ● Connected", Style::default().fg(Color::Green)),
    ]);

    let lines = vec![
        header,
        Line::from(""),
        stat_line("Name", &name),
        stat_line("Email / handle", &email),
        stat_line("Channel", &handle),
        stat_line("Signed in via", &format!("{} (browser cookies)", app.account_source)),
        Line::from(""),
        stat_line("Library songs", &app.library.len().to_string()),
        stat_line("Playlists", &app.playlists.len().to_string()),
        stat_line("Liked songs", &app.liked.len().to_string()),
        stat_line("Queue", &app.queue.len().to_string()),
    ];
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Account ")
            .padding(ratatui::widgets::Padding::new(2, 2, 1, 1)),
    );
    f.render_widget(p, area);
}

fn stat_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), Style::default().fg(Color::Gray)),
        Span::styled(value.to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ])
}

fn render_lyrics(f: &mut Frame, app: &App, area: Rect) {
    let lv = app.lyrics.as_ref().unwrap();
    let mut text: Vec<Line> = vec![
        Line::from(Span::styled(
            lv.title.clone(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for l in &lv.lines {
        text.push(Line::from(l.clone()));
    }
    if !lv.source.is_empty() {
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(
            format!("Source: {}", lv.source),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let p = Paragraph::new(text)
        .scroll((lv.scroll, 0))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Lyrics (j/k scroll · Esc close) ")
                .padding(ratatui::widgets::Padding::new(2, 2, 0, 0)),
        );
    f.render_widget(p, area);
}

fn render_browse(f: &mut Frame, app: &mut App, area: Rect) {
    let crumb = app
        .browse
        .iter()
        .map(|p| p.title.clone())
        .collect::<Vec<_>>()
        .join(" › ");
    let current = app.current_video_id();
    let current = current.as_deref();
    let page = app.browse.last_mut().unwrap();
    let items: Vec<ListItem> = page
        .items
        .iter()
        .map(|it| browse_item_line(it, current))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {crumb}  (Enter: open · Esc: back) ")),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 40))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut page.state);
}

fn browse_item_line(item: &BrowseItem, playing: Option<&str>) -> ListItem<'static> {
    match item {
        BrowseItem::Track(t) => {
            let is_playing = playing.is_some() && t.video_id.as_deref() == playing;
            let (marker, name_style) = if is_playing {
                ("♪ ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else {
                ("  ", Style::default().fg(Color::White))
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(ACCENT)),
                Span::styled(truncate(&t.title, 46), name_style),
                Span::styled(format!("  {}", truncate(&t.artist, 24)), Style::default().fg(Color::Gray)),
                Span::styled(format!("  {:>6}", t.duration_str()), Style::default().fg(Color::DarkGray)),
            ]))
        }
        BrowseItem::Artist { name, .. } => ListItem::new(Line::from(vec![
            Span::styled("  🎤 ", Style::default().fg(Color::Cyan)),
            Span::styled(name.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("   ▸ artist", Style::default().fg(Color::DarkGray)),
        ])),
        BrowseItem::Album { title, subtitle, .. } => ListItem::new(Line::from(vec![
            Span::styled("  💿 ", Style::default().fg(Color::LightMagenta)),
            Span::styled(truncate(title, 40), Style::default().fg(Color::White)),
            Span::styled(format!("  {subtitle}"), Style::default().fg(Color::DarkGray)),
            Span::styled("   ▸ album", Style::default().fg(Color::DarkGray)),
        ])),
        BrowseItem::Playlist { title, .. } => ListItem::new(Line::from(vec![
            Span::styled("  🎵 ", Style::default().fg(Color::Green)),
            Span::styled(truncate(title, 44), Style::default().fg(Color::White)),
            Span::styled("   ▸ playlist", Style::default().fg(Color::DarkGray)),
        ])),
    }
}

fn render_search(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let input_style = if app.searching_input {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let display = if app.searching_input {
        let v: Vec<char> = app.search_query.chars().collect();
        let i = app.search_cursor.min(v.len());
        let before: String = v[..i].iter().collect();
        let after: String = v[i..].iter().collect();
        format!("🔍 {before}│{after}")
    } else {
        format!("🔍 {}", app.search_query)
    };
    let input = Paragraph::new(display).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_style)
            .title(" Search (press / to edit, Enter to run) "),
    );
    f.render_widget(input, rows[0]);

    // Live suggestions: 🕘 = from your search history, 🔍 = prediction.
    let items: Vec<ListItem> = if app.search_suggestions.is_empty() {
        vec![ListItem::new(Span::styled(
            "  Type to search — ↑/↓ pick a suggestion, Enter to run",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.search_suggestions
            .iter()
            .enumerate()
            .map(|(i, (text, is_hist))| {
                let selected = app.suggestion_sel == Some(i);
                let icon = if *is_hist { "🕘 " } else { "🔍 " };
                let text_style = if selected {
                    Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("  {icon}")),
                    Span::styled(text.clone(), text_style),
                ]))
            })
            .collect()
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Suggestions "),
    );
    f.render_widget(list, rows[1]);
}

fn render_playlist_list(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .map(|p| {
            let count = p.count_str();
            let suffix = if count.is_empty() {
                String::new()
            } else {
                format!("  ({count})")
            };
            ListItem::new(Line::from(vec![
                Span::styled(p.title.clone(), Style::default().fg(Color::White)),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Your Playlists (Enter to open) "),
        )
        .highlight_style(Style::default().bg(ACCENT).fg(Color::Black))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.playlists_state);
}

fn render_track_view(f: &mut Frame, app: &mut App, area: Rect) {
    let current = app.current_video_id();
    let current = current.as_deref();
    let (tracks, title, state) = match app.active {
        Tab::Home => (
            &app.home,
            " Home — recommended for you ".to_string(),
            &mut app.home_state,
        ),
        Tab::Library => (
            &app.library,
            format!(" Library ({}) ", app.library.len()),
            &mut app.library_state,
        ),
        Tab::Liked => (
            &app.liked,
            format!(" Liked Songs ({}) ", app.liked.len()),
            &mut app.liked_state,
        ),
        Tab::Queue => (
            &app.queue,
            format!(" Queue ({}) ", app.queue.len()),
            &mut app.queue_state,
        ),
        Tab::Playlists => {
            let (name, songs) = app.open_playlist.as_ref().unwrap();
            let title = format!(" {} ({}) — Esc to go back ", name, songs.len());
            let list = track_list(songs, &title, current);
            f.render_stateful_widget(list, area, &mut app.open_playlist_state);
            return;
        }
        Tab::Search | Tab::Account => unreachable!(),
    };
    let list = track_list(tracks, &title, current);
    f.render_stateful_widget(list, area, state);
}

/// Build a styled track List with the now-playing row highlighted.
/// All cell content is owned, so the returned widget borrows nothing.
fn track_list(tracks: &[Track], title: &str, playing: Option<&str>) -> List<'static> {
    let items: Vec<ListItem> = tracks
        .iter()
        .map(|t| {
            let is_playing = playing.is_some() && t.video_id.as_deref() == playing;
            let marker = if is_playing { "♪ " } else { "  " };
            let name_style = if is_playing {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(ACCENT)),
                Span::styled(truncate(&t.title, 48), name_style),
                Span::styled(
                    format!("  {}", truncate(&t.artist, 28)),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("  {:>6}", t.duration_str()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (nothing here yet)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        items
    };
    List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title.to_string()))
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 40))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
}

fn render_now_playing(f: &mut Frame, app: &App, area: Rect) {
    let st = app.player.state();
    let block = Block::default().borders(Borders::ALL).title(" Now Playing ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let icon = if st.paused { "⏸" } else { "▶" };
    let label = match app.now_playing() {
        Some(t) => format!("{icon}  {}  —  {}", t.title, t.artist),
        None => format!("{icon}  Nothing playing"),
    };
    let vol = format!("🔊 {:>3}%", app.volume.round() as i64);
    let header = Line::from(vec![
        Span::styled(label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled(vol, Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(Paragraph::new(header), rows[0]);

    let dur = st.duration.max(0.0);
    let pos = st.time_pos.clamp(0.0, dur.max(0.0));
    let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(ACCENT).bg(Color::Rgb(40, 40, 40)))
        .ratio(ratio.clamp(0.0, 1.0))
        .label(format!(
            "{} / {}",
            fmt_secs(pos as u64),
            fmt_secs(dur as u64)
        ));
    f.render_widget(gauge, rows[1]);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let help = "space pause · n/p · ←/→ seek · +/- vol · / search · a queue · r radio · y lyrics · L/D rate · s sort · q quit";
    let line = Line::from(vec![
        Span::styled(format!(" {} ", app.status), Style::default().fg(Color::Yellow)),
        Span::styled(format!("│ {help}"), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        format!("{s:<width$}", width = max)
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
