use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{ActiveView, App, CleanupDialog, CleanupFocus, PresetPicker};
use crate::model::human_file_size;
use crate::preset::EncodingPreset;
use super::theme;

pub fn render_legend_popup(f: &mut Frame, _app: &App) {
    let area = f.area();
    dim_background(f, area);

    let popup = centered_fixed(56, 18, area);
    f.render_widget(Clear, popup);

    let key = Style::default().fg(theme::ACCENT).bg(theme::POPUP_BG).add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(theme::TEXT).bg(theme::POPUP_BG);
    let dim = Style::default().fg(theme::TEXT_DIM).bg(theme::POPUP_BG);
    let head = Style::default().fg(theme::TEXT_BRIGHT).bg(theme::POPUP_BG).add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Encoding Control", head)),
        Line::from(vec![
            Span::styled("    Enter  ", key), Span::styled("Start encoding queue", desc),
        ]),
        Line::from(vec![
            Span::styled("    Space  ", key), Span::styled("Pause / Resume current", desc),
        ]),
        Line::from(vec![
            Span::styled("    c      ", key), Span::styled("Cancel current encode", desc),
        ]),
        Line::from(vec![
            Span::styled("    C      ", key), Span::styled("Cancel all (kill current + drop queue)", desc),
        ]),
        Line::from(vec![
            Span::styled("    s      ", key), Span::styled("Stop queue (let current finish)", desc),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Queue Management", head)),
        Line::from(vec![
            Span::styled("    x/Del  ", key), Span::styled("Remove selected job", desc),
        ]),
        Line::from(vec![
            Span::styled("    \u{2191}/\u{2193}    ", key), Span::styled("Navigate queue", desc),
        ]),
        Line::from(vec![
            Span::styled("    p      ", key), Span::styled("Select preset", desc),
        ]),
        Line::from(vec![
            Span::styled("    P      ", key), Span::styled("Stamp preset + advance", desc),
        ]),
        Line::from(vec![
            Span::styled("    Tab    ", key), Span::styled("Switch pane focus", desc),
        ]),
        Line::from(vec![
            Span::styled("    \u{2190}      ", key), Span::styled("Back to list view", desc),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT).bg(theme::POPUP_BG))
        .title(Span::styled(
            " Keybindings ",
            Style::default().fg(theme::ACCENT).bg(theme::POPUP_BG).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(vec![
            Span::styled(" press any key to close ", dim),
        ]))
        .style(Style::default().bg(theme::POPUP_BG));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup);
}

/// Create a centered rect with fixed dimensions, clamped to fit the area
fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Dim the background buffer
fn dim_background(f: &mut Frame, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut f.buffer_mut()[(x, y)];
            cell.set_style(Style::default().fg(theme::TEXT_DIM).bg(theme::BG));
        }
    }
}

