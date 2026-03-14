use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use super::theme;

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

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(1),    // top flex spacer
            Constraint::Length(13),// logo (13 lines)
            Constraint::Length(1), // subtitle
            Constraint::Length(3), // input box
            Constraint::Min(1),    // bottom flex spacer
            Constraint::Length(1), // hint bar
        ])
        .split(area);

    // ASCII logo
    let logo_lines: Vec<Line> = theme::ASCII_LOGO
        .iter()
        .map(|line| {
            let style = if line.contains("MIGRATE") {
                theme::accent_bold()
            } else {
                theme::primary_bold()
            };
            Line::from(Span::styled(*line, style))
        })
        .collect();

    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    f.render_widget(logo, inner[1]);

    // Subtitle
    let subtitle = Paragraph::new(Line::from(vec![
        Span::styled("[ ", theme::dim()),
        Span::styled("AWS", theme::danger()),
        Span::styled(" → ", theme::dim()),
        Span::styled("UPCLOUD", theme::accent_bold()),
        Span::styled(" terraform migration engine", theme::primary()),
        Span::styled(" ]", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(subtitle, inner[2]);

    // Input box
    let cursor = if (app.tick / 4) % 2 == 0 { "█" } else { " " };
    let input_text = format!("{}{}", app.input_buf, cursor);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .title(Span::styled(" PATH TO TERRAFORM DIRECTORY ", theme::primary_bold()));

    let input_widget = Paragraph::new(Span::styled(input_text, theme::primary()))
        .block(input_block);
    f.render_widget(input_widget, inner[3]);

    // Hint bar
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[ENTER]", theme::accent_bold()),
        Span::styled(" scan   ", theme::dim()),
        Span::styled("[F]", theme::accent_bold()),
        Span::styled(" browse filesystem   ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, inner[5]);
}
