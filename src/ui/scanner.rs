use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};

use super::theme;
use crate::app::App;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // status line
            Constraint::Min(5),    // file list
            Constraint::Length(1), // hint bar
        ])
        .margin(1)
        .split(area);

    // Outer border
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(
            " ⚡ SCANNING TERRAFORM FILES ⚡ ",
            theme::accent_bold(),
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    // Header stats
    let header = Paragraph::new(Line::from(vec![
        Span::styled("FILES FOUND: ", theme::dim()),
        Span::styled(format!("{}", app.scan_files.len()), theme::accent_bold()),
        Span::styled("   TF FILES: ", theme::dim()),
        Span::styled(
            format!(
                "{}",
                app.scan_files.iter().filter(|f| f.ends_with(".tf")).count()
            ),
            theme::primary_bold(),
        ),
        Span::styled("   RESOURCES: ", theme::dim()),
        Span::styled(format!("{}", app.resources.len()), theme::success()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(header, outer[0]);

    // Spinner + current status
    let spin = theme::spinner(app.tick);
    let status_msg = if app.scan_complete {
        Line::from(vec![
            Span::styled("✓ ", theme::success()),
            Span::styled("SCAN COMPLETE — ", theme::success()),
            Span::styled(
                format!("{} resources found", app.resources.len()),
                Style::default()
                    .fg(theme::success_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(format!("{} ", spin), theme::accent()),
            Span::styled("SCANNING: ", theme::primary()),
            Span::styled(app.scan_current.as_deref().unwrap_or("..."), theme::dim()),
        ])
    };
    let status = Paragraph::new(status_msg).alignment(Alignment::Left);
    f.render_widget(status, outer[1]);

    // File list
    let file_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(" DISCOVERED FILES ", theme::primary()));

    let items: Vec<ListItem> = app
        .scan_files
        .iter()
        .rev()
        .take(outer[2].height.saturating_sub(2) as usize)
        .rev()
        .map(|f| {
            let style = if f.ends_with(".tf") {
                theme::primary()
            } else {
                theme::dim()
            };
            ListItem::new(Span::styled(format!("  {}", f), style))
        })
        .collect();

    let file_list = List::new(items).block(file_block);
    f.render_widget(file_list, outer[2]);

    // Hint bar
    let hint = if app.scan_complete {
        Paragraph::new(Line::from(vec![Span::styled(
            "Auto-advancing to RESOURCES view...",
            theme::dim(),
        )]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" Quit", theme::dim()),
        ]))
    };
    f.render_widget(hint.alignment(Alignment::Center), outer[3]);
}
