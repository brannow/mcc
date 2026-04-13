use ratatui::layout::Constraint;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use std::path::Path;

use crate::app::{App, ListRow, SortColumn};
use crate::model::{EncodeJob, EncodeJobStatus, human_bitrate, human_duration, human_file_size, ProbeStatus};
use super::theme;

/// Queue indicator for a file in the list view.
#[derive(Clone, Copy)]
enum QueueMark {
    None,
    Queued,
    Encoding,
    Done,
    Failed,
}

impl QueueMark {
    fn symbol(self) -> &'static str {
        match self {
            Self::None => " ",
            Self::Queued => "\u{25c6}",    // ◆
            Self::Encoding => "\u{25b6}",  // ▶
            Self::Done => "\u{2713}",      // ✓
            Self::Failed => "\u{2717}",    // ✗
        }
    }

    fn color(self) -> ratatui::style::Color {
        match self {
            Self::None => theme::TEXT_DIM,
            Self::Queued => theme::ACCENT,
            Self::Encoding => theme::CODEC_HEVC,
            Self::Done => theme::CODEC_HEVC,
            Self::Failed => theme::CODEC_ERROR,
        }
    }
}

fn sort_indicator(current: SortColumn, col: SortColumn, ascending: bool) -> &'static str {
    if current == col {
        if ascending { " ▲" } else { " ▼" }
    } else {
        ""
    }
}

// Column indices in display order: Name, Codec, Size, Bitrate, Resolution, Duration
const COL_NAME: usize = 0;
const COL_CODEC: usize = 1;
const COL_SIZE: usize = 2;
const COL_BITRATE: usize = 3;
const COL_RESOLUTION: usize = 4;
const COL_DURATION: usize = 5;

struct VisibleColumns {
    show: [bool; 6],
}

impl VisibleColumns {
    fn from_width(width: u16) -> Self {
        let w = width as usize;
        Self {
            show: [
                true,
                true,
                w >= 55,
                w >= 70,
                w >= 85,
                w >= 100,
            ],
        }
    }

    fn is_visible(&self, col: usize) -> bool {
        self.show[col]
    }
}

