use ratatui::style::{Color, Modifier, Style};

// UpCloud purple palette (RGB)
pub const PRIMARY: Color = Color::Rgb(160, 100, 255); // light purple (was electric cyan)
pub const ACCENT: Color = Color::Rgb(200, 0, 255); // hot magenta-purple
pub const SUCCESS: Color = Color::Rgb(0, 255, 120); // neon green
pub const WARNING: Color = Color::Rgb(255, 200, 0); // electric amber
pub const DANGER: Color = Color::Rgb(255, 50, 80); // hot red
pub const DIM: Color = Color::Rgb(75, 60, 105); // muted purple-slate
pub const MUTED: Color = Color::Rgb(145, 125, 175); // lavender mid-tone
pub const WHITE: Color = Color::Rgb(210, 220, 240); // cool white
pub const HCL_KEY: Color = Color::Rgb(185, 145, 255); // lavender for HCL keys (was soft cyan)
pub const HCL_VAL: Color = Color::Rgb(255, 200, 80); // amber for HCL values
pub const HCL_KW: Color = Color::Rgb(200, 80, 255); // magenta for keywords
pub const UPCLOUD_PURPLE: Color = Color::Rgb(123, 0, 255); // UpCloud brand purple #7b00ff
pub const UPCLOUD_SPARKLE: Color = Color::Rgb(200, 140, 255); // bright sparkle tint

pub fn primary() -> Style {
    Style::default().fg(PRIMARY)
}
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}
pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
pub fn primary_bold() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
}
pub fn success() -> Style {
    Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
}
pub fn warning() -> Style {
    Style::default().fg(WARNING)
}
pub fn danger() -> Style {
    Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
}
pub fn dim() -> Style {
    Style::default().fg(DIM)
}
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
pub fn white_bold() -> Style {
    Style::default().fg(WHITE).add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::default()
        .fg(Color::Rgb(0, 0, 20))
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn status_style(label: &str) -> Style {
    match label {
        "NATIVE" => Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        "COMPATIBLE" => Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
        "PARTIAL" => Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        "UNSUPPORTED" => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(MUTED),
    }
}

pub fn status_icon(label: &str) -> &'static str {
    match label {
        "NATIVE" => "◆",
        "COMPATIBLE" => "◈",
        "PARTIAL" => "◇",
        "UNSUPPORTED" => "✕",
        "KEPT" => "≡",
        _ => "·",
    }
}

pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(tick: u64) -> &'static str {
    SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
}

/// UpCloud brand logo (34 lines of raw ASCII)
pub const UPCLOUD_LOGO: &[&str] = &[
    "                                    ################################                                ",
    "                                    ###################################                             ",
    "                                                                ########                            ",
    "                                                                 ########                           ",
    "                                                              ##########                            ",
    "     #################################################################                              ",
    "  ###############################################################                                   ",
    "#########                                                                                           ",
    "########                                                                                            ",
    " #########                                                                                          ",
    "   #########################         ########        ##########################################     ",
    "        ####################         ########        #############################################  ",
    "                                     ########                                              #########",
    "                                     ########                                               ########",
    "                                     ########                                            ########## ",
    "                                     ############################################################   ",
    "                                     #######################################################        ",
];

/// Simple line-by-line HCL syntax highlighter.
/// Returns owned Lines suitable for use in a ratatui Paragraph.
pub fn highlight_hcl_line(line: &str) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};

    let trimmed = line.trim_start();

    // Comments
    if trimmed.starts_with('#') {
        return Line::from(Span::styled(line.to_owned(), dim()));
    }

    // Closing brace
    if trimmed == "}" || trimmed == "}}" {
        return Line::from(Span::styled(line.to_owned(), dim()));
    }

    // resource / data block declaration
    if trimmed.starts_with("resource \"")
        || trimmed.starts_with("data \"")
        || trimmed.starts_with("locals")
        || trimmed.starts_with("variable \"")
    {
        // Color: keyword in HCL_KW, rest of line in PRIMARY
        let parts: Vec<&str> = trimmed.splitn(2, '"').collect();
        let kw = parts[0];
        let rest = &trimmed[kw.len()..];
        let indent = &line[..line.len() - trimmed.len()];
        return Line::from(vec![
            Span::styled(indent.to_owned(), dim()),
            Span::styled(
                kw.trim_end().to_owned(),
                Style::default().fg(HCL_KW).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", rest),
                Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    // attribute = value (handles both `key = "val"` and `key = ref`)
    if let Some(eq_pos) = trimmed.find(" = ") {
        let indent_len = line.len() - trimmed.len();
        let indent = &line[..indent_len];
        let key = &trimmed[..eq_pos];
        let value = &trimmed[eq_pos + 3..];
        return Line::from(vec![
            Span::styled(indent.to_owned(), dim()),
            Span::styled(key.to_owned(), Style::default().fg(HCL_KEY)),
            Span::styled(" = ".to_owned(), dim()),
            Span::styled(value.to_owned(), Style::default().fg(HCL_VAL)),
        ]);
    }

    // Block opener (e.g. `network_interface {`)
    if trimmed.ends_with('{') {
        let indent_len = line.len() - trimmed.len();
        let indent = &line[..indent_len];
        let body = trimmed.trim_end_matches('{').trim_end();
        return Line::from(vec![
            Span::styled(indent.to_owned(), dim()),
            Span::styled(body.to_owned(), Style::default().fg(PRIMARY)),
            Span::styled(" {".to_owned(), dim()),
        ]);
    }

    Line::from(Span::styled(line.to_owned(), muted()))
}
