use std::path::PathBuf;
use tokio::sync::mpsc;
use walkdir::WalkDir;

use crate::model::MediaFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JunkType {
    DsStore,
    ResourceFork,
    ThumbsDb,
    DesktopIni,
}

impl JunkType {
    pub fn label(self) -> &'static str {
        match self {
            Self::DsStore => ".DS_Store",
            Self::ResourceFork => "._ resource forks",
            Self::ThumbsDb => "Thumbs.db",
            Self::DesktopIni => "desktop.ini",
        }
    }
}

#[derive(Debug, Clone)]
pub struct JunkFile {
    pub path: PathBuf,
    pub size: u64,
    pub junk_type: JunkType,
}

fn classify_junk(name: &str) -> Option<JunkType> {
    if name == ".DS_Store" {
        Some(JunkType::DsStore)
    } else if name.starts_with("._") {
        Some(JunkType::ResourceFork)
    } else if name.eq_ignore_ascii_case("Thumbs.db") {
        Some(JunkType::ThumbsDb)
    } else if name.eq_ignore_ascii_case("desktop.ini") {
        Some(JunkType::DesktopIni)
    } else {
        None
    }
}

pub enum ScanItem {
    Media(MediaFile),
    Junk(JunkFile),
}

/// Walks `root` and streams items via `tx` as they are discovered.
/// Intended to run on a blocking thread (tokio::task::spawn_blocking).
/// Dropping `tx` when done signals scan completion to the receiver.
/// Spawns the scanner on a blocking thread and returns a receiver that
/// emits each discovered item. The channel closes when scanning finishes.
pub fn start_background_scan(root: PathBuf, extensions: Vec<String>) -> mpsc::UnboundedReceiver<ScanItem> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn_blocking(move || scan_streaming(root, tx, &extensions));
    rx
}

pub fn scan_streaming(root: PathBuf, tx: mpsc::UnboundedSender<ScanItem>, extensions: &[String]) {
    for entry in WalkDir::new(&root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        if let Some(junk_type) = classify_junk(name) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if tx
                .send(ScanItem::Junk(JunkFile {
                    path: path.to_path_buf(),
                    size,
                    junk_type,
                }))
                .is_err()
            {
                return;
            }
            continue;
        }

        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_lowercase(),
            None => continue,
        };

        if !extensions.iter().any(|e| e == &ext) {
            continue;
        }

        let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if tx
            .send(ScanItem::Media(MediaFile::new(path.to_path_buf(), file_size)))
            .is_err()
        {
            return;
        }
    }
}

