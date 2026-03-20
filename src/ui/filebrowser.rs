use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};

use super::theme;
use crate::app::App;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(
            " ⚡ TERRAFORM DIRECTORY BROWSER ⚡ ",
            theme::accent_bold(),
        ))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1), // current path
            Constraint::Length(1), // tf file count in current dir
            Constraint::Min(4),    // directory list
            Constraint::Length(1), // hint bar
        ])
        .split(area);

    // Current directory line
    let cwd_str = app.fb_cwd.display().to_string();
    let tf_count = app.fb_entries.iter().filter(|(_, is_dir)| !is_dir).count();
    let cwd_style = if tf_count > 0 {
        theme::success()
    } else {
        theme::warning()
    };

    let cwd_widget = Paragraph::new(Line::from(vec![
        Span::styled("  DIR: ", theme::dim()),
        Span::styled(cwd_str, cwd_style),
    ]));
    f.render_widget(cwd_widget, layout[0]);

    // .tf file indicator
    let tf_line = if tf_count > 0 {
        Line::from(vec![
            Span::styled("  ◆ ", theme::success()),
            Span::styled(
                format!(
                    "{} .tf file{} found — press [S] to scan this directory",
                    tf_count,
                    if tf_count == 1 { "" } else { "s" }
                ),
                theme::success(),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ◇ ", theme::dim()),
            Span::styled(
                "No .tf files here — navigate to your terraform directory",
                theme::dim(),
            ),
        ])
    };
    f.render_widget(Paragraph::new(tf_line), layout[1]);

    // Directory list
    let items: Vec<ListItem> = app
        .fb_entries
        .iter()
        .map(|(name, is_dir)| {
            if name == "[..]" {
                ListItem::new(Line::from(vec![
                    Span::styled("  ↑ ", theme::accent()),
                    Span::styled(".. (go up)", theme::accent()),
                ]))
            } else if *is_dir {
                ListItem::new(Line::from(vec![
                    Span::styled("  ▶ ", theme::primary()),
                    Span::styled(name.clone(), theme::primary()),
                    Span::styled("/", theme::dim()),
                ]))
            } else {
                // .tf file
                ListItem::new(Line::from(vec![
                    Span::styled("    ", theme::dim()),
                    Span::styled(name.clone(), theme::success()),
                ]))
            }
        })
        .collect();

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .title(Span::styled(" NAVIGATE ", theme::accent_bold()));

    let list = List::new(items)
        .block(list_block)
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");

    let mut state = app.fb_state.clone();
    f.render_stateful_widget(list, layout[2], &mut state);

    // Hint bar
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[↑↓]", theme::accent_bold()),
        Span::styled(" navigate  ", theme::dim()),
        Span::styled("[Enter]", theme::accent_bold()),
        Span::styled(" open dir  ", theme::dim()),
        Span::styled("[S]", theme::accent_bold()),
        Span::styled(" scan current dir  ", theme::dim()),
        Span::styled("[Esc]", theme::accent_bold()),
        Span::styled(" back  ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, layout[3]);
}
