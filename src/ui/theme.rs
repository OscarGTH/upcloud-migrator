use ratatui::style::{Color, Modifier, Style};

// ── Theme detection ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Dark,
    Light,
}

fn detect_theme_mode() -> ThemeMode {
    // Explicit user override: UPCLOUD_MIGRATE_THEME=light|dark
    if let Ok(val) = std::env::var("UPCLOUD_MIGRATE_THEME") {
        if val.eq_ignore_ascii_case("light") {
            return ThemeMode::Light;
        }
        if val.eq_ignore_ascii_case("dark") {
            return ThemeMode::Dark;
        }
    }
    // COLORFGBG is set by many terminals: format is "fg_index;bg_index"
    // bg_index 7 = light gray, 15 = white → light terminal background
    if let Ok(val) = std::env::var("COLORFGBG")
        && let Some(bg) = val.split(';').next_back()
        && let Ok(n) = bg.trim().parse::<u8>()
        && matches!(n, 7 | 15)
    {
        return ThemeMode::Light;
    }
    // TERM_BACKGROUND_COLOR is set by some terminals (e.g. Terminal.app) as an RGB hex string.
    // A high luminance value indicates a light background.
    if let Ok(val) = std::env::var("TERM_BACKGROUND_COLOR") {
        // Example value: "#ffffff" or "rgb:ffff/ffff/ffff"
        let s = val.trim_start_matches('#');
        if s.len() == 6
            && let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&s[0..2], 16),
                u8::from_str_radix(&s[2..4], 16),
                u8::from_str_radix(&s[4..6], 16),
            )
        {
            // Perceived luminance; threshold 128
            let lum = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
            if lum > 128 {
                return ThemeMode::Light;
            }
        }
    }
    ThemeMode::Dark
}

pub fn mode() -> ThemeMode {
    use std::sync::OnceLock;
    static MODE: OnceLock<ThemeMode> = OnceLock::new();
    *MODE.get_or_init(detect_theme_mode)
}

// ── Color accessors (runtime-selected per theme) ─────────────────────────────

pub fn primary_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(160, 100, 255), // light purple
        ThemeMode::Light => Color::Rgb(100, 0, 200),  // deep violet
    }
}
pub fn accent_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(200, 0, 255), // hot magenta-purple
        ThemeMode::Light => Color::Rgb(155, 0, 200), // deep magenta
    }
}
pub fn success_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(0, 255, 120), // neon green
        ThemeMode::Light => Color::Rgb(0, 140, 60), // forest green
    }
}
pub fn warning_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(255, 200, 0), // electric amber
        ThemeMode::Light => Color::Rgb(170, 100, 0), // dark amber
    }
}
pub fn danger_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(255, 50, 80), // hot red
        ThemeMode::Light => Color::Rgb(200, 0, 30), // deep red
    }
}
pub fn dim_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(75, 60, 105), // muted purple-slate
        ThemeMode::Light => Color::Rgb(130, 115, 160), // medium gray-lavender
    }
}
pub fn muted_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(145, 125, 175), // lavender mid-tone
        ThemeMode::Light => Color::Rgb(70, 55, 110),  // dark lavender
    }
}
pub fn white_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(210, 220, 240), // cool white
        ThemeMode::Light => Color::Rgb(20, 10, 50),   // near-black on light bg
    }
}
pub fn hcl_key_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(185, 145, 255), // lavender
        ThemeMode::Light => Color::Rgb(80, 0, 180),   // deep indigo
    }
}
pub fn hcl_val_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(255, 200, 80), // amber
        ThemeMode::Light => Color::Rgb(150, 80, 0),  // dark amber/brown
    }
}
pub fn hcl_kw_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(200, 80, 255), // magenta
        ThemeMode::Light => Color::Rgb(155, 0, 200), // deep magenta
    }
}
pub fn upcloud_purple_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(123, 0, 255), // brand #7b00ff
        ThemeMode::Light => Color::Rgb(90, 0, 200), // darker brand purple
    }
}
pub fn upcloud_sparkle_color() -> Color {
    match mode() {
        ThemeMode::Dark => Color::Rgb(200, 140, 255), // bright sparkle
        ThemeMode::Light => Color::Rgb(110, 0, 220),  // deep sparkle
    }
}

// ── Style helpers ────────────────────────────────────────────────────────────

pub fn primary() -> Style {
    Style::default().fg(primary_color())
}
pub fn accent() -> Style {
    Style::default().fg(accent_color())
}
pub fn accent_bold() -> Style {
    Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD)
}
pub fn primary_bold() -> Style {
    Style::default()
        .fg(primary_color())
        .add_modifier(Modifier::BOLD)
}
pub fn success() -> Style {
    Style::default()
        .fg(success_color())
        .add_modifier(Modifier::BOLD)
}
pub fn warning() -> Style {
    Style::default().fg(warning_color())
}
pub fn danger() -> Style {
    Style::default()
        .fg(danger_color())
        .add_modifier(Modifier::BOLD)
}
pub fn dim() -> Style {
    Style::default().fg(dim_color())
}
pub fn muted() -> Style {
    Style::default().fg(muted_color())
}
pub fn white_bold() -> Style {
    Style::default()
        .fg(white_color())
        .add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 255, 255))
        .bg(accent_color())
        .add_modifier(Modifier::BOLD)
}

pub fn status_style(label: &str) -> Style {
    match label {
        "NATIVE" => Style::default()
            .fg(success_color())
            .add_modifier(Modifier::BOLD),
        "COMPATIBLE" => Style::default()
            .fg(primary_color())
            .add_modifier(Modifier::BOLD),
        "PARTIAL" => Style::default()
            .fg(warning_color())
            .add_modifier(Modifier::BOLD),
        "UNSUPPORTED" => Style::default()
            .fg(danger_color())
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(muted_color()),
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
                Style::default()
                    .fg(hcl_kw_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", rest),
                Style::default()
                    .fg(primary_color())
                    .add_modifier(Modifier::BOLD),
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
            Span::styled(key.to_owned(), Style::default().fg(hcl_key_color())),
            Span::styled(" = ".to_owned(), dim()),
            Span::styled(value.to_owned(), Style::default().fg(hcl_val_color())),
        ]);
    }

    // Block opener (e.g. `network_interface {`)
    if trimmed.ends_with('{') {
        let indent_len = line.len() - trimmed.len();
        let indent = &line[..indent_len];
        let body = trimmed.trim_end_matches('{').trim_end();
        return Line::from(vec![
            Span::styled(indent.to_owned(), dim()),
            Span::styled(body.to_owned(), Style::default().fg(primary_color())),
            Span::styled(" {".to_owned(), dim()),
        ]);
    }

    Line::from(Span::styled(line.to_owned(), muted()))
}
