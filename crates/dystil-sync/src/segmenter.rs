use chrono::{DateTime, Duration, Utc};
use dystil_protocol::{
    SegmentEnvelope, SegmentEvidenceItem, SegmentEvidenceKind, SegmentImageRef, EVIDENCE_VERSION,
    SEGMENTER_VERSION,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::types::SyncError;

#[derive(Debug, Clone)]
pub struct SegmentConfig {
    pub inactivity_seconds: i64,
    pub max_duration_seconds: i64,
    pub max_tokens: u32,
    pub policy_version: String,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            inactivity_seconds: 5 * 60,
            max_duration_seconds: 15 * 60,
            max_tokens: 10_000,
            policy_version: "compiled-v1".to_string(),
        }
    }
}

impl SegmentConfig {
    pub fn from_policy(policy: &dystil_protocol::SyncPolicy) -> Self {
        Self {
            inactivity_seconds: policy.segmenting.inactivity_seconds,
            max_duration_seconds: policy.segmenting.max_duration_seconds,
            max_tokens: policy.segmenting.max_tokens,
            policy_version: policy.policy_version.clone(),
        }
    }
}

pub fn build_segments(
    mut items: Vec<SegmentEvidenceItem>,
    config: &SegmentConfig,
    device_identity: &str,
    first_sequence: u64,
    previous_segment_id: Option<String>,
    closed_at: DateTime<Utc>,
) -> Result<Vec<SegmentEnvelope>, SyncError> {
    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let mut groups: Vec<Vec<SegmentEvidenceItem>> = Vec::new();
    let mut current = Vec::new();
    let mut current_tokens = 0_u32;

    for item in items {
        let item_tokens = estimate_tokens(&item.text);
        let split = current.last().map(|last: &SegmentEvidenceItem| {
            item.occurred_at - last.occurred_at > Duration::seconds(config.inactivity_seconds)
                || item.occurred_at - current[0].occurred_at
                    >= Duration::seconds(config.max_duration_seconds)
                || current_tokens.saturating_add(item_tokens) > config.max_tokens
        });
        if split == Some(true) {
            groups.push(std::mem::take(&mut current));
            current_tokens = 0;
        }
        current_tokens = current_tokens.saturating_add(item_tokens);
        current.push(item);
    }
    if !current.is_empty() {
        groups.push(current);
    }

    let mut envelopes = Vec::with_capacity(groups.len());
    let mut previous_id = previous_segment_id;
    for (index, group) in groups.into_iter().enumerate() {
        let sequence = first_sequence + index as u64;
        let segment_id = segment_id(device_identity, sequence);
        let start_time = group.first().expect("non-empty segment group").occurred_at;
        let end_time = group.last().expect("non-empty segment group").occurred_at;
        let token_estimate = group.iter().map(|item| estimate_tokens(&item.text)).sum();
        let image_refs = image_refs(&group);
        let mut envelope = SegmentEnvelope {
            segment_id: segment_id.clone(),
            revision: 1,
            device_sequence: sequence,
            previous_segment_id: previous_id,
            start_time,
            end_time,
            closed_at: closed_at.max(end_time),
            segmenter_version: SEGMENTER_VERSION.to_string(),
            evidence_version: EVIDENCE_VERSION.to_string(),
            content_hash: String::new(),
            token_estimate,
            sync_policy_version: Some(config.policy_version.clone()),
            items: group,
            image_refs,
        };
        envelope.refresh_content_hash()?;
        previous_id = Some(segment_id);
        envelopes.push(envelope);
    }
    Ok(envelopes)
}

pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.div_ceil(4).max(1)
}

fn image_refs(items: &[SegmentEvidenceItem]) -> Vec<SegmentImageRef> {
    let mut frame_ids = BTreeSet::new();
    for item in items {
        if item.kind != SegmentEvidenceKind::Screen {
            continue;
        }
        if let Some(frame_id) = item
            .metadata
            .get("frame_id")
            .and_then(|value| value.as_i64())
        {
            frame_ids.insert(frame_id);
        }
    }
    frame_ids
        .into_iter()
        .map(|frame_id| SegmentImageRef {
            client_image_key: format!("frame:{frame_id}"),
            frame_id,
            required: false,
        })
        .collect()
}

fn segment_id(device_identity: &str, sequence: u64) -> String {
    let digest = Sha256::digest(device_identity.as_bytes());
    format!("seg_{}_{sequence:08}", &hex::encode(digest)[..12])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: usize, seconds: i64, app: &str, text: &str) -> SegmentEvidenceItem {
        let base = DateTime::parse_from_rfc3339("2026-06-28T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        SegmentEvidenceItem {
            item_id: format!("item_{id}"),
            kind: SegmentEvidenceKind::Screen,
            occurred_at: base + Duration::seconds(seconds),
            source_id: format!("screen_frame:{id}"),
            source_payload_hash: format!("sha256:{id:064x}"),
            text: text.to_string(),
            app_name: Some(app.to_string()),
            window_name: None,
            browser_url: None,
            metadata: json!({"frame_id": id}),
        }
    }

    #[test]
    fn rapid_app_switches_do_not_split_segments() {
        let items = vec![
            item(1, 0, "Editor", "working on code"),
            item(2, 10, "Browser", "checking documentation"),
            item(3, 20, "Terminal", "running tests"),
        ];
        let segments = build_segments(
            items,
            &SegmentConfig::default(),
            "device",
            1,
            None,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].items.len(), 3);
    }

    #[test]
    fn inactivity_gap_splits_segments_and_links_sequence() {
        let items = vec![
            item(1, 0, "Editor", "first"),
            item(2, 301, "Editor", "second"),
        ];
        let segments = build_segments(
            items,
            &SegmentConfig::default(),
            "device",
            7,
            Some("seg_previous".to_string()),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].device_sequence, 7);
        assert_eq!(
            segments[0].previous_segment_id.as_deref(),
            Some("seg_previous")
        );
        assert_eq!(
            segments[1].previous_segment_id.as_deref(),
            Some(segments[0].segment_id.as_str())
        );
    }

    #[test]
    fn token_limit_splits_before_overflow() {
        let config = SegmentConfig {
            max_tokens: 2,
            ..SegmentConfig::default()
        };
        let segments = build_segments(
            vec![item(1, 0, "Editor", "1234"), item(2, 1, "Editor", "5678")],
            &config,
            "device",
            1,
            None,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(segments.len(), 1);

        let config = SegmentConfig {
            max_tokens: 1,
            ..SegmentConfig::default()
        };
        let segments = build_segments(
            vec![item(1, 0, "Editor", "1234"), item(2, 1, "Editor", "5678")],
            &config,
            "device",
            1,
            None,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn segment_carries_the_effective_policy_version() {
        let config = SegmentConfig {
            policy_version: "server-policy-2026-07-18".to_string(),
            ..SegmentConfig::default()
        };
        let segments = build_segments(
            vec![item(1, 0, "Editor", "one")],
            &config,
            "device",
            1,
            None,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(
            segments[0].sync_policy_version.as_deref(),
            Some("server-policy-2026-07-18")
        );
    }
}
