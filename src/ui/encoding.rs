use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::theme;
use crate::app::{App, EncodingPaneFocus};
use crate::model::{EncodeJobStatus, human_duration, human_file_size};

pub fn render_encoding_view(f: &mut Frame, app: &mut App, area: Rect) {
    if app.is_encoding_active() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        let queue_active = app.encoding_pane_focus == EncodingPaneFocus::Queue;
        render_queue_table(f, app, chunks[0], queue_active);
        render_telemetry(f, app, chunks[1], !queue_active);
    } else {
        render_queue_table(f, app, area, true);
    }
}

fn render_queue_table(f: &mut Frame, app: &mut App, area: Rect, active: bool) {
    let header_cells = [
        Cell::from(Span::styled(" #", theme::header_style())),
        Cell::from(Span::styled(" Name", theme::header_style())),
        Cell::from(Span::styled("Size", theme::header_style())),
        Cell::from(Span::styled("Preset", theme::header_style())),
        Cell::from(Span::styled("Status", theme::header_style())),
    ];

    let header = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::BG_HEADER));

    let rows: Vec<Row> = app
        .encode_queue
        .iter()
        .enumerate()
        .map(|(idx, job)| {
            let row_bg = if idx % 2 == 0 {
                theme::BG
            } else {
                theme::BG_ALT
            };

            let num_cell = Cell::from(Span::styled(
                format!(" {}", idx + 1),
                Style::default().fg(theme::TEXT_DIM),
            ));

            let name_cell = Cell::from(Span::styled(
                format!(" {}", job.file_name),
                Style::default().fg(theme::TEXT),
            ));

            let size_cell = Cell::from(Span::styled(
                human_file_size(job.file_size),
                Style::default().fg(theme::TEXT),
            ));

            let preset_cell = Cell::from(Span::styled(
                job.preset_name.as_str(),
                Style::default().fg(theme::ACCENT),
            ));

            let (status_text, status_color) = status_display(&job.status);
            let status_content = match &job.status {
                EncodeJobStatus::Encoding => {
                    if let Some(progress) = &job.progress {
                        let pct = if let Some(total) = job.total_frames.filter(|&t| t > 0) {
                            (progress.frame as f64 / total as f64 * 100.0).min(100.0)
                        } else {
                            progress.percent
                        };
                        format!("{} {:.1}%", status_text, pct)
                    } else {
                        status_text.to_string()
                    }
                }
                EncodeJobStatus::Done { saved_percent, .. } => {
                    format!("{} ({:.0}% saved)", status_text, saved_percent)
                }
                _ => status_text.to_string(),
            };
            let status_cell = Cell::from(Span::styled(
                status_content,
                Style::default().fg(status_color).add_modifier(
                    if matches!(
                        job.status,
                        EncodeJobStatus::Encoding | EncodeJobStatus::CopyingToTemp
                    ) {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            ));

            Row::new(vec![
                num_cell,
                name_cell,
                size_cell,
                preset_cell,
                status_cell,
            ])
            .style(Style::default().bg(row_bg))
        })
        .collect();

    let queued = app.queued_count();
    let finished = app.finished_count();
    let total = app.encode_queue.len();

    let title = Line::from(vec![
        Span::styled(" Encoding Queue ", theme::title_style()),
        Span::styled("─ ", Style::default().fg(theme::BORDER)),
        Span::styled(
            format!("{}", total),
            Style::default().fg(theme::TEXT_BRIGHT),
        ),
        Span::styled(" jobs ", Style::default().fg(theme::TEXT_DIM)),
        if queued > 0 {
            Span::styled(
                format!("({} queued) ", queued),
                Style::default().fg(theme::ACCENT),
            )
        } else if finished > 0 {
            Span::styled(
                format!("({} done) ", finished),
                Style::default().fg(theme::CODEC_HEVC),
            )
        } else {
            Span::raw("")
        },
    ]);

    let preset_info = if let Some(preset) = app.current_preset() {
        Line::from(vec![Span::styled(
            format!(" [{}] ", preset.name),
            Style::default().fg(theme::ACCENT),
        )])
    } else {
        Line::from(vec![Span::styled(
            " [no presets] ",
            Style::default().fg(theme::CODEC_ERROR),
        )])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            theme::border_active_style()
        } else {
            theme::border_style()
        })
        .title(title)
        .title_bottom(preset_info);

    let widths = [
        Constraint::Length(4),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(18),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .column_spacing(1);

    app.encode_queue_state
        .select(if app.encode_queue.is_empty() {
            None
        } else {
            Some(app.encode_queue_selected)
        });
    f.render_stateful_widget(table, area, &mut app.encode_queue_state);
}

fn render_telemetry(f: &mut Frame, app: &App, area: Rect, active: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            theme::border_active_style()
        } else {
            theme::border_style()
        })
        .title(Span::styled(" Current Encoding ", theme::title_style()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let job = match app.current_encoding_job() {
        Some(j) => j,
        None => {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No encoding in progress",
                    Style::default().fg(theme::TEXT_DIM),
                )),
            ]);
            f.render_widget(empty, inner);
            return;
        }
    };

    // Determine if we have enough space for a side-by-side graph
    let stats_min_width: u16 = 48;
    let graph_min_width: u16 = 15;
    let has_graph = job.progress.is_some()
        && !job.fps_stats.history.is_empty()
        && inner.width > stats_min_width + graph_min_width;

    let (stats_area, graph_area) = if has_graph {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(stats_min_width),
                Constraint::Min(graph_min_width),
            ])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    // ── Stats (left side) ──
    let mut lines = vec![
        Line::from(vec![
            Span::styled("  File:    ", Style::default().fg(theme::LABEL)),
            Span::styled(&job.file_name, Style::default().fg(theme::TEXT_BRIGHT)),
        ]),
        Line::from({
            let mut spans = vec![
                Span::styled("  Preset:  ", Style::default().fg(theme::LABEL)),
                Span::styled(&job.preset_name, Style::default().fg(theme::ACCENT)),
            ];
            if let Some(preset) = app.presets.iter().find(|p| p.name == job.preset_name) {
                spans.push(Span::styled(
                    format!("  {}", preset.summary()),
                    Style::default().fg(theme::TEXT_DIM),
                ));
            }
            spans
        }),
        Line::from(vec![
            Span::styled("  Status:  ", Style::default().fg(theme::LABEL)),
            Span::styled(
                job.status.label(),
                Style::default()
                    .fg(status_display(&job.status).1)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    if job.progress.is_none()
        && matches!(
            job.status,
            EncodeJobStatus::Encoding | EncodeJobStatus::CopyingToTemp
        )
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Waiting for ffmpeg...",
            Style::default().fg(theme::TEXT_DIM),
        )));
    }

    if let Some(progress) = &job.progress {
        let percent = if let Some(total) = job.total_frames.filter(|&t| t > 0) {
            (progress.frame as f64 / total as f64 * 100.0).min(100.0)
        } else {
            progress.percent
        };

        // Progress bar
        let bar_width = (stats_area.width as usize).saturating_sub(16).min(50);
        let filled = ((percent / 100.0) * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("[", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                "\u{25b0}".repeat(filled),
                Style::default().fg(theme::PROGRESS_DONE),
            ),
            Span::styled(
                "\u{25b1}".repeat(empty),
                Style::default().fg(theme::PROGRESS_REMAINING),
            ),
            Span::styled("] ", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                format!("{:.1}%", percent),
                Style::default()
                    .fg(theme::TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let frame_str = if let Some(total) = job.total_frames {
            format!("{}/{}", progress.frame, total)
        } else {
            format!("{}", progress.frame)
        };

        let stats = &job.fps_stats;
        lines.push(Line::from(vec![
            Span::styled("  Frame:   ", Style::default().fg(theme::LABEL)),
            Span::styled(frame_str, Style::default().fg(theme::TEXT)),
            Span::styled("  Size: ", Style::default().fg(theme::LABEL)),
            Span::styled(
                human_file_size(progress.total_size),
                Style::default().fg(theme::TEXT),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("  FPS:     ", Style::default().fg(theme::LABEL)),
            Span::styled(
                format!("{:.1}", stats.current),
                Style::default().fg(theme::TEXT_BRIGHT),
            ),
            Span::styled("  min:", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                format!("{:.1}", stats.min),
                Style::default().fg(theme::TEXT),
            ),
            Span::styled("  max:", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                format!("{:.1}", stats.max),
                Style::default().fg(theme::TEXT),
            ),
            Span::styled("  avg:", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                format!("{:.1}", stats.avg),
                Style::default().fg(theme::TEXT),
            ),
        ]));

        // ETA + elapsed time
        let elapsed_str = job
            .started_at
            .map(|t| human_duration(t.elapsed().as_secs_f64()));

        if stats.avg > 0.0 {
            if let Some(total) = job.total_frames.filter(|&t| t > 0) {
                let remaining_frames = total.saturating_sub(progress.frame);
                let remaining_secs = remaining_frames as f64 / stats.avg;
                let eta_m = (remaining_secs as u64) / 60;
                let eta_s = (remaining_secs as u64) % 60;
                let mut spans = vec![
                    Span::styled("  ETA:     ", Style::default().fg(theme::LABEL)),
                    Span::styled(
                        format!("~{}m {:02}s", eta_m, eta_s),
                        Style::default().fg(theme::TEXT),
                    ),
                ];
                if let Some(ref elapsed) = elapsed_str {
                    spans.push(Span::styled(
                        "  elapsed: ",
                        Style::default().fg(theme::TEXT_DIM),
                    ));
                    spans.push(Span::styled(
                        elapsed.clone(),
                        Style::default().fg(theme::TEXT),
                    ));
                }
                lines.push(Line::from(spans));
            }
        } else if let Some(elapsed) = elapsed_str {
            lines.push(Line::from(vec![
                Span::styled("  Elapsed: ", Style::default().fg(theme::LABEL)),
                Span::styled(elapsed, Style::default().fg(theme::TEXT)),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), stats_area);

    // ── FPS graph (right side) ──
    if let Some(graph_area) = graph_area {
        // Pad: 2 left, 1 right
        let padded = Rect {
            x: graph_area.x + 2,
            y: graph_area.y,
            width: graph_area.width.saturating_sub(3),
            height: graph_area.height,
        };
        let graph_w = padded.width as usize;
        let graph_h = padded.height as usize;

        if graph_h >= 4 && graph_w >= 5 {
            // Layout: title (1) + top border (1) + graph + bottom border (1)
            let graph_data_h = graph_h.saturating_sub(3);
            let braille_rows = job
                .fps_stats
                .braille_graph(graph_w.saturating_sub(2), graph_data_h);

            let stats = &job.fps_stats;
            let mut graph_lines: Vec<Line> = Vec::new();

            // Title
            let max_label = format!("{:.0}", stats.max);
            let min_label = format!("{:.0}", stats.min);
            let title = format!(" FPS  (avg {:.0})", stats.avg);
            graph_lines.push(Line::from(Span::styled(
                title,
                Style::default().fg(theme::TEXT_DIM),
            )));

            // Top border with max value
            let top_border_w = graph_w.saturating_sub(max_label.len() + 2);
            graph_lines.push(Line::from(vec![Span::styled(
                format!("{} \u{250c}{}", max_label, "\u{2500}".repeat(top_border_w)),
                Style::default().fg(theme::BORDER),
            )]));

            // Graph rows with left border
            for row_str in &braille_rows {
                graph_lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:>width$} \u{2502}", "", width = max_label.len()),
                        Style::default().fg(theme::BORDER),
                    ),
                    Span::styled(row_str.clone(), Style::default().fg(theme::ACCENT)),
                ]));
            }

            // Bottom border with min value
            let bot_border_w = graph_w.saturating_sub(min_label.len() + 2);
            graph_lines.push(Line::from(vec![Span::styled(
                format!("{} \u{2514}{}", min_label, "\u{2500}".repeat(bot_border_w)),
                Style::default().fg(theme::BORDER),
            )]));

            f.render_widget(Paragraph::new(graph_lines), padded);
        }
    }
}

fn status_display(status: &EncodeJobStatus) -> (&str, ratatui::style::Color) {
    match status {
        EncodeJobStatus::Queued => ("Queued", theme::TEXT_DIM),
        EncodeJobStatus::CopyingToTemp => ("Copying...", theme::ACCENT),
        EncodeJobStatus::Encoding => ("Encoding", theme::CODEC_HEVC),
        EncodeJobStatus::Paused => ("Paused", theme::CODEC_H264),
        EncodeJobStatus::Validating => ("Validating", theme::ACCENT),
        EncodeJobStatus::Done { .. } => ("Done", theme::CODEC_HEVC),
        EncodeJobStatus::Failed(_) => ("Failed", theme::CODEC_ERROR),
        EncodeJobStatus::Cancelled => ("Cancelled", theme::TEXT_DIM),
    }
}
