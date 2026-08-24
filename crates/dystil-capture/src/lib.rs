//! Platform-neutral capture orchestration for Dystil.
//!
//! Screen pixels are an optional evidence source. Accessibility capture and
//! persistence remain available in every visual capture mode.

#[cfg(feature = "native")]
pub mod a11y;
#[cfg(feature = "native")]
pub mod accessibility_provider;
#[cfg(feature = "native")]
pub mod activity_spans;
#[cfg(feature = "native")]
pub mod capture_loop;
#[cfg(feature = "native")]
pub mod capture_store;
mod config;
mod coordinator;
#[cfg(feature = "debug-capture")]
pub mod debug_capture;
mod error;
#[cfg(feature = "native")]
pub mod linker;
mod model;
#[cfg(feature = "native")]
pub mod monitor_selection;
#[cfg(not(target_os = "macos"))]
#[cfg(feature = "native")]
pub mod non_macos_visual_capture;
#[cfg(feature = "native")]
pub mod pii_removal;
mod policy;
mod provider;
#[cfg(feature = "native")]
pub mod redaction_worker;
#[cfg(feature = "native")]
pub mod screen;
#[cfg(feature = "native")]
pub mod screen_lock;
pub mod semantic_tree;
#[cfg(feature = "native")]
pub mod settled_state;
mod trigger;
mod trigger_bus;
#[cfg(feature = "native")]
mod ui_event_store;
#[cfg(feature = "native")]
mod ui_recorder;
#[cfg(feature = "native")]
mod ui_recorder_config;
#[cfg(target_os = "macos")]
#[cfg(feature = "native")]
pub mod visual_capture;
#[cfg(all(feature = "native", target_os = "windows"))]
pub mod wgc_capture;
#[cfg(feature = "native")]
pub mod window_pattern;

pub use config::{CaptureConfig, CaptureMode};
pub use coordinator::CaptureCoordinator;
pub use error::CaptureError;
pub use model::{
    AccessibilityLine, AccessibilityNode, AccessibilitySnapshot, AccessibilityTruncationReason,
    Bounds, CaptureContext, CaptureObservation, CaptureTrigger, DisplayGeometry,
    RequestedVisualCapture, ScreenPoint, StoredCapture, VisualCaptureStatus, VisualDemand,
    VisualSnapshot,
};
pub use policy::{
    creates_visual_demand, resets_pending_deadline, PendingVisualDemand, SettledVisualPolicy,
    DEFAULT_VISUAL_SETTLE_DELAY,
};
pub use provider::{
    AccessibilityProvider, CaptureStore, DisplayGeometryProvider, VisualProvider, VisualRequest,
};
pub use trigger::{CaptureTriggerMessage, TRIGGER_CHANNEL_BUFFER};
pub use trigger_bus::TriggerBus;
#[cfg(feature = "native")]
pub use ui_event_store::{insert_ui_event_batch, UiEventRecord};
#[cfg(feature = "native")]
pub use ui_recorder::{start_dystil_ui_recording, DystilUiRecorderHandle};
#[cfg(feature = "native")]
pub use ui_recorder_config::DystilUiRecorderConfig;
