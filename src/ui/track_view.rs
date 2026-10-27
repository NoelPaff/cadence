use crate::media::lyrics::Lyrics;
use crate::ui::App;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

pub fn render_track_view(app: &mut App, area: Rect, buf: &mut Buffer) {
    app.hitboxes.list_area = None;
    let palette = app.active_palette();

    let Some(track) = app.playlist.current_track().cloned() else {
        if let Some(ref mut cov) = app.current_cover {
            cov.clear_kitty_if_active();
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_focused))
            .title(Span::styled(
                " Track View ",
                Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
            ));
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "No track is currently loaded.",
                Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Select a song or album in Library View (F3) or Search View (F5).",
                Style::default().fg(palette.muted),
            )),
        ];
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block)
            .render(area, buf);
        return;
    };

    let show_vis = app.config.show_visualizer && area.height >= 14;
    let (upper_area, vis_area) = if show_vis {
        let vis_h = (area.height / 3).clamp(5, 11);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(vis_h)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let show_cover = app.config.show_cover && upper_area.width >= 48;
    let (cover_pane, meta_pane) = if show_cover {
        let desired_cover_w = ((upper_area.height.saturating_sub(2)) * 2 + 4)
            .min(upper_area.width / 2)
            .max(20);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(desired_cover_w), Constraint::Min(24)])
            .split(upper_area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, upper_area)
    };

    let cover_mode = app.config.cover_mode;
    if let Some(cp) = cover_pane {
        let cover_title = format!(" Artwork ({}) ", cover_mode.label());
        let cb = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border))
            .title(Span::styled(cover_title, Style::default().fg(palette.muted)));
        let inner_cov = cb.inner(cp);
        cb.render(cp, buf);
        if let Some(ref mut cover) = app.current_cover {
            cover.render(inner_cov, buf, cover_mode);
        }
    } else if let Some(ref mut cover) = app.current_cover {
        cover.clear_kitty_if_active();
    }

    // Metadata + Synced Lyrics pane
    let pos_idx = app.playlist.current_index().map(|i| i + 1).unwrap_or(1);
    let total_q = app.playlist.len();
    let palette_mode_str = if app.config.dynamic_album_colors {
        "Album Colors"
    } else {
        &app.config.theme
    };

    let meta_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border_focused))
        .title(Span::styled(
            format!(" Track {}/{} • Palette: {} ", pos_idx, total_q, palette_mode_str),
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " b:Cover Mode  i:Album/Theme Colors  t:Theme  v/V:Visualizer  L:Lyrics ",
            Style::default().fg(palette.muted),
        )));
    let meta_inner = meta_block.inner(meta_pane);
    meta_block.render(meta_pane, buf);

    let mut header_lines = vec![
        Line::from(Span::styled(
            format!(" {}", track.title),
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" by ", Style::default().fg(palette.muted)),
            Span::styled(
                track.display_artist().to_string(),
                Style::default()
                    .fg(palette.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let mut album_spans = vec![
        Span::styled(" on ", Style::default().fg(palette.muted)),
        Span::styled(track.display_album().to_string(), Style::default().fg(palette.fg)),
    ];
    if let Some(y) = track.year {
        album_spans.push(Span::styled(
            format!(" ({})", y),
            Style::default().fg(palette.muted),
        ));
    }
    if let Some(ref g) = track.genre {
        album_spans.push(Span::styled(
            format!(" • {}", g),
            Style::default().fg(palette.muted),
        ));
    }
    header_lines.push(Line::from(album_spans));
    header_lines.push(Line::from(Span::styled(
        format!(" {}", track.tech_summary()),
        Style::default().fg(palette.accent),
    )));

    let header_h = (header_lines.len() as u16 + 1).min(meta_inner.height);
    let meta_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_h), Constraint::Min(0)])
        .split(meta_inner);

    Paragraph::new(header_lines)
        .wrap(Wrap { trim: false })
        .render(meta_chunks[0], buf);

    if app.config.show_lyrics && meta_chunks[1].height >= 3 {
        render_lyrics_box(
            &app.current_lyrics,
            app.telemetry.position_ms,
            meta_chunks[1],
            buf,
            &palette,
        );
    }

    if let Some(va) = vis_area {
        let vb = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border))
            .title(Span::styled(
                format!(" Spectrum — {} ", app.config.visualizer_style.label()),
                Style::default().fg(palette.secondary),
            ));
        let inner_v = vb.inner(va);
        vb.render(va, buf);
        app.analyzer
            .render(inner_v, buf, app.config.visualizer_style, &palette);
    }
}

fn render_lyrics_box(
    lyrics: &Lyrics,
    elapsed_ms: u64,
    area: Rect,
    buf: &mut Buffer,
    palette: &crate::media::palette::ThemePalette,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.border))
        .title(Span::styled(
            " Lyrics ",
            Style::default().fg(palette.secondary),
        ));
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 {
        return;
    }

    match lyrics {
        Lyrics::None => {
            let msg = Paragraph::new(Line::from(Span::styled(
                " No .lrc or embedded lyrics found for this track (press L to hide panel).",
                Style::default().fg(palette.muted),
            )));
            msg.render(inner, buf);
        }
        Lyrics::Plain(lines) => {
            let visible: Vec<Line> = lines
                .iter()
                .take(inner.height as usize)
                .map(|l| Line::from(Span::styled(format!(" {}", l), Style::default().fg(palette.fg))))
                .collect();
            Paragraph::new(visible).render(inner, buf);
        }
        Lyrics::Synced(lines) => {
            let active = lyrics.active_synced_index(elapsed_ms).unwrap_or(0);
            let half = (inner.height as usize) / 2;
            let start = active.saturating_sub(half);
            let end = (start + inner.height as usize).min(lines.len());

            let mut rendered = Vec::with_capacity(end.saturating_sub(start));
            for (idx, item) in lines.iter().enumerate().take(end).skip(start) {
                let line = if Some(idx) == lyrics.active_synced_index(elapsed_ms) {
                    Line::from(vec![
                        Span::styled(" ♪ ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            item.text.clone(),
                            Style::default()
                                .fg(palette.primary)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else if idx < active {
                    Line::from(Span::styled(
                        format!("   {}", item.text),
                        Style::default().fg(palette.muted),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {}", item.text),
                        Style::default().fg(palette.fg),
                    ))
                };
                rendered.push(line);
            }
            Paragraph::new(rendered).render(inner, buf);
        }
    }
}
