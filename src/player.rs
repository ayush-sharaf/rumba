//! mpv playback backend.
//!
//! We launch `mpv --idle --no-video --input-ipc-server=<sock>` and drive it
//! over its JSON IPC socket. mpv resolves YouTube Music URLs itself via its
//! bundled yt-dlp hook, so we just hand it `watch?v=<id>` URLs.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default, Clone)]
pub struct PlaybackState {
    pub time_pos: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    /// Incremented every time mpv reports a track finishing (eof). The UI
    /// compares against a last-seen value to decide when to auto-advance.
    pub eof_count: u64,
}

pub struct Player {
    sock: Mutex<UnixStream>,
    state: Arc<Mutex<PlaybackState>>,
    _child: std::process::Child,
}

impl Player {
    pub fn spawn(mpv: &str, socket_path: &str, browser: Option<&str>) -> Result<Self> {
        let _ = std::fs::remove_file(socket_path);
        let mut cmd = std::process::Command::new(mpv);
        cmd.arg("--idle=yes")
            .arg("--no-video")
            .arg("--no-terminal")
            // bestaudio = highest-bitrate audio-only stream.
            .arg("--ytdl-format=bestaudio/best")
            .arg(format!("--input-ipc-server={socket_path}"));
        // Authenticate yt-dlp with the same browser session so it can fetch the
        // account's high-bitrate streams (e.g. ~256kbps opus) rather than the
        // ~128kbps anonymous ones.
        if let Some(b) = browser {
            cmd.arg(format!("--ytdl-raw-options=cookies-from-browser={b}"));
        }
        let child = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start mpv ({mpv})"))?;

        // The socket appears a beat after launch; retry briefly.
        let mut stream = None;
        for _ in 0..100 {
            if let Ok(s) = UnixStream::connect(socket_path) {
                stream = Some(s);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let stream = stream.context("could not connect to mpv IPC socket")?;
        let reader_stream = stream.try_clone()?;

        let state = Arc::new(Mutex::new(PlaybackState {
            volume: 100.0,
            ..Default::default()
        }));
        let state_for_reader = Arc::clone(&state);

        std::thread::spawn(move || {
            let reader = BufReader::new(reader_stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                handle_event(&v, &state_for_reader);
            }
        });

        let player = Self {
            sock: Mutex::new(stream),
            state,
            _child: child,
        };

        // Watch the properties the UI renders.
        player.send(json!({"command": ["observe_property", 1, "time-pos"]}))?;
        player.send(json!({"command": ["observe_property", 2, "duration"]}))?;
        player.send(json!({"command": ["observe_property", 3, "pause"]}))?;
        player.send(json!({"command": ["observe_property", 4, "volume"]}))?;
        Ok(player)
    }

    fn send(&self, cmd: Value) -> Result<()> {
        let mut line = serde_json::to_string(&cmd)?;
        line.push('\n');
        let mut sock = self.sock.lock().unwrap();
        sock.write_all(line.as_bytes())?;
        sock.flush()?;
        Ok(())
    }

    pub fn state(&self) -> PlaybackState {
        self.state.lock().unwrap().clone()
    }

    pub fn load(&self, video_id: &str) -> Result<()> {
        let url = format!("https://music.youtube.com/watch?v={video_id}");
        self.send(json!({"command": ["loadfile", url, "replace"]}))?;
        self.set_pause(false)
    }

    pub fn toggle_pause(&self) -> Result<()> {
        self.send(json!({"command": ["cycle", "pause"]}))
    }

    pub fn set_pause(&self, paused: bool) -> Result<()> {
        self.send(json!({"command": ["set_property", "pause", paused]}))
    }

    pub fn seek_relative(&self, secs: f64) -> Result<()> {
        self.send(json!({"command": ["seek", secs, "relative"]}))
    }

    /// Jump back to the start of the current track.
    pub fn restart(&self) -> Result<()> {
        self.send(json!({"command": ["seek", 0, "absolute"]}))
    }

    // Kept for completeness: rumba drives the system volume (see `sysvol`),
    // leaving mpv at full volume, but this remains available.
    #[allow(dead_code)]
    pub fn set_volume(&self, vol: f64) -> Result<()> {
        let vol = vol.clamp(0.0, 130.0);
        self.send(json!({"command": ["set_property", "volume", vol]}))
    }

    pub fn stop(&self) -> Result<()> {
        self.send(json!({"command": ["stop"]}))
    }
}

fn handle_event(v: &Value, state: &Arc<Mutex<PlaybackState>>) {
    match v.get("event").and_then(Value::as_str) {
        Some("property-change") => {
            let name = v.get("name").and_then(Value::as_str).unwrap_or("");
            let data = v.get("data");
            let mut s = state.lock().unwrap();
            match name {
                "time-pos" => s.time_pos = data.and_then(Value::as_f64).unwrap_or(0.0),
                "duration" => s.duration = data.and_then(Value::as_f64).unwrap_or(0.0),
                "pause" => s.paused = data.and_then(Value::as_bool).unwrap_or(false),
                "volume" => {
                    if let Some(vol) = data.and_then(Value::as_f64) {
                        s.volume = vol;
                    }
                }
                _ => {}
            }
        }
        Some("end-file") => {
            // reason == "eof" means the track played to completion.
            if v.get("reason").and_then(Value::as_str) == Some("eof") {
                let mut s = state.lock().unwrap();
                s.eof_count += 1;
                s.time_pos = 0.0;
            }
        }
        _ => {}
    }
}
