use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::encoder::{
    self, EncodeControl, EncodeEvent, EncodeRequest, EncodeResult, EncodeStatus, EncoderHandle,
};
use crate::model::{
    EncodeJob, EncodeJobStatus, FolderRow, FpsStats, MediaFile, ProbeStatus, human_file_size,
};
use crate::preset::{AppConfig, EncodingPreset};
use crate::prober::{self, ProbeResult, apply_probe_result};
use crate::scanner::{self, JunkFile, JunkType, ScanItem};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveView {
    List,
    Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EncodingPaneFocus {
    Queue,
    Telemetry,
}

#[derive(Debug, Clone, Copy)]
pub enum ListRow {
    Media(usize),
    Folder(usize),
}

fn row_path<'a>(
    files: &'a [MediaFile],
    folders: &'a [FolderRow],
    row: &ListRow,
) -> &'a std::path::Path {
    match *row {
        ListRow::Media(i) => files[i].path.as_path(),
        ListRow::Folder(i) => folders[i].path.as_path(),
    }
}

fn row_size(files: &[MediaFile], folders: &[FolderRow], row: &ListRow) -> u64 {
    match *row {
        ListRow::Media(i) => files[i].file_size,
        ListRow::Folder(i) => folders[i].recursive_size,
    }
}

fn row_media<'a>(files: &'a [MediaFile], row: &ListRow) -> Option<&'a MediaFile> {
    match *row {
        ListRow::Media(i) => files.get(i),
        ListRow::Folder(_) => None,
    }
}

fn cmp_files_by(files: &[MediaFile], a: usize, b: usize, col: SortColumn) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match col {
        SortColumn::Name => files[a].path.cmp(&files[b].path),
        SortColumn::Size => files[a].file_size.cmp(&files[b].file_size),
        SortColumn::Codec => files[a]
            .primary_video_codec()
            .cmp(&files[b].primary_video_codec()),
        SortColumn::Resolution => {
            let ra = files[a]
                .video_streams
                .first()
                .map(|v| v.width * v.height)
                .unwrap_or(0);
            let rb = files[b]
                .video_streams
                .first()
                .map(|v| v.width * v.height)
                .unwrap_or(0);
            ra.cmp(&rb)
        }
        SortColumn::Bitrate => files[a].primary_bitrate().cmp(&files[b].primary_bitrate()),
        SortColumn::Duration => files[a]
            .duration_secs
            .partial_cmp(&files[b].duration_secs)
            .unwrap_or(Ordering::Equal),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortColumn {
    Name,
    Codec,
    Resolution,
    Bitrate,
    Size,
    Duration,
}

impl SortColumn {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Codec,
            Self::Codec => Self::Size,
            Self::Size => Self::Bitrate,
            Self::Bitrate => Self::Resolution,
            Self::Resolution => Self::Duration,
            Self::Duration => Self::Name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CodecFilter {
    All,
    H264,
    Hevc,
    Av1,
}

impl CodecFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::H264,
            Self::H264 => Self::Hevc,
            Self::Hevc => Self::Av1,
            Self::Av1 => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Av1 => "AV1",
        }
    }

    pub fn matches(self, codec: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::H264 => codec == Some("h264"),
            Self::Hevc => codec == Some("hevc"),
            Self::Av1 => codec == Some("av1"),
        }
    }
}

// Cleanup dialog types
#[derive(Debug, Clone)]
pub struct JunkGroup {
    pub junk_type: JunkType,
    pub count: usize,
    pub total_size: u64,
    pub selected: bool,
}

