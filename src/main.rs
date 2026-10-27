use anyhow::{bail, Result};
use cadence::audio::AudioEngine;
use cadence::cli::{CliArgs, CliCommand};
use cadence::config::{AppConfig, AppPaths, SessionState};
use cadence::desktop::MprisBridge;
use cadence::media::cover::clear_kitty_graphics;
use cadence::model::track::{
    format_duration_ms, is_playlist_file, is_supported_audio_file, natural_cmp, Track,
};
use cadence::model::{MusicLibrary, Playlist};
use cadence::ui::input::handle_terminal_event;
use cadence::ui::{ActiveView, App};
use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::Widget;
use ratatui::Terminal;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use walkdir::WalkDir;

fn main() {
    if let Err(err) = run() {
        eprintln!("cadence: {}", err);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = CliArgs::parse();
    let command = args.resolve_command();

    if command == CliCommand::Version {
        println!("cadence {}", cadence::version());
        return Ok(());
    }

    let paths = AppPaths::resolve();
    let _ = paths.ensure_dirs();
    let mut config = AppConfig::load_or_create(&paths.config_file);

    // Handle persistent configuration commands that exit immediately
    match &command {
        CliCommand::SetPath(new_dir) => {
            let resolved = if new_dir.exists() {
                new_dir.canonicalize().unwrap_or_else(|_| new_dir.clone())
            } else {
                bail!("Directory does not exist: {}", new_dir.display());
            };
            if !resolved.is_dir() {
                bail!("Path is not a directory: {}", resolved.display());
            }
            config.music_dir = resolved.clone();
            config.save(&paths.config_file)?;
            let lib = MusicLibrary::scan_with_cache(&config.music_dir, Some(&paths.library_cache_file));
            println!(
                "Cadence library path set to {} ({} tracks indexed).",
                resolved.display(),
                lib.len()
            );
            return Ok(());
        }
        CliCommand::SetTheme(theme_name) => {
            config.theme = theme_name.trim().to_lowercase();
            config.dynamic_album_colors = false;
            config.save(&paths.config_file)?;
            println!("Cadence theme set to '{}'.", config.theme);
            return Ok(());
        }
        _ => {}
    }

    if let Some(ref override_dir) = args.library_override {
        config.music_dir = override_dir.clone();
    }
    if args.no_cover {
        config.show_cover = false;
    }

    let library =
        MusicLibrary::scan_with_cache(&config.music_dir, Some(&paths.library_cache_file));

    let audio = AudioEngine::spawn(config.volume, args.null_audio);
    let mpris = MprisBridge::start(config.enable_mpris);

    let mut app = App::new(
        config.clone(),
        paths,
        library,
        audio,
        mpris,
        args.quit_on_stop,
        args.exact,
    );

    // Populate initial queue & playback state from CLI command
    let auto_play = prepare_initial_playback(&mut app, &command)?;

    if args.no_ui {
        run_headless_loop(&mut app, auto_play)?;
    } else {
        run_tui_loop(&mut app, auto_play)?;
    }

    app.save_session();
    Ok(())
}

fn prepare_initial_playback(app: &mut App, command: &CliCommand) -> Result<bool> {
    match command {
        CliCommand::Interactive => {
            if app.config.restore_session {
                let session = SessionState::load(&app.paths.session_file);
                app.restore_session(&session);
            }
            Ok(false)
        }
        CliCommand::All => {
            if app.library.is_empty() {
                bail!(
                    "No audio tracks found in library ({}). Use `cadence path <dir>` or `cadence play <path>`.",
                    app.library.root.display()
                );
            }
            let all_tracks = app.library.tracks().to_vec();
            app.playlist.set_tracks(all_tracks, Some(0), true);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::Albums => {
            if app.library.is_empty() {
                bail!(
                    "No audio tracks found in library ({}).",
                    app.library.root.display()
                );
            }
            let ordered_albums = app.library.all_albums_shuffled();
            app.playlist.set_tracks(ordered_albums, Some(0), false);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::ShuffleQuery(q) => {
            let matched = app.library.match_smart_query(q, app.exact_search).ok_or_else(|| {
                anyhow::anyhow!(
                    "No library match for '{}' in {} ({} tracks indexed)",
                    q,
                    app.library.root.display(),
                    app.library.len()
                )
            })?;
            app.playlist.set_tracks(matched.tracks, Some(0), true);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::DirQuery(q) => {
            let matched = app.library.match_directory(q, app.exact_search).ok_or_else(|| {
                anyhow::anyhow!("No directory matching '{}' in {}", q, app.library.root.display())
            })?;
            app.playlist.set_tracks(matched.tracks, Some(0), false);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::SongQuery(q) => {
            let matched = app.library.match_song(q, app.exact_search).ok_or_else(|| {
                anyhow::anyhow!("No song matching '{}' in {}", q, app.library.root.display())
            })?;
            app.playlist.set_tracks(matched.tracks, Some(0), false);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::LoadPlaylist(name) => {
            let direct = PathBuf::from(name);
            let m3u_path = if direct.exists() && direct.is_file() {
                direct
            } else {
                app.library
                    .find_playlist(name, Some(&app.paths.playlists_dir), app.exact_search)
                    .ok_or_else(|| anyhow::anyhow!("No .m3u/.m3u8 playlist matching '{}'", name))?
            };
            let raw_paths = Playlist::load_m3u_paths(&m3u_path)?;
            let tracks: Vec<Track> = raw_paths
                .into_iter()
                .filter(|p| p.exists() && is_supported_audio_file(p))
                .map(|p| app.library.resolve_or_extract(&p))
                .collect();
            if tracks.is_empty() {
                bail!("Playlist '{}' contains no valid local tracks", m3u_path.display());
            }
            app.playlist.set_tracks(tracks, Some(0), false);
            app.view = ActiveView::Playlist;
            Ok(true)
        }
        CliCommand::PlayPaths(paths) => {
            let tracks = collect_tracks_from_paths(&app.library, paths);
            if tracks.is_empty() {
                bail!("No playable audio files found in specified path(s)");
            }
            app.playlist.set_tracks(tracks, Some(0), false);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::SmartQuery(q) => {
            let matched = app.library.match_smart_query(q, app.exact_search).ok_or_else(|| {
                anyhow::anyhow!(
                    "No match for '{}' in {} ({} tracks indexed)",
                    q,
                    app.library.root.display(),
                    app.library.len()
                )
            })?;
            app.playlist
                .set_tracks(matched.tracks, Some(0), matched.suggest_shuffle);
            app.view = ActiveView::Track;
            Ok(true)
        }
        CliCommand::Version | CliCommand::SetPath(_) | CliCommand::SetTheme(_) => Ok(false),
    }
}

fn collect_tracks_from_paths(library: &MusicLibrary, inputs: &[PathBuf]) -> Vec<Track> {
    let mut out = Vec::new();
    for p in inputs {
        if !p.exists() {
            continue;
        }
        if p.is_file() {
            if is_playlist_file(p) {
                if let Ok(entries) = Playlist::load_m3u_paths(p) {
                    for e in entries {
                        if e.exists() && is_supported_audio_file(&e) {
                            out.push(library.resolve_or_extract(&e));
                        }
                    }
                }
            } else if is_supported_audio_file(p) {
                out.push(library.resolve_or_extract(p));
            }
        } else if p.is_dir() {
            let mut dir_tracks: Vec<Track> = WalkDir::new(p)
                .follow_links(true)
                .max_depth(12)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_supported_audio_file(e.path()))
                .map(|e| library.resolve_or_extract(e.path()))
                .collect();
            dir_tracks.sort_by(|a, b| {
                let pa = a.path.parent().unwrap_or_else(|| Path::new(""));
                let pb = b.path.parent().unwrap_or_else(|| Path::new(""));
                natural_cmp(&pa.to_string_lossy(), &pb.to_string_lossy())
                    .then_with(|| a.album_order_cmp(b))
            });
            out.extend(dir_tracks);
        }
    }
    out
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        clear_kitty_graphics();
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
    }
}

fn install_panic_restore_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        clear_kitty_graphics();
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
        default_hook(info);
    }));
}

struct AppSurface<'a>(&'a mut App);

