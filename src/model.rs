use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum ProbeStatus {
    Pending,
    #[allow(dead_code)]
    Probing,
    Done,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct VideoStream {
    pub codec: String,
    pub codec_long: Option<String>,
    pub width: u32,
    pub height: u32,
    pub bitrate: Option<u64>,
    pub fps: Option<f64>,
    pub pixel_format: Option<String>,
    pub frame_count: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AudioStream {
    pub codec: String,
    pub codec_long: Option<String>,
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate: Option<u64>,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubtitleStream {
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub file_size: u64,
    pub probe_status: ProbeStatus,
    pub container_format: Option<String>,
    pub duration_secs: Option<f64>,
    pub video_streams: Vec<VideoStream>,
    pub audio_streams: Vec<AudioStream>,
    pub subtitle_streams: Vec<SubtitleStream>,
}

impl MediaFile {
    pub fn new(path: PathBuf, file_size: u64) -> Self {
        Self {
            path,
            file_size,
            probe_status: ProbeStatus::Pending,
            container_format: None,
            duration_secs: None,
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_streams: Vec::new(),
        }
    }

    pub fn file_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("???")
    }

    pub fn primary_video_codec(&self) -> Option<&str> {
        self.video_streams.first().map(|v| v.codec.as_str())
    }

    pub fn resolution_str(&self) -> String {
        match self.video_streams.first() {
            Some(v) if v.width > 0 => format!("{}x{}", v.width, v.height),
            _ => String::new(),
        }
    }

    pub fn primary_bitrate(&self) -> Option<u64> {
        self.video_streams.first().and_then(|v| v.bitrate)
    }

    pub fn is_probed(&self) -> bool {
        matches!(self.probe_status, ProbeStatus::Done | ProbeStatus::Error(_))
    }
}

#[derive(Debug, Clone)]
pub struct FolderRow {
    pub path: PathBuf,
    pub recursive_size: u64,
    pub file_count: usize,
}

pub fn human_file_size(bytes: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    const KB: u64 = 1 << 10;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn human_bitrate(bits_per_sec: u64) -> String {
    if bits_per_sec >= 1_000_000 {
        format!("{:.2} Mbps", bits_per_sec as f64 / 1_000_000.0)
    } else if bits_per_sec >= 1_000 {
        format!("{:.0} Kbps", bits_per_sec as f64 / 1_000.0)
    } else {
        format!("{} bps", bits_per_sec)
    }
}

// ── Encoding types ──────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)] // variants constructed by encoder (not yet implemented)
pub enum EncodeJobStatus {
    Queued,
    CopyingToTemp,
    Encoding,
    Paused,
    Validating,
    Done {
        encoded_size: u64,
        saved_percent: f64,
    },
    Failed(String),
    Cancelled,
}

impl EncodeJobStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Queued => "Queued",
            Self::CopyingToTemp => "Copying",
            Self::Encoding => "Encoding",
            Self::Paused => "Paused",
            Self::Validating => "Validating",
            Self::Done { .. } => "Done",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed(_) | Self::Cancelled)
    }

    pub fn is_removable(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Done { .. } | Self::Failed(_) | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct FfmpegProgress {
    pub frame: u64,
    pub fps: f64,
    pub bitrate_kbps: f64,
    pub total_size: u64,
    pub out_time_secs: f64,
    pub speed: f64,
    pub percent: f64,
}

/// Max samples kept in rolling history. Each braille char encodes 2 samples,
/// so 200 samples = up to 100 chars wide — plenty for any terminal.
const FPS_HISTORY_LEN: usize = 200;

/// How many progress ticks to skip between graph samples (~1 tick/sec from ffmpeg,
/// so 2 = ~2 second intervals).
const GRAPH_SAMPLE_INTERVAL: u64 = 2;

#[derive(Debug, Clone, Default)]
pub struct FpsStats {
    pub current: f64,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub history: Vec<f64>,
    sample_count: u64,
    sample_sum: f64,
    ticks_since_sample: u64,
}

impl FpsStats {
    pub fn update(&mut self, fps: f64) {
        if fps <= 0.0 {
            return;
        }
        self.current = fps;
        if self.sample_count == 0 {
            self.min = fps;
            self.max = fps;
        } else {
            if fps < self.min {
                self.min = fps;
            }
            if fps > self.max {
                self.max = fps;
            }
        }
        self.sample_count += 1;
        self.sample_sum += fps;
        self.avg = self.sample_sum / self.sample_count as f64;

        // Only push a graph sample every N ticks
        self.ticks_since_sample += 1;
        if self.ticks_since_sample >= GRAPH_SAMPLE_INTERVAL {
            self.ticks_since_sample = 0;
            if self.history.len() >= FPS_HISTORY_LEN {
                self.history.remove(0);
            }
            self.history.push(fps);
        }
    }

    /// Render a braille dot graph for the given `width` (in terminal columns)
    /// and `height` (in terminal rows). Each braille character is a 2x4 dot grid,
    /// so we get `width * 2` horizontal points and `height * 4` vertical levels.
    pub fn braille_graph(&self, width: usize, height: usize) -> Vec<String> {
        let dots_x = width * 2;
        let dots_y = height * 4;

        if self.history.is_empty() || dots_x == 0 || dots_y == 0 {
            return vec![String::new(); height];
        }

        // Take the last dots_x samples (or pad left with None)
        let sample_count = self.history.len().min(dots_x);
        let offset = self.history.len().saturating_sub(dots_x);
        let samples = &self.history[offset..];

        let lo = samples.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let range = if (hi - lo) < 0.01 { 1.0 } else { hi - lo };

        // Map each sample to a dot row (0 = bottom, dots_y-1 = top)
        let dot_rows: Vec<usize> = samples
            .iter()
            .map(|&v| {
                let normalized = (v - lo) / range;
                (normalized * (dots_y - 1) as f64).round() as usize
            })
            .collect();

        // Build a 2D grid of braille dots: grid[char_row][char_col] stores
        // which of the 8 dots in that braille char are set.
        // Braille dot positions (Unicode offset bits):
        //   col0: row0=0x01, row1=0x02, row2=0x04, row3=0x40
        //   col1: row0=0x08, row1=0x10, row2=0x20, row3=0x80
        let mut grid = vec![vec![0u8; width]; height];

        let dot_bits: [[u8; 4]; 2] = [
            [0x01, 0x02, 0x04, 0x40], // left column (dot_x even)
            [0x08, 0x10, 0x20, 0x80], // right column (dot_x odd)
        ];

        // Pad: right-align the data points
        let x_offset = dots_x.saturating_sub(sample_count);
        for (i, &dot_y) in dot_rows.iter().enumerate() {
            let dx = x_offset + i;
            let char_col = dx / 2;
            let sub_col = dx % 2;

            // dot_y is from bottom; convert to row from top
            let inverted_y = (dots_y - 1) - dot_y;
            let char_row = inverted_y / 4;
            let sub_row = inverted_y % 4;

            if char_col < width && char_row < height {
                grid[char_row][char_col] |= dot_bits[sub_col][sub_row];
            }
        }

        // Convert grid to braille strings
        grid.iter()
            .map(|row| {
                row.iter()
                    .map(|&bits| char::from_u32(0x2800 + bits as u32).unwrap_or(' '))
                    .collect()
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct EncodeJob {
    #[allow(dead_code)] // used by encoder for job tracking
    pub id: u64,
    pub file_index: usize,
    pub file_name: String,
    pub file_size: u64,
    pub duration_secs: Option<f64>,
    pub total_frames: Option<u64>,
    pub status: EncodeJobStatus,
    pub progress: Option<FfmpegProgress>,
    pub fps_stats: FpsStats,
    pub started_at: Option<std::time::Instant>,
    pub elapsed_secs: Option<f64>,
    pub preset_name: String,
}

pub fn human_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

impl fmt::Display for VideoStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}x{}", self.codec, self.width, self.height)?;
        if let Some(fps) = self.fps {
            write!(f, " {:.3}fps", fps)?;
        }
        Ok(())
    }
}

impl fmt::Display for AudioStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}ch {}Hz",
            self.codec, self.channels, self.sample_rate
        )?;
        if let Some(lang) = &self.language {
            write!(f, " [{}]", lang)?;
        }
        Ok(())
    }
}

impl fmt::Display for SubtitleStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.codec)?;
        if let Some(lang) = &self.language {
            write!(f, " [{}]", lang)?;
        }
        if let Some(title) = &self.title {
            write!(f, " \"{}\"", title)?;
        }
        Ok(())
    }
}