impl JunkGroup {
    pub fn label(&self) -> String {
        format!(
            "{} ({} files, {})",
            self.junk_type.label(),
            self.count,
            human_file_size(self.total_size),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CleanupFocus {
    List,
    DeleteButton,
    CancelButton,
}

pub struct CleanupDialog {
    pub groups: Vec<JunkGroup>,
    pub cursor: usize,
    pub focus: CleanupFocus,
    pub status_message: Option<String>,
}

// Preset picker popup
pub struct PresetPicker {
    pub cursor: usize,
}

impl PresetPicker {
    fn new(current_preset: usize) -> Self {
        Self {
            cursor: current_preset,
        }
    }
}

impl CleanupDialog {
    fn new(junk_files: &[JunkFile]) -> Self {
        let mut map: HashMap<JunkType, (usize, u64)> = HashMap::new();
        for jf in junk_files {
            let entry = map.entry(jf.junk_type).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += jf.size;
        }

        let mut groups: Vec<JunkGroup> = map
            .into_iter()
            .map(|(junk_type, (count, total_size))| JunkGroup {
                junk_type,
                count,
                total_size,
                selected: true, // all selected by default
            })
            .collect();

        // Stable order
        groups.sort_by_key(|g| match g.junk_type {
            JunkType::DsStore => 0,
            JunkType::ResourceFork => 1,
            JunkType::ThumbsDb => 2,
            JunkType::DesktopIni => 3,
        });

        Self {
            groups,
            cursor: 0,
            focus: CleanupFocus::List,
            status_message: None,
        }
    }

    pub fn selected_count(&self) -> usize {
        self.groups
            .iter()
            .filter(|g| g.selected)
            .map(|g| g.count)
            .sum()
    }

    pub fn selected_size(&self) -> u64 {
        self.groups
            .iter()
            .filter(|g| g.selected)
            .map(|g| g.total_size)
            .sum()
    }

    pub fn selected_types(&self) -> Vec<JunkType> {
        self.groups
            .iter()
            .filter(|g| g.selected)
            .map(|g| g.junk_type)
            .collect()
    }

    fn handle_key(&mut self, code: KeyCode) -> CleanupAction {
        match self.focus {
            CleanupFocus::List => match code {
                KeyCode::Up if self.cursor > 0 => {
                    self.cursor -= 1;
                }
                KeyCode::Down => {
                    if self.cursor + 1 < self.groups.len() {
                        self.cursor += 1;
                    } else {
                        self.focus = CleanupFocus::DeleteButton;
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(group) = self.groups.get_mut(self.cursor) {
                        group.selected = !group.selected;
                    }
                }
                KeyCode::Char('a') => {
                    let all_selected = self.groups.iter().all(|g| g.selected);
                    for g in &mut self.groups {
                        g.selected = !all_selected;
                    }
                }
                KeyCode::Enter => {
                    self.focus = CleanupFocus::DeleteButton;
                }
                KeyCode::Esc => return CleanupAction::Close,
                _ => {}
            },
            CleanupFocus::DeleteButton => match code {
                KeyCode::Left => {
                    // already on delete (leftmost)
                }
                KeyCode::Right => {
                    self.focus = CleanupFocus::CancelButton;
                }
                KeyCode::Up => {
                    self.focus = CleanupFocus::List;
                    self.cursor = self.groups.len().saturating_sub(1);
                }
                KeyCode::Enter => {
                    if self.selected_count() > 0 {
                        return CleanupAction::Delete;
                    }
                    return CleanupAction::Close;
                }
                KeyCode::Esc => return CleanupAction::Close,
                _ => {}
            },
            CleanupFocus::CancelButton => match code {
                KeyCode::Left => {
                    self.focus = CleanupFocus::DeleteButton;
                }
                KeyCode::Right => {
                    // already on cancel (rightmost)
                }
                KeyCode::Up => {
                    self.focus = CleanupFocus::List;
                    self.cursor = self.groups.len().saturating_sub(1);
                }
                KeyCode::Enter => return CleanupAction::Close,
                KeyCode::Esc => return CleanupAction::Close,
                _ => {}
            },
        }
        CleanupAction::None
    }
}

#[derive(Debug, PartialEq)]
enum CleanupAction {
    None,
    Close,
    Delete,
}

pub struct App {
    pub root_path: PathBuf,
    pub files: Vec<MediaFile>,
    pub folders: Vec<FolderRow>,
    pub grouped: bool,
    pub filtered_rows: Vec<ListRow>,
    pub selected: usize,
    pub detail_open: bool,
    pub detail_focused: bool,
    pub detail_scroll: u16,
    pub detail_view_height: u16,
    pub sort_column: SortColumn,
    pub sort_ascending: bool,
    pub codec_filter: CodecFilter,
    pub should_quit: bool,
    pub total_files: usize,
    pub probed_count: usize,
    pub junk_files: Vec<JunkFile>,
    pub cleanup_dialog: Option<CleanupDialog>,
    pub preset_picker: Option<PresetPicker>,
    pub list_state: TableState,
    pub scan_in_progress: bool,
    pub presets: Vec<EncodingPreset>,
    pub probe_concurrency: usize,
    pub media_extensions: Vec<String>,
    pub skip_codecs: Vec<String>,

    // Encoding queue state
    pub active_view: ActiveView,
    pub encode_queue: Vec<EncodeJob>,
    pub encode_queue_selected: usize,
    pub encode_queue_state: TableState,
    pub encoding_pane_focus: EncodingPaneFocus,
    pub show_legend: bool,
    pub show_quit_confirm: bool,
    pub selected_preset: Option<usize>,
    next_job_id: u64,

    encoder: EncoderHandle,

    scan_rx: Option<mpsc::UnboundedReceiver<ScanItem>>,
    probe_result_rx: mpsc::UnboundedReceiver<ProbeResult>,
    probe_path_tx: mpsc::UnboundedSender<PathBuf>,
}

impl App {
    pub fn new(root_path: PathBuf, config: AppConfig) -> Self {
        let (probe_path_tx, probe_result_rx) =
            prober::start_background_prober(config.probe_concurrency);
        let scan_rx =
            scanner::start_background_scan(root_path.clone(), config.media_extensions.clone());
        let encoder = encoder::start_encoder();

        Self {
            root_path,
            files: Vec::new(),
            folders: Vec::new(),
            grouped: false,
            filtered_rows: Vec::new(),
            selected: 0,
            detail_open: false,
            detail_focused: false,
            detail_scroll: 0,
            detail_view_height: 20,
            sort_column: SortColumn::Name,
            sort_ascending: true,
            codec_filter: CodecFilter::All,
            should_quit: false,
            total_files: 0,
            probed_count: 0,
            junk_files: Vec::new(),
            cleanup_dialog: None,
            preset_picker: None,
            list_state: TableState::default(),
            scan_in_progress: true,
            probe_concurrency: config.probe_concurrency,
            media_extensions: config.media_extensions,
            skip_codecs: config.skip_codecs,
            presets: config.presets,
            active_view: ActiveView::List,
            encode_queue: Vec::new(),
            encode_queue_selected: 0,
            encode_queue_state: TableState::default(),
            encoding_pane_focus: EncodingPaneFocus::Queue,
            show_legend: false,
            show_quit_confirm: false,
            selected_preset: None,
            next_job_id: 0,
            encoder,
            scan_rx: Some(scan_rx),
            probe_result_rx,
            probe_path_tx,
        }
    }

    /// Reset file/scan state and kick off a fresh streaming scan + prober.
    /// Preserves user preferences (sort, filter, focus).
    fn start_scan(&mut self) {
        self.files = Vec::new();
        self.folders = Vec::new();
        self.junk_files = Vec::new();
        self.filtered_rows = Vec::new();
        self.total_files = 0;
        self.probed_count = 0;
        self.selected = 0;
        self.detail_open = false;

        let (probe_path_tx, probe_result_rx) =
            prober::start_background_prober(self.probe_concurrency);
        self.probe_path_tx = probe_path_tx;
        self.probe_result_rx = probe_result_rx;
        self.scan_rx = Some(scanner::start_background_scan(
            self.root_path.clone(),
            self.media_extensions.clone(),
        ));
        self.scan_in_progress = true;
    }

    pub fn poll_scan_results(&mut self) {
        let rx = match self.scan_rx.as_mut() {
            Some(rx) => rx,
            None => return,
        };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(ScanItem::Media(file)) => {
                    let _ = self.probe_path_tx.send(file.path.clone());
                    self.files.push(file);
                    self.total_files += 1;
                    changed = true;
                }
                Ok(ScanItem::Junk(junk)) => {
                    self.junk_files.push(junk);
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.scan_in_progress = false;
                    self.scan_rx = None;
                    self.files.sort_by(|a, b| a.path.cmp(&b.path));
                    self.rebuild_folders();
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.rebuild_rows();
        }
    }

    pub fn poll_probe_results(&mut self) {
        let mut changed = false;
        while let Ok(result) = self.probe_result_rx.try_recv() {
            if let Some(file) = self.files.iter_mut().find(|f| f.path == result.path) {
                match result.data {
                    Ok(data) => {
                        apply_probe_result(file, data);
                        file.probe_status = ProbeStatus::Done;
                    }
                    Err(e) => {
                        file.probe_status = ProbeStatus::Error(e);
                    }
                }
                self.probed_count += 1;
                changed = true;
            }
        }
        if changed {
            self.rebuild_rows();
        }
    }

    fn rebuild_folders(&mut self) {
        let mut map: HashMap<PathBuf, (u64, usize)> = HashMap::new();
        for file in &self.files {
            let mut cursor = file.path.parent();
            while let Some(dir) = cursor {
                if dir == self.root_path {
                    break;
                }
                if !dir.starts_with(&self.root_path) {
                    break;
                }
                let entry = map.entry(dir.to_path_buf()).or_insert((0, 0));
                entry.0 += file.file_size;
                entry.1 += 1;
                cursor = dir.parent();
            }
        }
        let mut folders: Vec<FolderRow> = map
            .into_iter()
            .map(|(path, (recursive_size, file_count))| FolderRow {
                path,
                recursive_size,
                file_count,
            })
            .collect();
        folders.sort_by(|a, b| a.path.cmp(&b.path));
        self.folders = folders;
    }

    pub fn handle_event(&mut self) -> std::io::Result<()> {
        if event::poll(std::time::Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                return Ok(());
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if self.is_encoding_active() || self.queued_count() > 0 {
                    self.show_quit_confirm = true;
                } else {
                    self.should_quit = true;
                }
                return Ok(());
            }

            // Quit confirm dialog intercepts all input
            if self.show_quit_confirm {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.should_quit = true;
                    }
                    _ => {
                        self.show_quit_confirm = false;
                    }
                }
                return Ok(());
            }

            // Dialog intercepts all input when open
            if let Some(dialog) = &mut self.cleanup_dialog {
                let action = dialog.handle_key(key.code);
                match action {
                    CleanupAction::Close => {
                        self.cleanup_dialog = None;
                    }
                    CleanupAction::Delete => {
                        self.execute_cleanup();
                    }
                    CleanupAction::None => {}
                }
                return Ok(());
            }

            // Preset picker intercepts all input when open
            if self.preset_picker.is_some() {
                self.handle_preset_picker_key(key.code);
                return Ok(());
            }

            // Global keys (available in all views)
            match key.code {
                KeyCode::Left => {
                    self.active_view = ActiveView::List;
                    return Ok(());
                }
                KeyCode::Right => {
                    self.active_view = ActiveView::Encoding;
                    return Ok(());
                }
                _ => {}
            }

            // View-specific input
            match self.active_view {
                ActiveView::List => self.handle_list_event(key),
                ActiveView::Encoding => self.handle_encoding_event(key),
            }
        }
        Ok(())
    }

    fn handle_list_event(&mut self, key: crossterm::event::KeyEvent) {
        // List-view-specific keys
        match key.code {
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_selected_in_os();
                return;
            }
            KeyCode::Char(' ') => {
                self.detail_open = !self.detail_open;
                if !self.detail_open {
                    self.detail_focused = false;
                    self.detail_scroll = 0;
                }
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                if self.detail_open {
                    self.detail_focused = !self.detail_focused;
                }
                return;
            }
            KeyCode::Char('s') => {
                self.cycle_sort();
                return;
            }
            KeyCode::Char('S') => {
                self.sort_ascending = !self.sort_ascending;
                self.apply_sort();
                return;
            }
            KeyCode::Char('f') => {
                self.cycle_filter();
                return;
            }
            KeyCode::Char('g') => {
                self.toggle_grouped();
                return;
            }
            KeyCode::Char('d') => {
                self.open_cleanup_dialog();
                return;
            }
            KeyCode::Char('r') => {
                if !self.is_scanning() {
                    self.rescan();
                }
                return;
            }
            KeyCode::Char('p') => {
                self.open_preset_picker();
                return;
            }
            KeyCode::Char('e') => {
                self.enqueue_all_encodeable();
                return;
            }
            KeyCode::Enter => {
                match self.selected_row() {
                    Some(ListRow::Media(file_idx)) if !self.try_unqueue_file(file_idx) => {
                        self.enqueue_file(file_idx);
                    }
                    Some(ListRow::Media(_)) => {}
                    Some(ListRow::Folder(folder_idx)) => {
                        self.toggle_folder_queue(folder_idx);
                    }
                    None => {}
                }
                return;
            }
            _ => {}
        }

        // Pane-specific navigation
        if self.detail_open && self.detail_focused {
            let max_scroll = self
                .detail_line_count()
                .saturating_sub(self.detail_view_height);
            let page = self.detail_view_height.max(1);
            let is_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Up => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.detail_scroll = self.detail_scroll.saturating_add(1).min(max_scroll);
                }
                KeyCode::PageUp => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(page);
                }
                KeyCode::PageDown => {
                    self.detail_scroll = self.detail_scroll.saturating_add(page).min(max_scroll);
                }
                KeyCode::Char('v') if is_ctrl => {
                    self.detail_scroll = self.detail_scroll.saturating_add(page).min(max_scroll);
                }
                KeyCode::Char('y') if is_ctrl => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(page);
                }
                KeyCode::Home => self.detail_scroll = 0,
                KeyCode::End => self.detail_scroll = max_scroll,
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down => self.move_selection(1),
                KeyCode::PageUp => self.move_selection(-20),
                KeyCode::PageDown => self.move_selection(20),
                KeyCode::Home => self.selected = 0,
                KeyCode::End => {
                    self.selected = self.filtered_rows.len().saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    fn handle_encoding_event(&mut self, key: crossterm::event::KeyEvent) {
        // Legend popup intercepts: any key dismisses it
        if self.show_legend {
            self.show_legend = false;
            return;
        }

        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.encoding_pane_focus = match self.encoding_pane_focus {
                    EncodingPaneFocus::Queue => EncodingPaneFocus::Telemetry,
                    EncodingPaneFocus::Telemetry => EncodingPaneFocus::Queue,
                };
            }
            KeyCode::Char('x') | KeyCode::Delete
                if self.encoding_pane_focus == EncodingPaneFocus::Queue =>
            {
                self.remove_from_queue(self.encode_queue_selected);
            }
            KeyCode::Enter if !self.is_encoding_active() && self.queued_count() > 0 => {
                // Start encoding if not already running
                self.start_encoding();
            }
            KeyCode::Char(' ') => {
                // Toggle pause/resume
                if self.is_encoding_paused() {
                    self.resume_encoding();
                } else if self.is_encoding_active() {
                    self.pause_encoding();
                }
            }
            KeyCode::Char('c') if self.is_encoding_active() => {
                // Cancel current encode, next queued job picks up
                self.cancel_current_encode();
            }
            KeyCode::Char('C') => {
                // Cancel everything: kill current + drop all queued jobs
                self.cancel_all();
            }
            KeyCode::Char('s') => {
                // Stop queue: drop all queued jobs, let current encode finish
                self.stop_queue();
            }
            KeyCode::Char('p') => {
                self.open_preset_picker();
            }
            KeyCode::Char('P') => {
                self.stamp_preset_and_advance();
            }
            KeyCode::Char('h') => {
                self.show_legend = true;
            }
            _ => {}
        }

        // Queue navigation (when queue pane is focused)
        if self.encoding_pane_focus == EncodingPaneFocus::Queue && !self.encode_queue.is_empty() {
            let len = self.encode_queue.len();
            match key.code {
                KeyCode::Up => {
                    self.encode_queue_selected = self.encode_queue_selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.encode_queue_selected = (self.encode_queue_selected + 1).min(len - 1);
                }
                KeyCode::Home => self.encode_queue_selected = 0,
                KeyCode::End => self.encode_queue_selected = len - 1,
                _ => {}
            }
        }
    }

    fn open_selected_in_os(&self) {
        let path = match self.selected_row() {
            Some(ListRow::Media(i)) => self.files[i].path.clone(),
            Some(ListRow::Folder(i)) => self.folders[i].path.clone(),
            None => return,
        };
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(target_os = "linux")]
        let mut cmd = std::process::Command::new("xdg-open");
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        };
        let _ = cmd
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    fn open_cleanup_dialog(&mut self) {
        if self.junk_files.is_empty() {
            return;
        }
        self.cleanup_dialog = Some(CleanupDialog::new(&self.junk_files));
    }

    fn execute_cleanup(&mut self) {
        let dialog = match &self.cleanup_dialog {
            Some(d) => d,
            None => return,
        };

        let selected_types = dialog.selected_types();
        let mut deleted = 0usize;
        let mut failed = 0usize;

        // Collect paths to delete
        let to_delete: Vec<PathBuf> = self
            .junk_files
            .iter()
            .filter(|jf| selected_types.contains(&jf.junk_type))
            .map(|jf| jf.path.clone())
            .collect();

        for path in &to_delete {
            match std::fs::remove_file(path) {
                Ok(_) => deleted += 1,
                Err(_) => failed += 1,
            }
        }

        // Remove deleted files from junk_files list
        self.junk_files
            .retain(|jf| !selected_types.contains(&jf.junk_type));

        // Update dialog with result message or close
        if failed > 0 {
            if let Some(d) = &mut self.cleanup_dialog {
                d.status_message = Some(format!("Deleted {} files, {} failed", deleted, failed));
                // Rebuild groups with remaining files
                *d = CleanupDialog::new(&self.junk_files);
                d.status_message = Some(format!("Deleted {} files, {} failed", deleted, failed));
            }
        } else {
            self.cleanup_dialog = None;
        }
    }

    pub fn junk_count(&self) -> usize {
        self.junk_files.len()
    }

    pub fn is_scanning(&self) -> bool {
        self.scan_in_progress || self.probed_count < self.total_files
    }

    fn rescan(&mut self) {
        self.start_scan();
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered_rows.is_empty() {
            return;
        }
        let max = self.filtered_rows.len() - 1;
        let new = self.selected as i32 + delta;
        self.selected = new.clamp(0, max as i32) as usize;
        self.detail_scroll = 0;
    }

    fn cycle_sort(&mut self) {
        self.sort_column = self.sort_column.next();
        self.sort_ascending = true;
        self.apply_sort();
    }

    fn cycle_filter(&mut self) {
        self.codec_filter = self.codec_filter.next();
        self.rebuild_rows();
    }

    pub fn toggle_grouped(&mut self) {
        self.grouped = !self.grouped;
        self.selected = 0;
        self.detail_scroll = 0;
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let passing_file_indices: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                self.codec_filter.matches(f.primary_video_codec())
                    || (self.codec_filter != CodecFilter::All && !f.is_probed())
            })
            .map(|(i, _)| i)
            .collect();

        if self.grouped {
            self.build_grouped_order(passing_file_indices);
        } else {
            self.filtered_rows = passing_file_indices
                .into_iter()
                .map(ListRow::Media)
                .collect();
            self.apply_sort();
        }

        if self.selected >= self.filtered_rows.len() {
            self.selected = self.filtered_rows.len().saturating_sub(1);
        }
    }

    /// Build an already-ordered row list for group mode: folders act as real
    /// containers, files live under their immediate parent folder, and sort
    /// happens within each bucket. Root-level files are pinned at the top.
    fn build_grouped_order(&mut self, passing: Vec<usize>) {
        let col = self.sort_column;
        let asc = self.sort_ascending;

        let mut root_files: Vec<usize> = Vec::new();
        let mut buckets: HashMap<PathBuf, Vec<usize>> = HashMap::new();

        for fi in passing {
            match self.files[fi].path.parent() {
                Some(p) if p == self.root_path => root_files.push(fi),
                Some(p) => buckets.entry(p.to_path_buf()).or_default().push(fi),
                None => root_files.push(fi),
            }
        }

        let sort_file_vec = |v: &mut Vec<usize>, files: &[MediaFile]| {
            v.sort_by(|&a, &b| {
                let ord = cmp_files_by(files, a, b, col);
                if asc { ord } else { ord.reverse() }
            });
        };

        sort_file_vec(&mut root_files, &self.files);

        let folder_idx_by_path: HashMap<PathBuf, usize> = self
            .folders
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();

        let mut folder_buckets: Vec<(usize, Vec<usize>)> = Vec::new();
        for (path, mut bucket_files) in buckets {
            if let Some(&folder_idx) = folder_idx_by_path.get(&path) {
                sort_file_vec(&mut bucket_files, &self.files);
                folder_buckets.push((folder_idx, bucket_files));
            }
        }

        folder_buckets.sort_by(|a, b| {
            let ord = match col {
                SortColumn::Size => self.folders[a.0]
                    .recursive_size
                    .cmp(&self.folders[b.0].recursive_size),
                _ => self.folders[a.0].path.cmp(&self.folders[b.0].path),
            };
            if asc { ord } else { ord.reverse() }
        });

        let mut rows: Vec<ListRow> = Vec::with_capacity(
            root_files.len()
                + folder_buckets
                    .iter()
                    .map(|(_, f)| f.len() + 1)
                    .sum::<usize>(),
        );
        for fi in root_files {
            rows.push(ListRow::Media(fi));
        }
        for (folder_idx, bucket_files) in folder_buckets {
            rows.push(ListRow::Folder(folder_idx));
            for fi in bucket_files {
                rows.push(ListRow::Media(fi));
            }
        }

        self.filtered_rows = rows;
    }

    fn apply_sort(&mut self) {
        if self.grouped {
            self.rebuild_rows();
            return;
        }

        let files = &self.files;
        let folders = &self.folders;
        let col = self.sort_column;
        let asc = self.sort_ascending;

        self.filtered_rows.sort_by(|a, b| {
            let ord = match col {
                SortColumn::Name => {
                    let pa = row_path(files, folders, a);
                    let pb = row_path(files, folders, b);
                    pa.cmp(pb)
                }
                SortColumn::Size => {
                    let sa = row_size(files, folders, a);
                    let sb = row_size(files, folders, b);
                    sa.cmp(&sb)
                }
                SortColumn::Codec => {
                    let ca = row_media(files, a).and_then(|f| f.primary_video_codec());
                    let cb = row_media(files, b).and_then(|f| f.primary_video_codec());
                    ca.cmp(&cb)
                }
                SortColumn::Resolution => {
                    let ra = row_media(files, a)
                        .and_then(|f| f.video_streams.first())
                        .map(|v| v.width * v.height)
                        .unwrap_or(0);
                    let rb = row_media(files, b)
                        .and_then(|f| f.video_streams.first())
                        .map(|v| v.width * v.height)
                        .unwrap_or(0);
                    ra.cmp(&rb)
                }
                SortColumn::Bitrate => {
                    let ba = row_media(files, a).and_then(|f| f.primary_bitrate());
                    let bb = row_media(files, b).and_then(|f| f.primary_bitrate());
                    ba.cmp(&bb)
                }
                SortColumn::Duration => {
                    let da = row_media(files, a).and_then(|f| f.duration_secs);
                    let db = row_media(files, b).and_then(|f| f.duration_secs);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                }
            };
            if asc { ord } else { ord.reverse() }
        });
    }

    pub fn selected_row(&self) -> Option<ListRow> {
        self.filtered_rows.get(self.selected).copied()
    }

    pub fn selected_file(&self) -> Option<&MediaFile> {
        match self.selected_row()? {
            ListRow::Media(i) => self.files.get(i),
            ListRow::Folder(_) => None,
        }
    }

    /// Estimate how many lines the detail view has for the selected row
    fn detail_line_count(&self) -> u16 {
        match self.selected_row() {
            Some(ListRow::Folder(_)) => return 6, // FOLDER header + path + size + count
            Some(ListRow::Media(_)) => {}
            None => return 0,
        }
        let file = match self.selected_file() {
            Some(f) => f,
            None => return 0,
        };
        let mut lines: u16 = 0;
        // FILE section: header + path + size + optional format + optional duration
        lines += 1 + 1 + 1;
        if file.container_format.is_some() {
            lines += 1;
        }
        if file.duration_secs.is_some() {
            lines += 1;
        }

        if !file.is_probed() {
            return lines + 2; // "Probing..." or error
        }

        for vs in &file.video_streams {
            lines += 2; // blank + header
            lines += 1; // codec
            if vs.codec_long.is_some() {
                lines += 1;
            }
            if vs.width > 0 {
                lines += 1;
            }
            if vs.bitrate.is_some() {
                lines += 1;
            }
            if vs.fps.is_some() {
                lines += 1;
            }
            if vs.pixel_format.is_some() {
                lines += 1;
            }
        }
        for audio in &file.audio_streams {
            lines += 2 + 1 + 1 + 1; // blank + header + codec + channels + sample_rate
            if audio.codec_long.is_some() {
                lines += 1;
            }
            if audio.bitrate.is_some() {
                lines += 1;
            }
            if audio.language.is_some() {
                lines += 1;
            }
        }
        if !file.subtitle_streams.is_empty() {
            lines += 2; // blank + header
            lines += file.subtitle_streams.len() as u16;
        }
        lines
    }

    // ── Encoder polling + control ────────────────────────────────

    pub fn poll_encode_events(&mut self) {
        while let Ok(event) = self.encoder.event_rx.try_recv() {
            match event {
                EncodeEvent::StatusChange { job_id, status } => {
                    if let Some(job) = self.encode_queue.iter_mut().find(|j| j.id == job_id) {
                        if job.started_at.is_none()
                            && matches!(
                                status,
                                EncodeStatus::CopyingToTemp | EncodeStatus::Encoding
                            )
                        {
                            job.started_at = Some(std::time::Instant::now());
                        }
                        job.status = match status {
                            EncodeStatus::CopyingToTemp => EncodeJobStatus::CopyingToTemp,
                            EncodeStatus::Encoding | EncodeStatus::Resumed => {
                                EncodeJobStatus::Encoding
                            }
                            EncodeStatus::Paused => EncodeJobStatus::Paused,
                            EncodeStatus::Validating => EncodeJobStatus::Validating,
                        };
                    }
                }
                EncodeEvent::Progress { job_id, progress } => {
                    if let Some(job) = self.encode_queue.iter_mut().find(|j| j.id == job_id) {
                        job.fps_stats.update(progress.fps);
                        job.progress = Some(progress);
                    }
                }
                EncodeEvent::Completed { job_id, result } => {
                    // Snapshot elapsed time before it keeps ticking
                    if let Some(job) = self.encode_queue.iter_mut().find(|j| j.id == job_id) {
                        job.elapsed_secs = job.started_at.map(|t| t.elapsed().as_secs_f64());
                    }

                    // Find the job's file_index before mutating status
                    let file_index = self
                        .encode_queue
                        .iter()
                        .find(|j| j.id == job_id)
                        .map(|j| j.file_index);

                    if let Some(job) = self.encode_queue.iter_mut().find(|j| j.id == job_id) {
                        job.status = match result {
                            EncodeResult::Success {
                                encoded_size,
                                saved_percent,
                                ref final_path,
                            } => {
                                // Update the MediaFile to reflect new codec/size/path
                                if let Some(fi) = file_index
                                    && let Some(file) = self.files.get_mut(fi)
                                {
                                    file.file_size = encoded_size;
                                    file.path = final_path.clone();
                                    if let Some(vs) = file.video_streams.first_mut() {
                                        vs.codec = "hevc".to_string();
                                        vs.codec_long = None;
                                        vs.bitrate = None; // unknown until re-probed
                                    }
                                }
                                // Update queue entry so the size column reflects the encoded size
                                job.file_size = encoded_size;
                                EncodeJobStatus::Done {
                                    encoded_size,
                                    saved_percent,
                                }
                            }
                            EncodeResult::Failed(msg) => EncodeJobStatus::Failed(msg),
                            EncodeResult::Cancelled => EncodeJobStatus::Cancelled,
                        };
                    }
                    self.rebuild_rows();
                    // Auto-start next queued job
                    self.start_next_encode();
                }
            }
        }
    }

    /// Send the next Queued job to the encoder worker.
    pub fn start_next_encode(&mut self) {
        // Don't start if something is already encoding
        if self.current_encoding_job().is_some() {
            return;
        }
        // Find next Queued job that has a preset assigned; jobs without a preset
        // stay parked until the user picks one.
        let next = self
            .encode_queue
            .iter()
            .find(|j| matches!(j.status, EncodeJobStatus::Queued) && j.preset_name.is_some());
        let next = match next {
            Some(j) => j,
            None => return,
        };

        let source_path = self.files.get(next.file_index).map(|f| f.path.clone());
        let source_path = match source_path {
            Some(p) => p,
            None => return,
        };

        let preset_name = match next.preset_name.as_deref() {
            Some(n) => n,
            None => return,
        };
        let preset = self.presets.iter().find(|p| p.name == preset_name).cloned();
        let preset = match preset {
            Some(p) => p,
            None => return,
        };

        let request = EncodeRequest {
            job_id: next.id,
            source_path,
            file_size: next.file_size,
            duration_secs: next.duration_secs,
            preset,
        };

        let _ = self.encoder.request_tx.send(request);
    }

    /// Start encoding the queue (called when user presses the start key).
    pub fn start_encoding(&mut self) {
        self.start_next_encode();
    }

    pub fn pause_encoding(&mut self) {
        let _ = self.encoder.control_tx.send(EncodeControl::Pause);
    }

    pub fn resume_encoding(&mut self) {
        let _ = self.encoder.control_tx.send(EncodeControl::Resume);
    }

    pub fn cancel_current_encode(&mut self) {
        let _ = self.encoder.control_tx.send(EncodeControl::Cancel);
    }

    /// Cancel everything: kill current encode + flush the entire queue.
    pub fn cancel_all(&mut self) {
        if self.is_encoding_active() {
            let _ = self.encoder.control_tx.send(EncodeControl::Cancel);
        }
        self.encode_queue.clear();
        self.encode_queue_selected = 0;
    }

    /// Drop all queued jobs but let the currently encoding item finish.
    pub fn stop_queue(&mut self) {
        self.encode_queue
            .retain(|j| !matches!(j.status, EncodeJobStatus::Queued));
        if self.encode_queue_selected >= self.encode_queue.len() {
            self.encode_queue_selected = self.encode_queue.len().saturating_sub(1);
        }
    }

    /// Remove stale h264_*/hevc_* temp files from all preset temp dirs.
    pub fn cleanup_temp_dirs(&self) {
        let mut dirs_seen = std::collections::HashSet::new();
        for preset in &self.presets {
            if !dirs_seen.insert(preset.temp_dir.clone()) {
                continue;
            }
            let entries = match std::fs::read_dir(&preset.temp_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("h264_") || name.starts_with("hevc_") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    /// Whether encoding is currently active (any job encoding/copying/validating).
    pub fn is_encoding_active(&self) -> bool {
        self.current_encoding_job().is_some()
    }

    /// Whether the current job is paused.
    pub fn is_encoding_paused(&self) -> bool {
        self.encode_queue
            .iter()
            .any(|j| matches!(j.status, EncodeJobStatus::Paused))
    }

    pub fn codec_counts(&self) -> (usize, usize, usize, usize) {
        let mut h264 = 0;
        let mut hevc = 0;
        let mut av1 = 0;
        let mut other = 0;
        for f in &self.files {
            match f.primary_video_codec() {
                Some("h264") => h264 += 1,
                Some("hevc") => hevc += 1,
                Some("av1") => av1 += 1,
                Some(_) => other += 1,
                None => {}
            }
        }
        (h264, hevc, av1, other)
    }

    // ── Encoding queue management ───────────────────────────────

    /// Whether a file can be encoded: probed successfully and not already HEVC/AV1.
    pub fn is_encodeable(&self, file_index: usize) -> bool {
        let file = match self.files.get(file_index) {
            Some(f) => f,
            None => return false,
        };
        if file.probe_status != ProbeStatus::Done {
            return false;
        }
        match file.primary_video_codec() {
            None => false,
            Some(codec) => !self.skip_codecs.iter().any(|s| s == codec),
        }
    }

    /// Whether a file is already in the queue (not finished).
    pub fn is_in_queue(&self, file_index: usize) -> bool {
        self.encode_queue
            .iter()
            .any(|j| j.file_index == file_index && !j.status.is_finished())
    }

    /// Name of the currently selected preset, or None if none chosen yet.
    pub fn current_preset(&self) -> Option<&EncodingPreset> {
        self.presets.get(self.selected_preset?)
    }

    /// Open the preset picker popup.
    fn open_preset_picker(&mut self) {
        if self.presets.is_empty() {
            return;
        }
        self.preset_picker = Some(PresetPicker::new(self.selected_preset.unwrap_or(0)));
    }

    /// Handle key input while the preset picker is open.
    fn handle_preset_picker_key(&mut self, code: KeyCode) {
        let len = self.presets.len();
        if len == 0 {
            self.preset_picker = None;
            return;
        }

        let picker = match &mut self.preset_picker {
            Some(p) => p,
            None => return,
        };

        match code {
            KeyCode::Up => {
                picker.cursor = picker.cursor.saturating_sub(1);
            }
            KeyCode::Down => {
                picker.cursor = (picker.cursor + 1).min(len - 1);
            }
            KeyCode::Home => picker.cursor = 0,
            KeyCode::End => picker.cursor = len - 1,
            KeyCode::Enter => {
                let chosen = picker.cursor;
                self.preset_picker = None;
                self.apply_preset_choice(chosen);
            }
            KeyCode::Esc | KeyCode::Char('p') => {
                self.preset_picker = None;
            }
            // Number keys for quick selection (1-9)
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('1' as usize);
                if idx < len {
                    self.preset_picker = None;
                    self.apply_preset_choice(idx);
                }
            }
            _ => {}
        }
    }

    /// Apply the chosen preset. In List view, sets the default. In Encoding view,
    /// also updates the currently selected queued job.
    fn apply_preset_choice(&mut self, preset_index: usize) {
        self.selected_preset = Some(preset_index);

        // In encoding view: update the selected queue item's preset (if it's still queued)
        if self.active_view == ActiveView::Encoding
            && !self.encode_queue.is_empty()
            && let Some(job) = self.encode_queue.get_mut(self.encode_queue_selected)
            && matches!(job.status, EncodeJobStatus::Queued)
            && let Some(preset) = self.presets.get(preset_index)
        {
            job.preset_name = Some(preset.name.clone());
        }
    }

    /// Stamp the current default preset onto the selected queue item and move down.
    /// Designed for rapid P,P,P... hammering through the queue.
    fn stamp_preset_and_advance(&mut self) {
        if self.encode_queue.is_empty() || self.presets.is_empty() {
            return;
        }
        let preset_name = match self.selected_preset.and_then(|i| self.presets.get(i)) {
            Some(p) => p.name.clone(),
            None => return,
        };
        if let Some(job) = self.encode_queue.get_mut(self.encode_queue_selected)
            && matches!(job.status, EncodeJobStatus::Queued)
        {
            job.preset_name = Some(preset_name);
        }
        // Advance to next row
        let len = self.encode_queue.len();
        if self.encode_queue_selected + 1 < len {
            self.encode_queue_selected += 1;
        }
    }

    /// Add a single file to the encoding queue. Returns the job id, or None
    /// if the file isn't encodeable or is already queued. Jobs may be queued
    /// without a preset; the encoder skips them until one is assigned.
    pub fn enqueue_file(&mut self, file_index: usize) -> Option<u64> {
        if !self.is_encodeable(file_index) || self.is_in_queue(file_index) {
            return None;
        }
        let preset_name = self.current_preset().map(|p| p.name.clone());
        let file = &self.files[file_index];
        let id = self.next_job_id;
        self.next_job_id += 1;

        self.encode_queue.push(EncodeJob {
            id,
            file_index,
            file_name: file.file_name().to_string(),
            file_size: file.file_size,
            duration_secs: file.duration_secs,
            total_frames: file.video_streams.first().and_then(|v| v.frame_count),
            status: EncodeJobStatus::Queued,
            progress: None,
            fps_stats: FpsStats::default(),
            started_at: None,
            elapsed_secs: None,
            preset_name,
        });

        Some(id)
    }

    /// Add all encodeable files to the queue. Returns the number of files added.
    pub fn enqueue_all_encodeable(&mut self) -> usize {
        let indices: Vec<usize> = (0..self.files.len())
            .filter(|&i| self.is_encodeable(i) && !self.is_in_queue(i))
            .collect();
        let mut count = 0;
        for i in indices {
            if self.enqueue_file(i).is_some() {
                count += 1;
            }
        }
        count
    }

    /// Add all encodeable files in a folder (non-recursive) to the queue.
    /// Add all encodeable files in a folder (non-recursive) to the queue.
    fn enqueue_folder(&mut self, folder_idx: usize) -> usize {
        let folder_path = match self.folders.get(folder_idx) {
            Some(f) => f.path.clone(),
            None => return 0,
        };
        let indices: Vec<usize> = (0..self.files.len())
            .filter(|&i| {
                self.files[i].path.parent() == Some(folder_path.as_path())
                    && self.is_encodeable(i)
                    && !self.is_in_queue(i)
            })
            .collect();
        let mut count = 0;
        for i in indices {
            if self.enqueue_file(i).is_some() {
                count += 1;
            }
        }
        count
    }

    /// Toggle folder queue: if all encodeable files are queued, unqueue them all.
    /// Otherwise, enqueue the missing ones.
    pub fn toggle_folder_queue(&mut self, folder_idx: usize) {
        let folder_path = match self.folders.get(folder_idx) {
            Some(f) => f.path.clone(),
            None => return,
        };
        let encodeable: Vec<usize> = (0..self.files.len())
            .filter(|&i| {
                self.files[i].path.parent() == Some(folder_path.as_path()) && self.is_encodeable(i)
            })
            .collect();

        if encodeable.is_empty() {
            return;
        }

        let all_queued = encodeable.iter().all(|&i| self.is_in_queue(i));

        if all_queued {
            // Unqueue all — remove Queued jobs for these files
            for &file_idx in &encodeable {
                self.try_unqueue_file(file_idx);
            }
        } else {
            // Enqueue missing ones
            self.enqueue_folder(folder_idx);
        }
    }

    /// Remove a job from the queue by queue index. Only removable if Queued/Done/Failed/Cancelled.
    pub fn remove_from_queue(&mut self, queue_index: usize) -> bool {
        if let Some(job) = self.encode_queue.get(queue_index)
            && job.status.is_removable()
        {
            self.encode_queue.remove(queue_index);
            if self.encode_queue_selected >= self.encode_queue.len()
                && self.encode_queue_selected > 0
            {
                self.encode_queue_selected -= 1;
            }
            return true;
        }
        false
    }

    /// Remove a queued job by file index (toggle behavior for Enter key in list view).
    /// Only removes jobs with Queued status. Returns true if a job was removed.
    pub fn try_unqueue_file(&mut self, file_index: usize) -> bool {
        if let Some(pos) = self
            .encode_queue
            .iter()
            .position(|j| j.file_index == file_index && matches!(j.status, EncodeJobStatus::Queued))
        {
            self.encode_queue.remove(pos);
            if self.encode_queue_selected >= self.encode_queue.len()
                && self.encode_queue_selected > 0
            {
                self.encode_queue_selected -= 1;
            }
            return true;
        }
        false
    }

    /// The currently active (encoding/copying/validating) job, if any.
    pub fn current_encoding_job(&self) -> Option<&EncodeJob> {
        self.encode_queue.iter().find(|j| {
            matches!(
                j.status,
                EncodeJobStatus::CopyingToTemp
                    | EncodeJobStatus::Encoding
                    | EncodeJobStatus::Paused
                    | EncodeJobStatus::Validating
            )
        })
    }

    /// Count of jobs still queued (not started yet).
    pub fn queued_count(&self) -> usize {
        self.encode_queue
            .iter()
            .filter(|j| matches!(j.status, EncodeJobStatus::Queued))
            .count()
    }

    /// Count of finished jobs (done + failed + cancelled).
    pub fn finished_count(&self) -> usize {
        self.encode_queue
            .iter()
            .filter(|j| j.status.is_finished())
            .count()
    }
}
