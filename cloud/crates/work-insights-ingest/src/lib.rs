use std::collections::HashSet;
use std::io::Read;

use dystil_protocol::{
    SegmentRevisionAck, SegmentUploadRequest, SegmentUploadResponse,
    WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION,
};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use work_insights_db::segments as db_segments;
use work_insights_db::Principal;

const MAX_DECOMPRESSED_BYTES: usize = 25 * 1024 * 1024;
const MAX_SEGMENTS_PER_UPLOAD: usize = 32;
const MAX_ITEMS_PER_SEGMENT: usize = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum IngestProcessError {
    #[error("{0}")]
    BadPayload(String),
    #[error(transparent)]
    Temporary(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl IngestProcessError {
    pub fn is_bad_payload(&self) -> bool {
        matches!(self, Self::BadPayload(_) | Self::Json(_))
    }
}

pub async fn process_segment_upload(
    pool: &PgPool,
    principal: &Principal,
    compressed_body: &[u8],
    compressed_sha256: &str,
) -> Result<SegmentUploadResponse, IngestProcessError> {
    if compressed_body.is_empty() {
        return Err(IngestProcessError::BadPayload(
            "upload body must not be empty".to_string(),
        ));
    }
    if sha256_hex(compressed_body) != compressed_sha256 {
        return Err(IngestProcessError::BadPayload(
            "upload sha256 mismatch".to_string(),
        ));
    }

    let body = gunzip_limited(compressed_body, MAX_DECOMPRESSED_BYTES)?;
    let mut raw_request: serde_json::Value = serde_json::from_slice(&body)?;
    let legacy_acks = strip_legacy_audio_evidence(&mut raw_request)?;
    let mut request: SegmentUploadRequest = serde_json::from_value(raw_request)?;
    if request.segments.is_empty() && !legacy_acks.is_empty() {
        return Ok(SegmentUploadResponse {
            ok: true,
            inserted_count: 0,
            deduped_count: 0,
            accepted: legacy_acks,
        });
    }
    validate_segment_upload(&mut request)?;
    let stats = db_segments::apply_segment_upload(pool, principal, &request)
        .await
        .map_err(|err| match err {
            work_insights_db::DbError::Sqlx(err) => IngestProcessError::Temporary(err),
            work_insights_db::DbError::Json(err) => IngestProcessError::Json(err),
            work_insights_db::DbError::Other(msg) => IngestProcessError::BadPayload(msg),
        })?;

    let mut accepted: Vec<SegmentRevisionAck> = request
        .segments
        .into_iter()
        .map(|segment| SegmentRevisionAck {
            segment_id: segment.segment_id,
            revision: segment.revision,
            status: "accepted".to_string(),
        })
        .collect();
    accepted.extend(legacy_acks);

    Ok(SegmentUploadResponse {
        ok: true,
        inserted_count: stats.inserted_count,
        deduped_count: stats.deduped_count,
        accepted,
    })
}

/// Temporary server-only compatibility for pre-schema-3 desktop clients.
/// It strips legacy audio evidence before the typed schema-3 protocol is
/// deserialized, so the current cloud never writes new audio data. Empty
/// audio-only segments are acknowledged as discarded to stop old clients
/// retrying them indefinitely.
fn strip_legacy_audio_evidence(
    request: &mut serde_json::Value,
) -> Result<Vec<SegmentRevisionAck>, IngestProcessError> {
    let schema_version = request
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| IngestProcessError::BadPayload("missing schema_version".to_string()))?;
    if schema_version >= WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION as u64 {
        return Ok(Vec::new());
    }
    let segments = request
        .get_mut("segments")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| IngestProcessError::BadPayload("segments must be an array".to_string()))?;
    let mut discarded = Vec::new();
    segments.retain_mut(|segment| {
        let Some(object) = segment.as_object_mut() else {
            return true;
        };
        object.remove("audio_state");
        object.remove("audio_detail");
        let Some(items) = object
            .get_mut("items")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return true;
        };
        items.retain(|item| item.get("kind").and_then(serde_json::Value::as_str) != Some("audio"));
        if !items.is_empty() {
            return true;
        }
        if let (Some(segment_id), Some(revision)) = (
            object.get("segment_id").and_then(serde_json::Value::as_str),
            object.get("revision").and_then(serde_json::Value::as_u64),
        ) {
            discarded.push(SegmentRevisionAck {
                segment_id: segment_id.to_string(),
                revision: revision as u32,
                status: "discarded_audio".to_string(),
            });
        }
        false
    });
    request["schema_version"] = serde_json::Value::from(WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION);
    Ok(discarded)
}

