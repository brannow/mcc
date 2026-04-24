use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::model::FfmpegProgress;
use crate::preset::EncodingPreset;

// ── Channel types ──────────────────────────────────────────────

/// Request sent from App to encoder worker to start a job.
#[derive(Debug)]
pub struct EncodeRequest {
    pub job_id: u64,
    pub source_path: PathBuf,
    pub file_size: u64,
    pub duration_secs: Option<f64>,
    pub preset: EncodingPreset,
}

/// Control signals sent from App to encoder worker.
#[derive(Debug)]
pub enum EncodeControl {
    Pause,
    Resume,
    Cancel,
}

/// Events sent from encoder worker back to App.
#[derive(Debug)]
pub enum EncodeEvent {
    StatusChange {
        job_id: u64,
        status: EncodeStatus,
    },
    Progress {
        job_id: u64,
        progress: FfmpegProgress,
    },
    Completed {
        job_id: u64,
        result: EncodeResult,
    },
}

#[derive(Debug)]
pub enum EncodeStatus {
    CopyingToTemp,
    Encoding,
    Paused,
    Resumed,
    Validating,
}

#[derive(Debug)]
pub enum EncodeResult {
    Success {
        encoded_size: u64,
        saved_percent: f64,
        final_path: PathBuf,
    },
    Failed(String),
    Cancelled,
}

// ── Encoder worker ─────────────────────────────────────────────

pub struct EncoderHandle {
    pub request_tx: mpsc::UnboundedSender<EncodeRequest>,
    pub control_tx: mpsc::UnboundedSender<EncodeControl>,
    pub event_rx: mpsc::UnboundedReceiver<EncodeEvent>,
}

pub fn start_encoder() -> EncoderHandle {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<EncodeRequest>();
    let (control_tx, control_rx) = mpsc::unbounded_channel::<EncodeControl>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<EncodeEvent>();

    tokio::spawn(async move {
        encoder_loop(&mut request_rx, control_rx, event_tx).await;
    });

    EncoderHandle {
        request_tx,
        control_tx,
        event_rx,
    }
}

async fn encoder_loop(
    request_rx: &mut mpsc::UnboundedReceiver<EncodeRequest>,
    mut control_rx: mpsc::UnboundedReceiver<EncodeControl>,
    event_tx: mpsc::UnboundedSender<EncodeEvent>,
) {
    while let Some(request) = request_rx.recv().await {
        let result = encode_one(&request, &mut control_rx, &event_tx).await;

        let _ = event_tx.send(EncodeEvent::Completed {
            job_id: request.job_id,
            result,
        });

        // Drain stale control signals between jobs
        while control_rx.try_recv().is_ok() {}
    }
}

