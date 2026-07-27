use chrono::{DateTime, Duration, Utc};
use dystil_protocol::{SegmentEvidenceItem, SegmentEvidenceKind, EVIDENCE_VERSION};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::types::{
    CaptureEvent, CaptureEventType, InputEventPayload, ScreenFramePayload, SyncError,
};

#[derive(Debug, Clone)]
pub struct EvidenceFilterConfig {
    pub screen_dedupe_seconds: i64,
}

impl Default for EvidenceFilterConfig {
    fn default() -> Self {
        Self {
            screen_dedupe_seconds: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FilterDecision {
    pub event_id: String,
    pub kept: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FilterOutcome {
    pub items: Vec<SegmentEvidenceItem>,
    pub decisions: Vec<FilterDecision>,
}

pub(crate) fn filter_events(
    events: &[CaptureEvent],
    config: &EvidenceFilterConfig,
) -> Result<FilterOutcome, SyncError> {
    let mut items = Vec::new();
    let mut decisions = Vec::new();
    let mut last_screen: Option<(String, DateTime<Utc>)> = None;

    for event in events {
        let (candidate, reason) = match event.event_type {
            CaptureEventType::ScreenFrame => screen_item(event, config, &mut last_screen)?,
            CaptureEventType::InputEvent => input_item(event)?,
        };
        let kept = candidate.is_some();
        decisions.push(FilterDecision {
            event_id: event.event_id.clone(),
            kept,
            reason: if kept { None } else { Some(reason) },
        });
        if let Some(item) = candidate {
            items.push(sanitize_evidence_item(item));
        }
    }

    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    Ok(FilterOutcome { items, decisions })
}

fn screen_item(
    event: &CaptureEvent,
    config: &EvidenceFilterConfig,
    last_screen: &mut Option<(String, DateTime<Utc>)>,
) -> Result<(Option<SegmentEvidenceItem>, String), SyncError> {
    let payload: ScreenFramePayload = serde_json::from_value(event.payload.clone())?;
    let text = normalize_text(payload.frame_text.as_deref().unwrap_or_default());
    if text.is_empty() {
        return Ok((None, "empty text".to_string()));
    }
    if let Some((prev_text, prev_time)) = last_screen.as_ref() {
        if prev_text == &text
            && event.occurred_at - *prev_time <= Duration::seconds(config.screen_dedupe_seconds)
        {
            let secs = (event.occurred_at - *prev_time).num_seconds();
            return Ok((
                None,
                format!(
                    "duplicate text within {}s (same as frame at {})",
                    secs,
                    prev_time.format("%H:%M:%S")
                ),
            ));
        }
    }
    *last_screen = Some((text.clone(), event.occurred_at));

    Ok((
        Some(SegmentEvidenceItem {
            item_id: evidence_item_id(event),
            kind: SegmentEvidenceKind::Screen,
            occurred_at: event.occurred_at,
            source_id: event.event_id.clone(),
            source_payload_hash: event.payload_hash.clone(),
            text,
            app_name: payload.app_name,
            window_name: payload.window_name,
            browser_url: payload.browser_url,
            metadata: json!({
                "frame_id": payload.frame_id,
                "document_path": payload.document_path,
                "text_source": payload.text_source,
                "capture_trigger": payload.capture_trigger,
                "content_hash": payload.content_hash,
                "simhash": payload.simhash,
                "focused": payload.focused,
            }),
        }),
        String::new(),
    ))
}

fn input_item(event: &CaptureEvent) -> Result<(Option<SegmentEvidenceItem>, String), SyncError> {
    let payload: InputEventPayload = serde_json::from_value(event.payload.clone())?;
    let event_type = payload.event_type_detail.trim().to_lowercase();
    let entered_text = normalize_text(payload.text_content.as_deref().unwrap_or_default());
    if should_skip_input(&event_type, &entered_text) {
        return Ok((None, format!("noisy event ({} without text)", event_type)));
    }

    let element_label = payload
        .element
        .as_ref()
        .and_then(element_label_from_value)
        .unwrap_or_default();
    let text = if !entered_text.is_empty() {
        format!("{}: {}", coarse_input_name(&event_type), entered_text)
    } else if !element_label.is_empty() {
        format!("{}: {}", coarse_input_name(&event_type), element_label)
    } else {
        coarse_input_name(&event_type).to_string()
    };

    Ok((
        Some(SegmentEvidenceItem {
            item_id: evidence_item_id(event),
            kind: SegmentEvidenceKind::Input,
            occurred_at: event.occurred_at,
            source_id: event.event_id.clone(),
            source_payload_hash: event.payload_hash.clone(),
            text,
            app_name: payload.app_name,
            window_name: payload.window_title,
            browser_url: payload.browser_url,
            metadata: json!({
                "ui_event_id": payload.ui_event_id,
                "session_id": payload.session_id,
                "event_type": payload.event_type_detail,
                "frame_id": payload.frame_id,
                "element": payload.element,
            }),
        }),
        String::new(),
    ))
}

fn should_skip_input(event_type: &str, text: &str) -> bool {
    if !text.is_empty() {
        return false;
    }
    let noisy = [
        "move", "hover", "scroll", "key", "key_down", "key_up", "keydown", "keyup",
    ];
    noisy.iter().any(|value| event_type.contains(value))
}

fn coarse_input_name(event_type: &str) -> &str {
    if event_type.contains("click") {
        "clicked"
    } else if event_type.contains("paste") {
        "pasted text"
    } else if event_type.contains("copy") {
        "copied content"
    } else if event_type.contains("submit") {
        "submitted"
    } else if event_type.contains("select") {
        "selected"
    } else if event_type.contains("key") || event_type.contains("text") {
        "entered text"
    } else {
        "performed action"
    }
}

fn element_label_from_value(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["name", "value", "description", "role"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(normalize_text)
        .find(|value| !value.is_empty())
}

pub fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// PostgreSQL JSONB rejects the NUL character (U+0000). Capture data comes
/// from external OCR and accessibility APIs, so clean every string that is
/// retained in an evidence item before it is persisted and hashed.
fn sanitize_evidence_item(mut item: SegmentEvidenceItem) -> SegmentEvidenceItem {
    item.item_id = strip_nuls(item.item_id);
    item.source_id = strip_nuls(item.source_id);
    item.source_payload_hash = strip_nuls(item.source_payload_hash);
    item.text = strip_nuls(item.text);
    item.app_name = item.app_name.map(strip_nuls);
    item.window_name = item.window_name.map(strip_nuls);
    item.browser_url = item.browser_url.map(strip_nuls);
    sanitize_json_nuls(&mut item.metadata);
    item
}

fn strip_nuls(value: String) -> String {
    value.replace('\0', "")
}

fn sanitize_json_nuls(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                sanitize_json_nuls(value);
            }
        }
        Value::Object(values) => {
            let original = std::mem::take(values);
            for (key, mut value) in original {
                sanitize_json_nuls(&mut value);
                values.insert(strip_nuls(key), value);
            }
        }
        Value::String(value) => *value = strip_nuls(std::mem::take(value)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn evidence_item_id(event: &CaptureEvent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event.event_id.as_bytes());
    hasher.update(b"|");
    hasher.update(event.payload_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(EVIDENCE_VERSION.as_bytes());
    format!("item_{}", &hex::encode(hasher.finalize())[..24])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event_type: CaptureEventType, id: i64, payload: Value) -> CaptureEvent {
        CaptureEvent {
            event_id: format!("{}:{id}", event_type.as_str()),
            event_type,
            occurred_at: Utc::now() + Duration::seconds(id),
            source_table: "test".to_string(),
            source_id: id,
            payload_hash: format!("sha256:{:064x}", id),
            payload,
        }
    }

    fn screen_payload(text: &str) -> Value {
        serde_json::to_value(ScreenFramePayload {
            frame_id: 1,
            app_name: Some("Editor".to_string()),
            window_name: Some("main.rs".to_string()),
            browser_url: None,
            document_path: None,
            focused: Some(true),
            device_name: None,
            capture_trigger: Some("input_event".to_string()),
            text_source: Some("accessibility".to_string()),
            frame_text: Some(text.to_string()),
            content_hash: None,
            simhash: None,
        })
        .unwrap()
    }

    #[test]
    fn screen_filter_keeps_full_text_once_and_dedupes_adjacent_frames() {
        let events = vec![
            event(
                CaptureEventType::ScreenFrame,
                1,
                screen_payload("hello   world"),
            ),
            event(
                CaptureEventType::ScreenFrame,
                2,
                screen_payload("hello world"),
            ),
        ];
        let outcome = filter_events(&events, &EvidenceFilterConfig::default()).unwrap();
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].text, "hello world");
        assert_eq!(outcome.items[0].metadata["capture_trigger"], "input_event");
        assert_eq!(outcome.items[0].metadata["text_source"], "accessibility");
        assert!(outcome.items[0].metadata.get("elements").is_none());
        assert!(outcome.items[0].metadata.get("ocr_detail").is_none());
        assert_eq!(outcome.decisions.len(), 2);
        assert!(outcome.decisions[0].kept);
        assert!(outcome.decisions[0].reason.is_none());
        assert!(!outcome.decisions[1].kept);
        assert!(outcome.decisions[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("duplicate"));
    }

    #[test]
    fn input_filter_drops_unlabelled_scroll_and_keeps_click() {
        let scroll = serde_json::to_value(InputEventPayload {
            ui_event_id: 1,
            session_id: None,
            relative_ms: 0,
            event_type_detail: "scroll".to_string(),
            x: None,
            y: None,
            delta_x: None,
            delta_y: Some(4),
            button: None,
            click_count: None,
            key_code: None,
            modifiers: None,
            text_content: None,
            app_name: None,
            app_pid: None,
            window_title: None,
            browser_url: None,
            frame_id: None,
            element: None,
        })
        .unwrap();
        let click = serde_json::to_value(InputEventPayload {
            ui_event_id: 2,
            event_type_detail: "click".to_string(),
            element: Some(json!({"name": "Save"})),
            ..serde_json::from_value(scroll.clone()).unwrap()
        })
        .unwrap();
        let outcome = filter_events(
            &[
                event(CaptureEventType::InputEvent, 1, scroll),
                event(CaptureEventType::InputEvent, 2, click),
            ],
            &EvidenceFilterConfig::default(),
        )
        .unwrap();
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].text, "clicked: Save");
        assert_eq!(outcome.decisions.len(), 2);
        assert!(!outcome.decisions[0].kept);
        assert!(outcome.decisions[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("scroll"));
        assert!(outcome.decisions[1].kept);
        assert!(outcome.decisions[1].reason.is_none());
    }

    #[test]
    fn input_filter_drops_empty_raw_key_but_keeps_entered_text() {
        let raw_key = serde_json::to_value(InputEventPayload {
            ui_event_id: 1,
            session_id: None,
            relative_ms: 0,
            event_type_detail: "key".to_string(),
            x: None,
            y: None,
            delta_x: None,
            delta_y: None,
            button: None,
            click_count: None,
            key_code: Some(53),
            modifiers: None,
            text_content: None,
            app_name: Some("Editor".to_string()),
            app_pid: None,
            window_title: None,
            browser_url: None,
            frame_id: None,
            element: None,
        })
        .unwrap();
        let text = serde_json::to_value(InputEventPayload {
            ui_event_id: 2,
            event_type_detail: "text".to_string(),
            text_content: Some("hello".to_string()),
            ..serde_json::from_value(raw_key.clone()).unwrap()
        })
        .unwrap();

        let outcome = filter_events(
            &[
                event(CaptureEventType::InputEvent, 1, raw_key),
                event(CaptureEventType::InputEvent, 2, text),
            ],
            &EvidenceFilterConfig::default(),
        )
        .unwrap();

        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].text, "entered text: hello");
        assert!(!outcome.decisions[0].kept);
        assert!(outcome.decisions[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("key"));
        assert!(outcome.decisions[1].kept);
    }

    #[test]
    fn sanitizes_nuls_from_item_fields_and_nested_metadata() {
        let item = sanitize_evidence_item(SegmentEvidenceItem {
            item_id: "item\0id".to_string(),
            kind: SegmentEvidenceKind::Screen,
            occurred_at: Utc::now(),
            source_id: "source\0id".to_string(),
            source_payload_hash: "sha256:\0hash".to_string(),
            text: "visible\0 text".to_string(),
            app_name: Some("App\0Name".to_string()),
            window_name: Some("Window\0Name".to_string()),
            browser_url: Some("https://example.test/\0path".to_string()),
            metadata: json!({
                "key\0name": "value\0text",
                "nested": ["array\0value", { "inner": "more\0text" }],
            }),
        });

        let encoded = serde_json::to_string(&item).unwrap();
        assert!(!encoded.contains("\\u0000"));
        assert_eq!(item.item_id, "itemid");
        assert_eq!(item.metadata["keyname"], "valuetext");
        assert_eq!(item.metadata["nested"][0], "arrayvalue");
        assert_eq!(item.metadata["nested"][1]["inner"], "moretext");
    }
}