fn validate_segment_upload(request: &mut SegmentUploadRequest) -> Result<(), IngestProcessError> {
    if !(1..=WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION).contains(&request.schema_version) {
        return Err(IngestProcessError::BadPayload(format!(
            "unsupported segment schema_version {}",
            request.schema_version
        )));
    }
    if request.segments.is_empty() || request.segments.len() > MAX_SEGMENTS_PER_UPLOAD {
        return Err(IngestProcessError::BadPayload(format!(
            "segment upload must contain 1-{MAX_SEGMENTS_PER_UPLOAD} segments"
        )));
    }

    let mut revision_keys = HashSet::new();
    for segment in &mut request.segments {
        if segment.segment_id.trim().is_empty() || segment.segment_id.len() > 128 {
            return Err(IngestProcessError::BadPayload(
                "segment_id must contain 1-128 characters".to_string(),
            ));
        }
        if segment.revision == 0 || segment.revision > i32::MAX as u32 {
            return Err(IngestProcessError::BadPayload(
                "segment revision is out of range".to_string(),
            ));
        }
        if segment.device_sequence == 0 || segment.device_sequence > i64::MAX as u64 {
            return Err(IngestProcessError::BadPayload(
                "device_sequence is out of range".to_string(),
            ));
        }
        if !revision_keys.insert((segment.segment_id.as_str(), segment.revision)) {
            return Err(IngestProcessError::BadPayload(
                "segment upload contains duplicate revisions".to_string(),
            ));
        }
        if segment.start_time > segment.end_time || segment.end_time > segment.closed_at {
            return Err(IngestProcessError::BadPayload(
                "segment times must satisfy start_time <= end_time <= closed_at".to_string(),
            ));
        }
        if segment.previous_segment_id.as_deref() == Some(segment.segment_id.as_str()) {
            return Err(IngestProcessError::BadPayload(
                "segment cannot reference itself as previous_segment_id".to_string(),
            ));
        }
        if segment.segmenter_version.trim().is_empty() || segment.evidence_version.trim().is_empty()
        {
            return Err(IngestProcessError::BadPayload(
                "segment processor versions must not be empty".to_string(),
            ));
        }
        if segment.items.is_empty() || segment.items.len() > MAX_ITEMS_PER_SEGMENT {
            return Err(IngestProcessError::BadPayload(format!(
                "segment must contain 1-{MAX_ITEMS_PER_SEGMENT} evidence items"
            )));
        }
        if segment.token_estimate == 0 || segment.token_estimate > 100_000 {
            return Err(IngestProcessError::BadPayload(
                "segment token_estimate is out of range".to_string(),
            ));
        }
        if !is_sha256(&segment.content_hash) {
            tracing::error!(
                segment_id = %segment.segment_id,
                revision = segment.revision,
                request_schema_version = request.schema_version,
                segmenter_version = %segment.segmenter_version,
                evidence_version = %segment.evidence_version,
                sync_policy_version = ?segment.sync_policy_version,
                received_content_hash = %segment.content_hash,
                "segment content hash has an invalid format"
            );
            return Err(IngestProcessError::BadPayload(
                "segment content_hash does not match canonical content".to_string(),
            ));
        }

        let computed_content_hash = segment.computed_content_hash()?;
        if computed_content_hash != segment.content_hash {
            // Temporary compatibility while clients roll out deterministic
            // JSON canonicalization. The authenticated server remains the
            // authority for the stored hash; malformed hashes and every
            // other segment validation error still reject the request.
            tracing::warn!(
                segment_id = %segment.segment_id,
                revision = segment.revision,
                request_schema_version = request.schema_version,
                evidence_version = %segment.evidence_version,
                sync_policy_version = ?segment.sync_policy_version,
                received_content_hash = %segment.content_hash,
                canonical_content_hash = %computed_content_hash,
                "normalizing segment content hash during canonicalization rollout"
            );
            segment.content_hash = computed_content_hash;
        }

        let mut item_ids = HashSet::new();
        for item in &segment.items {
            if item.item_id.trim().is_empty() || !item_ids.insert(item.item_id.as_str()) {
                return Err(IngestProcessError::BadPayload(
                    "segment evidence item IDs must be non-empty and unique".to_string(),
                ));
            }
            if item.source_id.trim().is_empty()
                || item.text.trim().is_empty()
                || !is_sha256(&item.source_payload_hash)
            {
                return Err(IngestProcessError::BadPayload(
                    "segment evidence items require source identity, source hash, and text"
                        .to_string(),
                ));
            }
            if item.occurred_at < segment.start_time || item.occurred_at > segment.end_time {
                return Err(IngestProcessError::BadPayload(
                    "segment evidence item occurred_at is outside segment bounds".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn gunzip_limited(body: &[u8], max_bytes: usize) -> Result<Vec<u8>, IngestProcessError> {
    let mut decoder = GzDecoder::new(body);
    let mut out = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = decoder
            .read(&mut chunk)
            .map_err(|err| IngestProcessError::BadPayload(format!("gzip decode failed: {err}")))?;
        if read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read]);
        if out.len() > max_bytes {
            return Err(IngestProcessError::BadPayload(
                "decompressed body exceeds server limit".to_string(),
            ));
        }
    }
    Ok(out)
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dystil_protocol::{
        SegmentEnvelope, SegmentEvidenceItem, SegmentEvidenceKind, EVIDENCE_VERSION,
        SEGMENTER_VERSION,
    };

    fn sample_upload() -> SegmentUploadRequest {
        let now = Utc::now();
        let mut segment = SegmentEnvelope {
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
            token_estimate: 3,
            sync_policy_version: None,
            items: vec![SegmentEvidenceItem {
                item_id: "item_1".to_string(),
                kind: SegmentEvidenceKind::Screen,
                occurred_at: now,
                source_id: "screen_frame:1".to_string(),
                source_payload_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                text: "hello from editor".to_string(),
                app_name: Some("Editor".to_string()),
                window_name: None,
                browser_url: None,
                metadata: serde_json::json!({}),
            }],
            image_refs: Vec::new(),
        };
        segment.refresh_content_hash().unwrap();
        SegmentUploadRequest {
            schema_version: WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION,
            client_sent_at: now,
            segments: vec![segment],
        }
    }

    #[test]
    fn legacy_ingress_strips_audio_before_schema_three_deserialization() {
        let mut request = serde_json::json!({
            "schema_version": 2,
            "segments": [{
                "segment_id": "seg_legacy_1", "revision": 1,
                "audio_state": "complete", "audio_detail": {"chunks_checked": 1},
                "items": [
                    {"kind": "audio", "source_id": "audio_transcription:1"},
                    {"kind": "screen", "source_id": "screen_frame:2"}
                ]
            }]
        });
        let discarded = strip_legacy_audio_evidence(&mut request).unwrap();
        assert!(discarded.is_empty());
        assert_eq!(
            request["schema_version"],
            WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION
        );
        let segment = &request["segments"][0];
        assert!(segment.get("audio_state").is_none());
        assert_eq!(segment["items"].as_array().unwrap().len(), 1);
        assert_eq!(segment["items"][0]["kind"], "screen");
    }

    #[test]
    fn legacy_audio_only_segment_is_acknowledged_without_storage() {
        let mut request = serde_json::json!({
            "schema_version": 2,
            "segments": [{
                "segment_id": "seg_legacy_audio", "revision": 3,
                "items": [{"kind": "audio", "source_id": "audio_transcription:1"}]
            }]
        });
        let discarded = strip_legacy_audio_evidence(&mut request).unwrap();
        assert_eq!(request["segments"].as_array().unwrap().len(), 0);
        assert_eq!(discarded[0].segment_id, "seg_legacy_audio");
        assert_eq!(discarded[0].status, "discarded_audio");
    }

    #[test]
    fn validates_canonical_hash() {
        let mut request = sample_upload();
        validate_segment_upload(&mut request).unwrap();
    }

    #[test]
    fn accepts_accessibility_tree_and_diagnostics_in_screen_metadata() {
        let mut request = sample_upload();
        request.segments[0].items[0].text = "Accessibility structure captured".to_string();
        request.segments[0].items[0].metadata = serde_json::json!({
            "frame_id": 1,
            "accessibility_tree": {"role": "window", "children": []},
            "ax_capture_diagnostics": {"source": "ax", "node_count": 1}
        });
        request.segments[0].refresh_content_hash().unwrap();

        validate_segment_upload(&mut request).unwrap();
    }

    #[test]
    fn normalizes_changed_content_hash() {
        let mut request = sample_upload();
        request.segments[0].items[0].text = "changed after hashing".to_string();
        let expected = request.segments[0].computed_content_hash().unwrap();
        validate_segment_upload(&mut request).unwrap();
        assert_eq!(request.segments[0].content_hash, expected);
    }

    #[test]
    fn normalizes_mismatched_v2_segment_hash() {
        let mut request = sample_upload();
        let segment = &mut request.segments[0];
        segment.sync_policy_version = Some("server-default-v1".to_string());
        segment.content_hash =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
        let expected = segment.computed_content_hash().unwrap();

        validate_segment_upload(&mut request).unwrap();
        assert_eq!(request.segments[0].content_hash, expected);
    }

    #[test]
    fn rejects_malformed_content_hash() {
        let mut request = sample_upload();
        request.segments[0].content_hash = "not-a-hash".to_string();
        assert!(validate_segment_upload(&mut request).is_err());
    }
}
