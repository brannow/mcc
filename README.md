# MCC - Media Control Center

Rust TUI for batch media file discovery, inspection (`ffprobe`), and HEVC encoding (`ffmpeg`).

![Rust](https://img.shields.io/badge/Rust-2024_edition-orange)

## Requirements

| | Binary | Docker |
|---|---|---|
| `ffmpeg` | required in PATH | included |
| `ffprobe` | required in PATH | included |
| Rust toolchain | build only | not needed |

## Quick Start

### Docker

```bash
docker run -it -v /path/to/media:/media ghcr.io/OWNER/mcc
```

### Binary

```bash
mcc /path/to/media
```

## Installation

### Docker (recommended)

Pre-built multi-arch images (`linux/amd64`, `linux/arm64`):

```bash
docker pull ghcr.io/OWNER/mcc:latest
```

### From Source

```bash
cargo build --release
./target/release/mcc /path/to/media
```

### GitHub Releases

Pre-built binaries for Linux (amd64, arm64) and macOS (Intel, Apple Silicon) are attached to each [release](../../releases).

## Docker Usage

**Basic** - scan a directory:
```bash
docker run -it -v ~/Movies:/media mcc
```

**Custom temp dir** - fast SSD for encoding:
```bash
docker run -it \
  -e MCC_TEMP_DIR=/encoding \
  -v ~/Movies:/media \
  -v /fast/ssd:/encoding \
  mcc
```

**Custom config**:
```bash
docker run -it \
  -v ~/Movies:/media \
  -v ./encoding.yaml:/etc/mcc/encoding.yaml \
  mcc
```

### Docker Volumes

| Mount Point | Purpose |
|---|---|
| `/media` | Media source directory (read/write) |
| `/tmp/encoding` | Encoding temp files (default `temp_dir`) |
| `/etc/mcc/encoding.yaml` | Config override |

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `MCC_TEMP_DIR` | `/tmp/encoding` | Override `temp_dir` in bundled config |

## Configuration

`encoding.yaml` - auto-discovered from working directory, or via `--config`:

```yaml
temp_dir: /tmp/encoding
probe_concurrency: 8
media_extensions: [mkv, mp4, avi, m4v, webm, mov, wmv, flv, ts, mpg, mpeg]
skip_codecs: [hevc, av1]

presets:
  movies:
    target_format: mkv
    target_codec: hevc
    ffmpeg_args:
      - -crf
      - "20"
      - -preset
      - medium
      - -map
      - "0"
      - -c
      - copy
      - -c:v
      - libx265
```

### Global Settings

| Key | Default | Description |
|---|---|---|
| `temp_dir` | system temp | Working directory for encode pipeline |
| `probe_concurrency` | `8` | Max parallel `ffprobe` processes |
| `media_extensions` | 11 common formats | File types the scanner picks up |
| `skip_codecs` | `hevc`, `av1` | Already-encoded codecs to skip |

### Preset Settings

| Key | Required | Description |
|---|---|---|
| `target_format` | yes | Output container (`mkv`, `mp4`) |
| `target_codec` | no | Expected codec after encoding (default: `hevc`) |
| `temp_dir` | no | Override global `temp_dir` for this preset |
| `ffmpeg_args` | yes | Arguments passed to `ffmpeg` |

## Keybindings

### List View

| Key | Action |
|---|---|
| `Enter` | Toggle enqueue selected file/folder |
| `e` | Enqueue all encodeable files |
| `p` / `1-9` | Preset picker / quick select |
| `f` | Cycle codec filter (All/H.264/HEVC/AV1) |
| `s` / `S` | Cycle sort column / toggle direction |
| `Space` | Toggle detail pane |
| `g` | Toggle group-by-folder |
| `d` | Cleanup junk files |
| `r` | Rescan |
| `Left/Right` | Switch List/Encoding view |

### Encoding View

| Key | Action |
|---|---|
| `Enter` | Start encoding queue |
| `Space` | Pause/resume current encode |
| `c` / `C` | Cancel current / cancel all |
| `s` | Stop queue (finish current, drop rest) |
| `x` | Remove selected job from queue |
| `p` / `P` | Preset picker / stamp preset and advance |
| `h` | Show legend |

### Global

| Key | Action |
|---|---|
| `Ctrl+C` | Quit (confirms if encoding active) |
| `Ctrl+O` | Open selected in OS file manager |

## License

MIT
