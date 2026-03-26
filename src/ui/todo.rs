use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};

use super::theme;
use crate::app::App;
use crate::todo::TodoStatus;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(" ⚡ TODO REVIEW ⚡ ", theme::accent_bold()))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    if app.todos.is_empty() {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("✓ ", theme::success()),
            Span::styled("No remaining TODOs — files are complete!", theme::success()),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(msg, area);
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("[Tab]", theme::accent_bold()),
            Span::styled(" back to Generator  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" quit", theme::dim()),
        ]))
        .alignment(Alignment::Center);
        // render hint at bottom
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        f.render_widget(hint, inner[1]);
        return;
    }

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .margin(1)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    render_list(f, app, split[0]);
    render_detail(f, app, split[1]);
}

fn todo_icon(status: &TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "·",
        TodoStatus::Loading => "⟳",
        TodoStatus::Resolved => "✓",
        TodoStatus::Skipped => "-",
    }
}

fn render_list(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let resolved = app
        .todos
        .iter()
        .filter(|t| t.status == TodoStatus::Resolved)
        .count();
    let total = app.todos.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .title(Span::styled(
            format!(" TODOs {}/{} ", resolved, total),
            theme::primary_bold(),
        ));

    let items: Vec<ListItem> = app
        .todos
        .iter()
        .enumerate()
        .map(|(i, todo)| {
            let icon = todo_icon(&todo.status);
            let is_current = i == app.todo_idx;

            let icon_style = match &todo.status {
                TodoStatus::Resolved => theme::success(),
                TodoStatus::Skipped => theme::dim(),
                TodoStatus::Loading => theme::warning(),
                TodoStatus::Pending => {
                    if is_current {
                        theme::accent()
                    } else {
                        theme::dim()
                    }
                }
            };

            let file_style = if is_current {
                theme::primary()
            } else {
                theme::muted()
            };

            let short = if todo.placeholder.len() > 22 {
                format!("{}…", &todo.placeholder[..21])
            } else {
                todo.placeholder.clone()
            };

            let line = Line::from(vec![
                Span::styled(format!(" {} ", icon), icon_style),
                Span::styled(format!("{}:{} ", todo.file, todo.line_no), theme::dim()),
                Span::styled(short, file_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.todo_idx));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(theme::accent_color())
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_detail(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),    // context + suggestion
            Constraint::Length(3), // input
            Constraint::Length(1), // hints
        ])
        .split(area);

    // ── Context + AI suggestion ───────────────────────────────────────────────
    let item = match app.todos.get(app.todo_idx) {
        Some(i) => i,
        None => return,
    };

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(
            format!(" {} line {} ", item.file, item.line_no),
            theme::muted(),
        ));

    let mut lines: Vec<Line> = Vec::new();

    // Placeholder header
    lines.push(Line::from(vec![
        Span::styled("  placeholder: ", theme::dim()),
        Span::styled(item.placeholder.clone(), theme::warning()),
    ]));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(4) as usize),
        theme::dim(),
    )));

    // Context lines (mark the TODO line with an arrow)
    for ctx_line in &item.context {
        if ctx_line == &item.line_content {
            lines.push(Line::from(vec![
                Span::styled("▶ ", theme::accent()),
                Span::styled(ctx_line.trim_start().to_string(), theme::warning()),
            ]));
        } else {
            lines.push(theme::highlight_hcl_line(ctx_line));
        }
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(4) as usize),
        theme::dim(),
    )));

    // AI suggestion or prompt
    match &item.status {
        TodoStatus::Loading => {
            let spin = theme::spinner(app.tick);
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", spin), theme::accent()),
                Span::styled("Asking AI...", theme::primary()),
            ]));
        }
        TodoStatus::Resolved => {
            if let Some(res) = &item.resolution {
                lines.push(Line::from(vec![
                    Span::styled("  ✓ resolved: ", theme::success()),
                    Span::styled(res.clone(), theme::success()),
                ]));
            }
        }
        TodoStatus::Skipped => {
            lines.push(Line::from(Span::styled("  - skipped", theme::dim())));
        }
        _ => {
            if let Some(suggestion) = &item.ai_suggestion {
                lines.push(Line::from(vec![
                    Span::styled("  AI suggests: ", theme::accent()),
                    Span::styled(suggestion.clone(), theme::warning()),
                ]));
                lines.push(Line::from(Span::styled(
                    "  Press ENTER (empty input) to accept AI suggestion",
                    theme::dim(),
                )));
            } else {
                let api_hint = if app.api_key.is_some() {
                    "[A] ask AI"
                } else {
                    "Set LLM_API_KEY env var for AI suggestions"
                };
                lines.push(Line::from(Span::styled(
                    format!("  {}", api_hint),
                    theme::dim(),
                )));
            }
        }
    }

    let detail = Paragraph::new(lines).block(detail_block);
    f.render_widget(detail, layout[0]);

    // ── Input field ───────────────────────────────────────────────────────────
    let (input_text, input_border_style, input_title) = if app.todo_input_active {
        let cursor = if (app.tick / 4).is_multiple_of(2) {
            "█"
        } else {
            " "
        };
        (
            format!("{}{}", app.todo_input, cursor),
            theme::accent(),
            " REPLACEMENT VALUE — typing ",
        )
    } else {
        (
            app.todo_input.clone(),
            theme::dim(),
            " REPLACEMENT VALUE — [Enter] to edit ",
        )
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(input_border_style)
        .title(Span::styled(input_title, theme::primary_bold()));

    let input = Paragraph::new(Span::styled(input_text, theme::primary())).block(input_block);
    f.render_widget(input, layout[1]);

    // ── Hints ─────────────────────────────────────────────────────────────────
    let has_key = app.api_key.is_some();
    let hints = if app.todo_input_active {
        // Typing mode — show editing hints only.
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", theme::accent_bold()),
            Span::styled(" apply  ", theme::dim()),
            Span::styled("[Esc]", theme::accent_bold()),
            Span::styled(" cancel  ", theme::dim()),
            Span::styled("[Tab]", theme::accent_bold()),
            Span::styled(" Generator", theme::dim()),
        ]))
    } else {
        // Browse mode — show navigation and command hints.
        Paragraph::new(Line::from(vec![
            Span::styled("[↑↓]", theme::accent_bold()),
            Span::styled(" nav  ", theme::dim()),
            Span::styled("[N]", theme::accent_bold()),
            Span::styled(" next pending  ", theme::dim()),
            Span::styled(
                "[A]",
                if has_key {
                    theme::accent_bold()
                } else {
                    theme::dim()
                },
            ),
            Span::styled(" AI suggest  ", theme::dim()),
            Span::styled("[Enter]", theme::accent_bold()),
            Span::styled(" edit  ", theme::dim()),
            Span::styled("[S]", theme::accent_bold()),
            Span::styled(" skip  ", theme::dim()),
            Span::styled("[Tab]", theme::accent_bold()),
            Span::styled(" Generator  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" quit", theme::dim()),
        ]))
    };
    let hints = hints.alignment(Alignment::Center);
    f.render_widget(hints, layout[2]);
}