pub fn render_file_list(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect, active: bool) {
    let vis = VisibleColumns::from_width(area.width);

    // Header
    let all_headers = [
        (COL_NAME, format!(" Name{}", sort_indicator(app.sort_column, SortColumn::Name, app.sort_ascending))),
        (COL_CODEC, format!("Codec{}", sort_indicator(app.sort_column, SortColumn::Codec, app.sort_ascending))),
        (COL_SIZE, format!("Size{}", sort_indicator(app.sort_column, SortColumn::Size, app.sort_ascending))),
        (COL_BITRATE, format!("Bitrate{}", sort_indicator(app.sort_column, SortColumn::Bitrate, app.sort_ascending))),
        (COL_RESOLUTION, format!("Res{}", sort_indicator(app.sort_column, SortColumn::Resolution, app.sort_ascending))),
        (COL_DURATION, format!("Duration{}", sort_indicator(app.sort_column, SortColumn::Duration, app.sort_ascending))),
    ];

    let header_cells: Vec<Cell> = all_headers
        .iter()
        .filter(|(col, _)| vis.is_visible(*col))
        .map(|(_, h)| Cell::from(Span::styled(h.as_str(), theme::header_style())))
        .collect();

    let header = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::BG_HEADER));

    let queue = &app.encode_queue;
    let rows: Vec<Row> = app
        .filtered_rows
        .iter()
        .enumerate()
        .map(|(row_idx, row)| {
            let row_bg = if row_idx % 2 == 0 { theme::BG } else { theme::BG_ALT };
            match *row {
                ListRow::Media(file_idx) => {
                    let file = &app.files[file_idx];
                    let in_subfolder = file
                        .path
                        .parent()
                        .map(|p| p != app.root_path)
                        .unwrap_or(false);
                    let mark = queue_mark_for_index(queue, file_idx);
                    build_media_row(file, row_bg, &vis, app.grouped && in_subfolder, mark, queue, file_idx)
                }
                ListRow::Folder(folder_idx) => {
                    let folder = &app.folders[folder_idx];
                    let display = folder_display_path(&folder.path, &app.root_path);
                    build_folder_row(
                        &display,
                        folder.recursive_size,
                        folder.file_count,
                        row_bg,
                        &vis,
                    )
                }
            }
        })
        .collect();

    // Column widths
    let all_widths = [
        (COL_NAME, Constraint::Min(20)),
        (COL_CODEC, Constraint::Length(8)),
        (COL_SIZE, Constraint::Length(10)),
        (COL_BITRATE, Constraint::Length(12)),
        (COL_RESOLUTION, Constraint::Length(12)),
        (COL_DURATION, Constraint::Length(10)),
    ];

    let widths: Vec<Constraint> = all_widths
        .iter()
        .filter(|(col, _)| vis.is_visible(*col))
        .map(|(_, w)| *w)
        .collect();

    // Title with path
    let filter_label = app.codec_filter.label();
    let root_display = app.root_path.to_string_lossy();
    let max_path_len = (area.width as usize).saturating_sub(40);
    let root_short = if root_display.len() > max_path_len && max_path_len > 4 {
        format!("...{}", &root_display[root_display.len() - max_path_len + 3..])
    } else {
        root_display.to_string()
    };

    let title = Line::from(vec![
        Span::styled(" MCC ", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("─ ", Style::default().fg(theme::BORDER)),
        Span::styled(root_short, Style::default().fg(theme::TEXT_DIM)),
        Span::styled(" ─ ", Style::default().fg(theme::BORDER)),
        Span::styled(format!("{}", app.files.len()), Style::default().fg(theme::TEXT_BRIGHT)),
        Span::styled(" files ", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(format!("[{}] ", filter_label), Style::default().fg(theme::ACCENT)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active { theme::border_active_style() } else { theme::border_style() })
        .title(title);

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .column_spacing(1);

    app.list_state.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut app.list_state);
}

fn folder_display_path(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy();
    if s.is_empty() {
        "./".to_string()
    } else {
        format!("{}/", s)
    }
}

fn build_media_row<'a>(
    file: &'a crate::model::MediaFile,
    row_bg: ratatui::style::Color,
    vis: &VisibleColumns,
    indented: bool,
    mark: QueueMark,
    queue: &[EncodeJob],
    file_index: usize,
) -> Row<'a> {
    let codec = file.primary_video_codec();
    let color = theme::codec_color(codec);

    // Show encoding status instead of codec when file is in the queue
    let codec_cell = if let Some(job) = queue.iter().find(|j| j.file_index == file_index && !j.status.is_finished()) {
        match &job.status {
            EncodeJobStatus::Queued => {
                Cell::from(Span::styled("queued", Style::default().fg(theme::ACCENT)))
            }
            EncodeJobStatus::CopyingToTemp => {
                Cell::from(Span::styled("copy..", Style::default().fg(theme::ACCENT)))
            }
            EncodeJobStatus::Encoding => {
                let pct = job.progress.as_ref().map(|p| {
                    if let Some(total) = job.total_frames.filter(|&t| t > 0) {
                        (p.frame as f64 / total as f64 * 100.0).min(100.0)
                    } else {
                        p.percent
                    }
                }).unwrap_or(0.0);
                Cell::from(Span::styled(
                    format!("{:.0}%", pct),
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ))
            }
            EncodeJobStatus::Paused => {
                let pct = job.progress.as_ref().map(|p| {
                    if let Some(total) = job.total_frames.filter(|&t| t > 0) {
                        (p.frame as f64 / total as f64 * 100.0).min(100.0)
                    } else {
                        p.percent
                    }
                }).unwrap_or(0.0);
                Cell::from(Span::styled(
                    format!("{:.0}%\u{258b}", pct), // ▋ pause indicator
                    Style::default().fg(theme::CODEC_H264).add_modifier(Modifier::BOLD),
                ))
            }
            EncodeJobStatus::Validating => {
                Cell::from(Span::styled("check", Style::default().fg(theme::ACCENT)))
            }
            _ => Cell::from(Span::styled(codec.unwrap_or("-"), Style::default().fg(color))),
        }
    } else {
        match &file.probe_status {
            ProbeStatus::Pending | ProbeStatus::Probing => {
                Cell::from(Span::styled("···", Style::default().fg(theme::CODEC_PENDING)))
            }
            ProbeStatus::Error(_) => {
                Cell::from(Span::styled("ERR", Style::default().fg(theme::CODEC_ERROR)))
            }
            ProbeStatus::Done => Cell::from(Span::styled(
                codec.unwrap_or("-"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
        }
    };

    let size_cell = Cell::from(
        Line::from(human_file_size(file.file_size))
            .alignment(ratatui::layout::Alignment::Right),
    )
    .style(Style::default().fg(theme::TEXT));

    let bitrate_cell = Cell::from(
        Line::from(match (file.is_probed(), file.primary_bitrate()) {
            (true, Some(br)) => human_bitrate(br),
            _ => String::new(),
        })
        .alignment(ratatui::layout::Alignment::Right),
    )
    .style(Style::default().fg(theme::TEXT));

    let resolution_cell = Cell::from(
        Line::from(if file.is_probed() { file.resolution_str() } else { String::new() })
            .alignment(ratatui::layout::Alignment::Right),
    )
    .style(Style::default().fg(theme::TEXT_DIM));

    let duration_cell = Cell::from(
        Line::from(match (file.is_probed(), file.duration_secs) {
            (true, Some(d)) => human_duration(d),
            _ => String::new(),
        })
        .alignment(ratatui::layout::Alignment::Right),
    )
    .style(Style::default().fg(theme::TEXT_DIM));

    let indent = if indented { " " } else { "" };
    let name_cell = Cell::from(Line::from(vec![
        Span::styled(
            format!(" {}", mark.symbol()),
            Style::default().fg(mark.color()),
        ),
        Span::styled(
            format!("{}{}", indent, file.file_name()),
            Style::default().fg(theme::TEXT),
        ),
    ]));

    let all_cells = [
        (COL_NAME, name_cell),
        (COL_CODEC, codec_cell),
        (COL_SIZE, size_cell),
        (COL_BITRATE, bitrate_cell),
        (COL_RESOLUTION, resolution_cell),
        (COL_DURATION, duration_cell),
    ];

    let cells: Vec<Cell> = all_cells
        .into_iter()
        .filter(|(col, _)| vis.is_visible(*col))
        .map(|(_, cell)| cell)
        .collect();

    Row::new(cells).style(Style::default().bg(row_bg))
}

fn build_folder_row<'a>(
    display_path: &str,
    recursive_size: u64,
    file_count: usize,
    row_bg: ratatui::style::Color,
    vis: &VisibleColumns,
) -> Row<'a> {
    let header_fg = theme::SECTION_HEADER;
    let name_style = Style::default().fg(header_fg).add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme::TEXT_DIM);

    let name_cell = Cell::from(Line::from(vec![
        Span::styled(" ▸ ", Style::default().fg(theme::ACCENT)),
        Span::styled(display_path.to_string(), name_style),
        Span::styled(
            format!("  {} files", file_count),
            dim_style,
        ),
    ]));

    let empty = Cell::from("");

    let size_cell = Cell::from(
        Line::from(human_file_size(recursive_size))
            .alignment(ratatui::layout::Alignment::Right),
    )
    .style(Style::default().fg(header_fg).add_modifier(Modifier::BOLD));

    let all_cells = [
        (COL_NAME, name_cell),
        (COL_CODEC, empty.clone()),
        (COL_SIZE, size_cell),
        (COL_BITRATE, empty.clone()),
        (COL_RESOLUTION, empty.clone()),
        (COL_DURATION, empty),
    ];

    let cells: Vec<Cell> = all_cells
        .into_iter()
        .filter(|(col, _)| vis.is_visible(*col))
        .map(|(_, cell)| cell)
        .collect();

    Row::new(cells).style(Style::default().bg(row_bg))
}

fn queue_mark_for_index(queue: &[crate::model::EncodeJob], file_index: usize) -> QueueMark {
    for job in queue {
        if job.file_index != file_index {
            continue;
        }
        return match &job.status {
            EncodeJobStatus::Queued => QueueMark::Queued,
            EncodeJobStatus::CopyingToTemp
            | EncodeJobStatus::Encoding
            | EncodeJobStatus::Paused
            | EncodeJobStatus::Validating => QueueMark::Encoding,
            EncodeJobStatus::Done { .. } => QueueMark::Done,
            EncodeJobStatus::Failed(_) | EncodeJobStatus::Cancelled => QueueMark::Failed,
        };
    }
    QueueMark::None
}
