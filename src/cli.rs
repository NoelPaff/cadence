use clap::Parser;
use std::path::PathBuf;

const AFTER_HELP: &str = "\
SMART COMMANDS & QUERIES:
  cadence                        Open interactive TUI music player
  cadence <query>                Smart-match artist, album, or track and play immediately
  cadence <artist>:<album>:<trk> Narrow match with colon-separated hierarchy filters
  cadence all                    Enqueue all library tracks shuffled and play
  cadence albums                 Play all albums in random album order (tracks in order)
  cadence shuffle <query>        Match query and force shuffle mode ON
  cadence dir <name>             Match directory name only and play in track order
  cadence song <name>            Match individual song title/filename only
  cadence play <path...>         Play specific file(s), folder(s), or .m3u/.m3u8 playlists
  cadence list <name>            Load and play a saved .m3u/.m3u8 playlist by name
  cadence path <dir>             Save default music library directory and exit
  cadence theme <name>           Apply and save color theme (midnight, catppuccin, gruvbox,
                                 nord, tokyonight, rosepine, solarized, amber)

KEYBINDINGS OVERVIEW:
  F2 / 1     Playlist View       Space / p   Play / Pause
  F3 / 2     Library View        h/l, ←/→    Prev / Next Track ([ / ] in all views)
  F4 / 3     Track View          Shift+←/→   Seek -5s / +5s
  F5 / 4     Search View         + / - / m   Volume Up / Down / Mute
  F6 / 5 / ? Help & Settings     s / r       Toggle Shuffle / Cycle Repeat
  v / V      Toggle/Style Vis    i / t / b   Album Colors / Cycle Theme / Cover Mode
  u          Rescan Library      .           Save Track to Default Playlist (cadence.m3u8)
  q / Esc    Quit Cadence";

/// Fast, minimalist, immersive local terminal music player
#[derive(Parser, Debug, Clone)]
#[command(
    name = "cadence",
    author,
    about = "Cadence — Fast, minimalist, immersive local terminal music player",
    after_help = AFTER_HELP,
    disable_version_flag = true
)]
pub struct CliArgs {
    /// Print version information and exit
    #[arg(short = 'v', short_alias = 'V', long = "version")]
    pub version: bool,

    /// Hide album cover artwork
    #[arg(long = "nocover")]
    pub no_cover: bool,

    /// Run headless in the terminal with no TUI interface
    #[arg(long = "noui")]
    pub no_ui: bool,

    /// Automatically exit when playback reaches the end of the playlist
    #[arg(short = 'q', long = "quitonstop")]
    pub quit_on_stop: bool,

    /// Require case-insensitive exact substring matches instead of fuzzy scoring
    #[arg(short = 'e', long = "exact")]
    pub exact: bool,

    /// Override music library root directory for this session
    #[arg(long = "library", value_name = "DIR")]
    pub library_override: Option<PathBuf>,

    /// Use a precision-timed silent audio output sink (useful for headless/SSH/testing)
    #[arg(long = "null-audio")]
    pub null_audio: bool,

    /// Command keyword or smart search query terms
    #[arg(trailing_var_arg = true, allow_hyphen_values = false)]
    pub positional: Vec<String>,
}

/// High-level parsed CLI intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// Launch interactive TUI without replacing session queue
    Interactive,
    /// Print version and exit
    Version,
    /// Persist default library path (`cadence path <dir>`)
    SetPath(PathBuf),
    /// Persist theme selection (`cadence theme <name>`)
    SetTheme(String),
    /// Enqueue all tracks in library shuffled (`cadence all`)
    All,
    /// Enqueue all albums in random album order (`cadence albums`)
    Albums,
    /// Match query and force shuffle (`cadence shuffle <query>`)
    ShuffleQuery(String),
    /// Match directory only (`cadence dir <query>`)
    DirQuery(String),
    /// Match single track only (`cadence song <query>`)
    SongQuery(String),
    /// Play explicit file/folder/playlist paths (`cadence play <paths...>`)
    PlayPaths(Vec<PathBuf>),
    /// Load named `.m3u` / `.m3u8` playlist (`cadence list <name>`)
    LoadPlaylist(String),
    /// Smart search across artists, albums, directories, and tracks (`cadence <query>`)
    SmartQuery(String),
}

impl CliArgs {
    pub fn resolve_command(&self) -> CliCommand {
        if self.version {
            return CliCommand::Version;
        }
        if self.positional.is_empty() {
            return CliCommand::Interactive;
        }

        let head_lower = self.positional[0].to_lowercase();
        let rest = &self.positional[1..];

        match head_lower.as_str() {
            "all" if rest.is_empty() => CliCommand::All,
            "albums" if rest.is_empty() => CliCommand::Albums,
            "path" if !rest.is_empty() => {
                let joined = rest.join(" ");
                CliCommand::SetPath(expand_tilde(&joined))
            }
            "theme" if !rest.is_empty() => CliCommand::SetTheme(rest.join(" ")),
            "shuffle" if !rest.is_empty() => CliCommand::ShuffleQuery(rest.join(" ")),
            "dir" if !rest.is_empty() => CliCommand::DirQuery(rest.join(" ")),
            "song" if !rest.is_empty() => CliCommand::SongQuery(rest.join(" ")),
            "list" if !rest.is_empty() => CliCommand::LoadPlaylist(rest.join(" ")),
            "play" if !rest.is_empty() => {
                let paths = rest.iter().map(|s| expand_tilde(s)).collect();
                CliCommand::PlayPaths(paths)
            }
            _ => {
                // Also check if the single argument is an existing file/directory path on disk
                if self.positional.len() == 1 {
                    let candidate = expand_tilde(&self.positional[0]);
                    if candidate.exists()
                        && (self.positional[0].contains('/')
                            || self.positional[0].starts_with('.')
                            || self.positional[0].starts_with('~'))
                    {
                        return CliCommand::PlayPaths(vec![candidate]);
                    }
                }
                CliCommand::SmartQuery(self.positional.join(" "))
            }
        }
    }
}

pub fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smart_queries_and_keywords() {
        let a = CliArgs::parse_from(["cadence"]);
        assert_eq!(a.resolve_command(), CliCommand::Interactive);

        let a = CliArgs::parse_from(["cadence", "-v"]);
        assert_eq!(a.resolve_command(), CliCommand::Version);

        let a = CliArgs::parse_from(["cadence", "all"]);
        assert_eq!(a.resolve_command(), CliCommand::All);

        let a = CliArgs::parse_from(["cadence", "albums"]);
        assert_eq!(a.resolve_command(), CliCommand::Albums);

        let a = CliArgs::parse_from(["cadence", "jazz", "at", "mladost"]);
        assert_eq!(
            a.resolve_command(),
            CliCommand::SmartQuery("jazz at mladost".to_string())
        );

        let a = CliArgs::parse_from(["cadence", "-e", "-q", "dir", "mladost"]);
        assert!(a.exact);
        assert!(a.quit_on_stop);
        assert_eq!(
            a.resolve_command(),
            CliCommand::DirQuery("mladost".to_string())
        );

        let a = CliArgs::parse_from(["cadence", "shuffle", "miles:kind"]);
        assert_eq!(
            a.resolve_command(),
            CliCommand::ShuffleQuery("miles:kind".to_string())
        );
    }
}