impl Widget for AppSurface<'_> {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        self.0.render_frame(area, buf);
    }
}

fn run_tui_loop(app: &mut App, auto_play: bool) -> Result<()> {
    install_panic_restore_hook();
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    if auto_play && !app.playlist.is_empty() {
        let start = app.playlist.current_index().unwrap_or(0);
        app.start_playlist_index(start);
    }

    let tick_rate = Duration::from_millis(22); // ~45 FPS smooth visualizer & UI responsiveness

    while !app.should_quit {
        app.tick();
        if app.should_quit {
            break;
        }

        terminal.draw(|frame| {
            let area = frame.area();
            frame.render_widget(AppSurface(app), area);
        })?;

        if event::poll(tick_rate)? {
            let ev = event::read()?;
            handle_terminal_event(app, ev);
            // Drain any burst mouse/scroll events immediately so scrolling feels crisp
            while !app.should_quit && event::poll(Duration::from_millis(0))? {
                handle_terminal_event(app, event::read()?);
            }
        }
    }

    Ok(())
}

fn run_headless_loop(app: &mut App, auto_play: bool) -> Result<()> {
    if !auto_play && app.playlist.is_empty() {
        if app.library.is_empty() {
            bail!(
                "No tracks queued for --noui mode (library {} is empty).",
                app.library.root.display()
            );
        }
        // In --noui with no explicit query, play all shuffled
        let all = app.library.tracks().to_vec();
        app.playlist.set_tracks(all, Some(0), true);
    }

    if !app.playlist.is_empty() {
        let idx = app.playlist.current_index().unwrap_or(0);
        app.start_playlist_index(idx);
    }

    let is_tty = io::stdout().is_terminal();
    let interrupted = Arc::new(AtomicBool::new(false));
    let mut last_printed_track: Option<PathBuf> = None;

    while !app.should_quit && !interrupted.load(Ordering::Relaxed) {
        app.tick();

        if let Some(track) = app.playlist.current_track() {
            if last_printed_track.as_ref() != Some(&track.path) {
                last_printed_track = Some(track.path.clone());
                if is_tty {
                    println!(
                        "\r♫ Playing: {} — {} [{}]",
                        track.display_artist(),
                        track.title,
                        track.duration_formatted()
                    );
                } else {
                    println!(
                        "Playing: {} - {} ({})",
                        track.display_artist(),
                        track.title,
                        track.duration_formatted()
                    );
                }
            } else if is_tty {
                let dur = if app.telemetry.duration_ms > 0 {
                    app.telemetry.duration_ms
                } else {
                    track.duration_ms
                };
                print!(
                    "\r  {} / {}  (Vol {}%)   ",
                    format_duration_ms(app.telemetry.position_ms),
                    format_duration_ms(dur),
                    app.config.volume
                );
                let _ = io::stdout().flush();
            }
        }

        if app.should_quit {
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }

    if is_tty {
        println!();
    }
    Ok(())
}
