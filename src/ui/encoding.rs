use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::theme;
use crate::app::{App, EncodingPaneFocus};
use crate::model::{EncodeJobStatus, human_duration, human_file_size};

pub fn render_encoding_view(f: &mut Frame, app: &mut App, area: Rect) {
    let show_detail = app
        .encode_queue
        .get(app.encode_queue_selected)
        .is_some_and(|j| {
            matches!(
                j.status,
                EncodeJobStatus::Encoding
                    | EncodeJobStatus::CopyingToTemp
                    | EncodeJobStatus::Paused
                    | EncodeJobStatus::Validating
                    | EncodeJobStatus::Failed(_)
                    | EncodeJobStatus::Done { .. }
            )
        });

    if show_detail {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        let queue_active = app.encoding_pane_focus == EncodingPaneFocus::Queue;
        render_queue_table(f, app, chunks[0], queue_active);
        render_detail(f, app, chunks[1], !queue_active);
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

fn render_detail(f: &mut Frame, app: &App, area: Rect, active: bool) {
    let job = match app.encode_queue.get(app.encode_queue_selected) {
        Some(j) => j,
        None => return,
    };

    let title = match &job.status {
        EncodeJobStatus::Encoding | EncodeJobStatus::Paused => " Current Encoding ",
        EncodeJobStatus::CopyingToTemp | EncodeJobStatus::Validating => " Current Encoding ",
        EncodeJobStatus::Failed(_) => " Failed ",
        EncodeJobStatus::Done { .. } => " Completed ",
        _ => " Details ",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            theme::border_active_style()
        } else {
            theme::border_style()
        })
        .title(Span::styled(title, theme::title_style()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // For finished jobs, show the summary view
    if matches!(
        job.status,
        EncodeJobStatus::Failed(_) | EncodeJobStatus::Done { .. }
    ) {
        render_job_summary(f, job, inner);
        return;
    }

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
        let mut size_spans = vec![
            Span::styled("  Frame:   ", Style::default().fg(theme::LABEL)),
            Span::styled(frame_str, Style::default().fg(theme::TEXT)),
            Span::styled("  Size: ", Style::default().fg(theme::LABEL)),
            Span::styled(
                human_file_size(progress.total_size),
                Style::default().fg(theme::TEXT),
            ),
        ];

        // Estimated final size + savings based on current progress
        if percent > 5.0 && progress.total_size > 0 {
            let estimated_total = progress.total_size as f64 / (percent / 100.0);
            let estimated_saved = if job.file_size > 0 {
                (1.0 - estimated_total / job.file_size as f64) * 100.0
            } else {
                0.0
            };
            size_spans.push(Span::styled(
                format!("  ~{}", human_file_size(estimated_total as u64)),
                Style::default().fg(theme::TEXT_DIM),
            ));
            if estimated_saved > 0.0 {
                size_spans.push(Span::styled(
                    format!(" ({:.0}% saved)", estimated_saved),
                    Style::default().fg(theme::CODEC_HEVC),
                ));
            } else {
                size_spans.push(Span::styled(
                    format!(" (+{:.0}%)", estimated_saved.abs()),
                    Style::default().fg(theme::CODEC_ERROR),
                ));
            }
        }

        lines.push(Line::from(size_spans));

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

        // ETA from frame count, or fall back to duration-based percent
        let eta_str = if stats.avg > 0.0 {
            if let Some(total) = job.total_frames.filter(|&t| t > 0) {
                let remaining_frames = total.saturating_sub(progress.frame);
                let remaining_secs = remaining_frames as f64 / stats.avg;
                Some(remaining_secs)
            } else if let Some(dur) = job.duration_secs.filter(|&d| d > 0.0) {
                let remaining_secs = dur - progress.out_time_secs;
                if remaining_secs > 0.0 && progress.speed > 0.0 {
                    Some(remaining_secs / progress.speed)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut eta_line: Vec<Span> = Vec::new();
        if let Some(remaining_secs) = eta_str {
            let eta_m = (remaining_secs as u64) / 60;
            let eta_s = (remaining_secs as u64) % 60;
            eta_line.push(Span::styled(
                "  ETA:     ",
                Style::default().fg(theme::LABEL),
            ));
            eta_line.push(Span::styled(
                format!("~{}m {:02}s", eta_m, eta_s),
                Style::default().fg(theme::TEXT),
            ));
        }
        if let Some(ref elapsed) = elapsed_str {
            let label = if eta_line.is_empty() {
                "  Elapsed: "
            } else {
                "  elapsed: "
            };
            eta_line.push(Span::styled(label, Style::default().fg(theme::TEXT_DIM)));
            eta_line.push(Span::styled(
                elapsed.clone(),
                Style::default().fg(theme::TEXT),
            ));
        }
        if !eta_line.is_empty() {
            lines.push(Line::from(eta_line));
        }
    }

    f.render_widget(Paragraph::new(lines), stats_area);

    // ── FPS graph (right side) ──
    if let Some(graph_area) = graph_area {
        render_fps_graph(f, &job.fps_stats, graph_area);
    }
}

fn render_job_summary(f: &mut Frame, job: &crate::model::EncodeJob, area: Rect) {
    let stats = &job.fps_stats;
    let show_graph =
        matches!(job.status, EncodeJobStatus::Done { .. }) && !stats.history.is_empty();

    let stats_min_width: u16 = 48;
    let graph_min_width: u16 = 15;
    let (stats_area, graph_area) = if show_graph && area.width > stats_min_width + graph_min_width {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(stats_min_width),
                Constraint::Min(graph_min_width),
            ])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  File:    ", Style::default().fg(theme::LABEL)),
            Span::styled(&job.file_name, Style::default().fg(theme::TEXT_BRIGHT)),
        ]),
        Line::from(vec![
            Span::styled("  Preset:  ", Style::default().fg(theme::LABEL)),
            Span::styled(&job.preset_name, Style::default().fg(theme::ACCENT)),
        ]),
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

    match &job.status {
        EncodeJobStatus::Failed(reason) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Error:",
                Style::default()
                    .fg(theme::CODEC_ERROR)
                    .add_modifier(Modifier::BOLD),
            )));
            for line in reason.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default().fg(theme::CODEC_ERROR),
                )));
            }
        }
        EncodeJobStatus::Done {
            encoded_size,
            saved_percent,
        } => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Size:    ", Style::default().fg(theme::LABEL)),
                Span::styled(
                    human_file_size(*encoded_size),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!("  ({:.0}% saved)", saved_percent),
                    Style::default().fg(theme::CODEC_HEVC),
                ),
            ]));
            if stats.avg > 0.0 {
                lines.push(Line::from(vec![
                    Span::styled("  FPS:     ", Style::default().fg(theme::LABEL)),
                    Span::styled(
                        format!(
                            "min:{:.0}  max:{:.0}  avg:{:.0}",
                            stats.min, stats.max, stats.avg
                        ),
                        Style::default().fg(theme::TEXT),
                    ),
                ]));
            }
            if let Some(secs) = job.elapsed_secs {
                lines.push(Line::from(vec![
                    Span::styled("  Time:    ", Style::default().fg(theme::LABEL)),
                    Span::styled(human_duration(secs), Style::default().fg(theme::TEXT)),
                ]));
            }
        }
        _ => {}
    }

    f.render_widget(Paragraph::new(lines), stats_area);

    // FPS graph for completed jobs
    if let Some(graph_area) = graph_area {
        render_fps_graph(f, stats, graph_area);
    }
}

