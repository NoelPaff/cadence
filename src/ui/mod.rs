pub mod help_view;
pub mod input;
pub mod library_view;
pub mod playlist_view;
pub mod search_view;
pub mod track_view;

use crate::audio::visualizer::SpectrumAnalyzer;
use crate::audio::{AudioCommand, AudioEngine, AudioEvent, AudioTelemetry};
use crate::config::{AppConfig, AppPaths, RepeatMode, SessionState};
use crate::desktop::{MprisBridge, MprisCommand, MprisStateSnapshot};
use crate::media::cover::CoverArt;
use crate::media::lyrics::Lyrics;
use crate::media::metadata::{load_cover_image_for_track, write_mpris_cover_cache};
use crate::media::palette::ThemePalette;
use crate::model::track::format_duration_ms;
use crate::model::{MusicLibrary, Playlist};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Playlist,
    Library,
    Track,
    Search,
    Help,
}

impl ActiveView {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Self::Playlist,
            1 => Self::Library,
            2 => Self::Track,
            3 => Self::Search,
            4 => Self::Help,
            _ => Self::Library,
        }
    }

    pub fn to_index(self) -> u8 {
        match self {
            Self::Playlist => 0,
            Self::Library => 1,
            Self::Track => 2,
            Self::Search => 3,
            Self::Help => 4,
        }
    }

    pub fn next(self) -> Self {
        Self::from_index((self.to_index() + 1) % 5)
    }

    pub fn prev(self) -> Self {
        Self::from_index((self.to_index() + 4) % 5)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputFocus {
    #[default]
    Normal,
    SearchQuery,
    LibraryFilter,
    SavePlaylistPrompt,
}

#[derive(Debug, Clone, Default)]
pub struct UiHitboxes {
    pub tabs: Vec<(Rect, ActiveView)>,
    pub list_area: Option<Rect>,
    pub transport_area: Option<Rect>,
    pub seek_bar: Option<Rect>,
    pub btn_prev: Option<Rect>,
    pub btn_play: Option<Rect>,
    pub btn_next: Option<Rect>,
    pub btn_shuffle: Option<Rect>,
    pub btn_repeat: Option<Rect>,
}

/// Complete interactive application coordinator.
pub struct App {
    pub config: AppConfig,
    pub paths: AppPaths,
    pub library: MusicLibrary,
    pub playlist: Playlist,
    pub audio: AudioEngine,
    pub mpris: MprisBridge,
    pub analyzer: SpectrumAnalyzer,
    pub telemetry: AudioTelemetry,

    pub view: ActiveView,
    pub focus: InputFocus,
    pub playlist_selected: usize,
    pub library_selected: usize,
    pub search_selected: usize,
    pub library_filter: String,
    pub search_query: String,
    pub save_playlist_input: String,
    pub pending_g_press: Option<Instant>,

    pub current_cover: Option<CoverArt>,
    pub current_art_url: Option<String>,
    pub current_lyrics: Lyrics,
    pub static_palette: ThemePalette,

    pub quit_on_stop: bool,
    pub exact_search: bool,
    pub should_quit: bool,

    pub hitboxes: UiHitboxes,
    pub is_scanning: bool,
    scan_rx: Option<Receiver<MusicLibrary>>,
    toast: Option<(String, Instant)>,

    next_token_counter: u64,
    gapless_token_target: Option<(u64, usize)>,
}

impl App {
    pub fn new(
        config: AppConfig,
        paths: AppPaths,
        library: MusicLibrary,
        audio: AudioEngine,
        mpris: MprisBridge,
        quit_on_stop: bool,
        exact_search: bool,
    ) -> Self {
        let static_palette = ThemePalette::by_name(&config.theme, Some(&paths.themes_dir));
        let view = if library.is_empty() {
            ActiveView::Help
        } else {
            ActiveView::Library
        };

        Self {
            config,
            paths,
            library,
            playlist: Playlist::new(),
            audio,
            mpris,
            analyzer: SpectrumAnalyzer::new(),
            telemetry: AudioTelemetry::default(),
            view,
            focus: InputFocus::Normal,
            playlist_selected: 0,
            library_selected: 0,
            search_selected: 0,
            library_filter: String::new(),
            search_query: String::new(),
            save_playlist_input: String::new(),
            pending_g_press: None,
            current_cover: None,
            current_art_url: None,
            current_lyrics: Lyrics::None,
            static_palette,
            quit_on_stop,
            exact_search,
            should_quit: false,
            hitboxes: UiHitboxes::default(),
            is_scanning: false,
            scan_rx: None,
            toast: None,
            next_token_counter: 1,
            gapless_token_target: None,
        }
    }

    pub fn active_palette(&self) -> ThemePalette {
        if self.config.dynamic_album_colors {
            if let Some(ref cov) = self.current_cover {
                return cov.extracted_palette;
            }
        }
        self.static_palette
    }

    pub fn switch_view(&mut self, next: ActiveView) {
        if self.view != next {
            if let Some(ref mut cov) = self.current_cover {
                cov.clear_kitty_if_active();
            }
            self.view = next;
            if next != ActiveView::Search && self.focus == InputFocus::SearchQuery {
                self.focus = InputFocus::Normal;
            }
            if next != ActiveView::Library && self.focus == InputFocus::LibraryFilter {
                self.focus = InputFocus::Normal;
            }
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn alloc_token(&mut self) -> u64 {
        let t = self.next_token_counter;
        self.next_token_counter = self.next_token_counter.wrapping_add(1).max(1);
        t
    }

    /// Starts playing `index` in `self.playlist` immediately and primes gapless next.
    pub fn start_playlist_index(&mut self, index: usize) {
        let Some(track) = self.playlist.select_index(index).cloned() else {
            return;
        };
        self.playlist_selected = index;
        let token = self.alloc_token();
        self.audio.send(AudioCommand::PlayTrack {
            path: track.path.clone(),
            queue_token: token,
        });
        self.refresh_media_assets_for_current(true);
        self.sync_gapless_next();
    }

    pub fn sync_gapless_next(&mut self) {
        if let Some(next_idx) = self.playlist.peek_natural_next_index() {
            if let Some(next_path) = self
                .playlist
                .tracks()
                .get(next_idx)
                .map(|t| t.path.clone())
            {
                let tok = self.alloc_token();
                self.gapless_token_target = Some((tok, next_idx));
                self.audio
                    .send(AudioCommand::SetGaplessNext(Some((next_path, tok))));
                return;
            }
        }
        self.gapless_token_target = None;
        self.audio.send(AudioCommand::SetGaplessNext(None));
    }

    fn refresh_media_assets_for_current(&mut self, notify: bool) {
        if let Some(ref mut old) = self.current_cover {
            old.clear_kitty_if_active();
        }
        let Some(track) = self.playlist.current_track().cloned() else {
            self.current_cover = None;
            self.current_art_url = None;
            self.current_lyrics = Lyrics::None;
            return;
        };

        if let Some(img) = load_cover_image_for_track(&track.path) {
            self.current_art_url =
                write_mpris_cover_cache(&img, &self.paths.cover_cache_dir, &track);
            self.current_cover = Some(CoverArt::from_image(img));
        } else {
            let proc_cov = CoverArt::procedural_for_track(&track);
            self.current_art_url =
                write_mpris_cover_cache(&proc_cov.image, &self.paths.cover_cache_dir, &track);
            self.current_cover = Some(proc_cov);
        }

        self.current_lyrics = Lyrics::load_for_track(&track.path);

        if notify && self.config.desktop_notifications {
            self.mpris.send_track_notification(
                &track.title,
                track.display_artist(),
                track.display_album(),
                self.current_art_url.as_deref(),
            );
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.telemetry.is_playing || self.telemetry.is_paused {
            self.audio.send(AudioCommand::TogglePause);
        } else if !self.playlist.is_empty() {
            let idx = self.playlist.current_index().unwrap_or(0);
            self.start_playlist_index(idx);
        }
    }

    pub fn stop_playback(&mut self) {
        self.audio.send(AudioCommand::Stop);
        self.gapless_token_target = None;
    }

    pub fn play_next_manual(&mut self) {
        if self.playlist.advance_next(true).is_some() {
            if let Some(idx) = self.playlist.current_index() {
                self.start_playlist_index(idx);
            }
        } else {
            self.stop_playback();
            if self.quit_on_stop {
                self.should_quit = true;
            }
        }
    }

    pub fn play_previous_or_restart(&mut self) {
        if self.telemetry.position_ms > 3_200 {
            self.audio.send(AudioCommand::SeekAbsoluteMs(0));
            return;
        }
        if self.playlist.advance_prev().is_some() {
            if let Some(idx) = self.playlist.current_index() {
                self.start_playlist_index(idx);
            }
        }
    }

    pub fn adjust_volume(&mut self, delta: i16) {
        let cur = self.config.volume as i16;
        let next = (cur + delta).clamp(0, 100) as u8;
        self.config.volume = next;
        self.audio.send(AudioCommand::SetVolume(next));
        if next > 0 && self.telemetry.muted {
            self.audio.send(AudioCommand::SetMuted(false));
        }
        let _ = self.config.save(&self.paths.config_file);
    }

    pub fn toggle_mute(&mut self) {
        let next_mute = !self.telemetry.muted;
        self.audio.send(AudioCommand::SetMuted(next_mute));
        self.set_toast(if next_mute { "Audio Muted" } else { "Audio Unmuted" });
    }

    pub fn toggle_shuffle(&mut self) {
        self.playlist.toggle_shuffle();
        if let Some(idx) = self.playlist.current_index() {
            self.playlist_selected = idx;
        }
        self.sync_gapless_next();
        self.set_toast(if self.playlist.shuffled {
            "Shuffle: ON"
        } else {
            "Shuffle: OFF (Order Restored)"
        });
    }

    pub fn cycle_repeat(&mut self) {
        self.playlist.repeat = self.playlist.repeat.next();
        self.sync_gapless_next();
        self.set_toast(self.playlist.repeat.badge());
    }

    pub fn trigger_background_rescan(&mut self) {
        if self.is_scanning {
            return;
        }
        self.is_scanning = true;
        self.set_toast("Scanning music library in background...");
        let root = self.config.music_dir.clone();
        let cache = self.paths.library_cache_file.clone();
        let tree_mode = self.library.tree_mode;
        let expanded = self.library.expanded_keys.clone();

        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        thread::spawn(move || {
            let mut scanned = MusicLibrary::scan_with_cache(&root, Some(&cache));
            scanned.tree_mode = tree_mode;
            for k in expanded {
                scanned.expanded_keys.insert(k);
            }
            let _ = tx.send(scanned);
        });
    }

    /// Periodic tick (~45 Hz): processes audio events, MPRIS commands, scanner completion, and FFT update.
    pub fn tick(&mut self) {
        self.telemetry = self.audio.snapshot();

        while let Some(ev) = self.audio.try_recv_event() {
            match ev {
                AudioEvent::TrackStarted { duration_ms, .. } => {
                    if duration_ms > 0 {
                        if let Some(idx) = self.playlist.current_index() {
                            if let Some( expected ) = self.playlist.tracks().get(idx) {
                                if expected.duration_ms == 0 {
                                    // Duration discovered dynamically at decode time
                                    self.telemetry.duration_ms = duration_ms;
                                }
                            }
                        }
                    }
                }
                AudioEvent::GaplessTransitioned { queue_token, .. } => {
                    if let Some((expected_tok, next_idx)) = self.gapless_token_target.take() {
                        if expected_tok == queue_token {
                            self.playlist.select_index(next_idx);
                            self.playlist_selected = next_idx;
                            self.refresh_media_assets_for_current(true);
                            self.sync_gapless_next();
                        }
                    }
                }
                AudioEvent::PlaybackFinished { queue_token } => {
                    if queue_token == self.telemetry.queue_token || self.telemetry.queue_token == 0 {
                        if self.playlist.advance_next(false).is_some() {
                            if let Some(idx) = self.playlist.current_index() {
                                self.start_playlist_index(idx);
                            }
                        } else if self.quit_on_stop {
                            self.should_quit = true;
                        }
                    }
                }
                AudioEvent::DecodeError { path, message } => {
                    let fname = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("file");
                    self.set_toast(format!("Skip unreadable '{}': {}", fname, message));
                    if self.playlist.len() > 1 && self.playlist.advance_next(true).is_some() {
                        if let Some(idx) = self.playlist.current_index() {
                            self.start_playlist_index(idx);
                        }
                    } else if self.quit_on_stop {
                        self.should_quit = true;
                    }
                }
            }
        }

        // Poll MPRIS2 media key / desktop applet commands
        while let Some(mcmd) = self.mpris.try_recv() {
            match mcmd {
                MprisCommand::Play => {
                    if self.telemetry.is_paused {
                        self.audio.send(AudioCommand::SetPaused(false));
                    } else if !self.telemetry.is_playing && !self.playlist.is_empty() {
                        let idx = self.playlist.current_index().unwrap_or(0);
                        self.start_playlist_index(idx);
                    }
                }
                MprisCommand::Pause => self.audio.send(AudioCommand::SetPaused(true)),
                MprisCommand::PlayPause => self.toggle_pause(),
                MprisCommand::Stop => self.stop_playback(),
                MprisCommand::Next => self.play_next_manual(),
                MprisCommand::Previous => self.play_previous_or_restart(),
                MprisCommand::SeekMicros(offset_us) => {
                    self.audio
                        .send(AudioCommand::SeekRelativeMs(offset_us / 1000));
                }
                MprisCommand::SetPositionMicros(pos_us) => {
                    if pos_us >= 0 {
                        self.audio
                            .send(AudioCommand::SeekAbsoluteMs((pos_us / 1000) as u64));
                    }
                }
                MprisCommand::SetVolume(vol_f) => {
                    let pct = (vol_f.clamp(0.0, 1.0) * 100.0).round() as u8;
                    self.config.volume = pct;
                    self.audio.send(AudioCommand::SetVolume(pct));
                }
                MprisCommand::SetShuffle(shuf) => {
                    if self.playlist.shuffled != shuf {
                        self.toggle_shuffle();
                    }
                }
                MprisCommand::SetLoopStatus(status) => {
                    self.playlist.repeat = match status.as_str() {
                        "Track" => RepeatMode::One,
                        "Playlist" => RepeatMode::All,
                        _ => RepeatMode::Off,
                    };
                    self.sync_gapless_next();
                }
                MprisCommand::Quit => self.should_quit = true,
            }
        }

        // Check background rescan completion
        if let Some(ref rx) = self.scan_rx {
            if let Ok(new_lib) = rx.try_recv() {
                let count = new_lib.len();
                self.library = new_lib;
                self.is_scanning = false;
                self.scan_rx = None;
                self.set_toast(format!("Library updated: {} tracks indexed", count));
            }
        }

        // Expire old toast notifications
        if let Some((_, ts)) = self.toast {
            if ts.elapsed() > Duration::from_millis(3400) {
                self.toast = None;
            }
        }

        // Update FFT spectrum analyzer
        if self.config.show_visualizer {
            let active = self.telemetry.is_playing && !self.telemetry.muted;
            self.analyzer
                .update(&self.audio.visualizer_tap, 72, active);
        }

        // Sync state back to MPRIS2
        self.publish_mpris_snapshot();
    }

    fn publish_mpris_snapshot(&mut self) {
        let playback_status = if self.telemetry.is_playing {
            "Playing"
        } else if self.telemetry.is_paused {
            "Paused"
        } else {
            "Stopped"
        };

        let (track_id, title, artist, album, dur_us) =
            if let Some(t) = self.playlist.current_track() {
                let idx = self.playlist.current_index().unwrap_or(0);
                let dur = if self.telemetry.duration_ms > 0 {
                    self.telemetry.duration_ms
                } else {
                    t.duration_ms
                };
                (
                    format!("/org/cadence/track/{}", idx),
                    t.title.clone(),
                    t.display_artist().to_string(),
                    t.display_album().to_string(),
                    (dur * 1000) as i64,
                )
            } else {
                (
                    "/org/cadence/track/none".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    0,
                )
            };

        self.mpris.update_state(MprisStateSnapshot {
            playback_status,
            loop_status: MprisStateSnapshot::loop_str(self.playlist.repeat),
            shuffle: self.playlist.shuffled,
            volume: self.config.volume as f64 / 100.0,
            position_us: (self.telemetry.position_ms * 1000) as i64,
            track_id,
            title,
            artist,
            album,
            duration_us: dur_us,
            art_url: self.current_art_url.clone(),
        });
    }

    pub fn save_session(&self) {
        let session = SessionState {
            queue_paths: self.playlist.tracks().iter().map(|t| t.path.clone()).collect(),
            current_index: self.playlist.current_index(),
            shuffle: self.playlist.shuffled,
            repeat: self.playlist.repeat,
            last_view_index: self.view.to_index(),
        };
        let _ = session.save(&self.paths.session_file);
        let _ = self.config.save(&self.paths.config_file);
    }

    pub fn restore_session(&mut self, session: &SessionState) {
        if session.queue_paths.is_empty() {
            return;
        }
        let tracks: Vec<_> = session
            .queue_paths
            .iter()
            .filter(|p| p.exists())
            .map(|p| self.library.resolve_or_extract(p))
            .collect();
        if !tracks.is_empty() {
            self.playlist.repeat = session.repeat;
            self.playlist.set_tracks(tracks, session.current_index, false);
            self.playlist.shuffled = session.shuffle;
            self.playlist_selected = self.playlist.current_index().unwrap_or(0);
            self.refresh_media_assets_for_current(false);
            self.view = ActiveView::from_index(session.last_view_index);
        }
    }

    /// Renders the entire terminal UI frame.
    pub fn render_frame(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 6 {
            Paragraph::new("Terminal too small")
                .render(area, buf);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Header & View Tabs
                Constraint::Min(3),    // Active View Body
                Constraint::Length(4), // Transport & Seek Bar
            ])
            .split(area);

        self.render_top_bar(chunks[0], buf);

        match self.view {
            ActiveView::Playlist => playlist_view::render_playlist_view(self, chunks[1], buf),
            ActiveView::Library => library_view::render_library_view(self, chunks[1], buf),
            ActiveView::Track => track_view::render_track_view(self, chunks[1], buf),
            ActiveView::Search => search_view::render_search_view(self, chunks[1], buf),
            ActiveView::Help => help_view::render_help_view(self, chunks[1], buf),
        }

        self.render_transport_bar(chunks[2], buf);

        if self.focus == InputFocus::SavePlaylistPrompt {
            self.render_save_playlist_modal(area, buf);
        }
    }

    fn render_top_bar(&mut self, area: Rect, buf: &mut Buffer) {
        let palette = self.active_palette();
        self.hitboxes.tabs.clear();

        let tabs = [
            (ActiveView::Playlist, " F2 Playlist "),
            (ActiveView::Library, " F3 Library "),
            (ActiveView::Track, " F4 Track "),
            (ActiveView::Search, " F5 Search "),
            (ActiveView::Help, " F6 Help "),
        ];

        let brand = " ♫ CADENCE  ";
        let mut x = area.x;
        let brand_span = Span::styled(
            brand,
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_span(x, area.y, &brand_span, area.width);
        x += brand.chars().count() as u16;

        for (v, label) in tabs {
            let w = label.chars().count() as u16;
            if x + w > area.x + area.width {
                break;
            }
            let active = self.view == v;
            let style = if active {
                Style::default()
                    .fg(Color::Rgb(16, 18, 26))
                    .bg(palette.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted)
            };
            buf.set_span(x, area.y, &Span::styled(label, style), w);
            self.hitboxes.tabs.push((Rect::new(x, area.y, w, 1), v));
            x += w + 1;
        }

        // Right-aligned toast or volume badge
        let right_text = if let Some((ref msg, _)) = self.toast {
            format!("✦ {} ", msg)
        } else {
            let vol_str = if self.telemetry.muted {
                "Muted".to_string()
            } else {
                format!("Vol {}%", self.config.volume)
            };
            format!("{} ", vol_str)
        };
        let rw = right_text.chars().count() as u16;
        if x + rw < area.x + area.width {
            let rx = area.x + area.width - rw;
            let rstyle = if self.toast.is_some() {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.secondary)
            };
            buf.set_span(rx, area.y, &Span::styled(right_text, rstyle), rw);
        }

        // Subtle separator line on row 2 of header
        let sep = "─".repeat(area.width as usize);
        buf.set_span(
            area.x,
            area.y + 1,
            &Span::styled(sep, Style::default().fg(palette.border)),
            area.width,
        );
    }

    fn render_transport_bar(&mut self, area: Rect, buf: &mut Buffer) {
        let palette = self.active_palette();
        self.hitboxes.transport_area = Some(area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 2 || inner.width < 16 {
            return;
        }

        let top_row = inner.y;
        let bot_row = inner.y + 1;

        // Top row: Transport buttons (left) + Active Song Title — Artist (center) + Mode Badges (right)
        let mut cx = inner.x;
        let prev_lbl = " ⏮ ";
        let play_lbl = if self.telemetry.is_playing {
            " ⏸ "
        } else {
            " ▶ "
        };
        let next_lbl = " ⏭ ";

        buf.set_span(
            cx,
            top_row,
            &Span::styled(prev_lbl, Style::default().fg(palette.secondary)),
            3,
        );
        self.hitboxes.btn_prev = Some(Rect::new(cx, top_row, 3, 1));
        cx += 3;

        buf.set_span(
            cx,
            top_row,
            &Span::styled(
                play_lbl,
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            3,
        );
        self.hitboxes.btn_play = Some(Rect::new(cx, top_row, 3, 1));
        cx += 3;

        buf.set_span(
            cx,
            top_row,
            &Span::styled(next_lbl, Style::default().fg(palette.secondary)),
            3,
        );
        self.hitboxes.btn_next = Some(Rect::new(cx, top_row, 3, 1));
        cx += 4;

        let shuf_lbl = if self.playlist.shuffled {
            "[Shuffle:ON]"
        } else {
            "[Shuffle:OFF]"
        };
        let rep_lbl = match self.playlist.repeat {
            RepeatMode::Off => "[Repeat:OFF]",
            RepeatMode::One => "[Repeat:ONE]",
            RepeatMode::All => "[Repeat:ALL]",
        };
        let right_total = (shuf_lbl.len() + 1 + rep_lbl.len() + 1) as u16;
        let right_start = (inner.x + inner.width).saturating_sub(right_total);

        if right_start > cx + 8 {
            let max_song_w = (right_start - cx - 1) as usize;
            let now_str = if let Some(t) = self.playlist.current_track() {
                format!("{} — {}", t.title, t.display_artist())
            } else {
                "Stopped".to_string()
            };
            let truncated: String = now_str.chars().take(max_song_w).collect();
            buf.set_span(
                cx,
                top_row,
                &Span::styled(
                    truncated,
                    Style::default()
                        .fg(palette.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                max_song_w as u16,
            );
        }

        if right_start >= inner.x {
            let sw = shuf_lbl.len() as u16;
            let shuf_style = if self.playlist.shuffled {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted)
            };
            buf.set_span(right_start, top_row, &Span::styled(shuf_lbl, shuf_style), sw);
            self.hitboxes.btn_shuffle = Some(Rect::new(right_start, top_row, sw, 1));

            let rx = right_start + sw + 1;
            let rw = rep_lbl.len() as u16;
            let rep_style = if self.playlist.repeat != RepeatMode::Off {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted)
            };
            buf.set_span(rx, top_row, &Span::styled(rep_lbl, rep_style), rw);
            self.hitboxes.btn_repeat = Some(Rect::new(rx, top_row, rw, 1));
        }

        // Bottom row: ` 01:24 ━━━━━━━━●────────── 04:12 ` (clickable smooth sub-cell progress bar!)
        let dur_ms = if self.telemetry.duration_ms > 0 {
            self.telemetry.duration_ms
        } else {
            self.playlist
                .current_track()
                .map(|t| t.duration_ms)
                .unwrap_or(0)
        };
        let pos_ms = self.telemetry.position_ms.min(dur_ms.max(self.telemetry.position_ms));
        let left_t = format!(" {} ", format_duration_ms(pos_ms));
        let right_t = format!(" {} ", format_duration_ms(dur_ms));
        let lw = left_t.len() as u16;
        let rw = right_t.len() as u16;

        buf.set_span(
            inner.x,
            bot_row,
            &Span::styled(left_t, Style::default().fg(palette.muted)),
            lw,
        );
        if inner.width > lw + rw + 4 {
            let bar_x = inner.x + lw;
            let bar_w = inner.width - lw - rw;
            self.hitboxes.seek_bar = Some(Rect::new(bar_x, bot_row, bar_w, 1));

            let ratio = if dur_ms > 0 {
                (pos_ms as f64 / dur_ms as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let filled = (ratio * bar_w as f64).round() as u16;

            for i in 0..bar_w {
                let (sym, st) = if dur_ms > 0 && i == filled.min(bar_w.saturating_sub(1)) {
                    ("●", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD))
                } else if i < filled {
                    ("━", Style::default().fg(palette.primary))
                } else {
                    ("─", Style::default().fg(palette.border))
                };
                if let Some(cell) = buf.cell_mut((bar_x + i, bot_row)) {
                    cell.set_symbol(sym);
                    cell.set_style(st);
                }
            }

            buf.set_span(
                bar_x + bar_w,
                bot_row,
                &Span::styled(right_t, Style::default().fg(palette.muted)),
                rw,
            );
        }
    }

    fn render_save_playlist_modal(&self, area: Rect, buf: &mut Buffer) {
        let palette = self.active_palette();
        let w = 52u16.min(area.width.saturating_sub(4));
        let h = 5u16.min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let modal = Rect::new(x, y, w, h);

        Clear.render(modal, buf);
        let b = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.primary))
            .title(Span::styled(
                " Save Queue as .m3u8 Playlist (Enter to save, Esc to cancel) ",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  Filename: {}█", self.save_playlist_input),
                Style::default().fg(palette.fg),
            )),
        ])
        .alignment(Alignment::Left)
        .block(b)
        .render(modal, buf);
    }
}