async fn encode_one(
    request: &EncodeRequest,
    control_rx: &mut mpsc::UnboundedReceiver<EncodeControl>,
    event_tx: &mpsc::UnboundedSender<EncodeEvent>,
) -> EncodeResult {
    let job_id = request.job_id;
    let preset = &request.preset;

    // Ensure temp dir exists
    if let Err(e) = tokio::fs::create_dir_all(&preset.temp_dir).await {
        return EncodeResult::Failed(format!("Failed to create temp dir: {}", e));
    }

    let source_ext = request
        .source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv");
    let source_md5 = simple_hash(&request.source_path);
    let temp_source = preset
        .temp_dir
        .join(format!("h264_{}.{}", source_md5, source_ext));
    let temp_output = preset
        .temp_dir
        .join(format!("hevc_{}.{}", source_md5, preset.target_format));

    // Step 1: Copy source to temp
    let _ = event_tx.send(EncodeEvent::StatusChange {
        job_id,
        status: EncodeStatus::CopyingToTemp,
    });

    if let Err(e) = tokio::fs::copy(&request.source_path, &temp_source).await {
        cleanup_temp(&temp_source, &temp_output).await;
        return EncodeResult::Failed(format!("Copy to temp failed: {}", e));
    }

    // Step 2: Run ffmpeg
    let _ = event_tx.send(EncodeEvent::StatusChange {
        job_id,
        status: EncodeStatus::Encoding,
    });

    let encode_result = run_ffmpeg(
        job_id,
        &temp_source,
        &temp_output,
        preset,
        request.duration_secs,
        control_rx,
        event_tx,
    )
    .await;

    match encode_result {
        FfmpegResult::Success => {}
        FfmpegResult::Cancelled => {
            cleanup_temp(&temp_source, &temp_output).await;
            return EncodeResult::Cancelled;
        }
        FfmpegResult::Failed(msg) => {
            // Determine which fallback to try based on error
            let fallback = if msg.contains("attached pic") || msg.contains("attached_pic") {
                Some(Fallback::AttachedPic)
            } else if msg.contains("unknown codec")
                || msg.contains("Unknown codec")
                || msg.contains("Could not find codec parameters")
                || msg.contains("unknown encoder")
            {
                Some(Fallback::ReencodeAudio)
            } else if msg.contains("Only audio, video, and subtitles") {
                Some(Fallback::StreamFilter)
            } else if msg.contains("Subtitle") || msg.contains("subtitle") {
                Some(Fallback::SubtitleCodec)
            } else {
                None
            };

            match fallback {
                Some(fb) => {
                    let retry = run_ffmpeg_with_fallback(
                        job_id,
                        &temp_source,
                        &temp_output,
                        preset,
                        request.duration_secs,
                        control_rx,
                        event_tx,
                        fb,
                    )
                    .await;
                    match retry {
                        FfmpegResult::Success => {}
                        FfmpegResult::Cancelled => {
                            cleanup_temp(&temp_source, &temp_output).await;
                            return EncodeResult::Cancelled;
                        }
                        FfmpegResult::Failed(msg2) => {
                            // If re-encoding audio also failed with codec errors,
                            // last resort: video-only (drop undecodable audio)
                            if matches!(fb, Fallback::ReencodeAudio)
                                && (msg2.contains("unknown codec")
                                    || msg2.contains("Could not find codec parameters"))
                            {
                                let last_try = run_ffmpeg_with_fallback(
                                    job_id,
                                    &temp_source,
                                    &temp_output,
                                    preset,
                                    request.duration_secs,
                                    control_rx,
                                    event_tx,
                                    Fallback::VideoOnly,
                                )
                                .await;
                                match last_try {
                                    FfmpegResult::Success => {}
                                    FfmpegResult::Cancelled => {
                                        cleanup_temp(&temp_source, &temp_output).await;
                                        return EncodeResult::Cancelled;
                                    }
                                    FfmpegResult::Failed(msg3) => {
                                        cleanup_temp(&temp_source, &temp_output).await;
                                        return EncodeResult::Failed(msg3);
                                    }
                                }
                            } else {
                                cleanup_temp(&temp_source, &temp_output).await;
                                return EncodeResult::Failed(msg2);
                            }
                        }
                    }
                }
                None => {
                    cleanup_temp(&temp_source, &temp_output).await;
                    return EncodeResult::Failed(msg);
                }
            }
        }
    }

    // Step 3: Copy encoded file next to original as _hevc_<name>.<target_format>
    let original_dir = request
        .source_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let original_stem = request
        .source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    let hevc_copy_name = format!("_hevc_{}.{}", original_stem, preset.target_format);
    let hevc_copy_path = original_dir.join(&hevc_copy_name);

    if let Err(e) = tokio::fs::copy(&temp_output, &hevc_copy_path).await {
        cleanup_temp(&temp_source, &temp_output).await;
        return EncodeResult::Failed(format!("Copy encoded file failed: {}", e));
    }

    // Clean up temp files
    cleanup_temp(&temp_source, &temp_output).await;

    // Step 4: Validate
    let _ = event_tx.send(EncodeEvent::StatusChange {
        job_id,
        status: EncodeStatus::Validating,
    });

    match validate_encoded(
        &hevc_copy_path,
        &request.source_path,
        request.duration_secs,
        &preset.target_codec,
    )
    .await
    {
        Ok(()) => {
            // Check if encoding actually saved space
            let encoded_size = match tokio::fs::metadata(&hevc_copy_path).await {
                Ok(m) => m.len(),
                Err(e) => {
                    return EncodeResult::Failed(format!(
                        "Failed to read encoded file size: {}",
                        e
                    ));
                }
            };

            if encoded_size >= request.file_size {
                let _ = tokio::fs::remove_file(&hevc_copy_path).await;
                let increase = (encoded_size as f64 / request.file_size as f64 - 1.0) * 100.0;
                return EncodeResult::Failed(format!(
                    "Skipped: HEVC encode is larger than original ({:.1}% bigger, {} → {}). Keeping original file.",
                    increase,
                    format_size(request.file_size),
                    format_size(encoded_size),
                ));
            }

            // Copy permissions from original
            if let Ok(meta) = std::fs::metadata(&request.source_path) {
                let _ = std::fs::set_permissions(&hevc_copy_path, meta.permissions());
            }

            // Remove original
            if let Err(e) = tokio::fs::remove_file(&request.source_path).await {
                let _ = tokio::fs::remove_file(&hevc_copy_path).await;
                return EncodeResult::Failed(format!("Failed to remove original: {}", e));
            }

            // Rename: strip encoding hints and remove _hevc_ prefix
            let final_name = remove_encoding_hints(original_stem);
            let final_path = original_dir.join(format!("{}.{}", final_name, preset.target_format));

            // Avoid overwriting if final path already exists (shouldn't happen, but safe)
            let final_path = if final_path.exists() && final_path != request.source_path {
                // Keep the _hevc_ prefixed name
                hevc_copy_path.clone()
            } else {
                final_path
            };

            if final_path != hevc_copy_path
                && let Err(e) = tokio::fs::rename(&hevc_copy_path, &final_path).await
            {
                // Non-fatal: file is encoded, just couldn't rename
                eprintln!("Warning: rename failed: {}", e);
            }

            let saved_percent = if request.file_size > 0 {
                (1.0 - (encoded_size as f64 / request.file_size as f64)) * 100.0
            } else {
                0.0
            };

            EncodeResult::Success {
                encoded_size,
                saved_percent,
                final_path,
            }
        }
        Err(reason) => {
            // Validation failed — remove the encoded copy
            let _ = tokio::fs::remove_file(&hevc_copy_path).await;
            EncodeResult::Failed(format!("Validation failed: {}", reason))
        }
    }
}

