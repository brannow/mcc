use ratatui::layout::{Constraint, Direction, Layout, Alignment};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{ActiveView, App};
use super::theme;

pub fn render_status_bar(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    // Split into: progress bar (if scanning) | stats | keybindings
    let is_scanning = app.probed_count < app.total_files;

    if is_scanning {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),  // progress bar
                Constraint::Min(20),    // stats
                Constraint::Length(55), // keybindings
            ])
            .split(area);

        // Progress bar
        let progress = if app.total_files > 0 {
            app.probed_count as f64 / app.total_files as f64
        } else {
            0.0
        };
        let bar_width = (chunks[0].width as usize).saturating_sub(4);
        let filled = (progress * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);
        let bar_line = Line::from(vec![
            Span::styled(" [", Style::default().fg(theme::TEXT_DIM)),
            Span::styled("▰".repeat(filled), Style::default().fg(theme::PROGRESS_DONE)),
            Span::styled("▱".repeat(empty), Style::default().fg(theme::PROGRESS_REMAINING)),
            Span::styled("] ", Style::default().fg(theme::TEXT_DIM)),
        ]);
        f.render_widget(Paragraph::new(bar_line).style(Style::default().bg(theme::STATUS_BG)), chunks[0]);

        // Stats
        render_stats(f, app, chunks[1]);

        // Keybindings
        render_keybindings(f, app, chunks[2]);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(20),    // stats
                Constraint::Length(55), // keybindings
            ])
            .split(area);

        render_stats(f, app, chunks[0]);
        render_keybindings(f, app, chunks[1]);
    }
}

fn render_stats(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let (h264, hevc, av1, other) = app.codec_counts();

    let stats = Line::from(vec![
        Span::styled(" h264:", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(format!("{}", h264), Style::default().fg(theme::CODEC_H264).add_modifier(Modifier::BOLD)),
        Span::styled("  hevc:", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(format!("{}", hevc), Style::default().fg(theme::CODEC_HEVC).add_modifier(Modifier::BOLD)),
        Span::styled("  av1:", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(format!("{}", av1), Style::default().fg(theme::CODEC_AV1).add_modifier(Modifier::BOLD)),
        if other > 0 {
            Span::styled(format!("  other:{}", other), Style::default().fg(theme::TEXT_DIM))
        } else {
            Span::raw("")
        },
        Span::styled(
            format!("  [{}/{}]", app.probed_count, app.total_files),
            Style::default().fg(theme::TEXT_DIM),
        ),
        if app.junk_count() > 0 {
            Span::styled(
                format!("  trash:{}", app.junk_count()),
                Style::default().fg(theme::CODEC_ERROR),
            )
        } else {
            Span::raw("")
        },
        // Mini encoding indicator (visible from any view)
        if app.is_encoding_active() {
            if app.is_encoding_paused() {
                Span::styled("  ENC:PAUSED", Style::default().fg(theme::CODEC_H264).add_modifier(Modifier::BOLD))
            } else {
                let pct = app.current_encoding_job()
                    .and_then(|j| {
                        let progress = j.progress.as_ref()?;
                        let p = if let Some(total) = j.total_frames.filter(|&t| t > 0) {
                            (progress.frame as f64 / total as f64 * 100.0).min(100.0)
                        } else {
                            progress.percent
                        };
                        Some(format!("{:.0}%", p))
                    })
                    .unwrap_or_else(|| "...".to_string());
                Span::styled(format!("  ENC:{}", pct), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            }
        } else if app.queued_count() > 0 {
            Span::styled(
                format!("  Q:{}", app.queued_count()),
                Style::default().fg(theme::ACCENT),
            )
        } else {
            Span::raw("")
        },
    ]);

    f.render_widget(
        Paragraph::new(stats).style(Style::default().bg(theme::STATUS_BG)),
        area,
    );
}

fn render_keybindings(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let all_keys: Vec<(&str, &str)> = match app.active_view {
        ActiveView::List => vec![
            ("\u{23ce}", "Enqueue"),
            ("e", "Enq All"),
            ("p", "Preset"),
            ("␣", "Detail"),
            ("s", "Sort"),
            ("f", "Filter"),
            ("g", "Group"),
            ("d", "Clean"),
            ("r", "Rescan"),
            ("\u{2192}", "Encode"),
        ],
        ActiveView::Encoding => {
            let mut keys: Vec<(&str, &str)> = Vec::new();
            if !app.is_encoding_active() && app.queued_count() > 0 {
                keys.push(("\u{23ce}", "Start"));
            }
            if app.is_encoding_active() {
                if app.is_encoding_paused() {
                    keys.push(("␣", "Resume"));
                } else {
                    keys.push(("␣", "Pause"));
                }
                keys.push(("c", "Cancel"));
            }
            if app.is_encoding_active() || app.queued_count() > 0 {
                keys.push(("C", "Cancel All"));
            }
            if app.queued_count() > 0 {
                keys.push(("s", "Stop Q"));
            }
            keys.push(("x", "Remove"));
            keys.push(("p", "Preset"));
            keys.push(("P", "Stamp"));
            keys.push(("h", "Help"));
            keys.push(("\u{2190}", "List"));
            keys.push(("^C", "Quit"));
            keys
        },
    };
    // Only show as many as fit
    let available = area.width as usize;
    let mut spans = Vec::new();
    let mut used = 0;
    for (key, label) in all_keys.iter().rev() {
        let entry_len = key.len() + 1 + label.len() + 1; // "k:Label "
        if used + entry_len > available {
            break;
        }
        spans.push((key, label));
        used += entry_len;
    }
    spans.reverse();
    let key_spans: Vec<Span> = spans
        .iter()
        .flat_map(|(key, label)| {
            vec![
                Span::styled(**key, Style::default().fg(theme::ACCENT)),
                Span::styled(format!(":{} ", label), Style::default().fg(theme::TEXT_DIM)),
            ]
        })
        .collect();
    let keys = Line::from(key_spans);

    f.render_widget(
        Paragraph::new(keys)
            .alignment(Alignment::Right)
            .style(Style::default().bg(theme::STATUS_BG)),
        area,
    );
}
