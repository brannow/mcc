use ratatui::style::{Color, Modifier, Style};

// Base palette
pub const BG: Color = Color::Rgb(22, 22, 30);
pub const POPUP_BG: Color = Color::Rgb(10, 10, 16);
pub const BG_ALT: Color = Color::Rgb(30, 30, 42);
pub const BG_HEADER: Color = Color::Rgb(40, 40, 55);
pub const BG_SELECTED: Color = Color::Rgb(50, 50, 75);
pub const BORDER: Color = Color::Rgb(60, 60, 80);
pub const BORDER_ACTIVE: Color = Color::Rgb(100, 100, 140);
pub const TEXT: Color = Color::Rgb(200, 200, 210);
pub const TEXT_DIM: Color = Color::Rgb(100, 100, 120);
pub const TEXT_BRIGHT: Color = Color::Rgb(230, 230, 240);
pub const ACCENT: Color = Color::Rgb(130, 140, 255);

// Codec colors
pub const CODEC_H264: Color = Color::Rgb(255, 180, 60);
pub const CODEC_HEVC: Color = Color::Rgb(80, 220, 120);
pub const CODEC_AV1: Color = Color::Rgb(80, 200, 255);
pub const CODEC_OTHER: Color = Color::Rgb(180, 180, 190);
pub const CODEC_PENDING: Color = Color::Rgb(80, 80, 100);
pub const CODEC_ERROR: Color = Color::Rgb(255, 80, 80);

// Section headers in detail
pub const SECTION_HEADER: Color = Color::Rgb(130, 140, 255);
pub const LABEL: Color = Color::Rgb(120, 120, 150);

// Status bar
pub const STATUS_BG: Color = Color::Rgb(30, 30, 45);
pub const PROGRESS_DONE: Color = Color::Rgb(80, 220, 120);
pub const PROGRESS_REMAINING: Color = Color::Rgb(45, 45, 60);

pub fn codec_color(codec: Option<&str>) -> Color {
    match codec {
        Some("h264") => CODEC_H264,
        Some("hevc") => CODEC_HEVC,
        Some("av1") => CODEC_AV1,
        Some(_) => CODEC_OTHER,
        None => CODEC_PENDING,
    }
}

pub fn header_style() -> Style {
    Style::default()
        .fg(TEXT_BRIGHT)
        .bg(BG_HEADER)
        .add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default()
        .bg(BG_SELECTED)
        .fg(TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD)
}

pub fn border_style() -> Style {
    Style::default().fg(BORDER)
}

pub fn border_active_style() -> Style {
    Style::default().fg(BORDER_ACTIVE)
}

pub fn title_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