// ── ffmpeg execution ───────────────────────────────────────────

enum FfmpegResult {
    Success,
    Failed(String),
    Cancelled,
}

async fn run_ffmpeg(
    job_id: u64,
    input: &Path,
    output: &Path,
    preset: &EncodingPreset,
    duration_secs: Option<f64>,
    control_rx: &mut mpsc::UnboundedReceiver<EncodeControl>,
    event_tx: &mpsc::UnboundedSender<EncodeEvent>,
) -> FfmpegResult {
    run_ffmpeg_inner(
        job_id,
        input,
        output,
        &preset.ffmpeg_args,
        duration_secs,
        control_rx,
        event_tx,
    )
    .await
}

#[derive(Clone, Copy)]
enum Fallback {
    /// Replace "-map 0" with "-map 0:v:0 -map 0:a? -map 0:s?" to skip attached pictures
    AttachedPic,
    /// Replace "-map 0" with "-map 0:v -map 0:a -map 0:s?" to skip data/attachment streams
    StreamFilter,
    /// Insert "-c:s srt" after "-c copy" for subtitle codec issues
    SubtitleCodec,
    /// Replace "-c copy" with "-c:a aac -c:s copy" for unknown/unsupported audio codecs
    ReencodeAudio,
    /// Last resort: video + subtitles only, drop undecodable audio
    VideoOnly,
}

