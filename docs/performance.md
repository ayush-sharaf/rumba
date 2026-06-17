# Performance: why a terminal client is lighter

rumba plays YouTube Music from a native terminal UI and hands audio to a headless
`mpv` in **audio-only** mode. Compared with playing music.youtube.com in a browser
tab — or in a Chromium/Electron desktop wrapper — it removes the single biggest
cost (an embedded browser engine) and only ever streams audio.

This page mixes **numbers measured on a real machine** (Apple Silicon Mac, Brave,
June 2026) with **published figures** for things that can't be measured locally
(Electron desktop apps, video-tier bandwidth). Measured values are labelled
*(measured)*; everything else is cited.

## TL;DR

- **RAM:** the YouTube Music **browser tab alone used 402–517 MB** *(measured)*;
  rumba's TUI process used **24.6 MB** *(measured)* plus a 64–103 MB `mpv`
  backend — roughly **16–21× less RAM for the UI**, and ~4–8× less for the whole
  app vs a tab plus its share of the browser.
- **Bandwidth:** rumba streamed **107 MiB/hour (~112 MB/hr)** of Opus 248 kbps
  audio *(measured)*. The web app pulls a *similar* audio bitrate, so the win is
  not a smaller audio stream — it's that rumba is **always audio-only (never
  fetches video)** and carries no ads, cover-art images, JavaScript, or telemetry.
- **Disk:** an **8.7 MB** single binary *(measured)* vs a full browser / a
  150–250 MB+ Electron bundle. [3]

## How rumba plays audio

`mpv` is launched headless with `--no-video --ytdl-format=bestaudio/best` and fed a
bare `https://music.youtube.com/watch?v=<id>` URL. Its bundled `yt-dlp` resolves the
**direct audio stream** (with your session cookies it selects the top audio-only
format — measured here as format `774`, **Opus 248 kbps**). No video is ever
requested or decoded; the UI is plain terminal text via `ratatui`/`crossterm` — no
HTML, no JS engine, no GPU compositor.

## RAM

| Component | RAM | Source |
| --- | --- | --- |
| rumba TUI process | **24.6 MB** | *(measured)* |
| rumba's `mpv` backend | **64–103 MB** (≈65–75 settled) | *(measured)* |
| **rumba total** | **~90–130 MB** | *(measured)* |
| YouTube Music tab renderer (Brave) | **402–517 MB** + ~97 MB helper | *(measured)* |
| (whole Brave session for context) | 2.8 GB across 18 processes | *(measured)* |
| Media-heavy browser tab (general) | 300–800 MB | [1] |
| Chrome with YouTube open | ~260 MB; ~360 MB while video plays | [2] |
| Electron desktop app baseline | 50–100 MB just to start | [3] |
| Electron media/chat apps (Spotify/Discord/Slack) | 200–500 MB; Discord seen ~1 GB | [3] |

The measured 402–517 MB for one YouTube Music tab sits right in the published
300–800 MB range for media-heavy tabs [1], and that is the *same* Chromium engine
the YouTube Music desktop app ships. rumba carries none of it — the same
architectural reason native terminal clients like **ncspot** (Spotify) report a
**40–65 % lower RAM footprint** than the official client (~726 MB), by rendering
straight to the terminal and skipping GPU compositing and font rasterization. [7]

## Bandwidth

**rumba, measured:** downloading exactly 60 s of the `bestaudio` stream rumba uses
produced **1.79 MiB**, i.e. **~107 MiB/hour (~112 MB/hr)** at Opus 248 kbps — within
~5 % of the bitrate math (`256 kbps ≈ 115 MB/hr`).

YouTube Music's audio tiers are **48 / 128 / 256 kbps** (AAC & Opus) [5], so
audio-only playback ranges roughly **22 → 58 → 115 MB/hr**. The web app streams a
comparable audio bitrate, **so the audio bandwidth itself is similar.** rumba's
advantage is structural:

1. **It never fetches video.** The web player can serve a music video for tracks
   that have one — and video is an order of magnitude heavier:

   | Resolution | Data/hour | Source |
   | --- | --- | --- |
   | 480p | ~480–660 MB | [4][6] |
   | 720p | ~1.2–1.5 GB | [4][6] |
   | 1080p | ~2.1–3 GB | [4][6] |

2. **No page overhead:** no ads (on a non-Premium account the web app streams ad
   segments), no cover-art/thumbnail images, no JavaScript bundles, no prefetch,
   no analytics/telemetry.

> Per-tab browser bandwidth is hard to measure cleanly (the player buffers whole
> songs ahead, and Chromium routes stream bytes through a separate network process),
> so we do not quote a single browser MB/hr figure. The honest summary: **audio
> bandwidth is comparable; rumba saves by never pulling video and by dropping all
> page overhead.**

