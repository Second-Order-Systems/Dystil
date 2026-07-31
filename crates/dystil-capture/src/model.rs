use std::sync::Arc;

use chrono::{DateTime, Utc};
use image::DynamicImage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityLine {
    pub char_start: u32,
    pub char_count: u32,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityNode {
    #[serde(default)]
    pub node_id: u32,
    #[serde(default)]
    pub parent_node_id: Option<u32>,
    pub role: String,
    pub text: String,
    pub depth: u8,
    pub bounds: Option<Bounds>,
    pub on_screen: Option<bool>,
    pub lines: Option<Vec<AccessibilityLine>>,
    pub automation_id: Option<String>,
    pub class_name: Option<String>,
    pub value: Option<String>,
    pub help_text: Option<String>,
    pub url: Option<String>,
    pub placeholder: Option<String>,
    pub role_description: Option<String>,
    pub subrole: Option<String>,
    #[serde(default)]
    pub dom_identifier: Option<String>,
    #[serde(default)]
    pub dom_classes: Option<String>,
    pub is_enabled: Option<bool>,
    pub is_focused: Option<bool>,
    pub is_selected: Option<bool>,
    pub is_expanded: Option<bool>,
    pub is_password: Option<bool>,
    pub is_keyboard_focusable: Option<bool>,
    pub accelerator_key: Option<String>,
    pub access_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityTruncationReason {
    None,
    Timeout,
    MaxNodes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilitySnapshot {
    pub captured_at: DateTime<Utc>,
    pub context: CaptureContext,
    pub text: String,
    pub nodes: Vec<AccessibilityNode>,
    pub node_count: usize,
    pub walk_duration_ms: u64,
    pub content_hash: u64,
    pub simhash: u64,
    pub truncated: bool,
    pub truncation_reason: AccessibilityTruncationReason,
    pub max_depth_reached: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTrigger {
    AppSwitch,
    WindowFocus,
    Click,
    TypingPause,
    ScrollStop,
    KeyPress,
    Clipboard,
    VisualChange,
    Idle,
    Manual,
    ActivitySettled,
}

impl CaptureTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AppSwitch => "app_switch",
            Self::WindowFocus => "window_focus",
            Self::Click => "click",
            Self::TypingPause => "typing_pause",
            Self::ScrollStop => "scroll_stop",
            Self::KeyPress => "key_press",
            Self::Clipboard => "clipboard",
            Self::VisualChange => "visual_change",
            Self::Idle => "idle",
            Self::Manual => "manual",
            Self::ActivitySettled => "activity_settled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureContext {
    pub application: Option<String>,
    pub window: Option<String>,
    pub browser_url: Option<String>,
    pub document_path: Option<String>,
    pub display_id: Option<String>,
    pub monitor_id: Option<u32>,
    pub device_name: Option<String>,
    pub focused: Option<bool>,
    /// Trigger location in the event source's virtual-desktop coordinates.
    /// Providers may use it as a routing hint only: when its coordinate space
    /// cannot be reconciled with display geometry, capture must safely fan out
    /// rather than silently selecting an arbitrary display. It is not
    /// persisted.
    pub target: Option<ScreenPoint>,
}

impl CaptureContext {
    /// Keep captured values and fill unavailable fields from trigger/request
    /// context. AX is authoritative because it describes the window actually
    /// walked, which may differ from a trigger emitted moments earlier.
    pub fn with_fallback(&self, fallback: &Self) -> Self {
        Self {
            application: self
                .application
                .clone()
                .or_else(|| fallback.application.clone()),
            window: self.window.clone().or_else(|| fallback.window.clone()),
            browser_url: self
                .browser_url
                .clone()
                .or_else(|| fallback.browser_url.clone()),
            document_path: self
                .document_path
                .clone()
                .or_else(|| fallback.document_path.clone()),
            display_id: self
                .display_id
                .clone()
                .or_else(|| fallback.display_id.clone()),
            monitor_id: self.monitor_id.or(fallback.monitor_id),
            device_name: self
                .device_name
                .clone()
                .or_else(|| fallback.device_name.clone()),
            focused: self.focused.or(fallback.focused),
            target: self.target.or(fallback.target),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayGeometry {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}

/// Why a caller needs pixels while visual capture is on-demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualDemand {
    UserRequested,
    ActivitySettled,
    /// Reserved for the future diff-engine confidence escalation path.
    LowAccessibilityConfidence {
        reason: String,
    },
}

/// Owned visual evidence. Serialization is intentionally omitted: persistence
/// adapters decide how and where an image is encoded.
#[derive(Debug, Clone)]
pub struct VisualSnapshot {
    pub captured_at: DateTime<Utc>,
    pub image: Arc<DynamicImage>,
    pub monitor_id: Option<u32>,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualCaptureStatus {
    Captured,
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedVisualCapture {
    pub stored: StoredCapture,
    pub status: VisualCaptureStatus,
}

#[derive(Debug, Clone)]
pub struct CaptureObservation {
    pub captured_at: DateTime<Utc>,
    pub trigger: CaptureTrigger,
    pub context: CaptureContext,
    pub accessibility: Option<AccessibilitySnapshot>,
    pub visual: Option<VisualSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCapture {
    pub frame_id: i64,
    pub snapshot_path: Option<String>,
}
