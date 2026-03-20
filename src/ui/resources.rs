use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table,
    },
};

use super::theme;
use crate::app::App;
use crate::migration::types::{MigrationResult, MigrationStatus};

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(
            " ⚡ RESOURCE MIGRATION MAP ⚡ ",
            theme::accent_bold(),
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1), // stats bar
            Constraint::Min(5),    // main split
            Constraint::Length(1), // hint bar
        ])
        .split(area);

    // ── Stats bar ──────────────────────────────────────────────────────────────
    let total = app.migration_results.len();
    let native = count_status(&app.migration_results, MigrationStatus::Native);
    let compat = count_status(&app.migration_results, MigrationStatus::Compatible);
    let partial = count_status(&app.migration_results, MigrationStatus::Partial);
    let unsup = count_status(&app.migration_results, MigrationStatus::Unsupported);

    let stats = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {total}"), theme::white_bold()),
        Span::styled(" total  ", theme::dim()),
        Span::styled("◆", theme::success()),
        Span::styled(format!(" {native} native  "), theme::success()),
        Span::styled("◈", theme::primary()),
        Span::styled(format!(" {compat} compat  "), theme::primary()),
        Span::styled("◇", theme::warning()),
        Span::styled(format!(" {partial} partial  "), theme::warning()),
        Span::styled("✕", theme::danger()),
        Span::styled(format!(" {unsup} unsup  "), theme::danger()),
    ]));
    f.render_widget(stats, layout[0]);

    // ── Horizontal split: table (left) + preview (right) ───────────────────────
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(layout[1]);

    render_table(f, app, split[0]);
    render_preview(f, app, split[1]);

    // ── Hint bar ───────────────────────────────────────────────────────────────
    let hints = if app.resources_focus_preview {
        Paragraph::new(Line::from(vec![
            Span::styled("[↑↓]", theme::accent_bold()),
            Span::styled(" scroll preview  ", theme::dim()),
            Span::styled("[←/h]", theme::accent_bold()),
            Span::styled(" back to list  ", theme::dim()),
            Span::styled("[G/Tab]", theme::accent_bold()),
            Span::styled(" generate  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" quit", theme::dim()),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("[↑↓]", theme::accent_bold()),
            Span::styled(" scroll  ", theme::dim()),
            Span::styled("[→/l]", theme::accent_bold()),
            Span::styled(" focus preview  ", theme::dim()),
            Span::styled("[G/Tab]", theme::accent_bold()),
            Span::styled(" generate  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" quit", theme::dim()),
        ]))
    };
    f.render_widget(hints.alignment(Alignment::Center), layout[2]);
}

fn render_table(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let header_cells = ["  TYPE", "NAME", "STATUS"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::accent_bold()));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .migration_results
        .iter()
        .map(|r| {
            let icon = theme::status_icon(r.status.label());
            let type_label = r
                .resource_type
                .strip_prefix("aws_")
                .unwrap_or(&r.resource_type);

            Row::new(vec![
                Cell::from(format!("  {}", truncate(type_label, 24))).style(theme::primary()),
                Cell::from(truncate(&r.resource_name, 16)).style(theme::muted()),
                Cell::from(format!("{} {}", icon, r.status.label()))
                    .style(theme::status_style(r.status.label())),
            ])
            .height(1)
        })
        .collect();

    // Dim table border when preview has focus
    let border_style = if app.resources_focus_preview {
        theme::dim()
    } else {
        theme::primary()
    };
    let title_style = if app.resources_focus_preview {
        theme::muted()
    } else {
        theme::accent_bold()
    };

    let table_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(" RESOURCES ", title_style));

    let table = Table::new(
        rows,
        [
            Constraint::Min(20),    // type
            Constraint::Length(16), // name
            Constraint::Min(14),    // status
        ],
    )
    .header(header)
    .block(table_block)
    .row_highlight_style(theme::selected());

    let mut state = app.table_state.clone();
    f.render_stateful_widget(table, area, &mut state);
}

