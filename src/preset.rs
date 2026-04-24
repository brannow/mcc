use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const DEFAULT_PROBE_CONCURRENCY: usize = 8;

const DEFAULT_MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "webm", "mov", "wmv", "flv", "ts", "mpg", "mpeg",
];

const DEFAULT_SKIP_CODECS: &[&str] = &["hevc", "av1"];

const DEFAULT_TARGET_CODEC: &str = "hevc";

/// Top-level encoding.yaml structure.
#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    temp_dir: Option<PathBuf>,
    #[serde(default)]
    probe_concurrency: Option<usize>,
    #[serde(default)]
    media_extensions: Option<Vec<String>>,
    #[serde(default)]
    skip_codecs: Option<Vec<String>>,
    presets: HashMap<String, PresetEntry>,
}

/// A single preset entry as written in the YAML file (name derived from map key).
#[derive(Debug, Deserialize)]
struct PresetEntry {
    target_format: String,
    #[serde(default)]
    target_codec: Option<String>,
    #[serde(default)]
    temp_dir: Option<PathBuf>,
    ffmpeg_args: Vec<String>,
}

/// Resolved preset ready for use by the encoder.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EncodingPreset {
    pub name: String,
    pub target_format: String,
    pub target_codec: String,
    pub temp_dir: PathBuf,
    pub ffmpeg_args: Vec<String>,
}

impl EncodingPreset {
    /// Human-readable summary of key encoding parameters (for UI display).
    #[allow(dead_code)]
    pub fn summary(&self) -> String {
        let args = &self.ffmpeg_args;
        let mut parts = Vec::new();

        if let Some(i) = args.iter().position(|a| a == "-crf")
            && let Some(val) = args.get(i + 1)
        {
            parts.push(format!("CRF {}", val));
        }

        if let Some(i) = args.iter().position(|a| a == "-preset")
            && let Some(val) = args.get(i + 1)
        {
            parts.push(val.clone());
        }

        if let Some(i) = args.iter().position(|a| a == "-c:v")
            && let Some(val) = args.get(i + 1)
        {
            parts.push(val.clone());
        }

        if parts.is_empty() {
            "custom".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Resolved application configuration: global settings + presets.
#[derive(Debug)]
pub struct AppConfig {
    pub probe_concurrency: usize,
    pub media_extensions: Vec<String>,
    pub skip_codecs: Vec<String>,
    pub presets: Vec<EncodingPreset>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            probe_concurrency: DEFAULT_PROBE_CONCURRENCY,
            media_extensions: DEFAULT_MEDIA_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            skip_codecs: DEFAULT_SKIP_CODECS.iter().map(|s| s.to_string()).collect(),
            presets: Vec::new(),
        }
    }
}

/// Load encoding config from an explicit file path.
/// Returns an error if the file can't be read or parsed.
pub fn load_presets_from(path: &Path) -> Result<AppConfig, String> {
    load_config_file(path)
}

/// Auto-discover `encoding.yaml` in the given directory.
/// Returns defaults if the file doesn't exist.
pub fn load_presets(dir: &Path) -> AppConfig {
    let path = dir.join("encoding.yaml");
    match load_config_file(&path) {
        Ok(config) => config,
        Err(e) => {
            if path.exists() {
                eprintln!("Warning: failed to load {:?}: {}", path, e);
            }
            AppConfig::default()
        }
    }
}

fn load_config_file(path: &Path) -> Result<AppConfig, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
    let config: RawConfig =
        serde_yaml::from_str(&content).map_err(|e| format!("parse error: {}", e))?;

    let global_temp = config.temp_dir.unwrap_or_else(std::env::temp_dir);

    let mut presets: Vec<EncodingPreset> = config
        .presets
        .into_iter()
        .filter_map(|(key, entry)| {
            if entry.ffmpeg_args.is_empty() {
                eprintln!("Warning: preset '{}' has empty ffmpeg_args, skipping", key);
                return None;
            }
            Some(EncodingPreset {
                name: key,
                target_format: entry.target_format,
                target_codec: entry
                    .target_codec
                    .unwrap_or_else(|| DEFAULT_TARGET_CODEC.to_string()),
                temp_dir: entry.temp_dir.unwrap_or_else(|| global_temp.clone()),
                ffmpeg_args: entry.ffmpeg_args,
            })
        })
        .collect();

    presets.sort_by_key(|p| p.name.to_lowercase());

    Ok(AppConfig {
        probe_concurrency: config
            .probe_concurrency
            .unwrap_or(DEFAULT_PROBE_CONCURRENCY),
        media_extensions: config.media_extensions.unwrap_or_else(|| {
            DEFAULT_MEDIA_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect()
        }),
        skip_codecs: config
            .skip_codecs
            .unwrap_or_else(|| DEFAULT_SKIP_CODECS.iter().map(|s| s.to_string()).collect()),
        presets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary_extraction() {
        let preset = EncodingPreset {
            name: "test".to_string(),
            target_format: "mkv".to_string(),
            target_codec: "hevc".to_string(),
            temp_dir: PathBuf::from("/tmp"),
            ffmpeg_args: vec![
                "-crf".into(),
                "20".into(),
                "-preset".into(),
                "medium".into(),
                "-c:v".into(),
                "libx265".into(),
            ],
        };
        assert_eq!(preset.summary(), "CRF 20, medium, libx265");
    }

    #[test]
    fn test_load_encoding_yaml() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = load_presets(&project_root);
        assert!(
            !config.presets.is_empty(),
            "Should load at least one preset"
        );

        for preset in &config.presets {
            assert!(!preset.name.is_empty(), "Preset name must not be empty");
            assert!(
                !preset.target_format.is_empty(),
                "target_format must not be empty"
            );
            assert!(
                !preset.target_codec.is_empty(),
                "target_codec must not be empty"
            );
            assert!(
                !preset.ffmpeg_args.is_empty(),
                "ffmpeg_args must not be empty"
            );
            assert!(
                !preset.temp_dir.as_os_str().is_empty(),
                "temp_dir must be resolved"
            );
        }

        // Presets should be sorted by name
        let names: Vec<&str> = config.presets.iter().map(|p| p.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|a| a.to_lowercase());
        assert_eq!(names, sorted, "Presets should be sorted alphabetically");

        // Global settings should have sensible defaults
        assert!(config.probe_concurrency > 0);
        assert!(!config.media_extensions.is_empty());
        assert!(!config.skip_codecs.is_empty());
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let config = load_presets(Path::new("/nonexistent/path"));
        assert!(config.presets.is_empty());
        assert_eq!(config.probe_concurrency, 8);
        assert!(!config.media_extensions.is_empty());
        assert!(!config.skip_codecs.is_empty());
    }
}
