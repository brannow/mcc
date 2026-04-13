# MCC - Media Control Center

Rust TUI for batch media file discovery, inspection (ffprobe), and HEVC encoding (ffmpeg).
Replaces older PHP-based encoder in parent `docker-ffmpeg-batch-hevc-encoder` project.

## Tech Stack

- **TUI**: `ratatui` + `crossterm`
- **Async**: `tokio` (multi-threaded) - channels for inter-task communication
- **CLI**: `clap` (derive)
- **Config**: `serde_yaml` (`encoding.yaml`)
- **Rust edition**: 2024

## Architecture

```
src/
├── main.rs       # CLI args (clap), terminal setup, event loop
├── app.rs        # Central App struct: all state, event handling, queue management
├── model.rs      # Data types: MediaFile, VideoStream, AudioStream, EncodeJob, FpsStats
├── scanner.rs    # Background dir walker (walkdir), emits MediaFile/JunkFile via channel
├── prober.rs     # Concurrent ffprobe workers (semaphore=8), parses JSON output
├── encoder.rs    # Single ffmpeg worker: copy→encode→validate→replace pipeline
├── preset.rs     # Loads encoding.yaml, resolves EncodingPreset structs
└── ui/
    ├── mod.rs        # Root draw(), layout orchestration
    ├── list.rs       # File/folder table (codec, size, bitrate, resolution, duration)
    ├── detail.rs     # Right split: file metadata, streams info
    ├── encoding.rs   # Encode queue table + real-time telemetry (fps graph, ETA, progress)
    ├── popup.rs      # Overlays: legend, cleanup, preset picker, quit confirm
    ├── status_bar.rs # Bottom bar: scan progress, codec counts, context keybindings
    └── theme.rs      # Color palette, styling
```

## Data Flow

1. **Scan**: `scanner` walks root dir in blocking task, sends `ScanItem` via channel
2. **Probe**: `prober` receives paths, runs `ffprobe -print_format json`, updates `MediaFile`
3. **Display**: `App::rebuild_rows()` applies filter/sort/grouping, renders via ratatui
4. **Encode**: User enqueues files → `encoder` copies to temp dir → runs ffmpeg → validates (codec + duration) → replaces original

## Event Loop (non-blocking)

```
poll_scan_results() → poll_probe_results() → poll_encode_events() → ui::draw() → handle_event()
```

All worker communication via `tokio::sync::mpsc` unbounded channels. No shared mutable state.

## Encoding Pipeline Details

- Copies source to `temp_dir/h264_<hash>.<ext>` before encoding
- Post-encode validation: checks codec is HEVC, duration within 100s tolerance
- Subtitle fallback: retries with `-c:s srt` on subtitle codec errors
- Filename cleaning: strips h264/xvid markers, collapses empty brackets
- Pause/resume via Unix SIGSTOP/SIGCONT on ffmpeg process

## Config: `encoding.yaml`

```yaml
temp_dir: /tmp/encoding
presets:
  anime:
    target_format: mkv
    ffmpeg_args: [-tune, animation, -crf, "24", -preset, medium, -c:v, libx265, ...]
```

## UI Views

- **List View** (default): file table + optional detail split (Space). Sortable columns, codec filter (f), group-by-folder mode
- **Encoding View** (Right arrow): queue table + telemetry pane (fps braille graph, progress bar, ETA)
- **Popups**: legend (h), cleanup junk (d), preset picker (p/1-9), quit confirm

## Key Keybindings

| Key | Action |
|-----|--------|
| Enter/e | Enqueue selected/all |
| p / 1-9 | Preset picker / quick select |
| f | Cycle codec filter |
| s/S | Sort column / toggle direction |
| Space | Toggle detail pane / pause encode |
| Left/Right | Switch List/Encoding view |
| d | Cleanup junk files dialog |
| c/C | Cancel current/all encodes |

## Build & Run

```bash
cargo build --release
./target/release/mcc /path/to/media [--config encoding.yaml]
```

Requires `ffmpeg` and `ffprobe` in PATH.

## Conventions

- No persistence / database - fully stateless, in-memory only
- Workers are fire-and-forget tokio tasks communicating via channels
- UI rendering is side-effect-free (reads App state, returns widgets)
- Responsive columns: hide lower-priority columns on narrow terminals
