//! Dystil-owned capture configuration replacing `dystil_engine::RecordingConfig`.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DystilCaptureConfig {
    pub data_dir: PathBuf,
    pub disable_vision: bool,
    pub capture_scroll: Option<bool>,
    pub disable_clipboard_capture: bool,
    pub capture_on_clipboard: Option<bool>,
    pub disable_keyboard_capture: bool,
    pub ignored_windows: Vec<String>,
    pub included_windows: Vec<String>,
    pub ignored_urls: Vec<String>,
    pub ignore_incognito_windows: bool,
    pub prioritize_input_latency: bool,
    pub extraction_thread_priority: String,
    pub pause_extraction_on_input_ms: u64,
}
