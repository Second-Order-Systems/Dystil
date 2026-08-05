use chrono::{DateTime, Utc};
use dystil_protocol::{
    ImageCompleteItem, ImageFilterDecision, ImageSyncMode, ImageSyncPolicy, SegmentingPolicy,
    SyncPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub sync_interval_secs: u64,
    pub screen_settle_lag_secs: u64,
    pub cold_start_lookback_days: u64,
    pub request_timeout_secs: u64,
    pub policy: SyncPolicy,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            sync_interval_secs: 120,
            screen_settle_lag_secs: 15,
            cold_start_lookback_days: 7,
            request_timeout_secs: 30,
            policy: default_sync_policy(),
        }
    }
}

pub(crate) fn default_sync_policy() -> SyncPolicy {
    SyncPolicy {
        schema_version: 1,
        policy_version: "compiled-v1".to_string(),
        issued_at: Utc::now(),
        refresh_after_seconds: 60,
        image_sync: ImageSyncPolicy {
            mode: ImageSyncMode::AllWithShadow,
            evaluator_version: "image-filter-v1".to_string(),
            stable_text_change_min_seconds: 60,
            min_text_change_chars: 200,
            min_text_change_tokens: 40,
            text_change_jaccard_distance_threshold: 0.40,
            max_selected_per_minute: 3,
            candidate_min_gap_seconds: 20,
            max_uploads_per_pass: 100,
            max_upload_bytes_per_pass: 100 * 1024 * 1024,
            jpeg_quality: 86,
            max_jpeg_width: 1920,
        },
        segmenting: SegmentingPolicy {
            max_tokens: 10_000,
            inactivity_seconds: 5 * 60,
            max_duration_seconds: 15 * 60,
        },
    }
}

#[derive(Debug, Clone)]
pub struct DystilSync {
    pub db_path: PathBuf,
    pub state_db_path: PathBuf,
    pub cloud_base_url: String,
    pub device_token: String,
    pub machine_id: String,
    pub fallback_config: SyncConfig,
    pub request_timeout_secs: u64,
    pub app_version: Option<String>,
    pub build_channel: Option<String>,
    pub build_commit: Option<String>,
    pub sync_capabilities: Vec<String>,
    pub local_permissions: LocalSyncPermissions,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalSyncPermissions {
    pub segments: bool,
    pub screenshots: bool,
}

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub uploaded_segments: usize,
    pub processed_events: usize,
    pub uploaded_images: usize,
    pub config: SyncConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct ImageSyncCache {
    pub(crate) last_scanned_frame_id: i64,
    #[serde(default)]
    pub(crate) pending_complete: Vec<PendingCompleteImage>,
    #[serde(default)]
    pub(crate) pending_upload_retry: Vec<PendingUploadRetry>,
    #[serde(default)]
    pub(crate) monitor_state: BTreeMap<String, MonitorSelectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingCompleteImage {
    pub(crate) item: ImageCompleteItem,
    #[serde(default)]
    pub(crate) retry_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingUploadRetry {
    pub(crate) candidate: ImageCandidate,
    #[serde(default)]
    pub(crate) retry_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct MonitorSelectionState {
    pub(crate) last_app_name: Option<String>,
    pub(crate) last_window_name: Option<String>,
    pub(crate) last_browser_url: Option<String>,
    pub(crate) last_selected_text_signature: Vec<u64>,
    pub(crate) last_selected_at: Option<DateTime<Utc>>,
    pub(crate) initialized: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("{0}")]
    Message(String),
    #[error("device token rejected by server (401)")]
    Unauthorized,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImageCandidate {
    pub(crate) frame_id: i64,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) selection_reason: String,
    pub(crate) source_path: String,
    pub(crate) app_name: Option<String>,
    pub(crate) capture_trigger: Option<String>,
    pub(crate) text_source: Option<String>,
    pub(crate) filter_decision: ImageFilterDecision,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedImage {
    pub(crate) manifest: dystil_protocol::ImageManifest,
    pub(crate) complete_item: ImageCompleteItem,
    pub(crate) jpeg_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct StreamCursor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct SourceCursor {
    pub(crate) screen_frame: StreamCursor,
    pub(crate) input_event: StreamCursor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureEventType {
    ScreenFrame,
    InputEvent,
}

impl CaptureEventType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::ScreenFrame => "screen_frame",
            Self::InputEvent => "input_event",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CaptureEvent {
    pub(crate) event_id: String,
    pub(crate) event_type: CaptureEventType,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) source_table: String,
    pub(crate) source_id: i64,
    pub(crate) payload_hash: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ScreenFramePayload {
    pub(crate) frame_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) window_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) document_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) focused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) capture_trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_source: Option<String>,
    #[serde(rename = "full_text", skip_serializing_if = "Option::is_none")]
    pub(crate) frame_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) simhash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ax_capture_diagnostics: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct InputEventPayload {
    pub(crate) ui_event_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    pub(crate) relative_ms: i64,
    pub(crate) event_type_detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) delta_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) delta_y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) button: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) click_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) key_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) modifiers: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) app_pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) frame_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) element: Option<Value>,
}
