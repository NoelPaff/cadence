use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Album cover rendering mode in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CoverRenderMode {
    /// 24-bit Truecolor Unicode Half-Block (`▀`) — universal, flicker-free
    #[default]
    HalfBlock,
    /// Stylized 24-bit Colored ASCII art (` .:-=+*#%@`)
    Ascii,
    /// High-density 2x4 Colored Unicode Braille stipple art
    Braille,
    /// Native Kitty terminal graphics protocol (`\x1b_G...`)
    Kitty,
}

impl CoverRenderMode {
    pub fn next(self) -> Self {
        match self {
            Self::HalfBlock => Self::Ascii,
            Self::Ascii => Self::Braille,
            Self::Braille => Self::Kitty,
            Self::Kitty => Self::HalfBlock,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::HalfBlock => "Truecolor Blocks",
            Self::Ascii => "Colored ASCII",
            Self::Braille => "Braille Art",
            Self::Kitty => "Kitty Pixel",
        }
    }
}

/// Spectrum visualizer rendering style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VisualizerStyle {
    /// Classic high-res sub-cell spectrum bars with gravity peak caps
    #[default]
    Bars,
    /// Center-mirrored symmetric spectrum
    Mirrored,
    /// High-density Unicode Braille spectrum curve
    Braille,
    /// Segmented retro LED meter columns
    Led,
}

impl VisualizerStyle {
    pub fn next(self) -> Self {
        match self {
            Self::Bars => Self::Mirrored,
            Self::Mirrored => Self::Braille,
            Self::Braille => Self::Led,
            Self::Led => Self::Bars,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bars => "Spectrum Bars",
            Self::Mirrored => "Mirrored Stereo",
            Self::Braille => "Braille Wave",
            Self::Led => "LED Meter",
        }
    }
}

/// Repeat mode for playlist playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::One,
            Self::One => Self::All,
            Self::All => Self::Off,
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::Off => "Repeat: Off",
            Self::One => "Repeat: One",
            Self::All => "Repeat: All",
        }
    }
}

/// Persistent user configuration (`~/.config/cadence/config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Root music library directory
    pub music_dir: PathBuf,
    /// Default volume percentage (0..=100)
    pub volume: u8,
    /// Show album cover artwork
    pub show_cover: bool,
    /// Cover rendering protocol/style
    pub cover_mode: CoverRenderMode,
    /// Derive UI colors dynamically from current album cover
    pub dynamic_album_colors: bool,
    /// Named static color theme when dynamic_album_colors is false (or fallback)
    pub theme: String,
    /// Show real-time FFT spectrum visualizer
    pub show_visualizer: bool,
    /// Visualizer rendering style
    pub visualizer_style: VisualizerStyle,
    /// Number of spectrum bars to display (auto-clamped to pane width)
    pub visualizer_bars: usize,
    /// Show synced lyrics panel in Track View
    pub show_lyrics: bool,
    /// Enable MPRIS2 D-Bus media key & desktop integration
    pub enable_mpris: bool,
    /// Send desktop notification on track change
    pub desktop_notifications: bool,
    /// Restore previous playlist queue on interactive launch
    pub restore_session: bool,
    /// Filename for the quick-save default playlist (`.` shortcut)
    pub default_playlist_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let music_dir = dirs::audio_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            music_dir,
            volume: 80,
            show_cover: true,
            cover_mode: CoverRenderMode::HalfBlock,
            dynamic_album_colors: true,
            theme: "midnight".to_string(),
            show_visualizer: true,
            visualizer_style: VisualizerStyle::Bars,
            visualizer_bars: 36,
            show_lyrics: true,
            enable_mpris: true,
            desktop_notifications: false,
            restore_session: true,
            default_playlist_name: "cadence.m3u8".to_string(),
        }
    }
}

/// Persistent session state (`~/.local/share/cadence/session.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub queue_paths: Vec<PathBuf>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub last_view_index: u8,
}

pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub themes_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub library_cache_file: PathBuf,
    pub cover_cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub session_file: PathBuf,
    pub playlists_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("cadence");
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("cadence");
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("cadence");

        Self {
            config_file: config_dir.join("config.toml"),
            themes_dir: config_dir.join("themes"),
            config_dir,
            library_cache_file: cache_dir.join("library_v1.json"),
            cover_cache_dir: cache_dir.join("covers"),
            cache_dir,
            session_file: data_dir.join("session.json"),
            playlists_dir: data_dir.join("playlists"),
            data_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.themes_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(&self.cover_cache_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.playlists_dir)?;
        Ok(())
    }
}

impl AppConfig {
    pub fn load_or_create(path: &Path) -> Self {
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&raw) {
                return cfg;
            }
        }
        let cfg = AppConfig::default();
        let _ = cfg.save(path);
        cfg
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }

    pub fn default_playlist_path(&self, paths: &AppPaths) -> PathBuf {
        if self.music_dir.exists() && self.music_dir.is_dir() {
            self.music_dir.join(&self.default_playlist_name)
        } else {
            paths.playlists_dir.join(&self.default_playlist_name)
        }
    }
}

impl SessionState {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }
}
