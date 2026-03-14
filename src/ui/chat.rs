use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::App;
use crate::ui::theme::*;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    // Outer block
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(accent())
        .title(Span::styled(" ▷ AI TERRAFORM ADVISOR ", accent_bold()));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Split into: messages | input line | hints
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),      // message history
            Constraint::Length(3),   // input box
            Constraint::Length(1),   // hint bar
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
        // Role header line
        if msg.is_user {
            lines.push(Line::from(vec![
                Span::styled("  ▶ ", primary()),
                Span::styled("YOU", primary_bold()),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  ◆ ", accent()),
                Span::styled("AI", accent_bold()),
            ]));
        }

        // Content lines — each source line indented
        let style = if msg.is_user {
            Style::default().fg(WHITE)
        } else {
            Style::default().fg(PRIMARY)
        };

        for content_line in msg.content.lines() {
            lines.push(Line::from(vec![
                Span::raw("     "),
                Span::styled(content_line.to_owned(), style),
            ]));
        }

        // Blank separator
        lines.push(Line::from(""));
    }

    // Thinking indicator
    if app.chat_loading {
        let frame = spinner(app.tick);
        lines.push(Line::from(vec![
            Span::styled("  ◆ ", accent()),
            Span::styled("AI  ", accent_bold()),
            Span::styled(format!("{} thinking…", frame), dim()),
        ]));
        lines.push(Line::from(""));
    }

    // Scroll: treat chat_scroll=9999 as "scroll to bottom"
    let total_lines = lines.len() as u16;
    let visible = area.height.saturating_sub(2); // inside block borders
    let max_scroll = total_lines.saturating_sub(visible);
    // Store max_scroll so the key handler can compute correct up/down movement.
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
                .border_style(dim())
                .title(Span::styled(" conversation ", dim())),
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
        (primary_bold(), primary())
    };

    let display_text = if app.chat_loading {
        Span::styled(" waiting for AI…", dim())
    } else {
        Span::styled(
            format!(" {}{}", app.chat_input, cursor),
            Style::default().fg(WHITE),
        )
    };

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" ▶ ", prefix_style),
        display_text,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" type your message ", dim())),
    );

    f.render_widget(input, area);
}

fn render_hints(f: &mut Frame, app: &App, area: Rect) {
    let scroll_hint = if app.chat_messages.len() > 3 {
        "  [↑↓]=Scroll"
    } else {
        ""
    };

    let no_api = if app.api_key.is_none() {
        "  ⚠ Set LLM_API_KEY"
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled("[ENTER]=Send", dim()),
        Span::styled(scroll_hint, dim()),
        Span::styled("  [Esc/Tab]=Back to Generator", dim()),
        Span::styled("  [Q]=Quit", dim()),
        Span::styled(no_api, warning()),
    ]);

    f.render_widget(Paragraph::new(line), area);
}
