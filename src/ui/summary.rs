use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::migration::scorer::{migration_recommendation, top_blockers, StatusBreakdown};
use crate::migration::types::MigrationStatus;
use super::theme;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(" ⚡ MIGRATION SUMMARY ⚡ ", theme::accent_bold()))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),  // score gauge
            Constraint::Min(4),     // main content split
            Constraint::Length(3),  // recommendation
            Constraint::Length(1),  // hint bar
        ])
        .split(area);

    let score = app.overall_score;
    let score_pct = (score as u16).min(100);
    let gauge_color = match score as u8 {
        85..=100 => theme::SUCCESS,
        65..=84  => theme::PRIMARY,
        30..=64  => theme::WARNING,
        _        => theme::DANGER,
    };

    // ── Score gauge ───────────────────────────────────────────────────────────
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(gauge_color))
                .title(Span::styled(" OVERALL MIGRATION SCORE ", theme::primary_bold())),
        )
        .gauge_style(Style::default().fg(gauge_color).add_modifier(Modifier::BOLD))
        .percent(score_pct)
        .label(Span::styled(
            format!("{:.1}% MIGRATABLE", score),
            Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(gauge, layout[0]);

    // ── Main content: left = breakdown + blockers, right = resources needing attention
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(layout[1]);

    render_left(f, app, split[0]);
    render_right(f, app, split[1], app.summary_scroll);

    // ── Recommendation ────────────────────────────────────────────────────────
    let rec_text = migration_recommendation(score);
    let rec = Paragraph::new(Line::from(vec![
        Span::styled(" ▶ ", theme::accent_bold()),
        Span::styled(rec_text, theme::primary()),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::accent()),
    );
    f.render_widget(rec, layout[2]);

    // ── Hint bar ──────────────────────────────────────────────────────────────
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[G]", theme::accent_bold()),
        Span::styled(" generate  ", theme::dim()),
        Span::styled("[↑↓]", theme::accent_bold()),
        Span::styled(" scroll  ", theme::dim()),
        Span::styled("[Tab]", theme::accent_bold()),
        Span::styled(" →generator  ", theme::dim()),
        Span::styled("[Shift+Tab]", theme::accent_bold()),
        Span::styled(" →resources  ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, layout[3]);
}

fn render_left(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(3)])
        .split(area);

    // ── Status breakdown bars ─────────────────────────────────────────────────
    let breakdown = StatusBreakdown::from_results(&app.migration_results);
    let total = app.migration_results.len();

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(" STATUS BREAKDOWN ", theme::muted()));

    let bar_inner = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner[0]);

    f.render_widget(bar_block, inner[0]);
    render_bar(f, bar_inner[0], "◆ NATIVE    ", breakdown.native,      total, theme::SUCCESS);
    render_bar(f, bar_inner[1], "◈ COMPATIBLE", breakdown.compatible,   total, theme::PRIMARY);
    render_bar(f, bar_inner[2], "◇ PARTIAL   ", breakdown.partial,      total, theme::WARNING);
    render_bar(f, bar_inner[3], "✕ UNSUPPORTED", breakdown.unsupported, total, theme::DANGER);

    // ── Blockers ──────────────────────────────────────────────────────────────
    let blockers = top_blockers(&app.migration_results);
    let blocker_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::danger())
        .title(Span::styled(" UNSUPPORTED RESOURCES ", theme::danger()));

    let blocker_lines: Vec<Line> = if blockers.is_empty() {
        vec![Line::from(Span::styled(
            "  ✓ No unsupported resources — fully migratable!",
            theme::success(),
        ))]
    } else {
        blockers
            .iter()
            .enumerate()
            .map(|(i, (rt, count))| {
                let short = rt.strip_prefix("aws_").unwrap_or(rt);
                Line::from(vec![
                    Span::styled(format!("  {:2}. ", i + 1), theme::dim()),
                    Span::styled(short.to_string(), theme::danger()),
                    Span::styled(format!(" ×{} — no UpCloud equivalent", count), theme::muted()),
                ])
            })
            .collect()
    };

    let blocker_widget = Paragraph::new(blocker_lines).block(blocker_block);
    f.render_widget(blocker_widget, inner[1]);
}

fn render_right(f: &mut Frame, app: &App, area: ratatui::layout::Rect, scroll: usize) {
    // Show partial resources with their first note (what needs manual work)
    let partial_and_unsup: Vec<_> = app.migration_results.iter()
        .filter(|r| r.status == MigrationStatus::Partial || r.status == MigrationStatus::Unsupported)
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::warning())
        .title(Span::styled(" RESOURCES NEEDING ATTENTION ", theme::warning()));

    let lines: Vec<Line> = if partial_and_unsup.is_empty() {
        vec![Line::from(Span::styled(
            "  ✓ All resources map cleanly — nothing to review!",
            theme::success(),
        ))]
    } else {
        let mut out = Vec::new();
        for r in &partial_and_unsup {
            let short_type = r.resource_type.strip_prefix("aws_").unwrap_or(&r.resource_type);
            let (icon, style) = match r.status {
                MigrationStatus::Partial     => ("◇", theme::warning()),
                MigrationStatus::Unsupported => ("✕", theme::danger()),
                _ => ("·", theme::dim()),
            };
            out.push(Line::from(vec![
                Span::styled(format!(" {} ", icon), style),
                Span::styled(
                    format!("{} \"{}\"", short_type, r.resource_name),
                    Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD),
                ),
            ]));
            // First note as context
            if let Some(note) = r.notes.first() {
                out.push(Line::from(vec![
                    Span::styled("   └ ", theme::dim()),
                    Span::styled(note.clone(), theme::muted()),
                ]));
            }
        }
        out
    };

    let widget = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    f.render_widget(widget, area);
}

fn render_bar(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    label: &str,
    count: usize,
    total: usize,
    color: ratatui::style::Color,
) {
    if area.width < 20 { return; }
    let pct = if total == 0 { 0 } else { (count * 100 / total).min(100) as u16 };
    let bar_width = area.width.saturating_sub(30) as usize;
    let filled = (bar_width * pct as usize / 100).min(bar_width);
    let empty  = bar_width.saturating_sub(filled);

    let line = Line::from(vec![
        Span::styled(format!("{:<13} ", label), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty),  theme::dim()),
        Span::styled(format!(" {:3} ({:3}%)", count, pct), Style::default().fg(color)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
