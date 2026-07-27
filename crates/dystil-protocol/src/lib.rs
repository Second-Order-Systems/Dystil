use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub mod agent_mailbox;

pub const WORK_INSIGHTS_IMAGE_SCHEMA_VERSION: u32 = 1;
pub const WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION: u32 = 3;
pub const SEGMENTER_VERSION: &str = "local-segmenter-v1";
pub const EVIDENCE_VERSION: &str = "local-evidence-v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentEvidenceKind {
    Screen,
    Input,
}

impl SegmentEvidenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Input => "input",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentEvidenceItem {
    pub item_id: String,
    pub kind: SegmentEvidenceKind,
    pub occurred_at: DateTime<Utc>,
    pub source_id: String,
    pub source_payload_hash: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_url: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentImageRef {
    pub client_image_key: String,
    pub frame_id: i64,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentEnvelope {
    pub segment_id: String,
    pub revision: u32,
    pub device_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_segment_id: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
    pub segmenter_version: String,
    pub evidence_version: String,
    pub content_hash: String,
    pub token_estimate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_policy_version: Option<String>,
    pub items: Vec<SegmentEvidenceItem>,
    #[serde(default)]
    pub image_refs: Vec<SegmentImageRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageSyncMode {
    AllWithShadow,
    Filtered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageFilterDecision {
    pub evaluator_version: String,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_change_distance: Option<f64>,
    pub would_be_rate_limited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageSyncMetadata {
    pub sync_mode: ImageSyncMode,
    pub policy_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_source: Option<String>,
    pub filter_decision: ImageFilterDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageSyncPolicy {
    pub mode: ImageSyncMode,
    pub evaluator_version: String,
    pub stable_text_change_min_seconds: i64,
    pub min_text_change_chars: usize,
    pub min_text_change_tokens: usize,
    pub text_change_jaccard_distance_threshold: f64,
    pub max_selected_per_minute: usize,
    pub candidate_min_gap_seconds: i64,
    pub max_uploads_per_pass: usize,
    pub max_upload_bytes_per_pass: u64,
    pub jpeg_quality: u8,
    pub max_jpeg_width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentingPolicy {
    pub max_tokens: u32,
    pub inactivity_seconds: i64,
    pub max_duration_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncPolicy {
    pub schema_version: u32,
    pub policy_version: String,
    pub issued_at: DateTime<Utc>,
    pub refresh_after_seconds: u64,
    pub image_sync: ImageSyncPolicy,
    pub segmenting: SegmentingPolicy,
}

impl SegmentEnvelope {
    pub fn computed_content_hash(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.content_hash.clear();
        let mut canonical_value = serde_json::to_value(canonical)?;
        canonicalize_json_object_keys(&mut canonical_value);
        let bytes = serde_json::to_vec(&canonical_value)?;
        let digest = Sha256::digest(bytes);
        Ok(format!("sha256:{}", hex::encode(digest)))
    }

    pub fn refresh_content_hash(&mut self) -> Result<(), serde_json::Error> {
        self.content_hash = self.computed_content_hash()?;
        Ok(())
    }
}

/// Sort every JSON object recursively before hashing. `SegmentEvidenceItem`
/// metadata originates in several independent capture pipelines, so relying on
/// the insertion order of `serde_json::Value` maps would make a content hash
/// depend on the client's enabled serde_json features rather than its content.
fn canonicalize_json_object_keys(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                canonicalize_json_object_keys(value);
            }
        }
        Value::Object(values) => {
            let mut entries = std::mem::take(values).into_iter().collect::<Vec<_>>();
            for (_, value) in &mut entries {
                canonicalize_json_object_keys(value);
            }
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            *values = entries.into_iter().collect();
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentUploadRequest {
    pub schema_version: u32,
    pub client_sent_at: DateTime<Utc>,
    pub segments: Vec<SegmentEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentRevisionAck {
    pub segment_id: String,
    pub revision: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentUploadResponse {
    pub ok: bool,
    pub inserted_count: usize,
    pub deduped_count: usize,
    pub accepted: Vec<SegmentRevisionAck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceSyncStateResponse {
    pub ok: bool,
    pub max_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_segment_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageManifest {
    pub client_image_key: String,
    pub content_hash: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub selection_reason: String,
    pub linked_frame_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_frame_timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame_timestamp: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_metadata: Option<ImageSyncMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImagePrepareRequest {
    pub schema_version: u32,
    pub images: Vec<ImageManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageUploadTicket {
    pub image_id: String,
    pub object_key: String,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImagePrepareResult {
    pub client_image_key: String,
    pub image_id: String,
    pub upload_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_ticket: Option<ImageUploadTicket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImagePrepareResponse {
    pub ok: bool,
    pub results: Vec<ImagePrepareResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageCompleteItem {
    pub image_id: String,
    pub client_image_key: String,
    pub content_hash: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub selection_reason: String,
    pub linked_frame_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_metadata: Option<ImageSyncMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageCompleteRequest {
    pub schema_version: u32,
    pub images: Vec<ImageCompleteItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageCompleteResponse {
    pub ok: bool,
    pub completed: usize,
    pub linked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterDeviceRequest {
    pub device_label: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterDeviceResponse {
    pub ok: bool,
    pub device_id: String,
    pub device_token: String,
    pub device_label: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceSummary {
    pub device_id: String,
    pub org_id: String,
    pub user_id: String,
    pub device_label: String,
    pub platform: String,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListDevicesResponse {
    pub ok: bool,
    pub devices: Vec<DeviceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeDeviceResponse {
    pub ok: bool,
    pub device_id: String,
    pub revoked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_segment() -> SegmentEnvelope {
        let now = Utc::now();
        SegmentEnvelope {
            segment_id: "seg_device_1_00000001".to_string(),
            revision: 1,
            device_sequence: 1,
            previous_segment_id: None,
            start_time: now,
            end_time: now,
            closed_at: now,
            segmenter_version: SEGMENTER_VERSION.to_string(),
            evidence_version: EVIDENCE_VERSION.to_string(),
            content_hash: String::new(),
            token_estimate: 2,
            sync_policy_version: None,
            items: vec![SegmentEvidenceItem {
                item_id: "item_1".to_string(),
                kind: SegmentEvidenceKind::Screen,
                occurred_at: now,
                source_id: "screen_frame:1".to_string(),
                source_payload_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                text: "hello world".to_string(),
                app_name: Some("Editor".to_string()),
                window_name: None,
                browser_url: None,
                metadata: Value::Object(Default::default()),
            }],
            image_refs: Vec::new(),
        }
    }

    #[test]
    fn segment_content_hash_is_stable_and_ignores_existing_hash() {
        let mut segment = sample_segment();
        segment.refresh_content_hash().unwrap();
        let first = segment.content_hash.clone();
        assert_eq!(first.len(), 71);
        assert_eq!(segment.computed_content_hash().unwrap(), first);

        segment.content_hash =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string();
        assert_eq!(segment.computed_content_hash().unwrap(), first);
    }

    #[test]
    fn segment_content_hash_changes_with_evidence() {
        let mut segment = sample_segment();
        let first = segment.computed_content_hash().unwrap();
        segment.items[0].text = "changed".to_string();
        assert_ne!(segment.computed_content_hash().unwrap(), first);
    }

    #[test]
    fn segment_content_hash_survives_json_round_trip() {
        let mut segment = sample_segment();
        segment.items[0].metadata = serde_json::json!({
            "zebra": { "second": true, "first": false },
            "alpha": [ { "last": 2, "first": 1 } ],
        });
        segment.refresh_content_hash().unwrap();

        let encoded = serde_json::to_vec(&segment).unwrap();
        let decoded: SegmentEnvelope = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            decoded.computed_content_hash().unwrap(),
            segment.content_hash
        );
    }

    #[test]
    fn canonicalization_sorts_nested_object_keys() {
        let mut value = serde_json::json!({
            "zebra": { "second": true, "first": false },
            "alpha": [ { "last": 2, "first": 1 } ],
        });
        canonicalize_json_object_keys(&mut value);
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"alpha":[{"first":1,"last":2}],"zebra":{"first":false,"second":true}}"#
        );
    }
}
