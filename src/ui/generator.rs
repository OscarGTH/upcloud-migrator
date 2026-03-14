use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, GenStep};
use crate::zones::{zone_idx_to_visual_row, ZONES, ZONE_LIST_VISUAL_ROWS};
use super::theme;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme::primary())
        .title(Span::styled(" ⚡ FILE GENERATOR ⚡ ", theme::accent_bold()))
        .title_alignment(Alignment::Center);
    f.render_widget(outer_block, area);

    if app.gen_step == GenStep::AskZone {
        render_zone_picker(f, app);
    } else {
        render_generation_view(f, app);
    }
}

fn render_zone_picker(f: &mut Frame, app: &App) {
    let area = f.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(5),    // zone list
            Constraint::Length(1), // hint bar
        ])
        .split(area);

    // Build list items: region headers + zone rows
    let mut items: Vec<ListItem> = Vec::with_capacity(ZONE_LIST_VISUAL_ROWS);
    let mut last_region = "";

    for (idx, zone) in ZONES.iter().enumerate() {
        // Insert a region header whenever the region changes
        if zone.region != last_region {
            last_region = zone.region;
            let header = Line::from(vec![
                Span::styled(
                    format!(" ── {} ", zone.region),
                    theme::accent().add_modifier(Modifier::BOLD),
                ),
            ]);
            items.push(ListItem::new(header));
        }

        let is_hover = idx == app.zone_idx;
        let is_saved = zone.slug == app.target_zone;

        let prefix = if is_hover { "▶ " } else { "  " };
        let saved_marker = if is_saved { " ★" } else { "" };

        let slug_style = if is_hover {
            theme::primary().add_modifier(Modifier::BOLD)
        } else {
            theme::dim()
        };

        let row = Line::from(vec![
            Span::styled(format!("  {}{}", prefix, zone.slug), slug_style),
            Span::styled(
                format!("  {}{}", zone.city, saved_marker),
                if is_hover { theme::accent() } else { theme::dim() },
            ),
        ]);
        items.push(ListItem::new(row));
    }

    // Set ListState to highlight the hovered row
    let mut list_state = ListState::default();
    list_state.select(Some(zone_idx_to_visual_row(app.zone_idx)));

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .title(Span::styled(
            " SELECT TARGET ZONE  [↑↓ / j·k]  ENTER to confirm  ★ = last used ",
            theme::primary_bold(),
        ));

    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(list, layout[0], &mut list_state);

    // Hint bar
    let hints = Paragraph::new(Line::from(vec![
        Span::styled("[↑↓]", theme::accent_bold()),
        Span::styled(" Navigate  ", theme::dim()),
        Span::styled("[ENTER]", theme::accent_bold()),
        Span::styled(" Select  ", theme::dim()),
        Span::styled("[Tab]", theme::accent_bold()),
        Span::styled(" Back  ", theme::dim()),
        Span::styled("[Q]", theme::accent_bold()),
        Span::styled(" Quit", theme::dim()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hints, layout[1]);
}

fn render_generation_view(f: &mut Frame, app: &App) {
    let area = f.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // confirmed zone
            Constraint::Length(3), // output dir input
            Constraint::Min(5),    // log
            Constraint::Length(2), // summary
            Constraint::Length(1), // hints
        ])
        .split(area);

    // Confirmed zone (read-only)
    let zone_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(" TARGET ZONE ", theme::dim()));
    let zone_widget = Paragraph::new(Span::styled(
        app.target_zone.clone(),
        theme::success(),
    ))
    .block(zone_block);
    f.render_widget(zone_widget, layout[0]);

    // Output dir input
    let outdir_active = app.gen_step == GenStep::AskOutputDir;
    let cursor = if outdir_active && (app.tick / 4) % 2 == 0 { "█" } else { " " };
    let outdir_text = if outdir_active {
        format!("{}{}", app.input_buf, cursor)
    } else if let Some(p) = &app.output_path {
        p.display().to_string()
    } else {
        String::new()
    };
    let outdir_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if outdir_active { theme::accent() } else { theme::dim() })
        .title(Span::styled(
            " OUTPUT DIRECTORY ",
            if outdir_active { theme::primary_bold() } else { theme::dim() },
        ));
    let outdir_widget = Paragraph::new(Span::styled(
        outdir_text,
        if outdir_active { theme::primary() } else { theme::dim() },
    ))
    .block(outdir_block);
    f.render_widget(outdir_widget, layout[1]);

    // Generation log
    let log_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dim())
        .title(Span::styled(" GENERATION LOG ", theme::primary()));

    let log_height = layout[2].height.saturating_sub(2) as usize;
    let log_items: Vec<ListItem> = app
        .gen_log
        .iter()
        .rev()
        .take(log_height)
        .rev()
        .map(|line| {
            let style = if line.contains("[OK]") {
                theme::success()
            } else if line.contains("[ERR]") {
                theme::danger()
            } else if line.starts_with(">>") {
                theme::accent()
            } else {
                theme::dim()
            };
            ListItem::new(Span::styled(line.clone(), style))
        })
        .collect();

    let log_list = List::new(log_items).block(log_block);
    f.render_widget(log_list, layout[2]);

    // Summary line
    let summary = if app.gen_complete {
        Paragraph::new(Line::from(vec![
            Span::styled("✓ DONE — ", theme::success()),
            Span::styled(
                format!("Generated {} files", app.gen_files_count),
                Style::default()
                    .fg(theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
    } else if app.is_generating {
        let spin = theme::spinner(app.tick);
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{} ", spin), theme::accent()),
            Span::styled("Generating...", theme::primary()),
        ]))
    } else {
        Paragraph::new(Line::from(Span::styled(
            "Enter output directory and press ENTER to generate.",
            theme::dim(),
        )))
    };
    f.render_widget(summary.alignment(Alignment::Center), layout[3]);

    // Hints
    let hints = if app.gen_complete {
        Paragraph::new(Line::from(vec![
            Span::styled("[D]", theme::accent_bold()),
            Span::styled(" diff  ", theme::dim()),
            Span::styled("[T]", theme::accent_bold()),
            Span::styled(" TODOs  ", theme::dim()),
            Span::styled("[C]", theme::accent_bold()),
            Span::styled(" chat  ", theme::dim()),
            Span::styled("[V]", theme::accent_bold()),
            Span::styled(" validate  ", theme::dim()),
            Span::styled("[Tab]", theme::accent_bold()),
            Span::styled(" Resources  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" Quit", theme::dim()),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("[ENTER]", theme::accent_bold()),
            Span::styled(" Generate  ", theme::dim()),
            Span::styled("[Tab]", theme::accent_bold()),
            Span::styled(" Resources  ", theme::dim()),
            Span::styled("[Q]", theme::accent_bold()),
            Span::styled(" Quit", theme::dim()),
        ]))
    };
    f.render_widget(hints.alignment(Alignment::Center), layout[4]);
}
