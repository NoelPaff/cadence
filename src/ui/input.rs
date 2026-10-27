use crate::audio::AudioCommand;
use crate::media::palette::ThemePalette;
use crate::model::{LibraryNodeKind, Playlist};
use crate::ui::{ActiveView, App, InputFocus};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::{Duration, Instant};

pub fn handle_terminal_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(app, key),
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        Event::Resize(_, _) => {
            if let Some(ref mut cov) = app.current_cover {
                cov.clear_kitty_if_active();
            }
        }
        _ => {}
    }
}

fn handle_key_event(app: &mut App, key: KeyEvent) {
    // Ctrl+C always exits cleanly
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    // Function keys F2..F6 ALWAYS switch views immediately even from search/filter inputs
    match key.code {
        KeyCode::F(2) => {
            app.switch_view(ActiveView::Playlist);
            return;
        }
        KeyCode::F(3) => {
            app.switch_view(ActiveView::Library);
            return;
        }
        KeyCode::F(4) => {
            app.switch_view(ActiveView::Track);
            return;
        }
        KeyCode::F(5) => {
            app.switch_view(ActiveView::Search);
            app.focus = InputFocus::SearchQuery;
            return;
        }
        KeyCode::F(6) => {
            app.switch_view(ActiveView::Help);
            return;
        }
        _ => {}
    }

    // Modal text-input handling (Search box, Library filter, Save Playlist prompt)
    match app.focus {
        InputFocus::SearchQuery => {
            handle_search_input_keys(app, key);
            return;
        }
        InputFocus::LibraryFilter => {
            handle_library_filter_keys(app, key);
            return;
        }
        InputFocus::SavePlaylistPrompt => {
            handle_save_playlist_keys(app, key);
            return;
        }
        InputFocus::Normal => {}
    }

    // Check `gg` two-key chord
    if key.code == KeyCode::Char('g') && key.modifiers.is_empty() {
        if let Some(prev) = app.pending_g_press.take() {
            if prev.elapsed() <= Duration::from_millis(550) {
                jump_selection_top(app);
                return;
            }
        }
        app.pending_g_press = Some(Instant::now());
        return;
    } else {
        app.pending_g_press = None;
    }

    // Global & view-specific shortcuts in Normal mode
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.view == ActiveView::Library && !app.library_filter.is_empty() {
                app.library_filter.clear();
                app.library_selected = 0;
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::Char('1') => app.switch_view(ActiveView::Playlist),
        KeyCode::Char('2') => app.switch_view(ActiveView::Library),
        KeyCode::Char('3') => app.switch_view(ActiveView::Track),
        KeyCode::Char('4') => {
            app.switch_view(ActiveView::Search);
            app.focus = InputFocus::SearchQuery;
        }
        KeyCode::Char('5') | KeyCode::Char('?') => app.switch_view(ActiveView::Help),
        KeyCode::Tab => app.switch_view(app.view.next()),
        KeyCode::BackTab => app.switch_view(app.view.prev()),

        // Transport: Play / Pause
        KeyCode::Char(' ') | KeyCode::Char('p') => app.toggle_pause(),

        // Global Previous / Next track
        KeyCode::Char('[') => app.play_previous_or_restart(),
        KeyCode::Char(']') => app.play_next_manual(),

        // Seek -5s / +5s via Shift+Arrows or `<` / `>` / `,` / `;`
        KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.audio.send(AudioCommand::SeekRelativeMs(-5_000));
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.audio.send(AudioCommand::SeekRelativeMs(5_000));
        }
        KeyCode::Char('<') | KeyCode::Char(',') => {
            app.audio.send(AudioCommand::SeekRelativeMs(-5_000));
        }
        KeyCode::Char('>') | KeyCode::Char(';') => {
            app.audio.send(AudioCommand::SeekRelativeMs(5_000));
        }

        // Left / Right / h / l: Expand/Collapse in Library View, Previous/Next track in other views
        KeyCode::Left | KeyCode::Char('h') => {
            if app.view == ActiveView::Library {
                library_collapse_or_parent(app);
            } else {
                app.play_previous_or_restart();
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if app.view == ActiveView::Library {
                library_expand_or_play(app);
            } else {
                app.play_next_manual();
            }
        }

        // Volume & Mute
        KeyCode::Char('+') | KeyCode::Char('=') => app.adjust_volume(5),
        KeyCode::Char('-') | KeyCode::Char('_') => app.adjust_volume(-5),
        KeyCode::Char('m') => app.toggle_mute(),

        // Shuffle & Repeat
        KeyCode::Char('s') => app.toggle_shuffle(),
        KeyCode::Char('r') => app.cycle_repeat(),

        // Visuals, Themes & Lyrics
        KeyCode::Char('v') => {
            app.config.show_visualizer = !app.config.show_visualizer;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!(
                "Visualizer: {}",
                if app.config.show_visualizer { "ON" } else { "OFF" }
            ));
        }
        KeyCode::Char('V') => {
            app.config.visualizer_style = app.config.visualizer_style.next();
            app.config.show_visualizer = true;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!("Visualizer: {}", app.config.visualizer_style.label()));
        }
        KeyCode::Char('i') => {
            app.config.dynamic_album_colors = !app.config.dynamic_album_colors;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!(
                "Palette: {}",
                if app.config.dynamic_album_colors {
                    "Dynamic Album Cover"
                } else {
                    &app.config.theme
                }
            ));
        }
        KeyCode::Char('t') => {
            let next_name = ThemePalette::next_theme_name(&app.config.theme).to_string();
            app.config.theme = next_name.clone();
            app.config.dynamic_album_colors = false;
            app.static_palette =
                ThemePalette::by_name(&app.config.theme, Some(&app.paths.themes_dir));
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!("Theme: {}", next_name));
        }
        KeyCode::Char('b') => {
            if let Some(ref mut cov) = app.current_cover {
                cov.clear_kitty_if_active();
            }
            app.config.cover_mode = app.config.cover_mode.next();
            app.config.show_cover = true;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!("Cover Mode: {}", app.config.cover_mode.label()));
        }
        KeyCode::Char('L') => {
            app.config.show_lyrics = !app.config.show_lyrics;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!(
                "Lyrics Panel: {}",
                if app.config.show_lyrics { "ON" } else { "OFF" }
            ));
        }
        KeyCode::Char('N') => {
            app.config.desktop_notifications = !app.config.desktop_notifications;
            let _ = app.config.save(&app.paths.config_file);
            app.set_toast(format!(
                "Desktop Notifications: {}",
                if app.config.desktop_notifications {
                    "ON"
                } else {
                    "OFF"
                }
            ));
        }

        // Rescan library
        KeyCode::Char('u') => app.trigger_background_rescan(),

        // Library Tree Mode toggle
        KeyCode::Char('M') => {
            app.library.tree_mode = app.library.tree_mode.toggle();
            app.library_selected = 0;
            app.set_toast(format!("Library Mode: {}", app.library.tree_mode.label()));
        }

        // Quick-add to persistent default playlist (`cadence.m3u8`)
        KeyCode::Char('.') => save_target_to_default_playlist(app),

        // Export playlist prompt
        KeyCode::Char('w') if app.view == ActiveView::Playlist && !app.playlist.is_empty() => {
            app.save_playlist_input = "mix.m3u8".to_string();
            app.focus = InputFocus::SavePlaylistPrompt;
        }

        // Filter / Search focus
        KeyCode::Char('/') => match app.view {
            ActiveView::Library => app.focus = InputFocus::LibraryFilter,
            ActiveView::Search => app.focus = InputFocus::SearchQuery,
            _ => {
                app.switch_view(ActiveView::Search);
                app.focus = InputFocus::SearchQuery;
            }
        },

        // Reorder in Playlist View (placed before generic Up/Down so Alt+Up/Down matches first)
        KeyCode::Char('K')
            | KeyCode::Up
            if app.view == ActiveView::Playlist
                && (key.code == KeyCode::Char('K')
                    || key.modifiers.contains(KeyModifiers::ALT)) =>
        {
            if app.playlist_selected > 0
                && app
                    .playlist
                    .move_item(app.playlist_selected, app.playlist_selected - 1)
            {
                app.playlist_selected -= 1;
                app.sync_gapless_next();
            }
        }
        KeyCode::Char('J')
            | KeyCode::Down
            if app.view == ActiveView::Playlist
                && (key.code == KeyCode::Char('J')
                    || key.modifiers.contains(KeyModifiers::ALT)) =>
        {
            if app.playlist_selected + 1 < app.playlist.len()
                && app
                    .playlist
                    .move_item(app.playlist_selected, app.playlist_selected + 1)
            {
                app.playlist_selected += 1;
                app.sync_gapless_next();
            }
        }

        // Vertical list navigation
        KeyCode::Up | KeyCode::Char('k') => move_selection(app, -1),
        KeyCode::Down | KeyCode::Char('j') => move_selection(app, 1),
        KeyCode::PageUp => move_selection(app, -12),
        KeyCode::PageDown => move_selection(app, 12),
        KeyCode::Home => jump_selection_top(app),
        KeyCode::End | KeyCode::Char('G') => jump_selection_bottom(app),

        // Remove / Clear in Playlist View
        KeyCode::Char('d') | KeyCode::Delete if app.view == ActiveView::Playlist => {
            if !app.playlist.is_empty() {
                let removed_active = app.playlist.remove_at(app.playlist_selected);
                if app.playlist_selected >= app.playlist.len() && !app.playlist.is_empty() {
                    app.playlist_selected = app.playlist.len() - 1;
                }
                if removed_active {
                    if app.playlist.is_empty() {
                        app.stop_playback();
                    } else if let Some(idx) = app.playlist.current_index() {
                        app.start_playlist_index(idx);
                    }
                } else {
                    app.sync_gapless_next();
                }
            }
        }
        KeyCode::Char('C') if app.view == ActiveView::Playlist => {
            app.playlist.clear();
            app.playlist_selected = 0;
            app.stop_playback();
            app.set_toast("Cleared playlist queue");
        }

        // Enqueue / Play Next in Library & Search views
        KeyCode::Char('e') | KeyCode::Char('a') => enqueue_selected_item(app, false),
        KeyCode::Char('n') => enqueue_selected_item(app, true),

        // Enter action
        KeyCode::Enter => activate_selected_item(app),

        _ => {}
    }
}

