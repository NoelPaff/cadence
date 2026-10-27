use crate::ui::App;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget};

pub fn render_playlist_view(app: &mut App, area: Rect, buf: &mut Buffer) {
    let palette = app.active_palette();
    let show_side_panel = area.width >= 92 && (app.config.show_cover || app.config.show_visualizer);

    let (list_area, side_area) = if show_side_panel {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(63), Constraint::Percentage(37)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    app.hitboxes.list_area = Some(inner_rect(list_area));

    let total_dur = crate::model::track::format_duration_ms(app.playlist.total_duration_ms());
    let title = format!(
        " Playlist ({} tracks • {}) ",
        app.playlist.len(),
        total_dur
    );
    let hint = " Enter:Play  J/K:Move  d:Del  .:Fav  w:Save ";

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border_focused))
        .title(Span::styled(
            title,
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(palette.muted),
        )));

    if app.playlist.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Queue is empty.",
                Style::default()
                    .fg(palette.fg)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  • Press F3 (or 2) to browse your Music Library",
                Style::default().fg(palette.muted),
            )),
            Line::from(Span::styled(
                "  • Press F5 (or 4) to search tracks, albums, and artists",
                Style::default().fg(palette.muted),
            )),
            Line::from(Span::styled(
                "  • Or launch from CLI: cadence <artist/album/song> | cadence all",
                Style::default().fg(palette.secondary),
            )),
        ];
        Paragraph::new(empty_msg).block(block).render(list_area, buf);
    } else {
        let active_idx = app.playlist.current_index();
        let is_paused = app.telemetry.is_paused;
        let show_album_col = list_area.width >= 72;

        let header_cells = if show_album_col {
            vec!["#", "Title", "Artist", "Album", "Time"]
        } else {
            vec!["#", "Title", "Artist", "Time"]
        };
        let header = Row::new(
            header_cells
                .into_iter()
                .map(|h| Cell::from(h).style(Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD))),
        )
        .height(1);

        let rows: Vec<Row> = app
            .playlist
            .tracks()
            .iter()
            .enumerate()
            .map(|(idx, track)| {
                let is_playing_row = active_idx == Some(idx);
                let num_str = if is_playing_row {
                    if is_paused {
                        "⏸".to_string()
                    } else {
                        "▶".to_string()
                    }
                } else {
                    format!("{:>2}", idx + 1)
                };

                let base_style = if is_playing_row {
                    Style::default()
                        .fg(palette.playing)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.fg)
                };
                let sub_style = if is_playing_row {
                    Style::default().fg(palette.playing)
                } else {
                    Style::default().fg(palette.muted)
                };

                let mut cells = vec![
                    Cell::from(num_str).style(base_style),
                    Cell::from(track.title.clone()).style(base_style),
                    Cell::from(track.display_artist().to_string()).style(sub_style),
                ];
                if show_album_col {
                    cells.push(Cell::from(track.display_album().to_string()).style(sub_style));
                }
                cells.push(Cell::from(track.duration_formatted()).style(sub_style));
                Row::new(cells)
            })
            .collect();

        let mut table_state = TableState::default();
        let clamped_sel = app
            .playlist_selected
            .min(app.playlist.len().saturating_sub(1));
        app.playlist_selected = clamped_sel;
        table_state.select(Some(clamped_sel));

        let widths = if show_album_col {
            vec![
                Constraint::Length(4),
                Constraint::Percentage(38),
                Constraint::Percentage(26),
                Constraint::Percentage(24),
                Constraint::Length(7),
            ]
        } else {
            vec![
                Constraint::Length(4),
                Constraint::Percentage(52),
                Constraint::Percentage(34),
                Constraint::Length(7),
            ]
        };

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(
                Style::default()
                    .bg(palette.surface_highlight)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› ");

        StatefulWidget::render(table, list_area, buf, &mut table_state);
    }

    // Optional right-hand Now Playing mini showcase
    if let Some(side) = side_area {
        render_side_showcase(app, side, buf);
    } else if let Some(ref mut cov) = app.current_cover {
        cov.clear_kitty_if_active();
    }
}

fn render_side_showcase(app: &mut App, area: Rect, buf: &mut Buffer) {
    let palette = app.active_palette();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border))
        .title(Span::styled(
            " Now Playing ",
            Style::default().fg(palette.secondary),
        ));
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 6 || inner.height < 4 {
        return;
    }

    let (cover_area, vis_area) = if app.config.show_cover && app.config.show_visualizer && inner.height >= 12 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(inner);
        (Some(chunks[0]), Some(chunks[1]))
    } else if app.config.show_cover {
        (Some(inner), None)
    } else if app.config.show_visualizer {
        (None, Some(inner))
    } else {
        (None, None)
    };

    let mode = app.config.cover_mode;
    if let Some(ca) = cover_area {
        if let Some(ref mut cover) = app.current_cover {
            cover.render(ca, buf, mode);
        } else {
            let placeholder = Paragraph::new("No active track")
                .style(Style::default().fg(palette.muted));
            placeholder.render(ca, buf);
        }
    } else if let Some(ref mut cover) = app.current_cover {
        cover.clear_kitty_if_active();
    }

    if let Some(va) = vis_area {
        app.analyzer
            .render(va, buf, app.config.visualizer_style, &palette);
    }
}

fn inner_rect(r: Rect) -> Rect {
    Rect::new(
        r.x.saturating_add(1),
        r.y.saturating_add(2),
        r.width.saturating_sub(2),
        r.height.saturating_sub(3),
    )
}
