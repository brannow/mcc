use std::path::PathBuf;
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::model::{AudioStream, MediaFile, SubtitleStream, VideoStream};

// ffprobe JSON structures
#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    streams: Option<Vec<FfprobeStream>>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    codec_long_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    bit_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    pix_fmt: Option<String>,
    channels: Option<u32>,
    sample_rate: Option<String>,
    tags: Option<FfprobeTags>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeTags {
    language: Option<String>,
    title: Option<String>,
    #[serde(alias = "BPS")]
    bps: Option<String>,
    #[serde(alias = "BPS-eng")]
    bps_eng: Option<String>,
    #[serde(alias = "NUMBER_OF_FRAMES")]
    number_of_frames: Option<String>,
    #[serde(rename = "NUMBER_OF_FRAMES-eng")]
    number_of_frames_eng: Option<String>,
}

#[derive(Debug)]
pub struct ProbeResult {
    pub path: PathBuf,
    pub data: Result<ProbeData, String>,
}

#[derive(Debug)]
pub struct ProbeData {
    pub container_format: Option<String>,
    pub duration_secs: Option<f64>,
    pub format_bitrate: Option<u64>,
    pub video_streams: Vec<VideoStream>,
    pub audio_streams: Vec<AudioStream>,
    pub subtitle_streams: Vec<SubtitleStream>,
}

fn parse_frame_count(stream: &FfprobeStream) -> Option<u64> {
    // Try nb_frames field first, then NUMBER_OF_FRAMES-eng, then NUMBER_OF_FRAMES from tags
    stream.nb_frames.as_deref().and_then(|v| v.parse().ok())
        .or_else(|| stream.tags.as_ref().and_then(|t| t.number_of_frames_eng.as_deref()).and_then(|v| v.parse().ok()))
        .or_else(|| stream.tags.as_ref().and_then(|t| t.number_of_frames.as_deref()).and_then(|v| v.parse().ok()))
}

fn parse_bitrate(stream: &FfprobeStream) -> Option<u64> {
    // Try bit_rate field first, then tags.BPS, then tags.BPS-eng
    stream.bit_rate.as_deref().and_then(|b| b.parse().ok())
        .or_else(|| stream.tags.as_ref().and_then(|t| t.bps.as_deref()).and_then(|b| b.parse().ok()))
        .or_else(|| stream.tags.as_ref().and_then(|t| t.bps_eng.as_deref()).and_then(|b| b.parse().ok()))
}

fn parse_frame_rate(rate: &str) -> Option<f64> {
    let parts: Vec<&str> = rate.split('/').collect();
    if parts.len() == 2 {
        let num: f64 = parts[0].parse().ok()?;
        let den: f64 = parts[1].parse().ok()?;
        if den > 0.0 {
            return Some(num / den);
        }
    }
    rate.parse().ok()
}

async fn probe_file(path: &PathBuf) -> Result<ProbeData, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|e| format!("failed to run ffprobe: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe exited with {}",
            output.status.code().unwrap_or(-1)
        ));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse ffprobe JSON: {}", e))?;

    let mut video_streams = Vec::new();
    let mut audio_streams = Vec::new();
    let mut subtitle_streams = Vec::new();

    if let Some(streams) = parsed.streams {
        for s in streams {
            let codec_type = s.codec_type.as_deref().unwrap_or("");
            match codec_type {
                "video" => {
                    let bitrate = parse_bitrate(&s);
                    let frame_count = parse_frame_count(&s);
                    video_streams.push(VideoStream {
                        codec: s.codec_name.unwrap_or_default(),
                        codec_long: s.codec_long_name,
                        width: s.width.unwrap_or(0),
                        height: s.height.unwrap_or(0),
                        bitrate,
                        fps: s.r_frame_rate.as_deref().and_then(parse_frame_rate),
                        pixel_format: s.pix_fmt,
                        frame_count,
                    });
                }
                "audio" => {
                    let bitrate = parse_bitrate(&s);
                    audio_streams.push(AudioStream {
                        codec: s.codec_name.unwrap_or_default(),
                        codec_long: s.codec_long_name,
                        channels: s.channels.unwrap_or(0),
                        sample_rate: s
                            .sample_rate
                            .as_deref()
                            .and_then(|r| r.parse().ok())
                            .unwrap_or(0),
                        bitrate,
                        language: s.tags.as_ref().and_then(|t| t.language.clone()),
                    });
                }
                "subtitle" => {
                    subtitle_streams.push(SubtitleStream {
                        codec: s.codec_name.unwrap_or_default(),
                        language: s.tags.as_ref().and_then(|t| t.language.clone()),
                        title: s.tags.as_ref().and_then(|t| t.title.clone()),
                    });
                }
                _ => {}
            }
        }
    }

    let (container_format, duration_secs, format_bitrate) = match parsed.format {
        Some(fmt) => (
            fmt.format_name,
            fmt.duration.as_deref().and_then(|d| d.parse().ok()),
            fmt.bit_rate.as_deref().and_then(|b| b.parse().ok()),
        ),
        None => (None, None, None),
    };

    Ok(ProbeData {
        container_format,
        duration_secs,
        format_bitrate,
        video_streams,
        audio_streams,
        subtitle_streams,
    })
}

pub fn apply_probe_result(file: &mut MediaFile, data: ProbeData) {
    file.container_format = data.container_format;
    file.duration_secs = data.duration_secs;
    file.video_streams = data.video_streams;
    file.audio_streams = data.audio_streams;
    file.subtitle_streams = data.subtitle_streams;

    // Fallback bitrate for primary video stream: format-level, then calculated from file size
    if let Some(vs) = file.video_streams.first_mut() {
        if vs.bitrate.is_none() {
            vs.bitrate = data.format_bitrate.or_else(|| {
                data.duration_secs
                    .filter(|&d| d > 0.0)
                    .map(|d| (file.file_size * 8) / d as u64)
            });
        }
    }
}

/// Spawns background probing workers. Returns a receiver for completed results.
/// Send file paths via the returned sender, results come back on the receiver.
pub fn start_background_prober(
    concurrency: usize,
) -> (mpsc::UnboundedSender<PathBuf>, mpsc::UnboundedReceiver<ProbeResult>) {
    let (path_tx, mut path_rx) = mpsc::unbounded_channel::<PathBuf>();
    let (result_tx, result_rx) = mpsc::unbounded_channel::<ProbeResult>();

    tokio::spawn(async move {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
        let result_tx = std::sync::Arc::new(result_tx);

        while let Some(path) = path_rx.recv().await {
            let sem = semaphore.clone();
            let tx = result_tx.clone();
            tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let data = probe_file(&path).await;
                let _ = tx.send(ProbeResult { path, data });
            });
        }
    });

    (path_tx, result_rx)
}
