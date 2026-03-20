use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::migration::generator::SKIPPED_SENTINEL;
use crate::migration::types::MigrationStatus;
use super::theme;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let total = app.migration_results.len();

    if total == 0 {
        let msg = Paragraph::new("No migration results to review.")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Double)
                .border_style(theme::primary())
                .title(Span::styled(" ⚡ MAPPING REVIEW ⚡ ", theme::accent_bold()))
                .title_alignment(Alignment::Center));
        f.render_widget(msg, area);
        return;
    }

    let result = &app.migration_results[app.diff_idx];

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(
            format!(" ⚡ MAPPING REVIEW ({}/{}) ⚡ ", app.diff_idx + 1, total),
            theme::accent_bold(),
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(outer, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // resource header
            Constraint::Min(5),    // diff panels
            Constraint::Length(1), // hint bar
        ])
        .split(area);

    // ── Resource header ───────────────────────────────────────────────────────
    let (status_icon, status_style) = match result.status {
        MigrationStatus::Native      => ("■ NATIVE",      theme::success()),
        MigrationStatus::Compatible  => ("◈ COMPATIBLE",  theme::primary()),
        MigrationStatus::Partial     => ("◇ PARTIAL",     theme::warning()),
        MigrationStatus::Unsupported => ("✕ UNSUPPORTED", theme::danger()),
        MigrationStatus::Unknown     => ("? UNKNOWN",     theme::dim()),
    };

    let short_src = result.resource_type.strip_prefix("aws_").unwrap_or(&result.resource_type);
    let short_dst = result.upcloud_type.strip_prefix("upcloud_").unwrap_or(&result.upcloud_type);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(status_style);

    let header_text = Line::from(vec![
        Span::styled("  ", theme::dim()),
        Span::styled(
            format!("{} \"{}\"", short_src, result.resource_name),
            Style::default().fg(theme::HCL_KEY).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ──→  ", theme::dim()),
        Span::styled(
            format!("{} \"{}\"", short_dst, result.resource_name),
            Style::default().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", theme::dim()),
        Span::styled(status_icon, status_style.add_modifier(Modifier::BOLD)),
    ]);

    f.render_widget(Paragraph::new(header_text).block(header_block), layout[0]);

    // ── Side-by-side panels ───────────────────────────────────────────────────
    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[1]);

    render_source_panel(f, app, panels[0]);
    render_generated_panel(f, app, panels[1]);

    // ── Hint bar ──────────────────────────────────────────────────────────────
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[←/→]", theme::accent_bold()),
        Span::styled(" prev/next resource  ", theme::dim()),
        Span::styled("[↑↓]", theme::accent_bold()),
        Span::styled(" scroll  ", theme::dim()),
        Span::styled("[Esc]", theme::accent_bold()),
        Span::styled(" back  ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, layout[2]);
}

fn render_source_panel(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let result = &app.migration_results[app.diff_idx];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(
            format!(" ORIGINAL  {}  ", result.source_file.rsplit('/').next().unwrap_or("")),
            Style::default().fg(theme::HCL_KEY).add_modifier(Modifier::BOLD),
        ));

    let lines: Vec<Line> = match &result.source_hcl {
        None => vec![Line::from(Span::styled("(source not available)", theme::dim()))],
        Some(hcl) => hcl.lines().map(colorize_source_line).collect(),
    };

    let widget = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.diff_scroll as u16, 0));
    f.render_widget(widget, area);
}

fn render_generated_panel(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let result = &app.migration_results[app.diff_idx];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::primary())
        .title(Span::styled(" GENERATED (UpCloud) ", theme::primary_bold()));

    let lines: Vec<Line> = match &result.upcloud_hcl {
        None => {
            // No generated HCL — show notes and status
            let mut out = Vec::new();
            out.push(Line::from(Span::styled(
                match result.status {
                    MigrationStatus::Unsupported => "  ✕ No UpCloud equivalent",
                    MigrationStatus::Partial     => "  ◇ Partial mapping — see notes",
                    _                            => "  · No output generated",
                },
                theme::danger(),
            )));
            out.push(Line::from(""));
            for note in &result.notes {
                out.push(Line::from(vec![
                    Span::styled("  • ", theme::dim()),
                    Span::styled(note.clone(), theme::muted()),
                ]));
            }
            out
        }
        Some(hcl) => {
            // Prefer the resolved HCL (post-generation, with TODOs resolved) if available.
            let key = (result.resource_type.clone(), result.resource_name.clone());
            let resolved = app.resolved_hcl_map.get(&key).map(|s| s.as_str());

            // If this resource was skipped during generation, show a note instead of HCL.
            if resolved == Some(SKIPPED_SENTINEL) {
                vec![
                    Line::from(Span::styled("  — SKIPPED —", theme::dim())),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  This resource was not written to the output.",
                        theme::muted(),
                    )),
                    Line::from(Span::styled(
                        "  UpCloud manages its access control separately.",
                        theme::muted(),
                    )),
                ]
            } else {
                let display_hcl = resolved.unwrap_or(hcl.as_str());
                let mut out = Vec::new();
                out.push(Line::from(vec![
                    Span::styled(
                        format!("# {} {}", result.resource_type, result.resource_name),
                        theme::dim(),
                    ),
                ]));
                for note in &result.notes {
                    out.push(Line::from(Span::styled(format!("# NOTE: {}", note), theme::dim())));
                }
                for line in display_hcl.lines() {
                    out.push(colorize_generated_line(line));
                }
                out
            }
        }
    };

    let widget = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.diff_scroll as u16, 0));
    f.render_widget(widget, area);
}

fn colorize_source_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    let line = line.to_string();

    if trimmed.starts_with("resource ") {
        Line::from(Span::styled(line, Style::default().fg(theme::HCL_KEY).add_modifier(Modifier::BOLD)))
    } else if trimmed.starts_with('#') || trimmed == "}" || trimmed == "{" {
        Line::from(Span::styled(line, theme::dim()))
    } else if trimmed.contains('=') {
        // Split at first `=` for key/value coloring
        if let Some(eq) = line.find('=') {
            let key = line[..eq + 1].to_string();
            let val = line[eq + 1..].to_string();
            Line::from(vec![
                Span::styled(key, Style::default().fg(theme::HCL_KEY)),
                Span::styled(val, theme::dim()),
            ])
        } else {
            Line::from(Span::styled(line, theme::dim()))
        }
    } else {
        Line::from(Span::styled(line, theme::dim()))
    }
}

fn colorize_generated_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    let line = line.to_string();

    if trimmed.contains("<TODO") {
        Line::from(Span::styled(line, Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)))
    } else if trimmed.starts_with("resource ") {
        Line::from(Span::styled(line, Style::default().fg(theme::PRIMARY).add_modifier(Modifier::BOLD)))
    } else if trimmed.starts_with('#') || trimmed == "}" || trimmed == "{" {
        Line::from(Span::styled(line, theme::dim()))
    } else if trimmed.contains('=') {
        if let Some(eq) = line.find('=') {
            let key = line[..eq + 1].to_string();
            let val = line[eq + 1..].to_string();
            Line::from(vec![
                Span::styled(key, Style::default().fg(theme::PRIMARY)),
                Span::styled(val, Style::default().fg(theme::WHITE)),
            ])
        } else {
            Line::from(Span::styled(line, theme::primary()))
        }
    } else {
        Line::from(Span::styled(line, theme::dim()))
    }
}
