# Cadence ♫

[![CI](https://github.com/NoelPaff/cadence/actions/workflows/ci.yml/badge.svg)](https://github.com/NoelPaff/cadence/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/NoelPaff/cadence)](https://github.com/NoelPaff/cadence/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-cyan.svg)](LICENSE)

**Cadence** is a fast, minimalist, immersive local terminal music player written from scratch in **Rust** for **Linux, macOS, and Windows**.

---

## Features

- **Instant Smart Playlists from the CLI**: Launch music by artist, album, directory, song, or colon-chained filters (`cadence miles:kind`, `cadence arana`, `cadence all`, `cadence albums`).
- **True Gapless Playback Engine**: Unified 48.0 kHz stereo `f32` audio pipeline with a stateful 4-point Catmull-Rom resampler and next-track pre-buffering—zero clicks or gaps even across `44.1 kHz MP3 → 96 kHz FLAC` transitions.
- **Broad Local Audio Support**:
  - Pure-Rust decoding (`symphonia`) for **MP3, FLAC, Ogg Vorbis, M4A/MP4, AAC, ALAC, WAV, AIFF, CAF, MKV/WebM**.
  - Dynamic native **Opus** decoding via `libopusfile` on Linux/macOS, plus automatic `ffmpeg` pipe fallback for exotic formats (**WMA, APE, WavPack, DSD**).
- **Cross-Platform Low-Latency Audio**:
  - **Linux**: Dynamically loads PipeWire/PulseAudio (`libpulse-simple.so.0`) or ALSA (`libasound.so.2`) at runtime—builds cleanly with **zero `-devel` C header packages required**.
  - **macOS & Windows**: Native **CoreAudio** and **WASAPI** output.
- **Rich Album Cover Art & Dynamic Palette Extraction**:
  - Renders embedded or folder cover art in **24-bit Truecolor Half-Blocks (`▀`)**, **Colored ASCII**, **High-Density Unicode Braille**, or **Kitty Pixel Graphics** (`b` to cycle).
  - Extracts dominant vibrant colors from the current album cover (`i` toggles Album Palette vs. 8 curated themes: `midnight`, `catppuccin`, `gruvbox`, `nord`, `tokyonight`, `rosepine`, `solarized`, `amber`, plus custom `~/.config/cadence/themes/*.toml`).
- **Real-Time 2048-pt FFT Spectrum Visualizer**: Hann-windowed logarithmic spectrum analyzer with gravity peak-hold caps and 4 styles (`Bars`, `Mirrored`, `Braille`, `LED Meter` via `v` / `V`).
- **Synced `.lrc` & Embedded Lyrics**: Real-time highlighted scrolling lyrics in Track View (`F4`, toggle with `L`).
- **Desktop Integration**: Pure-Rust `zbus` **MPRIS2** server (`org.mpris.MediaPlayer2.cadence`) on Linux for hardware media keys, `playerctl`, GNOME/KDE/Waybar media applets, and optional desktop notifications (`N`).

---

## Installation & Quick Start

### Option 1: One-Line Prebuilt Binary Install (No Rust Required)

**Linux & macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/NoelPaff/cadence/main/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/NoelPaff/cadence/main/install.ps1 | iex
```

Or download prebuilt archives directly from the [**GitHub Releases page**](https://github.com/NoelPaff/cadence/releases/latest) (`Linux x86_64`, `Linux ARM64`, `macOS Apple Silicon`, `macOS Intel`, `Windows x86_64`).

---

### Option 2: Install with Cargo (Rust)

If you have the [Rust toolchain](https://rustup.rs/) (`rustc >= 1.75`):

```bash
cargo install --git https://github.com/NoelPaff/cadence.git --locked
```

---

### Option 3: Clone, Build & Run Locally from Source

No special `-devel` C system packages are required to compile Cadence!

```bash
git clone https://github.com/NoelPaff/cadence.git
cd cadence

# Build and run directly
cargo run --release --locked

# Or build & install into ~/.local/bin/cadence
./install.sh --from-source
# (or: make install)
```

> **Optional Runtime Extras**:
> - For native `.opus` playback without `ffmpeg`, ensure `opusfile` runtime library is installed (`sudo dnf install opusfile` / `sudo apt install libopusfile0` / `brew install opusfile`).
> - Installing `ffmpeg` unlocks exotic formats such as `.wma`, `.ape`, `.wv` (WavPack), and `.dsf`.

---

## CLI Usage

By default, Cadence scans your OS music folder (`~/Music`). You can change it anytime with `cadence path <directory>`.

```bash
cadence                        # Open interactive TUI
cadence <query>                # Smart-match artist, album, directory, or song and play
cadence <artist>:<album>:<trk> # Narrow match using colon-separated hierarchy filters
cadence all                    # Enqueue entire music library shuffled
cadence albums                 # Play all albums in random album order (tracks in order)
cadence shuffle <query>        # Match query and force shuffle ON
cadence dir <name>             # Match directory name only
cadence song <name>            # Match individual track only
cadence play <path...>         # Play specific files, folders, or .m3u/.m3u8 playlists
cadence list <name>            # Load and play a saved .m3u/.m3u8 playlist
cadence path ~/Music           # Set and save default music library root
cadence theme catppuccin       # Set and save color theme
```

### Command-Line Flags

| Flag | Description |
| :--- | :--- |
| `-h`, `--help` | Show full CLI usage and shortcut overview |
| `-v`, `--version` | Print version and exit |
| `--nocover` | Hide album cover artwork |
| `--noui` | Run headless without TUI (supports MPRIS media keys) |
| `-q`, `--quitonstop` | Exit automatically when the queue finishes playing |
| `-e`, `--exact` | Require case-insensitive exact substring matches |
| `--library <DIR>` | Override music library directory for this session |
| `--null-audio` | Use precision-timed silent audio sink |

---

## Keybindings

| Key(s) | Action |
| :--- | :--- |
| `F2` / `1` | **Playlist View** (active queue, reorder, M3U export) |
| `F3` / `2` | **Library View** (Folder Tree & Artist/Album Tree) |
| `F4` / `3` | **Track View** (large cover art, audio specs, synced `.lrc` lyrics, visualizer) |
| `F5` / `4` | **Search View** (instant fuzzy/exact search) |
| `F6` / `5` / `?` | **Help & Diagnostics View** |
| `Space` / `p` | Play / Pause |
| `←` / `→` or `h` / `l` | Previous / Next track (Collapse / Expand in Library View) |
| `[` / `]` | Previous / Next track globally from any view |
| `Shift+←` / `Shift+→` (or `<` / `>`) | Seek `-5s` / `+5s` (or click anywhere on the progress bar) |
| `+` / `=` and `-`, `m` | Volume Up / Down (`5%` steps), Mute (`m`) |
| `s` | Toggle reversible Shuffle |
| `r` | Cycle Repeat (`Off` → `Repeat One` → `Repeat All`) |
| `v` / `V` | Toggle Spectrum Visualizer / Cycle Visualizer Style |
| `i` | Toggle Dynamic Album Cover Colors vs. Named Theme |
| `t` | Cycle Built-in Color Themes |
| `b` | Cycle Cover Render Mode (`Blocks` → `ASCII` → `Braille` → `Kitty`) |
| `L` | Toggle Synced Lyrics panel in Track View |
| `M` | Toggle Library between Folder Tree and Artist/Album Tree |
| `u` | Rescan Music Library in background |
| `.` (Period) | Append playing/selected track to default persistent playlist (`cadence.m3u8`) |
| `e` / `a`, `n` | Enqueue to end of playlist (`e`/`a`) or Play Next (`n`) |
| `J` / `K` (`Alt+↓`/`↑`) | Move selected track down / up in Playlist |
| `d` / `Delete`, `C` | Remove track from queue (`d`), Clear queue (`C`) |
| `w` | Save current queue as a `.m3u8` file |
| `gg` / `G` | Jump to top / bottom of list |
| `q` / `Esc` | Quit Cadence cleanly |

---

## License

Released under the [MIT License](LICENSE).