pub fn render_cleanup_dialog(f: &mut Frame, dialog: &CleanupDialog) {
    let area = f.area();
    dim_background(f, area);

    // Calculate content-driven size
    // Height: 2 (border) + 1 (desc) + 1 (blank) + groups + 1 (blank) + 1 (status) + 1 (blank) + 1 (buttons)
    let content_height = 2 + 1 + 1 + dialog.groups.len() as u16 + 1 + 1 + 1 + 1;
    // Width: find widest group label + padding
    let max_label_width = dialog
        .groups
        .iter()
        .map(|g| g.label().len())
        .max()
        .unwrap_or(20);
    let content_width = (max_label_width as u16 + 12).max(50); // 12 for checkbox + cursor + padding, min 50

    let popup = centered_fixed(content_width, content_height, area);
    f.render_widget(Clear, popup);

    // Dark red border
    let border_color = theme::CODEC_ERROR;
    let hint_label = Style::default().fg(theme::TEXT).bg(theme::POPUP_BG);
    let hint_key = Style::default()
        .fg(theme::ACCENT)
        .bg(theme::POPUP_BG)
        .add_modifier(Modifier::BOLD);
    let bottom_hint = Line::from(vec![
        Span::styled(" Space", hint_key),
        Span::styled(":select ", hint_label),
        Span::styled("a", hint_key),
        Span::styled(":all ", hint_label),
        Span::styled("↑↓←→", hint_key),
        Span::styled(":nav ", hint_label),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(theme::POPUP_BG))
        .title(Span::styled(
            " Junk Files ",
            Style::default()
                .fg(border_color)
                .bg(theme::POPUP_BG)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(bottom_hint)
        .style(Style::default().bg(theme::POPUP_BG));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                                // description
            Constraint::Length(1),                                // blank
            Constraint::Length(dialog.groups.len() as u16),       // checkbox list
            Constraint::Length(1),                                // blank
            Constraint::Length(1),                                // status
            Constraint::Length(1),                                // blank
            Constraint::Length(1),                                // buttons
        ])
        .split(inner);

    // Description
    let desc = Paragraph::new(Line::from(Span::styled(
        "  Select junk file types to delete:",
        Style::default().fg(theme::TEXT),
    )));
    f.render_widget(desc, chunks[0]);

    // Checkbox list
    let mut list_lines: Vec<Line> = Vec::new();
    for (i, group) in dialog.groups.iter().enumerate() {
        let is_cursor = dialog.focus == CleanupFocus::List && i == dialog.cursor;
        let checkbox = if group.selected { "[x]" } else { "[ ]" };
        let cursor_indicator = if is_cursor { ">" } else { " " };

        let line = Line::from(vec![
            Span::styled(
                format!("  {} ", cursor_indicator),
                Style::default().fg(if is_cursor { theme::ACCENT } else { theme::TEXT_DIM }),
            ),
            Span::styled(
                format!("{} ", checkbox),
                Style::default().fg(if group.selected { theme::CODEC_HEVC } else { theme::TEXT_DIM }),
            ),
            Span::styled(
                group.label(),
                Style::default().fg(if is_cursor { theme::TEXT_BRIGHT } else { theme::TEXT }),
            ),
        ]);
        list_lines.push(line);
    }


    f.render_widget(Paragraph::new(list_lines), chunks[2]);

    // Status line
    let status_text = if let Some(msg) = &dialog.status_message {
        Line::from(Span::styled(
            format!("  {}", msg),
            Style::default().fg(theme::CODEC_HEVC),
        ))
    } else {
        let sel_count = dialog.selected_count();
        let sel_size = dialog.selected_size();
        if sel_count > 0 {
            Line::from(vec![
                Span::styled("  Selected: ", Style::default().fg(theme::TEXT_DIM)),
                Span::styled(
                    format!("{} files ({})", sel_count, human_file_size(sel_size)),
                    Style::default().fg(theme::TEXT_BRIGHT),
                ),
            ])
        } else {
            Line::from(Span::styled(
                "  Nothing selected",
                Style::default().fg(theme::TEXT_DIM),
            ))
        }
    };
    f.render_widget(Paragraph::new(status_text), chunks[4]);

    // Buttons
    let btn_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(12),
            Constraint::Length(2),
            Constraint::Length(12),
            Constraint::Min(1),
        ])
        .split(chunks[6]);

    let delete_focused_bg = Color::Rgb(120, 30, 30);
    let delete_style = if dialog.focus == CleanupFocus::DeleteButton {
        Style::default()
            .fg(theme::TEXT_BRIGHT)
            .bg(delete_focused_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme::CODEC_ERROR)
    };

    let cancel_style = if dialog.focus == CleanupFocus::CancelButton {
        Style::default()
            .fg(theme::BG)
            .bg(theme::TEXT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::TEXT_DIM)
    };

    f.render_widget(
        Paragraph::new(Line::from(" [ Delete ] ")).style(delete_style).alignment(Alignment::Center),
        btn_layout[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(" [ Cancel ] ")).style(cancel_style).alignment(Alignment::Center),
        btn_layout[3],
    );
}

