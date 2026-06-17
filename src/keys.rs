//! Configurable key bindings.
//!
//! Normal-mode keys map to [`Action`]s through a [`Keymap`]. Defaults are built
//! in; `~/.config/rumba/config.toml` can override them, e.g.:
//!
//! ```toml
//! [keymap]
//! quit = "x"
//! play_pause = ["space", "p"]
//! download = "w"
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Quit,
    NextTab,
    PrevTab,
    Tab(usize),
    Search,
    Down,
    Up,
    Activate,
    Back,
    Enqueue,
    Radio,
    Lyrics,
    Like,
    Dislike,
    Sort,
    Download,
    PlayPause,
    Next,
    Prev,
    SeekFwd,
    SeekBack,
    VolUp,
    VolDown,
}

impl Action {
    fn from_name(s: &str) -> Option<Action> {
        Some(match s {
            "quit" => Action::Quit,
            "next_tab" => Action::NextTab,
            "prev_tab" => Action::PrevTab,
            "tab1" => Action::Tab(0),
            "tab2" => Action::Tab(1),
            "tab3" => Action::Tab(2),
            "tab4" => Action::Tab(3),
            "tab5" => Action::Tab(4),
            "tab6" => Action::Tab(5),
            "tab7" => Action::Tab(6),
            "search" => Action::Search,
            "down" => Action::Down,
            "up" => Action::Up,
            "activate" => Action::Activate,
            "back" => Action::Back,
            "enqueue" => Action::Enqueue,
            "radio" => Action::Radio,
            "lyrics" => Action::Lyrics,
            "like" => Action::Like,
            "dislike" => Action::Dislike,
            "sort" => Action::Sort,
            "download" => Action::Download,
            "play_pause" => Action::PlayPause,
            "next" => Action::Next,
            "prev" => Action::Prev,
            "seek_fwd" => Action::SeekFwd,
            "seek_back" => Action::SeekBack,
            "vol_up" => Action::VolUp,
            "vol_down" => Action::VolDown,
            _ => return None,
        })
    }
}

const DEFAULTS: &[(&str, Action)] = &[
    ("q", Action::Quit),
    ("ctrl-c", Action::Quit),
    ("tab", Action::NextTab),
    ("backtab", Action::PrevTab),
    ("1", Action::Tab(0)),
    ("2", Action::Tab(1)),
    ("3", Action::Tab(2)),
    ("4", Action::Tab(3)),
    ("5", Action::Tab(4)),
    ("6", Action::Tab(5)),
    ("7", Action::Tab(6)),
    ("/", Action::Search),
    ("j", Action::Down),
    ("down", Action::Down),
    ("k", Action::Up),
    ("up", Action::Up),
    ("enter", Action::Activate),
    ("esc", Action::Back),
    ("a", Action::Enqueue),
    ("r", Action::Radio),
    ("y", Action::Lyrics),
    ("L", Action::Like),
    ("D", Action::Dislike),
    ("s", Action::Sort),
    ("d", Action::Download),
    ("space", Action::PlayPause),
    ("n", Action::Next),
    ("p", Action::Prev),
    ("b", Action::Prev),
    ("right", Action::SeekFwd),
    ("l", Action::SeekFwd),
    ("left", Action::SeekBack),
    ("h", Action::SeekBack),
    ("+", Action::VolUp),
    ("=", Action::VolUp),
    ("-", Action::VolDown),
    ("_", Action::VolDown),
];

