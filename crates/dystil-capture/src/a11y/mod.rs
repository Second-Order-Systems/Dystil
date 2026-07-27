//! Owned accessibility and UI event capture for Dystil.
//!
//! Lifted verbatim from `dystil-a11y` — all types are now owned by
//! `dystil-capture` and exported from this module. External crates that
//! previously depended on `dystil-a11y` should use `dystil_capture::a11y`.

pub mod activity_feed;
pub mod budget;
pub mod config;
pub mod events;
pub mod incognito;
pub mod platform;
pub mod tree;
pub mod url_filter;

// Re-exports
pub use activity_feed::{ActivityFeed, ActivityKind, CaptureParams};
pub use config::{ExtractionThreadPriority, UiCaptureConfig};
pub use events::{
    AccessibilityNode, ElementBounds, ElementContext, EventData, EventType, Modifiers, UiEvent,
    WindowTreeSnapshot,
};
pub use platform::{
    check_input_monitoring, request_input_monitoring, PermissionStatus, RecordingHandle, UiRecorder,
};