fn handle_search_input_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.focus = InputFocus::Normal;
        }
        KeyCode::Up => move_selection(app, -1),
        KeyCode::Down => move_selection(app, 1),
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            move_selection(app, -1)
        }
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            move_selection(app, 1)
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            enqueue_selected_item(app, false);
        }
        KeyCode::Tab => {
            enqueue_selected_item(app, false);
        }
        KeyCode::Enter => {
            activate_selected_item(app);
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            app.search_selected = 0;
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.search_query.clear();
            app.search_selected = 0;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.search_query.push(c);
            app.search_selected = 0;
        }
        _ => {}
    }
}

fn handle_library_filter_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.library_filter.clear();
            app.library_selected = 0;
            app.focus = InputFocus::Normal;
        }
        KeyCode::Enter => {
            app.focus = InputFocus::Normal;
        }
        KeyCode::Up => move_selection(app, -1),
        KeyCode::Down => move_selection(app, 1),
        KeyCode::Backspace => {
            app.library_filter.pop();
            app.library_selected = 0;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.library_filter.push(c);
            app.library_selected = 0;
        }
        _ => {}
    }
}

fn handle_save_playlist_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.focus = InputFocus::Normal;
        }
        KeyCode::Backspace => {
            app.save_playlist_input.pop();
        }
        KeyCode::Enter => {
            let mut fname = app.save_playlist_input.trim().to_string();
            if !fname.is_empty() {
                if !fname.ends_with(".m3u") && !fname.ends_with(".m3u8") {
                    fname.push_str(".m3u8");
                }
                let target = if app.config.music_dir.is_dir() {
                    app.config.music_dir.join(&fname)
                } else {
                    app.paths.playlists_dir.join(&fname)
                };
                match app.playlist.save_m3u(&target) {
                    Ok(()) => app.set_toast(format!("Saved playlist to {}", target.display())),
                    Err(e) => app.set_toast(format!("Save failed: {}", e)),
                }
            }
            app.focus = InputFocus::Normal;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.save_playlist_input.push(c);
        }
        _ => {}
    }
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    let col = mouse.column;
    let row = mouse.row;

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if let Some(bar) = app.hitboxes.transport_area {
                if contains(bar, col, row) {
                    app.adjust_volume(3);
                    return;
                }
            }
            move_selection(app, -3);
        }
        MouseEventKind::ScrollDown => {
            if let Some(bar) = app.hitboxes.transport_area {
                if contains(bar, col, row) {
                    app.adjust_volume(-3);
                    return;
                }
            }
            move_selection(app, 3);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // 1. Check top view tabs
            for (tab_rect, view) in app.hitboxes.tabs.clone() {
                if contains(tab_rect, col, row) {
                    app.switch_view(view);
                    return;
                }
            }
            // 2. Check seek progress bar
            if let Some(seek_rect) = app.hitboxes.seek_bar {
                if contains(seek_rect, col, row) && seek_rect.width > 1 && app.telemetry.duration_ms > 0 {
                    let ratio = ((col.saturating_sub(seek_rect.x)) as f64)
                        / (seek_rect.width as f64);
                    let target_ms = (ratio.clamp(0.0, 1.0) * app.telemetry.duration_ms as f64) as u64;
                    app.audio.send(AudioCommand::SeekAbsoluteMs(target_ms));
                    return;
                }
            }
            // 3. Check transport buttons
            if let Some(r) = app.hitboxes.btn_prev {
                if contains(r, col, row) {
                    app.play_previous_or_restart();
                    return;
                }
            }
            if let Some(r) = app.hitboxes.btn_play {
                if contains(r, col, row) {
                    app.toggle_pause();
                    return;
                }
            }
            if let Some(r) = app.hitboxes.btn_next {
                if contains(r, col, row) {
                    app.play_next_manual();
                    return;
                }
            }
            if let Some(r) = app.hitboxes.btn_shuffle {
                if contains(r, col, row) {
                    app.toggle_shuffle();
                    return;
                }
            }
            if let Some(r) = app.hitboxes.btn_repeat {
                if contains(r, col, row) {
                    app.cycle_repeat();
                    return;
                }
            }
            // 4. Check list row click
            if let Some(list_rect) = app.hitboxes.list_area {
                if contains(list_rect, col, row) {
                    let rel_row = (row - list_rect.y) as usize;
                    match app.view {
                        ActiveView::Playlist if rel_row < app.playlist.len() => {
                            if app.playlist_selected == rel_row {
                                app.start_playlist_index(rel_row);
                            } else {
                                app.playlist_selected = rel_row;
                            }
                        }
                        ActiveView::Library => {
                            let items = app.library.visible_tree_items(&app.library_filter);
                            if rel_row < items.len() {
                                if app.library_selected == rel_row {
                                    activate_selected_item(app);
                                } else {
                                    app.library_selected = rel_row;
                                }
                            }
                        }
                        ActiveView::Search => {
                            let results = app
                                .library
                                .search_tracks(&app.search_query, app.exact_search);
                            if rel_row < results.len() {
                                if app.search_selected == rel_row {
                                    activate_selected_item(app);
                                } else {
                                    app.search_selected = rel_row;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
}

fn contains(r: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

fn move_selection(app: &mut App, delta: isize) {
    match app.view {
        ActiveView::Playlist => {
            let len = app.playlist.len();
            if len > 0 {
                app.playlist_selected = step_idx(app.playlist_selected, delta, len);
            }
        }
        ActiveView::Library => {
            let len = app.library.visible_tree_items(&app.library_filter).len();
            if len > 0 {
                app.library_selected = step_idx(app.library_selected, delta, len);
            }
        }
        ActiveView::Search => {
            let len = app
                .library
                .search_tracks(&app.search_query, app.exact_search)
                .len();
            if len > 0 {
                app.search_selected = step_idx(app.search_selected, delta, len);
            }
        }
        _ => {}
    }
}

fn jump_selection_top(app: &mut App) {
    match app.view {
        ActiveView::Playlist => app.playlist_selected = 0,
        ActiveView::Library => app.library_selected = 0,
        ActiveView::Search => app.search_selected = 0,
        _ => {}
    }
}

fn jump_selection_bottom(app: &mut App) {
    match app.view {
        ActiveView::Playlist => {
            app.playlist_selected = app.playlist.len().saturating_sub(1);
        }
        ActiveView::Library => {
            let len = app.library.visible_tree_items(&app.library_filter).len();
            app.library_selected = len.saturating_sub(1);
        }
        ActiveView::Search => {
            let len = app
                .library
                .search_tracks(&app.search_query, app.exact_search)
                .len();
            app.search_selected = len.saturating_sub(1);
        }
        _ => {}
    }
}

fn step_idx(cur: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let next = cur as isize + delta;
    next.clamp(0, len as isize - 1) as usize
}

fn library_expand_or_play(app: &mut App) {
    let items = app.library.visible_tree_items(&app.library_filter);
    let Some(item) = items.get(app.library_selected) else {
        return;
    };
    if item.is_container {
        if !app.library.expanded_keys.contains(&item.key) {
            app.library.expanded_keys.insert(item.key.clone());
        } else if app.library_selected + 1 < items.len() {
            app.library_selected += 1;
        }
    } else {
        activate_selected_item(app);
    }
}

fn library_collapse_or_parent(app: &mut App) {
    let items = app.library.visible_tree_items(&app.library_filter);
    let Some(item) = items.get(app.library_selected) else {
        return;
    };
    if item.is_container && app.library.expanded_keys.contains(&item.key) {
        app.library.expanded_keys.remove(&item.key);
    } else if item.depth > 0 {
        // Walk upward to enclosing container
        for prev in (0..app.library_selected).rev() {
            if items[prev].depth < item.depth && items[prev].is_container {
                let parent_key = items[prev].key.clone();
                app.library.expanded_keys.remove(&parent_key);
                app.library_selected = prev;
                break;
            }
        }
    }
}

fn activate_selected_item(app: &mut App) {
    match app.view {
        ActiveView::Playlist => {
            if app.playlist_selected < app.playlist.len() {
                app.start_playlist_index(app.playlist_selected);
            }
        }
        ActiveView::Library => {
            let items = app.library.visible_tree_items(&app.library_filter);
            if let Some(item) = items.get(app.library_selected) {
                let tracks = app.library.tracks_for_node(&item.kind);
                if !tracks.is_empty() {
                    let count = tracks.len();
                    let label = item.display_name.clone();
                    let shuf = app.playlist.shuffled && count > 1;
                    app.playlist.set_tracks(tracks, Some(0), shuf);
                    app.playlist_selected = 0;
                    app.start_playlist_index(0);
                    app.set_toast(format!("Playing '{}' ({} tracks)", label, count));
                }
            }
        }
        ActiveView::Search => {
            let results = app
                .library
                .search_tracks(&app.search_query, app.exact_search);
            if let Some(selected_track) = results.get(app.search_selected).cloned() {
                let title = selected_track.title.clone();
                app.playlist.set_tracks(vec![selected_track], Some(0), false);
                app.playlist_selected = 0;
                app.start_playlist_index(0);
                app.focus = InputFocus::Normal;
                app.set_toast(format!("Playing '{}'", title));
            }
        }
        _ => {}
    }
}

fn enqueue_selected_item(app: &mut App, play_next: bool) {
    match app.view {
        ActiveView::Library => {
            let items = app.library.visible_tree_items(&app.library_filter);
            if let Some(item) = items.get(app.library_selected) {
                let tracks = app.library.tracks_for_node(&item.kind);
                if tracks.is_empty() {
                    return;
                }
                let was_idle = app.playlist.is_empty();
                let count = tracks.len();
                let label = item.display_name.clone();
                if play_next && count == 1 {
                    app.playlist.enqueue_next(tracks.into_iter().next().unwrap());
                    app.set_toast(format!("Playing next: '{}'", label));
                } else {
                    app.playlist.enqueue(tracks);
                    app.set_toast(format!("Enqueued '{}' ({} tracks)", label, count));
                }
                if was_idle {
                    app.start_playlist_index(0);
                } else {
                    app.sync_gapless_next();
                }
            }
        }
        ActiveView::Search => {
            let results = app
                .library
                .search_tracks(&app.search_query, app.exact_search);
            if let Some(t) = results.get(app.search_selected).cloned() {
                let was_idle = app.playlist.is_empty();
                let title = t.title.clone();
                if play_next {
                    app.playlist.enqueue_next(t);
                    app.set_toast(format!("Playing next: '{}'", title));
                } else {
                    app.playlist.enqueue(vec![t]);
                    app.set_toast(format!("Enqueued '{}'", title));
                }
                if was_idle {
                    app.start_playlist_index(0);
                } else {
                    app.sync_gapless_next();
                }
            }
        }
        _ => {}
    }
}

fn save_target_to_default_playlist(app: &mut App) {
    let candidate = match app.view {
        ActiveView::Playlist => app
            .playlist
            .tracks()
            .get(app.playlist_selected)
            .or_else(|| app.playlist.current_track())
            .cloned(),
        ActiveView::Library => {
            let items = app.library.visible_tree_items(&app.library_filter);
            items
                .get(app.library_selected)
                .and_then(|it| match &it.kind {
                    LibraryNodeKind::Track(t) => Some(t.clone()),
                    _ => None,
                })
                .or_else(|| app.playlist.current_track().cloned())
        }
        ActiveView::Search => {
            let res = app
                .library
                .search_tracks(&app.search_query, app.exact_search);
            res.get(app.search_selected)
                .cloned()
                .or_else(|| app.playlist.current_track().cloned())
        }
        _ => app.playlist.current_track().cloned(),
    };

    let Some(track) = candidate else {
        app.set_toast("No track selected to save");
        return;
    };

    let m3u_path = app.config.default_playlist_path(&app.paths);
    match Playlist::append_track_to_m3u(&track, &m3u_path) {
        Ok(true) => app.set_toast(format!(
            "Saved '{}' to {}",
            track.title, app.config.default_playlist_name
        )),
        Ok(false) => app.set_toast(format!(
            "'{}' is already in {}",
            track.title, app.config.default_playlist_name
        )),
        Err(e) => app.set_toast(format!("Playlist save error: {}", e)),
    }
}
