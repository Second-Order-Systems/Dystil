//! Owned screen capture utilities for Dystil.
//!
//! Lifted from `dystil-screen`. OCR functions are excluded from this build;
//! monitor discovery, window filtering, and SCStream stream management are owned here.

pub mod browser_utils;
pub mod capture_screenshot_by_window;
pub mod monitor;

pub use capture_screenshot_by_window::{get_excluded_sck_window_ids, WindowFilters};
pub use monitor::{list_monitors, list_monitors_detailed, MonitorListError, SafeMonitor};
#[cfg(target_os = "macos")]
pub use monitor::{set_sck_capture_max_width, HdCapture};

/// SCStream invalidation flag — set by wake/unlock observers, consumed by the capture loop.
#[cfg(target_os = "macos")]
pub mod stream_invalidation {
    use std::sync::atomic::{AtomicBool, Ordering};
    static NEEDS_INVALIDATION: AtomicBool = AtomicBool::new(false);

    pub fn request() {
        NEEDS_INVALIDATION.store(true, Ordering::SeqCst);
    }

    pub fn take() -> bool {
        NEEDS_INVALIDATION.swap(false, Ordering::SeqCst)
    }

    pub fn invalidate_streams() {
        sck_rs::stop_all_streams();
    }

    pub fn invalidate_monitor_stream(monitor_id: u32) {
        sck_rs::invalidate_monitor_stream(monitor_id);
    }
}