pub fn render_preset_picker(
    f: &mut Frame,
    picker: &PresetPicker,
    presets: &[EncodingPreset],
    selected_preset: usize,
    active_view: ActiveView,
) {
    let area = f.area();
    dim_background(f, area);

    // Calculate dimensions based on content
    let max_name_len = presets.iter().map(|p| p.name.len()).max().unwrap_or(10);
    let max_summary_len = presets.iter().map(|p| p.summary().len()).max().unwrap_or(10);
    // "  1  > name    summary   [default] "
    let content_width = (6 + max_name_len + 2 + max_summary_len + 12) as u16;
    let content_width = content_width.max(40).min(70);
    // 2 (border) + 1 (hint line) + 1 (blank) + presets + 1 (blank) + 1 (context hint)
    let content_height = (2 + 1 + 1 + presets.len() + 1 + 1) as u16;

    let popup = centered_fixed(content_width, content_height, area);
    f.render_widget(Clear, popup);

    let hint_label = Style::default().fg(theme::TEXT).bg(theme::POPUP_BG);
    let hint_key = Style::default()
        .fg(theme::ACCENT)
        .bg(theme::POPUP_BG)
        .add_modifier(Modifier::BOLD);
    let bottom_hint = Line::from(vec![
        Span::styled(" ↑↓", hint_key),
        Span::styled(":nav ", hint_label),
        Span::styled("Enter", hint_key),
        Span::styled(":select ", hint_label),
        Span::styled("1-9", hint_key),
        Span::styled(":quick ", hint_label),
        Span::styled("Esc", hint_key),
        Span::styled(":cancel ", hint_label),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT).bg(theme::POPUP_BG))
        .title(Span::styled(
            " Select Preset ",
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::POPUP_BG)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(bottom_hint)
        .style(Style::default().bg(theme::POPUP_BG));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                       // context hint
            Constraint::Length(1),                       // blank
            Constraint::Length(presets.len() as u16),    // preset list
        ])
        .split(inner);

    // Context hint
    let context_text = match active_view {
        ActiveView::List => "  Set default preset for new queue items",
        ActiveView::Encoding => "  Set default + apply to selected queued job",
    };
    let context = Paragraph::new(Line::from(Span::styled(
        context_text,
        Style::default().fg(theme::TEXT_DIM),
    )));
    f.render_widget(context, chunks[0]);

    // Preset list
    let mut lines: Vec<Line> = Vec::new();
    for (i, preset) in presets.iter().enumerate() {
        let is_cursor = i == picker.cursor;
        let is_default = i == selected_preset;

        let cursor_char = if is_cursor { ">" } else { " " };
        let number = format!("{}", i + 1);
        let default_tag = if is_default { " [default]" } else { "" };

        let name_color = if is_cursor { theme::TEXT_BRIGHT } else { theme::TEXT };
        let summary_color = if is_cursor { theme::TEXT } else { theme::TEXT_DIM };

        let line = Line::from(vec![
            Span::styled(
                format!("  {} ", cursor_char),
                Style::default().fg(if is_cursor { theme::ACCENT } else { theme::TEXT_DIM }),
            ),
            Span::styled(
                format!("{} ", number),
                Style::default().fg(theme::TEXT_DIM),
            ),
            Span::styled(
                format!("{:<width$}", preset.name, width = max_name_len + 2),
                Style::default().fg(name_color).add_modifier(
                    if is_cursor { Modifier::BOLD } else { Modifier::empty() }
                ),
            ),
            Span::styled(
                preset.summary(),
                Style::default().fg(summary_color),
            ),
            Span::styled(
                default_tag.to_string(),
                Style::default().fg(theme::CODEC_HEVC),
            ),
        ]);
        lines.push(line);
    }

    f.render_widget(Paragraph::new(lines), chunks[2]);
}

pub fn render_quit_confirm(f: &mut Frame, app: &App) {
    let area = f.area();
    dim_background(f, area);

    let encoding = app.is_encoding_active();
    let queued = app.queued_count();

    let mut desc_parts: Vec<String> = Vec::new();
    if encoding {
        desc_parts.push("encoding in progress".to_string());
    }
    if queued > 0 {
        desc_parts.push(format!("{} queued job{}", queued, if queued == 1 { "" } else { "s" }));
    }
    let line1 = format!("  {} still {}.", if encoding { "Encoding" } else { "Queue" }, desc_parts.join(" and "));
    let line2 = "  Quitting will cancel and clean up temp files.";

    let content_width = line1.len().max(line2.len()) as u16 + 4; // +4 for borders + padding

    let popup = centered_fixed(content_width, 7, area);
    f.render_widget(Clear, popup);

    let warn = Style::default().fg(theme::CODEC_ERROR).bg(theme::POPUP_BG).add_modifier(Modifier::BOLD);
    let text = Style::default().fg(theme::TEXT).bg(theme::POPUP_BG);
    let key = Style::default().fg(theme::ACCENT).bg(theme::POPUP_BG).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme::TEXT_DIM).bg(theme::POPUP_BG);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(line1, text)),
        Line::from(Span::styled(line2, text)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", text),
            Span::styled("y/Enter", key),
            Span::styled(" quit   ", dim),
            Span::styled("any key", key),
            Span::styled(" cancel", dim),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::CODEC_ERROR).bg(theme::POPUP_BG))
        .title(Span::styled(" Quit? ", warn))
        .style(Style::default().bg(theme::POPUP_BG));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup);
}
