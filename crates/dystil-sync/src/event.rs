use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::types::{CaptureEvent, CaptureEventType, SyncError};
use crate::utils::sha256_hex;

pub(crate) fn build_event<T: Serialize>(
    event_type: CaptureEventType,
    occurred_at: DateTime<Utc>,
    source_table: &str,
    source_id: i64,
    payload: &T,
) -> Result<CaptureEvent, SyncError> {
    let payload_value = serde_json::to_value(payload)?;
    let payload_hash = format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(&payload_value)?)
    );
    Ok(CaptureEvent {
        event_id: format!("{}:{}", event_type.as_str(), source_id),
        event_type,
        occurred_at,
        source_table: source_table.to_string(),
        source_id,
        payload_hash,
        payload: payload_value,
    })
}
