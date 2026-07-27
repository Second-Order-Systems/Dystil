use chrono::Duration;
use dystil_protocol::SegmentEvidenceItem;
use sha2::{Digest, Sha256};

use crate::{EvidenceWindow, ExportedSegment};

#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub inactivity: Duration,
    pub max_duration: Duration,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            inactivity: Duration::minutes(5),
            max_duration: Duration::minutes(15),
        }
    }
}

pub fn build_evidence_windows(
    mut segments: Vec<ExportedSegment>,
    config: &WindowConfig,
) -> Vec<EvidenceWindow> {
    segments.sort_by(|left, right| {
        left.envelope
            .start_time
            .cmp(&right.envelope.start_time)
            .then_with(|| left.segment_id.cmp(&right.segment_id))
    });
    let mut windows = Vec::new();
    let mut current: Vec<ExportedSegment> = Vec::new();

    for segment in segments {
        let split_reason = current.last().and_then(|last| {
            if segment.device_id != last.device_id {
                Some("device_change")
            } else if segment.envelope.start_time - last.envelope.end_time > config.inactivity {
                Some("inactivity")
            } else if segment.envelope.end_time - current[0].envelope.start_time
                > config.max_duration
            {
                Some("max_duration")
            } else {
                None
            }
        });
        if let Some(reason) = split_reason {
            windows.push(finish_window(std::mem::take(&mut current), reason));
        }
        current.push(segment);
    }
    if !current.is_empty() {
        windows.push(finish_window(current, "end_of_input"));
    }
    windows
}

/// Builds logical work windows directly from local frame/UI evidence.
///
/// This is the path used by the desktop worker before transport segments exist.
/// A final window is marked `end_of_input`; callers should defer it until its
/// last item is older than the inactivity threshold.
pub fn build_evidence_windows_from_items(
    device_id: impl Into<String>,
    mut items: Vec<SegmentEvidenceItem>,
    config: &WindowConfig,
) -> Vec<EvidenceWindow> {
    let device_id = device_id.into();
    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    let mut windows = Vec::new();
    let mut current = Vec::new();
    for item in items {
        let split_reason = current.last().and_then(|last: &SegmentEvidenceItem| {
            if item.occurred_at - last.occurred_at > config.inactivity {
                Some("inactivity")
            } else if item.occurred_at - current[0].occurred_at > config.max_duration {
                Some("max_duration")
            } else {
                None
            }
        });
        if let Some(reason) = split_reason {
            windows.push(finish_item_window(
                &device_id,
                std::mem::take(&mut current),
                reason,
            ));
        }
        current.push(item);
    }
    if !current.is_empty() {
        windows.push(finish_item_window(&device_id, current, "end_of_input"));
    }
    windows
}

fn finish_window(segments: Vec<ExportedSegment>, reason: &str) -> EvidenceWindow {
    let first = segments.first().expect("non-empty window");
    let last = segments.last().expect("non-empty window");
    let start_time = first.envelope.start_time;
    let end_time = last.envelope.end_time;
    let device_id = first.device_id.clone();
    let segment_ids = segments
        .iter()
        .map(|segment| segment.segment_id.clone())
        .collect::<Vec<_>>();
    let mut items = segments
        .into_iter()
        .flat_map(|segment| segment.envelope.items)
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    let mut hasher = Sha256::new();
    hasher.update(device_id.as_bytes());
    hasher.update(start_time.to_rfc3339().as_bytes());
    for id in &segment_ids {
        hasher.update(id.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    EvidenceWindow {
        window_id: format!("win_{}", &digest[..20]),
        device_id,
        start_time,
        end_time,
        close_reason: reason.to_string(),
        segment_ids,
        items,
    }
}

fn finish_item_window(
    device_id: &str,
    items: Vec<SegmentEvidenceItem>,
    reason: &str,
) -> EvidenceWindow {
    let start_time = items.first().expect("non-empty window").occurred_at;
    let end_time = items.last().expect("non-empty window").occurred_at;
    let mut hasher = Sha256::new();
    hasher.update(device_id.as_bytes());
    hasher.update(start_time.to_rfc3339().as_bytes());
    for item in &items {
        hasher.update(item.item_id.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    EvidenceWindow {
        window_id: format!("win_{}", &digest[..20]),
        device_id: device_id.to_string(),
        start_time,
        end_time,
        close_reason: reason.to_string(),
        segment_ids: Vec::new(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use dystil_protocol::{SegmentEnvelope, EVIDENCE_VERSION, SEGMENTER_VERSION};

    fn segment(id: usize, minute: i64) -> ExportedSegment {
        let at = Utc
            .with_ymd_and_hms(2026, 7, 17, 10, minute as u32, 0)
            .unwrap();
        let mut envelope = SegmentEnvelope {
            segment_id: format!("seg_{id}"),
            revision: 1,
            device_sequence: id as u64,
            previous_segment_id: None,
            start_time: at,
            end_time: at + Duration::seconds(30),
            closed_at: at + Duration::minutes(1),
            segmenter_version: SEGMENTER_VERSION.to_string(),
            evidence_version: EVIDENCE_VERSION.to_string(),
            content_hash: String::new(),
            token_estimate: 1,
            sync_policy_version: None,
            items: Vec::new(),
            image_refs: Vec::new(),
        };
        envelope.refresh_content_hash().unwrap();
        ExportedSegment {
            org_id: "org".into(),
            user_id: "user".into(),
            device_id: "device".into(),
            segment_id: envelope.segment_id.clone(),
            revision: 1,
            content_hash: envelope.content_hash.clone(),
            envelope,
        }
    }

    #[test]
    fn merges_transport_segments_and_splits_on_inactivity() {
        let windows = build_evidence_windows(
            vec![segment(1, 0), segment(2, 1), segment(3, 10)],
            &WindowConfig::default(),
        );
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].segment_ids, ["seg_1", "seg_2"]);
        assert_eq!(windows[0].close_reason, "inactivity");
    }

    #[test]
    fn builds_local_windows_from_items() {
        use dystil_protocol::SegmentEvidenceKind;
        let base = Utc.with_ymd_and_hms(2026, 7, 17, 10, 0, 0).unwrap();
        let items = [0, 2, 9]
            .into_iter()
            .enumerate()
            .map(|(index, minute)| SegmentEvidenceItem {
                item_id: format!("item_{index}"),
                kind: SegmentEvidenceKind::Screen,
                occurred_at: base + Duration::minutes(minute),
                source_id: index.to_string(),
                source_payload_hash: "hash".into(),
                text: "text".into(),
                app_name: None,
                window_name: None,
                browser_url: None,
                metadata: serde_json::Value::Null,
            })
            .collect();
        let windows = build_evidence_windows_from_items("device", items, &WindowConfig::default());
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].close_reason, "inactivity");
        assert_eq!(windows[1].close_reason, "end_of_input");
    }
}