#[allow(clippy::too_many_arguments)]
async fn run_ffmpeg_with_fallback(
    job_id: u64,
    input: &Path,
    output: &Path,
    preset: &EncodingPreset,
    duration_secs: Option<f64>,
    control_rx: &mut mpsc::UnboundedReceiver<EncodeControl>,
    event_tx: &mpsc::UnboundedSender<EncodeEvent>,
    fallback: Fallback,
) -> FfmpegResult {
    // Remove existing output from failed first attempt
    let _ = tokio::fs::remove_file(output).await;

    let mut modified_args = Vec::new();
    match fallback {
        Fallback::AttachedPic => {
            // Replace "-map" "0" with "-map" "0:v:0" "-map" "0:a?" "-map" "0:s?"
            // This maps only the first (real) video stream, skipping attached pictures
            let mut i = 0;
            while i < preset.ffmpeg_args.len() {
                if preset.ffmpeg_args[i] == "-map"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("0")
                {
                    modified_args.push("-map".to_string());
                    modified_args.push("0:v:0".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:a?".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:s?".to_string());
                    i += 2;
                } else {
                    modified_args.push(preset.ffmpeg_args[i].clone());
                    i += 1;
                }
            }
        }
        Fallback::StreamFilter => {
            // Replace "-map" "0" with "-map" "0:v" "-map" "0:a" "-map" "0:s?"
            let mut i = 0;
            while i < preset.ffmpeg_args.len() {
                if preset.ffmpeg_args[i] == "-map"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("0")
                {
                    modified_args.push("-map".to_string());
                    modified_args.push("0:v".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:a".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:s?".to_string());
                    i += 2; // skip original "-map" "0"
                } else {
                    modified_args.push(preset.ffmpeg_args[i].clone());
                    i += 1;
                }
            }
        }
        Fallback::SubtitleCodec => {
            // Insert "-c:s srt" after "-c copy"
            let mut i = 0;
            while i < preset.ffmpeg_args.len() {
                modified_args.push(preset.ffmpeg_args[i].clone());
                if preset.ffmpeg_args[i] == "copy" && i > 0 && preset.ffmpeg_args[i - 1] == "-c" {
                    modified_args.push("-c:s".to_string());
                    modified_args.push("srt".to_string());
                }
                i += 1;
            }
        }
        Fallback::ReencodeAudio => {
            // Replace "-c copy" with "-c:a aac -c:s copy" to re-encode audio
            // Also filter streams to skip unsupported data tracks
            let mut i = 0;
            while i < preset.ffmpeg_args.len() {
                if preset.ffmpeg_args[i] == "-c"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("copy")
                {
                    modified_args.push("-c:a".to_string());
                    modified_args.push("aac".to_string());
                    modified_args.push("-c:s".to_string());
                    modified_args.push("copy".to_string());
                    i += 2; // skip original "-c" "copy"
                } else if preset.ffmpeg_args[i] == "-map"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("0")
                {
                    modified_args.push("-map".to_string());
                    modified_args.push("0:v".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:a".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:s?".to_string());
                    i += 2;
                } else {
                    modified_args.push(preset.ffmpeg_args[i].clone());
                    i += 1;
                }
            }
        }
        Fallback::VideoOnly => {
            // Map only video + optional subtitles, drop all audio
            // Remove "-c copy" since there's nothing left to copy
            let mut i = 0;
            while i < preset.ffmpeg_args.len() {
                if preset.ffmpeg_args[i] == "-map"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("0")
                {
                    modified_args.push("-map".to_string());
                    modified_args.push("0:v".to_string());
                    modified_args.push("-map".to_string());
                    modified_args.push("0:s?".to_string());
                    i += 2;
                } else if preset.ffmpeg_args[i] == "-c"
                    && preset.ffmpeg_args.get(i + 1).map(|s| s.as_str()) == Some("copy")
                {
                    modified_args.push("-c:s".to_string());
                    modified_args.push("copy".to_string());
                    i += 2;
                } else {
                    modified_args.push(preset.ffmpeg_args[i].clone());
                    i += 1;
                }
            }
        }
    }

    let _ = event_tx.send(EncodeEvent::StatusChange {
        job_id,
        status: EncodeStatus::Encoding,
    });

    run_ffmpeg_inner(
        job_id,
        input,
        output,
        &modified_args,
        duration_secs,
        control_rx,
        event_tx,
    )
    .await
}