#[derive(Deserialize, Default)]
struct Config {
    #[serde(default)]
    keymap: HashMap<String, KeySpec>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum KeySpec {
    One(String),
    Many(Vec<String>),
}

impl KeySpec {
    fn into_keys(self) -> Vec<String> {
        match self {
            KeySpec::One(s) => vec![s],
            KeySpec::Many(v) => v,
        }
    }
}

pub struct Keymap {
    map: HashMap<String, Action>,
}

impl Keymap {
    pub fn load(config_dir: &Path) -> Keymap {
        let mut map: HashMap<String, Action> =
            DEFAULTS.iter().map(|(k, a)| (k.to_string(), *a)).collect();

        if let Ok(txt) = std::fs::read_to_string(config_dir.join("config.toml")) {
            if let Ok(cfg) = toml::from_str::<Config>(&txt) {
                for (action_name, spec) in cfg.keymap {
                    if let Some(action) = Action::from_name(&action_name) {
                        // Drop default keys that pointed at this action, so the
                        // config fully replaces (not just adds to) the binding.
                        map.retain(|_, a| *a != action);
                        for key in spec.into_keys() {
                            map.insert(key, action);
                        }
                    }
                }
            }
        }
        Keymap { map }
    }

    pub fn action(&self, key: &KeyEvent) -> Option<Action> {
        key_to_string(key).and_then(|s| self.map.get(&s).copied())
    }
}

/// Normalise a key event to the string form used in bindings.
fn key_to_string(k: &KeyEvent) -> Option<String> {
    let base = match k.code {
        KeyCode::Char(' ') => "space".to_string(),
        // Normalise shifted letters to uppercase so `Shift+L` maps to "L"
        // regardless of whether the terminal reports it as 'L' or 'l'+Shift
        // (the Kitty keyboard protocol can do the latter).
        KeyCode::Char(c) if k.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() => {
            c.to_ascii_uppercase().to_string()
        }
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "backtab".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        _ => return None,
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        Some(format!("ctrl-{base}"))
    } else {
        Some(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn defaults_resolve() {
        let km = Keymap::load(std::path::Path::new("/no/such/rumba/dir"));
        let cases = [
            (ch('q'), Action::Quit),
            (KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), Action::PlayPause),
            (ch('/'), Action::Search),
            (ch('n'), Action::Next),
            (ch('p'), Action::Prev),
            (ch('a'), Action::Enqueue),
            (ch('r'), Action::Radio),
            (ch('y'), Action::Lyrics),
            (ch('s'), Action::Sort),
            (ch('d'), Action::Download),
            (ch('l'), Action::SeekFwd),
            (ch('h'), Action::SeekBack),
            (ch('+'), Action::VolUp),
            (ch('-'), Action::VolDown),
            (ch('1'), Action::Tab(0)),
            (ch('7'), Action::Tab(6)),
            (KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), Action::NextTab),
            (KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), Action::Activate),
            (KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), Action::Back),
            (KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Action::Down),
            (KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Quit),
        ];
        for (ev, want) in cases {
            assert_eq!(km.action(&ev), Some(want), "key {ev:?} should map to {want:?}");
        }
        assert_eq!(km.action(&ch('z')), None, "unbound key should be None");
    }

    #[test]
    fn shifted_letters_normalise() {
        let km = Keymap::load(std::path::Path::new("/no/such/rumba/dir"));
        // Terminal reports 'L' directly:
        assert_eq!(km.action(&ch('L')), Some(Action::Like));
        // Kitty protocol reports 'l' + Shift — must still be Like, not SeekFwd:
        let shifted = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SHIFT);
        assert_eq!(km.action(&shifted), Some(Action::Like));
        let shifted_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::SHIFT);
        assert_eq!(km.action(&shifted_d), Some(Action::Dislike));
    }

    #[test]
    fn config_overrides_apply() {
        let dir = std::env::temp_dir().join("rumba-keytest");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            "[keymap]\nquit = \"x\"\nplay_pause = [\"space\", \"k\"]\n",
        )
        .unwrap();
        let km = Keymap::load(&dir);
        assert_eq!(km.action(&ch('x')), Some(Action::Quit));
        assert_eq!(km.action(&ch('q')), None, "default key replaced by override");
        assert_eq!(km.action(&ch('k')), Some(Action::PlayPause));
        std::fs::remove_dir_all(&dir).ok();
    }
}
