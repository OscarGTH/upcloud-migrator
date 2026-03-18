use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::app::App;
use crate::ui::theme::*;

// Distinct colors for the chat UI
const USER_BG:   Color = Color::Rgb(10, 20, 45);   // deep navy for user messages
const AI_BG:     Color = Color::Rgb(15, 8,  30);   // deep purple for AI messages
const USER_FG:   Color = Color::Rgb(200, 170, 255); // light lavender text
const AI_FG:     Color = Color::Rgb(220, 180, 255); // light magenta text
const AI_BORDER: Color = Color::Rgb(120, 0, 200);   // vivid purple border

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(AI_BORDER))
        .title(Span::styled(
            " ◆ AI TERRAFORM ADVISOR ◆ ",
            Style::default().fg(AI_BORDER).add_modifier(Modifier::BOLD),
        ));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    render_messages(f, app, chunks[0]);
    render_input(f, app, chunks[1]);
    render_hints(f, app, chunks[2]);
}

fn render_messages(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    if app.chat_messages.is_empty() && !app.chat_loading {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No messages yet. Type a question below.",
            dim(),
        )));
        if app.api_key.is_none() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ⚠  LLM_API_KEY not set — AI features unavailable.",
                warning(),
            )));
        }
    }

    for msg in &app.chat_messages {
        if msg.is_user {
            // User header: bold light purple on dark navy
            lines.push(Line::from(vec![
                Span::styled(
                    "  ▶ YOU ",
                    Style::default()
                        .fg(Color::Rgb(160, 100, 255))
                        .bg(USER_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            for content_line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("     {}", content_line),
                        Style::default().fg(USER_FG).bg(USER_BG),
                    ),
                ]));
            }
        } else {
            // AI header: vivid magenta on dark purple
            lines.push(Line::from(vec![
                Span::styled(
                    "  ◆ AI  ",
                    Style::default()
                        .fg(Color::Rgb(230, 100, 255))
                        .bg(AI_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            for content_line in msg.content.lines() {
                // Highlight code blocks differently (lines starting with ``` or indented 4 spaces)
                let (text, style) = if content_line.starts_with("```")
                    || content_line.starts_with("    ")
                {
                    (
                        format!("     {}", content_line),
                        Style::default()
                            .fg(Color::Rgb(185, 145, 255))
                            .bg(Color::Rgb(15, 6, 30)),
                    )
                } else if content_line.trim_start().starts_with('#')
                    || content_line.trim_start().starts_with("**")
                {
                    (
                        format!("     {}", content_line),
                        Style::default()
                            .fg(Color::Rgb(255, 200, 80))
                            .bg(AI_BG)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        format!("     {}", content_line),
                        Style::default().fg(AI_FG).bg(AI_BG),
                    )
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
        }
        lines.push(Line::from(""));
    }

    if app.chat_loading {
        let frame = spinner(app.tick);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  ◆ AI  {} thinking…", frame),
                Style::default()
                    .fg(Color::Rgb(230, 100, 255))
                    .bg(AI_BG)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Estimate wrapped line count for scroll calculation.
    // Each logical line wraps based on content width (inner width - 2 for borders - 5 indent).
    let inner_width = area.width.saturating_sub(4).max(1) as usize;
    let rendered_rows: u16 = lines
        .iter()
        .map(|l| {
            let len: usize = l.spans.iter().map(|s| s.content.len()).sum();
            ((len.max(1) + inner_width - 1) / inner_width) as u16
        })
        .sum();

    let visible = area.height.saturating_sub(2);
    let max_scroll = rendered_rows.saturating_sub(visible);
    app.chat_scroll_max.set(max_scroll);

    let scroll_y = if app.chat_scroll as u16 >= max_scroll {
        max_scroll
    } else {
        app.chat_scroll as u16
    };

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(60, 30, 90)))
                .title(Span::styled(
                    " conversation ",
                    Style::default().fg(Color::Rgb(100, 60, 140)),
                )),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));

    f.render_widget(para, area);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let cursor = if app.tick % 8 < 4 { "█" } else { " " };
    let (prefix_style, border_style) = if app.chat_loading {
        (dim(), dim())
    } else {
        (
            Style::default()
                .fg(Color::Rgb(160, 100, 255))
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Rgb(130, 70, 220)),
        )
    };

    let display_text = if app.chat_loading {
        Span::styled(" waiting for AI…", dim())
    } else {
        Span::styled(
            format!(" {}{}", app.chat_input, cursor),
            Style::default().fg(Color::Rgb(220, 205, 255)),
        )
    };

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" ▶ ", prefix_style),
        display_text,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(
                " type your message ",
                Style::default().fg(Color::Rgb(60, 120, 160)),
            )),
    );

    f.render_widget(input, area);
}

fn render_hints(f: &mut Frame, app: &App, area: Rect) {
    let no_api = if app.api_key.is_none() {
        "  ⚠ Set LLM_API_KEY"
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled("[ENTER]", accent_bold()),
        Span::styled(" Send  ", dim()),
        Span::styled("[↑↓ / j·k]", accent_bold()),
        Span::styled(" Scroll  ", dim()),
        Span::styled("[Esc/Tab]", accent_bold()),
        Span::styled(" Back  ", dim()),
        Span::styled("[Q]", accent_bold()),
        Span::styled(" Quit", dim()),
        Span::styled(no_api, warning()),
    ]);

    f.render_widget(Paragraph::new(line), area);
}