async fn run_ffmpeg_inner(
    job_id: u64,
    input: &Path,
    output: &Path,
    ffmpeg_args: &[String],
    duration_secs: Option<f64>,
    control_rx: &mut mpsc::UnboundedReceiver<EncodeControl>,
    event_tx: &mpsc::UnboundedSender<EncodeEvent>,
) -> FfmpegResult {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-progress")
        .arg("pipe:1")
        .arg("-nostats");

    for arg in ffmpeg_args {
        cmd.arg(arg);
    }

    cmd.arg(output);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return FfmpegResult::Failed(format!("Failed to spawn ffmpeg: {}", e)),
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut stdout_reader = BufReader::new(stdout).lines();

    // Collect stderr in background for error reporting
    let stderr_handle = tokio::spawn(async move {
        let mut error_lines: Vec<String> = Vec::new();
        let mut tail_lines: Vec<String> = Vec::new();
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        // Capture lines containing actual errors
                        let lower = trimmed.to_lowercase();
                        if lower.contains("error")
                            || lower.contains("invalid")
                            || lower.contains("no such")
                            || lower.contains("unknown")
                            || lower.contains("unsupported")
                            || lower.contains("cannot")
                            || lower.contains("conversion failed")
                        {
                            error_lines.push(trimmed.clone());
                        }
                        // Rolling tail: keep last 10 lines
                        tail_lines.push(trimmed);
                        if tail_lines.len() > 10 {
                            tail_lines.remove(0);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        (error_lines, tail_lines)
    });

    let mut progress_builder = ProgressBuilder::default();
    let pid = child.id();

    loop {
        tokio::select! {
            line = stdout_reader.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(progress) = progress_builder.feed(&line, duration_secs) {
                            let _ = event_tx.send(EncodeEvent::Progress {
                                job_id,
                                progress,
                            });
                        }
                    }
                    Ok(None) => break, // stdout closed
                    Err(_) => break,
                }
            }
            ctrl = control_rx.recv() => {
                match ctrl {
                    Some(EncodeControl::Pause) => {
                        if let Some(pid) = pid {
                            send_signal(pid, Signal::Stop);
                            let _ = event_tx.send(EncodeEvent::StatusChange {
                                job_id,
                                status: EncodeStatus::Paused,
                            });
                        }
                    }
                    Some(EncodeControl::Resume) => {
                        if let Some(pid) = pid {
                            send_signal(pid, Signal::Continue);
                            let _ = event_tx.send(EncodeEvent::StatusChange {
                                job_id,
                                status: EncodeStatus::Resumed,
                            });
                        }
                    }
                    Some(EncodeControl::Cancel) => {
                        let _ = child.kill().await;
                        return FfmpegResult::Cancelled;
                    }
                    None => {
                        // Control channel closed, kill ffmpeg
                        let _ = child.kill().await;
                        return FfmpegResult::Cancelled;
                    }
                }
            }
        }
    }

    // Wait for ffmpeg to exit
    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => return FfmpegResult::Failed(format!("Failed to wait for ffmpeg: {}", e)),
    };

    if status.success() {
        FfmpegResult::Success
    } else {
        let (error_lines, tail_lines) = stderr_handle.await.unwrap_or_default();
        // Prefer specific error lines; fall back to tail of stderr
        let detail = if !error_lines.is_empty() {
            error_lines.join("\n")
        } else {
            tail_lines.join("\n")
        };
        FfmpegResult::Failed(format!(
            "ffmpeg exited with code {}:\n{}",
            status.code().unwrap_or(-1),
            detail
        ))
    }
}