fn render_fps_graph(f: &mut Frame, stats: &crate::model::FpsStats, graph_area: Rect) {
    let padded = Rect {
        x: graph_area.x + 2,
        y: graph_area.y,
        width: graph_area.width.saturating_sub(3),
        height: graph_area.height,
    };
    let graph_w = padded.width as usize;
    let graph_h = padded.height as usize;

    if graph_h < 4 || graph_w < 5 {
        return;
    }

    let graph_data_h = graph_h.saturating_sub(3);
    let braille_rows = stats.braille_graph(graph_w.saturating_sub(2), graph_data_h);

    let max_label = format!("{:.0}", stats.max);
    let min_label = format!("{:.0}", stats.min);
    let title = format!(" FPS  (avg {:.0})", stats.avg);

    let mut graph_lines: Vec<Line> = Vec::new();

    graph_lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(theme::TEXT_DIM),
    )));

    let top_border_w = graph_w.saturating_sub(max_label.len() + 2);
    graph_lines.push(Line::from(vec![Span::styled(
        format!("{} \u{250c}{}", max_label, "\u{2500}".repeat(top_border_w)),
        Style::default().fg(theme::BORDER),
    )]));

    for row_str in &braille_rows {
        graph_lines.push(Line::from(vec![
            Span::styled(
                format!("{:>width$} \u{2502}", "", width = max_label.len()),
                Style::default().fg(theme::BORDER),
            ),
            Span::styled(row_str.clone(), Style::default().fg(theme::ACCENT)),
        ]));
    }

    let bot_border_w = graph_w.saturating_sub(min_label.len() + 2);
    graph_lines.push(Line::from(vec![Span::styled(
        format!("{} \u{2514}{}", min_label, "\u{2500}".repeat(bot_border_w)),
        Style::default().fg(theme::BORDER),
    )]));

    f.render_widget(Paragraph::new(graph_lines), padded);
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
