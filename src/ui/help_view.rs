use crate::ui::App;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget, Wrap};

pub fn render_help_view(app: &mut App, area: Rect, buf: &mut Buffer) {
    if let Some(ref mut cov) = app.current_cover {
        cov.clear_kitty_if_active();
    }
    app.hitboxes.list_area = None;
    let palette = app.active_palette();

    let split_horiz = area.width >= 90;
    let (keys_area, diag_area) = if split_horiz {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let shortcuts: &[(&str, &str)] = &[
        ("F2..F6 / 1..5", "Switch Views (Playlist, Library, Track, Search, Help)"),
        ("Space / p", "Pause / Resume playback"),
        ("← / → or h / l", "Previous / Next track (Expand/Collapse in Library)"),
        ("[ / ]", "Previous / Next track globally from any view"),
        ("Shift+← / Shift+→", "Seek backward -5s / forward +5s (< / > also work)"),
        ("+ / = and -", "Adjust volume up / down (5% steps), m to Mute"),
        ("s", "Toggle reversible Shuffle"),
        ("r", "Cycle Repeat mode (Off → One → All)"),
        ("v / V", "Toggle Spectrum Visualizer / Cycle Visualizer Style"),
        ("i", "Toggle Dynamic Album Cover Colors vs. Named Theme"),
        ("t", "Cycle Color Themes (midnight, catppuccin, gruvbox...)"),
        ("b", "Cycle Cover Render Mode (Blocks → ASCII → Braille → Kitty)"),
        ("L", "Toggle Synced Lyrics (.lrc) panel in Track View"),
        ("M", "Switch Library between Folder Tree & Artist/Album Tree"),
        ("u", "Rescan Music Library in background"),
        (". (Period)", "Add playing/selected song to default cadence.m3u8"),
        ("e / a, n", "Append item to Queue (e/a) or Play Next (n)"),
        ("J / K, d, C", "Move track down/up, Remove track (d), Clear queue (C)"),
        ("w", "Export active queue to a saved .m3u8 playlist"),
        ("gg / G", "Jump to first / last item in current view"),
        ("/ , Esc, q", "Focus filter/search (/), Leave input / Quit (Esc, q)"),
    ];

    let rows: Vec<Row> = shortcuts
        .iter()
        .map(|(k, desc)| {
            Row::new(vec![
                Cell::from(*k).style(
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(*desc).style(Style::default().fg(palette.fg)),
            ])
        })
        .collect();

    let keys_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border_focused))
        .title(Span::styled(
            " Keybindings & Controls ",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ));

    let table = Table::new(rows, [Constraint::Length(20), Constraint::Min(28)]).block(keys_block);
    Widget::render(table, keys_area, buf);

    if let Some(da) = diag_area {
        let diag_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border))
            .title(Span::styled(
                " Engine & Configuration ",
                Style::default()
                    .fg(palette.secondary)
                    .add_modifier(Modifier::BOLD),
            ));

        let mpris_str = if app.mpris.is_active() {
            "Active (org.mpris.MediaPlayer2.cadence)"
        } else {
            "Disabled / No Session Bus"
        };
        let notif_str = if app.config.desktop_notifications {
            "Enabled (press N to toggle)"
        } else {
            "Disabled (press N to toggle)"
        };

        let info_lines = vec![
            Line::from(""),
            kv_line("Audio Sink:    ", &app.telemetry.backend_name, &palette),
            kv_line("MPRIS2 D-Bus:  ", mpris_str, &palette),
            kv_line("Notifications: ", notif_str, &palette),
            kv_line(
                "Library Path:  ",
                &app.library.root.display().to_string(),
                &palette,
            ),
            kv_line(
                "Indexed Songs: ",
                &format!("{} tracks", app.library.len()),
                &palette,
            ),
            kv_line("Theme:         ", &app.config.theme, &palette),
            kv_line(
                "Album Palette: ",
                if app.config.dynamic_album_colors {
                    "ON (press i to toggle)"
                } else {
                    "OFF (press i to toggle)"
                },
                &palette,
            ),
            kv_line("Cover Mode:    ", app.config.cover_mode.label(), &palette),
            kv_line(
                "Visualizer:    ",
                app.config.visualizer_style.label(),
                &palette,
            ),
            kv_line(
                "Default .m3u8: ",
                &app.config
                    .default_playlist_path(&app.paths)
                    .display()
                    .to_string(),
                &palette,
            ),
            kv_line(
                "Config File:   ",
                &app.paths.config_file.display().to_string(),
                &palette,
            ),
            Line::from(""),
            Line::from(Span::styled(
                " CLI Quick Examples:",
                Style::default()
                    .fg(palette.secondary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "   cadence <artist/album/song>",
                Style::default().fg(palette.muted),
            )),
            Line::from(Span::styled(
                "   cadence artist:album:track",
                Style::default().fg(palette.muted),
            )),
            Line::from(Span::styled(
                "   cadence all | cadence albums",
                Style::default().fg(palette.muted),
            )),
            Line::from(Span::styled(
                "   cadence dir <folder> | cadence song <title>",
                Style::default().fg(palette.muted),
            )),
        ];

        Paragraph::new(info_lines)
            .wrap(Wrap { trim: false })
            .block(diag_block)
            .render(da, buf);
    }
}

fn kv_line<'a>(
    key: &'a str,
    val: &str,
    palette: &crate::media::palette::ThemePalette,
) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!(" {}", key), Style::default().fg(palette.muted)),
        Span::styled(val.to_string(), Style::default().fg(palette.fg)),
    ])
}