fn render_preview(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let selected_idx = app.table_state.selected().unwrap_or(0);
    let result = app.migration_results.get(selected_idx);

    // Highlight border when preview has focus
    let border_style = if app.resources_focus_preview {
        theme::accent()
    } else {
        theme::dim()
    };
    let title_style = if app.resources_focus_preview {
        theme::accent_bold()
    } else {
        theme::muted()
    };

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(" ◈ PREVIEW ", title_style))
        .title_alignment(Alignment::Left);

    if result.is_none() || app.migration_results.is_empty() {
        let empty = Paragraph::new(Span::styled("  No resource selected", theme::dim()))
            .block(preview_block);
        f.render_widget(empty, area);
        return;
    }

    let r = result.unwrap();
    let inner_height = area.height.saturating_sub(2) as usize; // minus borders
    let sep_w = area.width.saturating_sub(2) as usize;

    let mut all_lines: Vec<Line<'static>> = Vec::new();

    // ── Header: aws type → upcloud type ────────────────────────────────────────
    let aws_label = r
        .resource_type
        .strip_prefix("aws_")
        .unwrap_or(&r.resource_type)
        .to_owned();
    all_lines.push(Line::from(vec![
        Span::styled(aws_label, theme::primary_bold()),
        Span::styled(format!(" \"{}\"", r.resource_name), theme::muted()),
    ]));
    all_lines.push(Line::from(vec![
        Span::styled("  ↳ ", theme::accent()),
        Span::styled(r.upcloud_type.clone(), theme::warning()),
    ]));

    // ── Status line ───────────────────────────────────────────────────────────
    let icon = theme::status_icon(r.status.label());
    let inner_w = area.width.saturating_sub(2) as usize;
    all_lines.push(Line::from(vec![
        Span::styled(
            format!("  {} ", icon),
            theme::status_style(r.status.label()),
        ),
        Span::styled(r.status.label(), theme::status_style(r.status.label())),
    ]));

    // ── Separator ─────────────────────────────────────────────────────────────
    all_lines.push(Line::from(Span::styled("─".repeat(sep_w), theme::dim())));

    // ── Notes ─────────────────────────────────────────────────────────────────
    for note in &r.notes {
        // Word-wrap long notes manually at inner_w - 4 chars (for "  · " prefix).
        // Work entirely in char indices to avoid panics on multi-byte characters (e.g. em dash).
        let note_width = inner_w.saturating_sub(4);
        let note_chars: Vec<char> = note.chars().collect();
        if note_chars.len() <= note_width {
            all_lines.push(Line::from(vec![
                Span::styled("  · ", theme::dim()),
                Span::styled(note.clone(), theme::muted()),
            ]));
        } else {
            let mut pos = 0usize; // char position
            let mut first = true;
            while pos < note_chars.len() {
                let end = (pos + note_width).min(note_chars.len());
                // Snap to word boundary (char-based)
                let chunk_end = if end < note_chars.len() {
                    note_chars[pos..end]
                        .iter()
                        .rposition(|&c| c == ' ')
                        .map(|i| pos + i)
                        .unwrap_or(end)
                } else {
                    end
                };
                let chunk: String = note_chars[pos..chunk_end]
                    .iter()
                    .collect::<String>()
                    .trim_start()
                    .to_string();
                if first {
                    all_lines.push(Line::from(vec![
                        Span::styled("  · ", theme::dim()),
                        Span::styled(chunk, theme::muted()),
                    ]));
                    first = false;
                } else {
                    all_lines.push(Line::from(vec![
                        Span::styled("    ", theme::dim()),
                        Span::styled(chunk, theme::muted()),
                    ]));
                }
                pos = chunk_end;
                if pos < note_chars.len() && note_chars[pos] == ' ' {
                    pos += 1;
                }
            }
        }
    }

    // ── HCL or snippet ────────────────────────────────────────────────────────
    let hcl_content = r.upcloud_hcl.as_deref().or(r.snippet.as_deref());

    if let Some(hcl) = hcl_content {
        let label = if r.upcloud_hcl.is_some() {
            " HCL "
        } else {
            " MERGE INTO "
        };
        all_lines.push(Line::from(Span::styled("─".repeat(sep_w), theme::dim())));
        all_lines.push(Line::from(vec![Span::styled(label, theme::accent())]));
        for line in hcl.lines() {
            all_lines.push(theme::highlight_hcl_line(line));
        }
    } else {
        all_lines.push(Line::from(Span::styled("─".repeat(sep_w), theme::dim())));
        all_lines.push(Line::from(Span::styled(
            "  No direct mapping — see MIGRATION_NOTES.md",
            theme::dim(),
        )));
    }

    // ── Apply scroll offset ────────────────────────────────────────────────────
    let total_lines = all_lines.len();
    let scroll = app
        .preview_scroll
        .min(total_lines.saturating_sub(inner_height));
    let visible: Vec<Line<'static>> = all_lines.into_iter().skip(scroll).collect();

    let preview = Paragraph::new(visible).block(preview_block);
    f.render_widget(preview, area);

    // ── Scrollbar ─────────────────────────────────────────────────────────────
    if total_lines > inner_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state =
            ScrollbarState::new(total_lines.saturating_sub(inner_height)).position(scroll);
        f.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn count_status(results: &[MigrationResult], status: MigrationStatus) -> usize {
    results.iter().filter(|r| r.status == status).count()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