## CPU & battery

rumba does native Opus/AAC decoding in `mpv` plus cheap terminal text drawing — no
HTML layout, no JavaScript execution, no video decode, no GPU compositing. The
browser/Electron path runs a JS engine and a page compositor continuously, and a
video decoder whenever a music video plays. (Directional — not separately
benchmarked here; consistent with the terminal-rendering efficiency noted in [7].)

## Disk / install

- rumba: **8.7 MB** single native binary *(measured)*, plus system `mpv`/`yt-dlp`.
- A Chromium/Electron desktop app bundles a whole browser runtime — typically
  **150–250 MB+** on disk. [3]

## Comparison

| Dimension | rumba (CLI) | YouTube Music web (tab) | YT Music desktop (Chromium/Electron) |
| --- | --- | --- | --- |
| UI/app RAM | **~25 MB TUI (+65–103 MB mpv)** *(meas.)* | **402–517 MB** *(meas.)* + browser baseline | 200–500 MB+ [3] |
| Bandwidth/hr | **~112 MB audio-only** *(meas.)* | similar audio **or 0.5–3 GB if video** + overhead | similar to web |
| Video ever fetched | **Never** | Possible per track | Possible per track |
| Ads / images / telemetry | None | Yes | Yes |
| Install size | **8.7 MB** *(meas.)* | uses existing browser | 150–250 MB+ |

## A note on ads

rumba contains **no ad-blocking code**. Because playback goes through `mpv`/`yt-dlp`
fetching the content audio stream directly — rather than YouTube's official player,
which is what inserts ad segments — ad breaks do not occur during playback. This is
a side effect of the architecture, not a feature, and it effectively yields
audio-only, ad-free playback without Premium. Using it this way is contrary to
YouTube's Terms of Service; respect them.

## Caveats

1. Browser/Electron numbers vary by platform, version, Premium status, and whether a
   music video is served; treat ranges as representative, not guarantees.
2. Measured figures are from one Apple Silicon Mac (Brave, June 2026). The browser
   tab RAM fluctuated 402–517 MB during the session.
3. Per-tab browser **bandwidth** was not cleanly isolable (ahead-buffering +
   Chromium's network-process model), so no single browser MB/hr is claimed.
4. Some widely-quoted "MB/hour" music figures conflate audio with video (e.g.
   "750 MB/hr for high quality"); those are inconsistent with a 256 kbps stream and
   are excluded in favour of bitrate-derived math and direct measurement.

## Sources

1. Nest — *Chrome Tabs Using Too Much RAM*: media-heavy tabs 300–800 MB. https://nestextended.com/blog/chrome-tabs-too-much-ram/
2. MakeUseOf — *Why Is Chrome Using So Much RAM*: YouTube ~260 MB, ~360 MB during video. https://www.makeuseof.com/tag/chrome-using-much-ram-fix-right-now/
3. WindowsForum / Electron docs — Electron baseline 50–100 MB; Spotify/Discord/Slack 200–500 MB; Chromium multi-process model. https://windowsforum.com/threads/why-windows-apps-hog-ram-electron-and-webview2-explained.392960/ · https://www.electronjs.org/blog/v8-memory-cage
4. Recharge — *How Much Data Does YouTube Use*: per-resolution video data/hour. https://www.recharge.com/blog/en-au/au/how-much-data-does-youtube-use-2026-au-guide
5. Free-Codecs — *YouTube Music Premium Settings*: tiers 48/128/256 kbps AAC & Opus. https://www.free-codecs.com/guides/how-to-get-better-sound-quality-on-youtube-music-premium-settings-guide.htm
6. BandwidthPlace — *Data Consumption: Netflix vs Spotify vs YouTube*: 480p 480–660 MB/hr, 720p 1.2–1.5 GB/hr, 1080p 2.1–3 GB/hr; Spotify 96 kbps = 43.2 MB/hr. https://www.bandwidthplace.com/article/data-consumption-netflix-vs-spotify-vs-youtube
7. ncspot (native terminal Spotify client) comparisons — official client ~726 MB, alternatives 40–65 % lower; direct terminal rendering avoids GPU/compositor overhead. https://www.linuxlinks.com/ncspot-ncurses-spotify-client/ · https://alternativeto.net/software/ncspot/

*Methodology: rumba TUI/mpv RAM via `ps -o rss`; audio bandwidth by downloading 60 s
of the `bestaudio` stream and measuring bytes; browser tab RAM by diffing Brave's
process list before/after opening a fresh music.youtube.com window. Bandwidth math
uses decimal MB (`kbps ÷ 8 × 3600 ÷ 1000`).*
