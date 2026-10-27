use crate::ui::{App, InputFocus};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget};

pub fn render_search_view(app: &mut App, area: Rect, buf: &mut Buffer) {
    if let Some(ref mut cov) = app.current_cover {
        cov.clear_kitty_if_active();
    }
    let palette = app.active_palette();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(area);

    let input_focused = app.focus == InputFocus::SearchQuery;
    let mode_tag = if app.exact_search { "Exact" } else { "Fuzzy" };
    let cursor = if input_focused { "█" } else { "" };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if input_focused {
            palette.primary
        } else {
            palette.border
        }))
        .title(Span::styled(
            format!(
                " Search [{}] — Type query or artist:album:track (/ to focus, Esc for nav) ",
                mode_tag
            ),
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ));

    Paragraph::new(format!(" 🔍 {}{}", app.search_query, cursor))
        .style(Style::default().fg(palette.fg))
        .block(input_block)
        .render(chunks[0], buf);

    let results = app
        .library
        .search_tracks(&app.search_query, app.exact_search);

    app.hitboxes.list_area = Some(Rect::new(
        chunks[1].x.saturating_add(1),
        chunks[1].y.saturating_add(2),
        chunks[1].width.saturating_sub(2),
        chunks[1].height.saturating_sub(3),
    ));

    let results_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if !input_focused {
            palette.border_focused
        } else {
            palette.border
        }))
        .title(Span::styled(
            format!(" Results ({} matches) ", results.len()),
            Style::default().fg(palette.secondary),
        ))
        .title_bottom(Line::from(Span::styled(
            " Enter:Play Track  Tab/Ctrl+A:Enqueue  ↑/↓:Navigate  .:Save to Default Playlist ",
            Style::default().fg(palette.muted),
        )));

    if results.is_empty() {
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No tracks matched your search query.",
                Style::default().fg(palette.muted),
            )),
        ])
        .block(results_block)
        .render(chunks[1], buf);
        return;
    }

    let clamped = app.search_selected.min(results.len().saturating_sub(1));
    app.search_selected = clamped;

    let playing_path = app.playlist.current_track().map(|t| &t.path);
    let header = Row::new(vec![
        Cell::from("Title").style(Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD)),
        Cell::from("Artist").style(Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD)),
        Cell::from("Album").style(Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD)),
        Cell::from("Time").style(Style::default().fg(palette.secondary).add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = results
        .iter()
        .map(|t| {
            let active = playing_path == Some(&t.path);
            let title_style = if active {
                Style::default()
                    .fg(palette.playing)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.fg)
            };
            let meta_style = if active {
                Style::default().fg(palette.playing)
            } else {
                Style::default().fg(palette.muted)
            };
            let prefix = if active { "▶ " } else { "" };
            Row::new(vec![
                Cell::from(format!("{}{}", prefix, t.title)).style(title_style),
                Cell::from(t.display_artist().to_string()).style(meta_style),
                Cell::from(t.display_album().to_string()).style(meta_style),
                Cell::from(t.duration_formatted()).style(meta_style),
            ])
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(clamped));

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(38),
            Constraint::Percentage(27),
            Constraint::Percentage(26),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(results_block)
    .row_highlight_style(
        Style::default()
            .bg(palette.surface_highlight)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("› ");

    StatefulWidget::render(table, chunks[1], buf, &mut state);
}
