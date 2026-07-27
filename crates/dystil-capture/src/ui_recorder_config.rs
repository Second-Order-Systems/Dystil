use crate::a11y::{ExtractionThreadPriority, UiCaptureConfig};

#[derive(Debug, Clone)]
pub struct DystilUiRecorderConfig {
    pub capture_clicks: bool,
    pub capture_scroll: bool,
    pub capture_clipboard: bool,
    pub capture_clipboard_content: bool,
    pub capture_text: bool,
    pub capture_keystrokes: bool,
    pub record_keyboard_events: bool,
    pub record_clipboard_events: bool,
    pub ignored_windows: Vec<String>,
    pub included_windows: Vec<String>,
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub typing_pause_delay_ms: u64,
    pub prioritize_input_latency: bool,
    pub extraction_thread_priority: ExtractionThreadPriority,
    pub pause_extraction_on_input_ms: u64,
}

impl DystilUiRecorderConfig {
    pub fn native_config(&self) -> UiCaptureConfig {
        let mut config = UiCaptureConfig::new();
        config.capture_clicks = self.capture_clicks;
        config.capture_scroll = self.capture_scroll;
        config.capture_clipboard = self.capture_clipboard;
        config.capture_clipboard_content = self.capture_clipboard_content;
        config.capture_text = self.capture_text;
        config.capture_keystrokes = self.capture_keystrokes;
        config.capture_window_focus = true;
        config.text_timeout_ms = self.typing_pause_delay_ms;
        config.ignored_windows = self.ignored_windows.clone();
        config.included_windows = self.included_windows.clone();
        config.prioritize_input_latency = self.prioritize_input_latency;
        config.extraction_thread_priority = self.extraction_thread_priority;
        config.pause_extraction_on_input_ms = self.pause_extraction_on_input_ms;
        config
    }
}