// ── Signal handling ────────────────────────────────────────────

enum Signal {
    Stop,
    Continue,
}

#[cfg(unix)]
fn send_signal(pid: u32, sig: Signal) {
    let signum = match sig {
        Signal::Stop => libc::SIGSTOP,
        Signal::Continue => libc::SIGCONT,
    };
    unsafe {
        libc::kill(pid as i32, signum);
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: u32, _sig: Signal) {
    // Pause/resume not supported on non-Unix
}

// ── Progress parsing ───────────────────────────────────────────

/// Accumulates ffmpeg `-progress pipe:1` key=value pairs and emits
/// a FfmpegProgress on each `progress=continue` line.
#[derive(Default)]
struct ProgressBuilder {
    frame: u64,
    fps: f64,
    bitrate_kbps: f64,
    total_size: u64,
    out_time_us: u64,
    speed: f64,
}

impl ProgressBuilder {
    fn feed(&mut self, line: &str, duration_secs: Option<f64>) -> Option<FfmpegProgress> {
        let (key, value) = line.split_once('=')?;
        let key = key.trim();
        let value = value.trim();

        match key {
            "frame" => self.frame = value.parse().unwrap_or(0),
            "fps" => self.fps = value.parse().unwrap_or(0.0),
            "bitrate" => {
                // "1234.5kbits/s" or "N/A"
                self.bitrate_kbps = value.trim_end_matches("kbits/s").parse().unwrap_or(0.0);
            }
            "total_size" => self.total_size = value.parse().unwrap_or(0),
            "out_time_us" | "out_time_ms" => {
                let v = value.parse().unwrap_or(0);
                if v > 0 {
                    self.out_time_us = v;
                }
            }
            "speed" => {
                // "2.34x" or "N/A"
                self.speed = value.trim_end_matches('x').parse().unwrap_or(0.0);
            }
            "progress" if value == "continue" || value == "end" => {
                let out_time_secs = self.out_time_us as f64 / 1_000_000.0;
                let percent = match duration_secs {
                    Some(d) if d > 0.0 => (out_time_secs / d * 100.0).min(100.0),
                    _ => 0.0,
                };

                return Some(FfmpegProgress {
                    frame: self.frame,
                    fps: self.fps,
                    bitrate_kbps: self.bitrate_kbps,
                    total_size: self.total_size,
                    out_time_secs,
                    speed: self.speed,
                    percent,
                });
            }
            _ => {}
        }

        None
    }
}

// ── Validation ─────────────────────────────────────────────────

async fn validate_encoded(
    encoded_path: &Path,
    _original_path: &Path,
    original_duration: Option<f64>,
    expected_codec: &str,
) -> Result<(), String> {
    // Check 1: Verify codec matches expected target
    let codec = get_video_codec(encoded_path).await?;
    if codec != expected_codec {
        return Err(format!("Expected {} codec, got: {}", expected_codec, codec));
    }

    // Check 2: Duration within 100s of original
    if let Some(orig_dur) = original_duration {
        let enc_dur = get_duration(encoded_path).await?;
        if (enc_dur - orig_dur).abs() > 100.0 {
            return Err(format!(
                "Duration mismatch: original {:.1}s, encoded {:.1}s",
                orig_dur, enc_dur
            ));
        }
    }

    // Check 3: ffmpeg integrity check — no errors
    let integrity = check_integrity(encoded_path).await?;
    if !integrity {
        return Err("Integrity check failed (ffmpeg reported errors)".to_string());
    }

    Ok(())
}

async fn get_video_codec(path: &Path) -> Result<String, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|e| format!("ffprobe failed: {}", e))?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase())
}

async fn get_duration(path: &Path) -> Result<f64, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|e| format!("ffprobe failed: {}", e))?;

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|e| format!("Failed to parse duration: {}", e))
}

