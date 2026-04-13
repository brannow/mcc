use std::path::Path;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::model::{human_bitrate, human_duration, human_file_size, FolderRow, MediaFile, ProbeStatus};
use super::theme;

pub enum DetailPayload {
    File(MediaFile),
    Folder(FolderRow),
}

fn label_value(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<16}", label),
            Style::default().fg(theme::LABEL),
        ),
        Span::styled(value.into(), Style::default().fg(theme::TEXT)),
    ])
}

fn section_header(title: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        title.into(),
        Style::default()
            .fg(theme::SECTION_HEADER)
            .add_modifier(Modifier::BOLD),
    ))
}

fn codec_value(codec: &str) -> Line<'static> {
    let color = theme::codec_color(Some(codec));
    Line::from(vec![
        Span::styled(
            format!("  {:<16}", "Codec:"),
            Style::default().fg(theme::LABEL),
        ),
        Span::styled(codec.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
    ])
}

pub fn render_detail(
    f: &mut Frame,
    payload: &DetailPayload,
    root_path: &Path,
    area: ratatui::layout::Rect,
    active: bool,
    scroll: u16,
) {
    let lines: Vec<Line> = match payload {
        DetailPayload::File(file) => build_file_lines(file, root_path),
        DetailPayload::Folder(folder) => build_folder_lines(folder, root_path),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active { theme::border_active_style() } else { theme::border_style() })
        .title(Span::styled(" Detail ", theme::title_style()));

    let inner_height = area.height.saturating_sub(2);
    let max_scroll = (lines.len() as u16).saturating_sub(inner_height);
    let clamped_scroll = scroll.min(max_scroll);

    let detail = Paragraph::new(lines).block(block).scroll((clamped_scroll, 0));
    f.render_widget(detail, area);
}

fn build_folder_lines(folder: &FolderRow, root_path: &Path) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let relative_path = folder
        .path
        .strip_prefix(root_path)
        .unwrap_or(&folder.path)
        .to_string_lossy()
        .to_string();

    lines.push(section_header(" FOLDER"));
    lines.push(label_value("Path:", format!("{}/", relative_path)));
    lines.push(label_value("Total size:", human_file_size(folder.recursive_size)));
    lines.push(label_value("Media files:", folder.file_count.to_string()));
    lines
}

fn build_file_lines(file: &MediaFile, root_path: &Path) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    let relative_path = file.path.strip_prefix(root_path)
        .unwrap_or(&file.path)
        .to_string_lossy()
        .to_string();

    lines.push(section_header(" FILE"));
    lines.push(label_value("Path:", relative_path));
    lines.push(label_value("Size:", human_file_size(file.file_size)));
    if let Some(fmt) = &file.container_format {
        lines.push(label_value("Format:", fmt.clone()));
    }
    if let Some(dur) = file.duration_secs {
        lines.push(label_value("Duration:", human_duration(dur)));
    }

    match &file.probe_status {
        ProbeStatus::Pending | ProbeStatus::Probing => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Probing...",
                Style::default().fg(theme::CODEC_PENDING),
            )));
        }
        ProbeStatus::Error(e) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  Error: {}", e),
                Style::default().fg(theme::CODEC_ERROR),
            )));
        }
        ProbeStatus::Done => {
            for (i, vs) in file.video_streams.iter().enumerate() {
                lines.push(Line::from(""));
                lines.push(section_header(format!(" VIDEO #{}", i + 1)));
                lines.push(codec_value(&vs.codec));
                if let Some(long) = &vs.codec_long {
                    lines.push(label_value("Decoder:", long.clone()));
                }
                if vs.width > 0 {
                    lines.push(label_value("Resolution:", format!("{}x{}", vs.width, vs.height)));
                }
                if let Some(br) = vs.bitrate {
                    lines.push(label_value("Bitrate:", human_bitrate(br)));
                }
                if let Some(fps) = vs.fps {
                    lines.push(label_value("FPS:", format!("{:.6}", fps)));
                }
                if let Some(fc) = vs.frame_count {
                    lines.push(label_value("Frames:", format!("{}", fc)));
                }
                if let Some(pf) = &vs.pixel_format {
                    lines.push(label_value("Pixel Format:", pf.clone()));
                }
            }

            for (i, audio) in file.audio_streams.iter().enumerate() {
                lines.push(Line::from(""));
                lines.push(section_header(format!(" AUDIO #{}", i + 1)));
                lines.push(label_value("Codec:", audio.codec.clone()));
                if let Some(long) = &audio.codec_long {
                    lines.push(label_value("Decoder:", long.clone()));
                }
                lines.push(label_value("Channels:", audio.channels.to_string()));
                lines.push(label_value("Sample Rate:", format!("{} Hz", audio.sample_rate)));
                if let Some(br) = audio.bitrate {
                    lines.push(label_value("Bitrate:", human_bitrate(br)));
                }
                if let Some(lang) = &audio.language {
                    lines.push(label_value("Language:", lang.clone()));
                }
            }

            if !file.subtitle_streams.is_empty() {
                lines.push(Line::from(""));
                lines.push(section_header(" SUBTITLES"));
                for sub in &file.subtitle_streams {
                    let mut desc = sub.codec.clone();
                    if let Some(lang) = &sub.language {
                        desc.push_str(&format!(" [{}]", lang));
                    }
                    if let Some(title) = &sub.title {
                        desc.push_str(&format!(" \"{}\"", title));
                    }
                    lines.push(label_value("Track:", desc));
                }
            }
        }
    }

    lines
}
