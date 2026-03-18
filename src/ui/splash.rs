use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use super::theme;

/// Pseudo-random sparkle: deterministic given (tick, row, col) so no RNG needed.
fn is_sparkle(tick: u64, row: usize, col: usize) -> bool {
    // lcg-style hash
    let h = tick
        .wrapping_mul(6364136223846793005)
        .wrapping_add((row as u64).wrapping_mul(1442695040888963407))
        .wrapping_add((col as u64).wrapping_mul(2862933555777941757));
    // ~8% of hash positions sparkle, cycling every ~3 ticks per position
    (h >> 56) < 20 && ((h >> 40) & 0x3) == (tick & 0x3) as u64
}

/// Build one line of the logo with per-character sparkle coloring.
fn logo_line_spans(line: &str, row: usize, tick: u64) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (col, ch) in line.chars().enumerate() {
        if ch == '#' {
            let style = if is_sparkle(tick, row, col) {
                Style::default()
                    .fg(theme::UPCLOUD_SPARKLE)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::UPCLOUD_PURPLE)
            };
            spans.push(Span::styled(ch.to_string(), style));
        } else {
            // spaces / blanks — transparent
            spans.push(Span::raw(ch.to_string()));
        }
    }
    Line::from(spans)
}

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(
            " ⚡ UPCLOUD MIGRATE // TERRAFORM MIGRATION ENGINE ⚡ ",
            theme::accent_bold(),
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    let logo_height = theme::UPCLOUD_LOGO.len() as u16;

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(1),                    // top flex spacer
            Constraint::Length(logo_height),        // UpCloud brand logo
            Constraint::Length(1),                 // gap
            Constraint::Length(1),                 // title line
            Constraint::Length(1),                 // subtitle line
            Constraint::Length(1),                 // gap
            Constraint::Length(3),                 // path input box
            Constraint::Min(1),                    // bottom flex spacer
            Constraint::Length(1),                 // hint bar
        ])
        .split(area);

    // ── Animated UpCloud logo ────────────────────────────────────────────────
    let logo_lines: Vec<Line> = theme::UPCLOUD_LOGO
        .iter()
        .enumerate()
        .map(|(row, line)| logo_line_spans(line, row, app.tick))
        .collect();

    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    f.render_widget(logo, inner[1]);

    // ── Title ────────────────────────────────────────────────────────────────
    let title = Paragraph::new(Line::from(Span::styled(
        "UpCloud Terraform Migrator Tool",
        Style::default()
            .fg(Color::Rgb(220, 180, 255))
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(title, inner[3]);

    // ── Subtitle ─────────────────────────────────────────────────────────────
    let subtitle = Paragraph::new(Line::from(vec![
        Span::styled("Converts ", theme::dim()),
        Span::styled("AWS", theme::danger()),
        Span::styled(" Terraform code to ", theme::dim()),
        Span::styled(
            "UpCloud",
            Style::default().fg(theme::UPCLOUD_PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Terraform!", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(subtitle, inner[4]);

    // ── Path input ───────────────────────────────────────────────────────────
    let cursor = if (app.tick / 4) % 2 == 0 { "█" } else { " " };
    let input_text = format!("{}{}", app.input_buf, cursor);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::UPCLOUD_PURPLE))
        .title(Span::styled(
            " PATH TO TERRAFORM DIRECTORY ",
            Style::default()
                .fg(Color::Rgb(220, 180, 255))
                .add_modifier(Modifier::BOLD),
        ));

    let input_widget =
        Paragraph::new(Span::styled(input_text, theme::primary())).block(input_block);
    f.render_widget(input_widget, inner[6]);

    // ── Hint bar ─────────────────────────────────────────────────────────────
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[ENTER]", theme::accent_bold()),
        Span::styled(" scan   ", theme::dim()),
        Span::styled("[F]", theme::accent_bold()),
        Span::styled(" browse filesystem   ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, inner[8]);
}