async fn check_integrity(path: &Path) -> Result<bool, String> {
    // Only check the video stream — copied audio may have pre-existing
    // non-fatal warnings (e.g. AAC env_facs_q) that aren't encode failures
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v", "-f", "null", "-"])
        .output()
        .await
        .map_err(|e| format!("ffmpeg integrity check failed: {}", e))?;

    Ok(output.stderr.is_empty())
}

// ── Filename cleaning ──────────────────────────────────────────

/// Port of PHP `removeEncodingHintsFromBasename()`.
/// Strips h264/xvid hints and collapses empty brackets/underscores.
pub fn remove_encoding_hints(stem: &str) -> String {
    let mut result = stem.to_string();

    let hints = [
        "h264", "H264", "h.264", "H.264", "h_264", "H_264", "h 264", "H 264", "xvid", "Xvid",
        "XVid", "XVId", "XVID", "XviD",
    ];

    // Apply 3 times like the PHP version (handles nested artifacts)
    for _ in 0..3 {
        for hint in &hints {
            result = result.replace(hint, "");
        }
        // Collapse artifacts
        result = result.replace("()", "");
        result = result.replace("[]", "");
        result = result.replace("(_)", "");
        while result.contains("__") {
            result = result.replace("__", "_");
        }
        while result.contains("..") {
            result = result.replace("..", ".");
        }
        while result.contains("  ") {
            result = result.replace("  ", " ");
        }
    }

    // Trim trailing dots and underscores
    result = result
        .trim_matches(|c| c == '.' || c == '_' || c == ' ')
        .to_string();

    if result.is_empty() {
        stem.to_string() // safety: don't produce empty filename
    } else {
        result
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Simple string hash for temp filenames (not cryptographic).
fn simple_hash(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut hash: u64 = 0;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
    }
    format!("{:016x}", hash)
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

async fn cleanup_temp(source: &Path, output: &Path) {
    let _ = tokio::fs::remove_file(source).await;
    let _ = tokio::fs::remove_file(output).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_encoding_hints() {
        assert_eq!(remove_encoding_hints("Movie.h264.720p"), "Movie.720p");
        assert_eq!(remove_encoding_hints("Movie.H264.720p"), "Movie.720p");
        assert_eq!(remove_encoding_hints("Movie.h.264.720p"), "Movie.720p");
        assert_eq!(remove_encoding_hints("Show_xvid_720p"), "Show_720p");
        assert_eq!(remove_encoding_hints("Movie"), "Movie");
    }

    #[test]
    fn test_remove_encoding_hints_empty_brackets() {
        assert_eq!(remove_encoding_hints("Movie.(h264).720p"), "Movie.720p");
        assert_eq!(remove_encoding_hints("Movie.[h264].720p"), "Movie.720p");
    }

    #[test]
    fn test_remove_encoding_hints_preserves_nonempty() {
        assert_eq!(remove_encoding_hints("somefile"), "somefile");
    }

    #[test]
    fn test_progress_parsing() {
        let mut builder = ProgressBuilder::default();
        assert!(builder.feed("frame=100", Some(60.0)).is_none());
        assert!(builder.feed("fps=30.0", Some(60.0)).is_none());
        assert!(builder.feed("bitrate=1500.0kbits/s", Some(60.0)).is_none());
        assert!(builder.feed("total_size=5000000", Some(60.0)).is_none());
        assert!(builder.feed("out_time_us=30000000", Some(60.0)).is_none());
        assert!(builder.feed("speed=2.0x", Some(60.0)).is_none());

        let progress = builder.feed("progress=continue", Some(60.0)).unwrap();
        assert_eq!(progress.frame, 100);
        assert!((progress.fps - 30.0).abs() < 0.01);
        assert!((progress.percent - 50.0).abs() < 0.01);
        assert!((progress.speed - 2.0).abs() < 0.01);
    }
}
