use crate::model::LibraryNodeKind;
use crate::ui::{App, InputFocus};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget};

pub fn render_library_view(app: &mut App, area: Rect, buf: &mut Buffer) {
    if let Some(ref mut cov) = app.current_cover {
        cov.clear_kitty_if_active();
    }
    let palette = app.active_palette();
    let filtering = app.focus == InputFocus::LibraryFilter || !app.library_filter.is_empty();

    let (filter_area, body_area) = if filtering {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(4)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(fa) = filter_area {
        let focused = app.focus == InputFocus::LibraryFilter;
        let cursor = if focused { "█" } else { "" };
        let b = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused {
                palette.primary
            } else {
                palette.border
            }))
            .title(Span::styled(
                " Filter Library (/ to edit, Esc to clear) ",
                Style::default().fg(palette.secondary),
            ));
        Paragraph::new(format!(" {}{}", app.library_filter, cursor))
            .style(Style::default().fg(palette.fg))
            .block(b)
            .render(fa, buf);
    }

    let items = app.library.visible_tree_items(&app.library_filter);
    let show_inspector = body_area.width >= 98;
    let (tree_area, info_area) = if show_inspector {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(body_area);
        (chunks[0], Some(chunks[1]))
    } else {
        (body_area, None)
    };

    app.hitboxes.list_area = Some(Rect::new(
        tree_area.x.saturating_add(1),
        tree_area.y.saturating_add(1),
        tree_area.width.saturating_sub(2),
        tree_area.height.saturating_sub(2),
    ));

    let scan_tag = if app.is_scanning { " • ⟳ Scanning..." } else { "" };
    let title = format!(
        " Library — {} ({} tracks{}) ",
        app.library.tree_mode.label(),
        app.library.len(),
        scan_tag
    );
    let bottom_hint = " Enter:Play  l/→:Expand  h/←:Collapse  e:Enqueue  n:Next  /:Filter  M:Mode  u:Rescan ";

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
            bottom_hint,
            Style::default().fg(palette.muted),
        )));

    if items.is_empty() {
        let msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  No matching audio files in {}", app.library.root.display()),
                Style::default().fg(palette.fg),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 'u' to rescan or run `cadence path <directory>` to set your music folder.",
                Style::default().fg(palette.muted),
            )),
        ];
        Paragraph::new(msg).block(block).render(tree_area, buf);
        return;
    }

    let clamped = app.library_selected.min(items.len().saturating_sub(1));
    app.library_selected = clamped;

    let playing_path = app.playlist.current_track().map(|t| &t.path);

    let list_items: Vec<ListItem> = items
        .iter()
        .map(|item| {
            let indent = "  ".repeat(item.depth);
            let icon = if item.is_container {
                if item.is_expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                match item.kind {
                    LibraryNodeKind::PlaylistFile(_) => "☰ ",
                    _ => "♪ ",
                }
            };

            let is_playing = match &item.kind {
                LibraryNodeKind::Track(t) => playing_path == Some(&t.path),
                _ => false,
            };

            let main_style = if is_playing {
                Style::default()
                    .fg(palette.playing)
                    .add_modifier(Modifier::BOLD)
            } else if item.is_container {
                Style::default()
                    .fg(palette.secondary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.fg)
            };

            Line::from(vec![
                Span::raw(indent),
                Span::styled(icon, Style::default().fg(palette.accent)),
                Span::styled(item.display_name.clone(), main_style),
                Span::styled(
                    format!("  ({})", item.sub_info),
                    Style::default().fg(palette.muted),
                ),
            ])
            .into()
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(clamped));

    let list = List::new(list_items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(palette.surface_highlight)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    StatefulWidget::render(list, tree_area, buf, &mut state);

    if let Some(ia) = info_area {
        let info_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border))
            .title(Span::styled(
                " Selection Details ",
                Style::default().fg(palette.secondary),
            ));

        let selected = &items[clamped];
        let mut lines = vec![Line::from("")];
        match &selected.kind {
            LibraryNodeKind::Directory(path) => {
                lines.push(Line::from(Span::styled(
                    format!("  📁 {}", selected.display_name),
                    Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  {}", selected.sub_info),
                    Style::default().fg(palette.fg),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", path.display()),
                    Style::default().fg(palette.muted),
                )));
            }
            LibraryNodeKind::Artist(artist) => {
                lines.push(Line::from(Span::styled(
                    format!("  👤 {}", artist),
                    Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  {}", selected.sub_info),
                    Style::default().fg(palette.fg),
                )));
            }
            LibraryNodeKind::Album { artist, album } => {
                lines.push(Line::from(Span::styled(
                    format!("  💿 {}", album),
                    Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  Artist: {}", artist),
                    Style::default().fg(palette.secondary),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  {}", selected.sub_info),
                    Style::default().fg(palette.muted),
                )));
            }
            LibraryNodeKind::Track(t) => {
                lines.push(Line::from(Span::styled(
                    format!("  ♪ {}", t.title),
                    Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  Artist: {}", t.display_artist()),
                    Style::default().fg(palette.fg),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  Album:  {}", t.display_album()),
                    Style::default().fg(palette.fg),
                )));
                if let Some(y) = t.year {
                    lines.push(Line::from(Span::styled(
                        format!("  Year:   {}", y),
                        Style::default().fg(palette.muted),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    format!("  Length: {}", t.duration_formatted()),
                    Style::default().fg(palette.muted),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", t.tech_summary()),
                    Style::default().fg(palette.accent),
                )));
            }
            LibraryNodeKind::PlaylistFile(p) => {
                lines.push(Line::from(Span::styled(
                    format!("  ☰ {}", selected.display_name),
                    Style::default().fg(palette.primary).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  {}", p.display()),
                    Style::default().fg(palette.muted),
                )));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Actions:",
            Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  • Enter : Replace queue & play",
            Style::default().fg(palette.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  • e / a : Append to queue",
            Style::default().fg(palette.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  • n     : Play immediately next",
            Style::default().fg(palette.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  • .     : Save to default .m3u8",
            Style::default().fg(palette.muted),
        )));

        Paragraph::new(lines).block(info_block).render(ia, buf);
    }
}
