use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, TableState,
    },
};

use crate::app::App;
use crate::pricing::short_upcloud_type;
use crate::ui::theme::{self, *};

/// Category color based on UpCloud resource type.
fn category_color(upcloud_type: &str) -> Color {
    if upcloud_type.contains("server") || upcloud_type.contains("kubernetes") {
        Color::Rgb(0, 210, 255) // compute → electric cyan
    } else if upcloud_type.contains("database") || upcloud_type.contains("valkey") {
        Color::Rgb(200, 80, 255) // database → magenta
    } else if upcloud_type.contains("loadbalancer") {
        Color::Rgb(255, 160, 0) // lb → amber
    } else if upcloud_type.contains("storage") {
        Color::Rgb(0, 210, 130) // storage → teal
    } else if upcloud_type.contains("gateway") {
        Color::Rgb(100, 200, 255) // gateway → light cyan
    } else {
        Color::Rgb(80, 90, 110) // network/other → dim slate
    }
}

/// Color code based on monthly cost level.
fn cost_color(monthly: f64) -> Style {
    match monthly as u64 {
        0 => Style::default().fg(DIM),
        1..=30 => Style::default().fg(SUCCESS),
        31..=150 => Style::default().fg(PRIMARY),
        151..=500 => Style::default().fg(WARNING),
        _ => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(255, 160, 0))) // amber border
        .title(Span::styled(
            " ₿ UPCLOUD PRICING ESTIMATE ",
            Style::default()
                .fg(Color::Rgb(255, 160, 0))
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);

    let inner = outer.inner(area);
    f.render_widget(outer, area);

    if !app.gen_complete {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Generate files first, then open pricing with [P].",
                theme::dim(),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::dim())
                .border_type(BorderType::Rounded),
        );
        f.render_widget(msg, inner);
        return;
    }

    // Layout: table | summary bar | hints
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // resource table
            Constraint::Length(3), // totals
            Constraint::Length(1), // hints
        ])
        .split(inner);

    render_table(f, app, chunks[0]);
    render_totals(f, app, chunks[1]);
    render_hints(f, chunks[2]);
}

fn render_table(f: &mut Frame, app: &App, area: Rect) {
    let costs = &app.pricing_costs;

    if costs.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No priced resources found in the generated output.",
                theme::dim(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  (Free resources: networks, routers, firewalls are €0/mo)",
                theme::muted(),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme::dim()),
        );
        f.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from(Span::styled(
            "TYPE",
            Style::default()
                .fg(Color::Rgb(200, 80, 255))
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "NAME",
            Style::default()
                .fg(Color::Rgb(200, 80, 255))
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "PLAN / SIZE",
            Style::default()
                .fg(Color::Rgb(200, 80, 255))
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "€/MO",
            Style::default()
                .fg(Color::Rgb(200, 80, 255))
                .add_modifier(Modifier::BOLD),
        )),
    ])
    .style(Style::default().bg(Color::Rgb(20, 10, 40))) // dark purple header bg
    .height(1);

    let rows: Vec<Row> = costs
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            // Alternate row background for readability
            let row_bg = if i % 2 == 0 {
                Color::Rgb(8, 12, 25) // very dark blue-black
            } else {
                Color::Rgb(12, 18, 35) // slightly lighter dark blue
            };

            let type_color = category_color(&entry.upcloud_type);
            let short_type = short_upcloud_type(&entry.upcloud_type);

            let plan_display = if entry.plan.is_empty() {
                "—".to_string()
            } else {
                entry.plan.clone()
            };

            let cost_str = if entry.monthly_eur == 0.0 {
                "free".to_string()
            } else {
                format!("{:.2}", entry.monthly_eur)
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    short_type.to_string(),
                    Style::default().fg(type_color),
                )),
                Cell::from(Span::styled(
                    entry.resource_name.clone(),
                    Style::default().fg(WHITE),
                )),
                Cell::from(Span::styled(
                    plan_display,
                    Style::default().fg(Color::Rgb(130, 200, 255)), // soft cyan
                )),
                Cell::from(Span::styled(cost_str, cost_color(entry.monthly_eur))),
            ])
            .style(Style::default().bg(row_bg))
            .height(1)
        })
        .collect();

    let visible_rows = area.height.saturating_sub(4) as usize; // header + borders
    let max_scroll = costs.len().saturating_sub(visible_rows);
    let scroll = app.pricing_scroll.min(max_scroll);

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Min(16),
            Constraint::Min(20),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(60, 40, 80))) // dark purple border
            .title(Span::styled(
                format!(" {} resources ", costs.len()),
                theme::muted(),
            )),
    )
    .column_spacing(1)
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(40, 20, 70))
            .add_modifier(Modifier::BOLD),
    );

    let mut table_state = TableState::default();
    table_state.select(None);
    // Apply scroll offset manually: skip first `scroll` rows
    // Use ratatui's offset support
    *table_state.offset_mut() = scroll;

    f.render_stateful_widget(table, area, &mut table_state);

    // Scrollbar
    if max_scroll > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        let mut sb_state = ScrollbarState::new(costs.len())
            .position(scroll)
            .viewport_content_length(visible_rows);
        let sb_area = Rect::new(area.x + area.width - 1, area.y + 1, 1, area.height - 2);
        f.render_stateful_widget(scrollbar, sb_area, &mut sb_state);
    }
}

fn render_totals(f: &mut Frame, app: &App, area: Rect) {
    let costs = &app.pricing_costs;
    let total: f64 = costs.iter().map(|e| e.monthly_eur).sum();
    let yearly = total * 12.0;

    let priced_count = costs.iter().filter(|e| e.monthly_eur > 0.0).count();
    let free_count = costs.iter().filter(|e| e.monthly_eur == 0.0).count();

    let total_style = if total == 0.0 {
        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
    } else if total < 100.0 {
        Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
    } else if total < 500.0 {
        Style::default().fg(WARNING).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    };

    let line = Line::from(vec![
        Span::styled(
            "  ESTIMATED TOTAL: ",
            Style::default()
                .fg(Color::Rgb(200, 80, 255))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("€{:.2}/mo", total), total_style),
        Span::styled("  │  ", theme::dim()),
        Span::styled(format!("€{:.0}/yr", yearly), Style::default().fg(MUTED)),
        Span::styled("  │  ", theme::dim()),
        Span::styled(
            format!("{} priced", priced_count),
            Style::default().fg(WARNING),
        ),
        Span::styled("  +  ", theme::dim()),
        Span::styled(format!("{} free", free_count), Style::default().fg(DIM)),
        Span::styled("  (excludes: data transfer, IPs, EKS nodes)", theme::dim()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(60, 40, 80)))
        .style(Style::default().bg(Color::Rgb(15, 8, 30))); // dark purple bg for totals

    f.render_widget(Paragraph::new(line).block(block), area);
}

fn render_hints(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled("[↑↓ / j·k]", theme::accent_bold()),
        Span::styled(" Scroll  ", theme::dim()),
        Span::styled("[Esc / Tab]", theme::accent_bold()),
        Span::styled(" Back to Generator  ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" Quit", theme::dim()),
    ]);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}
