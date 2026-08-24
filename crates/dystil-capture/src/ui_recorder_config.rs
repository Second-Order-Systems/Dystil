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
    /// Candidate-only Stage 2 behavior: merge the delayed precise UIA target
    /// into its physical click instead of storing/triggering a second click.
    pub merge_click_enrichment: bool,
    pub settled_state_scheduler: bool,
    pub scroll_stop_delay_ms: u64,
    /// Keep the baseline background UIA tree stream. Candidate policies can
    /// disable it while retaining click element enrichment.
    pub capture_background_trees: bool,
    /// Candidate-only click ownership accuracy experiment.
    pub precise_click_window_context: bool,
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
        config.capture_background_trees = self.capture_background_trees;
        config.precise_click_window_context = self.precise_click_window_context;
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_can_disable_background_trees_without_disabling_click_context() {
        let config = DystilUiRecorderConfig {
            capture_clicks: true,
            capture_scroll: false,
            capture_clipboard: false,
            capture_clipboard_content: false,
            capture_text: false,
            capture_keystrokes: false,
            record_keyboard_events: false,
            record_clipboard_events: false,
            ignored_windows: Vec::new(),
            included_windows: Vec::new(),
            batch_size: 1,
            batch_timeout_ms: 1,
            typing_pause_delay_ms: 1,
            prioritize_input_latency: false,
            extraction_thread_priority: ExtractionThreadPriority::default(),
            pause_extraction_on_input_ms: 0,
            merge_click_enrichment: false,
            settled_state_scheduler: false,
            scroll_stop_delay_ms: 300,
            capture_background_trees: false,
            precise_click_window_context: false,
        }
        .native_config();

        assert!(config.capture_clicks);
        assert!(config.capture_context);
        assert!(!config.capture_background_trees);
    }
}
