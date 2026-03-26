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
    use crate::ui::theme::{ThemeMode, mode};
    if upcloud_type.contains("server") || upcloud_type.contains("kubernetes") {
        match mode() {
            ThemeMode::Dark => Color::Rgb(0, 210, 255), // electric cyan
            ThemeMode::Light => Color::Rgb(0, 100, 180), // deep cyan
        }
    } else if upcloud_type.contains("database") || upcloud_type.contains("valkey") {
        match mode() {
            ThemeMode::Dark => Color::Rgb(200, 80, 255), // magenta
            ThemeMode::Light => Color::Rgb(140, 0, 200), // deep magenta
        }
    } else if upcloud_type.contains("loadbalancer") {
        match mode() {
            ThemeMode::Dark => Color::Rgb(255, 160, 0), // amber
            ThemeMode::Light => Color::Rgb(160, 90, 0), // dark amber
        }
    } else if upcloud_type.contains("storage") {
        match mode() {
            ThemeMode::Dark => Color::Rgb(0, 210, 130), // teal
            ThemeMode::Light => Color::Rgb(0, 130, 80), // dark teal
        }
    } else if upcloud_type.contains("gateway") {
        match mode() {
            ThemeMode::Dark => Color::Rgb(100, 200, 255), // light cyan
            ThemeMode::Light => Color::Rgb(0, 90, 170),   // deep blue
        }
    } else {
        match mode() {
            ThemeMode::Dark => Color::Rgb(80, 90, 110),  // dim slate
            ThemeMode::Light => Color::Rgb(80, 70, 100), // medium slate
        }
    }
}

/// Color code based on monthly cost level.
fn cost_color(monthly: f64) -> Style {
    match monthly as u64 {
        0 => Style::default().fg(dim_color()),
        1..=30 => Style::default().fg(success_color()),
        31..=150 => Style::default().fg(primary_color()),
        151..=500 => Style::default().fg(warning_color()),
        _ => Style::default()
            .fg(danger_color())
            .add_modifier(Modifier::BOLD),
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme::warning_color())) // amber border
        .title(Span::styled(
            " ₿ UPCLOUD PRICING ESTIMATE ",
            Style::default()
                .fg(theme::warning_color())
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
                .fg(theme::hcl_kw_color())
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "NAME",
            Style::default()
                .fg(theme::hcl_kw_color())
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "PLAN / SIZE",
            Style::default()
                .fg(theme::hcl_kw_color())
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "€/MO",
            Style::default()
                .fg(theme::hcl_kw_color())
                .add_modifier(Modifier::BOLD),
        )),
    ])
    .style(Style::default()) // let terminal decide header bg
    .height(1);

    let rows: Vec<Row> = costs
        .iter()
        .map(|entry| {
            // No alternating row backgrounds — let terminal decide
            let row_bg = Color::Reset;

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
                    Style::default().fg(white_color()),
                )),
                Cell::from(Span::styled(
                    plan_display,
                    Style::default().fg(theme::primary_color()),
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
            .border_style(Style::default().fg(theme::dim_color()))
            .title(Span::styled(
                format!(" {} resources ", costs.len()),
                theme::muted(),
            )),
    )
    .column_spacing(1)
    .row_highlight_style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED),
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
        Style::default()
            .fg(success_color())
            .add_modifier(Modifier::BOLD)
    } else if total < 100.0 {
        Style::default()
            .fg(primary_color())
            .add_modifier(Modifier::BOLD)
    } else if total < 500.0 {
        Style::default()
            .fg(warning_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(danger_color())
            .add_modifier(Modifier::BOLD)
    };

    let line = Line::from(vec![
        Span::styled(
            "  ESTIMATED TOTAL: ",
            Style::default()
                .fg(theme::hcl_kw_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("€{:.2}/mo", total), total_style),
        Span::styled("  │  ", theme::dim()),
        Span::styled(
            format!("€{:.0}/yr", yearly),
            Style::default().fg(muted_color()),
        ),
        Span::styled("  │  ", theme::dim()),
        Span::styled(
            format!("{} priced", priced_count),
            Style::default().fg(warning_color()),
        ),
        Span::styled("  +  ", theme::dim()),
        Span::styled(
            format!("{} free", free_count),
            Style::default().fg(dim_color()),
        ),
        Span::styled("  (excludes: data transfer, IPs, EKS nodes)", theme::dim()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim());

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
