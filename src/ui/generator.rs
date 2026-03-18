use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, GenStep};
use crate::zones::{zone_idx_to_visual_row, ZONES, ZONE_LIST_VISUAL_ROWS};
use super::theme;

// How many ticks to show the victory animation after generation completes.
const VICTORY_TICKS: u64 = 60; // ~3 seconds at 50ms tick

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    if app.gen_step == GenStep::AskZone {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(theme::primary())
            .title(Span::styled(" ⚡ FILE GENERATOR ⚡ ", theme::accent_bold()))
            .title_alignment(Alignment::Center);
        f.render_widget(outer, area);
        render_zone_picker(f, app);
    } else if app.is_generating
        || (app.gen_complete
            && app.tick.saturating_sub(app.gen_complete_tick) <= VICTORY_TICKS)
    {
        render_generating_animation(f, app);
    } else {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(theme::primary())
            .title(Span::styled(" ⚡ FILE GENERATOR ⚡ ", theme::accent_bold()))
            .title_alignment(Alignment::Center);
        f.render_widget(outer, area);
        render_generation_view(f, app);
    }
}

// ── Animation ────────────────────────────────────────────────────────────────

fn render_generating_animation(f: &mut Frame, app: &App) {
    let area = f.area();
    let tick = app.tick;
    let victory = app.gen_complete;
    let victory_age = tick.saturating_sub(app.gen_complete_tick);

    // Animated outer border: cycles PRIMARY↔ACCENT during generation,
    // flashes SUCCESS on completion.
    let border_style = if victory {
        if (victory_age / 3) % 2 == 0 {
            theme::success()
        } else {
            theme::primary()
        }
    } else {
        match (tick / 12) % 3 {
            0 => theme::primary(),
            1 => theme::accent(),
            _ => Style::default().fg(Color::Rgb(100, 0, 255)), // deep purple
        }
    };

    let title = if victory {
        " ✓ MIGRATION COMPLETE ✓ "
    } else {
        " ⚡ FILE GENERATOR ⚡ "
    };
    let title_style = if victory {
        theme::success()
    } else {
        theme::accent_bold()
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(border_style)
        .title(Span::styled(title, title_style))
        .title_alignment(Alignment::Center);
    f.render_widget(outer, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width < 4 || inner.height < 4 {
        return;
    }

    // Matrix rain fills the entire inner area — no text overlay.
    render_matrix_rain(f, tick, inner, victory);
}

// ── Matrix rain ───────────────────────────────────────────────────────────────

fn render_matrix_rain(f: &mut Frame, tick: u64, area: Rect, victory: bool) {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(area.height as usize);

    for row in 0..area.height {
        // RLE-encode spans by color to keep allocation count low.
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut run_color: Option<Color> = None;
        let mut run_buf = String::new();

        for col in 0..area.width {
            let (ch, color) = rain_cell(col, row, tick, area.height, victory);
            if run_color == Some(color) {
                run_buf.push(ch);
            } else {
                if !run_buf.is_empty() {
                    let style = run_color
                        .map(|c| Style::default().fg(c))
                        .unwrap_or_default();
                    spans.push(Span::styled(run_buf.clone(), style));
                    run_buf.clear();
                }
                run_color = Some(color);
                run_buf.push(ch);
            }
        }
        if !run_buf.is_empty() {
            let style = run_color
                .map(|c| Style::default().fg(c))
                .unwrap_or_default();
            spans.push(Span::styled(run_buf, style));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn rain_cell(col: u16, row: u16, tick: u64, height: u16, victory: bool) -> (char, Color) {
    if height == 0 {
        return (' ', Color::Reset);
    }

    // Only draw on even columns — gives a sparse, readable rain.
    if col % 2 == 1 {
        return (' ', Color::Reset);
    }

    let c = col as u64;
    let r = row as u64;
    let h = height as u64;

    // Each column has its own speed (1–3) and start phase.
    let speed = (c.wrapping_mul(7) % 3) + 1;
    let phase = c.wrapping_mul(97) % (h * 2);
    let head = (tick.wrapping_mul(speed).wrapping_add(phase)) / 3 % h;

    // Distance below the head (wraps around).
    let dist = if r >= head { r - head } else { h - head + r };
    let trail_len = (c.wrapping_mul(31) % 5) + 5; // 5–9 chars

    if dist > trail_len {
        return (' ', Color::Reset);
    }

    // Character changes slowly (every 5 ticks) to give a "flicker" feel.
    let char_seed = c
        .wrapping_mul(999_983)
        .wrapping_add(r.wrapping_mul(7_919))
        .wrapping_add(tick / 5);

    const CHARS: &[char] = &[
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
        '┼', '╬', '│', '─', '╭', '╮', '╰', '╯', '░', '▒', '>', '<', '=', '+', '~',
    ];
    let ch = CHARS[(char_seed as usize) % CHARS.len()];

    // Head is bright white; trail fades to near-black.
    let color = if victory {
        // Purple rain on victory.
        match dist {
            0 => Color::Rgb(255, 255, 255),
            1 => Color::Rgb(160, 100, 255),
            2 => Color::Rgb(120,  65, 220),
            3 => Color::Rgb( 90,  40, 180),
            4 => Color::Rgb( 65,  20, 140),
            5 | 6 => Color::Rgb(40, 10, 100),
            _ => Color::Rgb(22, 5, 60),
        }
    } else {
        // Classic green Matrix rain during generation.
        match dist {
            0 => Color::Rgb(220, 255, 220),
            1 => Color::Rgb(0, 255, 120),
            2 => Color::Rgb(0, 200, 100),
            3 => Color::Rgb(0, 150, 75),
            4 => Color::Rgb(0, 100, 50),
            5 | 6 => Color::Rgb(0, 55, 28),
            _ => Color::Rgb(0, 25, 13),
        }
    };

    (ch, color)
}

// ── Center overlay ────────────────────────────────────────────────────────────

// ── Zone picker ───────────────────────────────────────────────────────────────

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

    let mut items: Vec<ListItem> = Vec::with_capacity(ZONE_LIST_VISUAL_ROWS);
    let mut last_region = "";

    for (idx, zone) in ZONES.iter().enumerate() {
        if zone.region != last_region {
            last_region = zone.region;
            let header = Line::from(vec![Span::styled(
                format!(" ── {} ", zone.region),
                theme::accent().add_modifier(Modifier::BOLD),
            )]);
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

// ── Post-animation generation view (unchanged) ────────────────────────────────

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
    let log_lines: Vec<Line> = app
        .gen_log
        .iter()
        .filter(|line| {
            line.starts_with(">>")
                || line.contains("[OK]")
                || line.contains("[ERR]")
                || line.contains("[terraform fmt]")
        })
        .map(|line| {
            let style = if line.contains("[OK]") || line.contains("[terraform fmt] OK") {
                theme::success()
            } else if line.contains("[ERR]") {
                theme::danger()
            } else if line.starts_with(">>") {
                theme::accent()
            } else {
                theme::dim()
            };
            Line::from(Span::styled(line.clone(), style))
        })
        .collect();

    let total_log = log_lines.len();
    let log_scroll = (total_log.saturating_sub(log_height) as u16, 0);
    let log_widget = Paragraph::new(log_lines)
        .block(log_block)
        .wrap(Wrap { trim: false })
        .scroll(log_scroll);
    f.render_widget(log_widget, layout[2]);

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
            Span::styled("[P]", theme::accent_bold()),
            Span::styled(" pricing  ", theme::dim()),
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
