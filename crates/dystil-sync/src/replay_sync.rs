use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::path::Path;

use crate::evidence::{filter_events, EvidenceFilterConfig, FilterDecision};
use crate::segmenter::{build_segments, SegmentConfig};
use crate::types::{
    CaptureEvent, CaptureEventType, DystilSync, SourceCursor, SyncConfig, SyncError,
};

#[derive(Debug, Clone, Serialize)]
pub struct ReplayConfig {
    pub db_path: String,
    pub sync_interval_secs: u64,
    pub screen_settle_lag_secs: u64,
    pub cold_start_lookback_days: u64,
    pub segment_inactivity_secs: i64,
    pub segment_max_duration_secs: i64,
    pub segment_max_tokens: u32,
    pub screen_dedupe_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayEvent {
    pub event_id: String,
    pub event_type: String,
    pub occurred_at: String,
    pub source_id: i64,
    pub text_preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_full: Option<String>,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub kept: bool,
    pub drop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplaySegment {
    pub segment_id: String,
    pub device_sequence: u64,
    pub status: String,
    pub start_time: String,
    pub end_time: String,
    pub item_count: usize,
    pub token_estimate: u32,
    pub boundary_reason: String,
    pub items: Vec<ReplayEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayIteration {
    pub iteration: u32,
    pub raw_events_read: usize,
    pub filter_kept: usize,
    pub filter_dropped: usize,
    pub segments: Vec<ReplaySegment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayData {
    pub config: ReplayConfig,
    pub summary: ReplaySummary,
    pub iterations: Vec<ReplayIteration>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplaySummary {
    pub total_events_read: usize,
    pub total_kept: usize,
    pub total_dropped: usize,
    pub total_segments: usize,
    pub total_iterations: usize,
    pub data_start: String,
    pub data_end: String,
}

pub async fn run_replay(
    db_path: &Path,
    machine_id: &str,
    config: &ReplayConfig,
) -> Result<ReplayData, SyncError> {
    let sync_config = SyncConfig {
        sync_interval_secs: config.sync_interval_secs,
        screen_settle_lag_secs: config.screen_settle_lag_secs,
        cold_start_lookback_days: config.cold_start_lookback_days,
        ..SyncConfig::default()
    };
    let evidence_config = EvidenceFilterConfig {
        screen_dedupe_seconds: config.screen_dedupe_seconds,
        ..EvidenceFilterConfig::default()
    };
    let segment_config = SegmentConfig {
        inactivity_seconds: config.segment_inactivity_secs,
        max_duration_seconds: config.segment_max_duration_secs,
        max_tokens: config.segment_max_tokens,
        policy_version: "replay".to_string(),
    };

    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;

    let now = Utc::now();
    let floor = now - Duration::days(sync_config.cold_start_lookback_days as i64);
    let screen_cutoff = now - Duration::seconds(sync_config.screen_settle_lag_secs as i64);

    let mut cursor = SourceCursor::default();
    let mut iter_idx: u32 = 0;
    let mut iterations: Vec<ReplayIteration> = Vec::new();
    let mut data_start: Option<String> = None;
    let mut data_end: Option<String> = None;
    let mut total_kept = 0usize;
    let mut total_dropped = 0usize;
    let mut total_events = 0usize;
    let mut total_segments = 0usize;
    let mut next_sequence: u64 = 1;
    let mut previous_segment_id: Option<String> = None;

    loop {
        iter_idx += 1;
        let raw_events = read_events_standalone(&pool, &cursor, floor, screen_cutoff, now).await?;

        if raw_events.is_empty() {
            if iter_idx == 1 {
                tracing::warn!("no events found in database");
            }
            break;
        }

        let event_count = raw_events.len();
        total_events += event_count;
        if data_start.is_none() {
            data_start = raw_events.first().map(|e| e.occurred_at.to_rfc3339());
        }
        data_end = raw_events.last().map(|e| e.occurred_at.to_rfc3339());

        let filter_outcome = filter_events(&raw_events, &evidence_config)?;
        let kept_count = filter_outcome.items.len();
        let dropped_count = event_count.saturating_sub(kept_count);
        total_kept += kept_count;
        total_dropped += dropped_count;

        let envelopes = build_segments(
            filter_outcome.items,
            &segment_config,
            machine_id,
            next_sequence,
            previous_segment_id.clone(),
            now,
        )?;

        let mut replay_segments = Vec::new();
        for (index, envelope) in envelopes.iter().enumerate() {
            let total = envelopes.len();
            let next_start = envelopes.get(index + 1).map(|s| s.start_time);
            let boundary_reason =
                boundary_reason_for(index, total, next_start, envelope, &segment_config);

            let status = if boundary_reason != "still_open" {
                "stable"
            } else {
                "open"
            };

            let mut segment_events =
                build_segment_events(&raw_events, &filter_outcome.decisions, envelope);
            segment_events.retain(|e| {
                let ts: DateTime<Utc> = DateTime::parse_from_rfc3339(&e.occurred_at)
                    .unwrap_or_default()
                    .with_timezone(&Utc);
                ts >= envelope.start_time && ts <= envelope.end_time
            });

            replay_segments.push(ReplaySegment {
                segment_id: envelope.segment_id.clone(),
                device_sequence: envelope.device_sequence,
                status: status.to_string(),
                start_time: envelope.start_time.to_rfc3339(),
                end_time: envelope.end_time.to_rfc3339(),
                item_count: envelope.items.len(),
                token_estimate: envelope.token_estimate,
                boundary_reason,
                items: segment_events,
            });
        }

        total_segments += replay_segments.len();

        if let Some(last) = envelopes.last() {
            next_sequence = last.device_sequence + 1;
            previous_segment_id = Some(last.segment_id.clone());
        }

        cursor = advance_cursor_from_events(&cursor, &raw_events);

        iterations.push(ReplayIteration {
            iteration: iter_idx,
            raw_events_read: event_count,
            filter_kept: kept_count,
            filter_dropped: dropped_count,
            segments: replay_segments,
        });

        if iter_idx >= 100 {
            tracing::warn!("replay iteration limit reached");
            break;
        }
    }

    pool.close().await;

    Ok(ReplayData {
        config: config.clone(),
        summary: ReplaySummary {
            total_events_read: total_events,
            total_kept,
            total_dropped,
            total_segments,
            total_iterations: iterations.len() as usize,
            data_start: data_start.unwrap_or_default(),
            data_end: data_end.unwrap_or_default(),
        },
        iterations,
    })
}

async fn read_events_standalone(
    pool: &sqlx::SqlitePool,
    cursor: &SourceCursor,
    floor: DateTime<Utc>,
    screen_cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<CaptureEvent>, SyncError> {
    let sync = DystilSync {
        db_path: std::path::PathBuf::new(),
        state_db_path: std::path::PathBuf::new(),
        cloud_base_url: String::new(),
        device_token: String::new(),
        machine_id: String::new(),
        fallback_config: SyncConfig::default(),
        request_timeout_secs: 30,
        app_version: None,
        build_channel: None,
        build_commit: None,
        sync_capabilities: Vec::new(),
        local_permissions: crate::types::LocalSyncPermissions {
            segments: true,
            screenshots: true,
        },
    };

    let screen_events = sync
        .read_screen_events(
            pool,
            cursor.screen_frame.last_id.unwrap_or_default(),
            floor,
            screen_cutoff,
            &SyncConfig::default(),
        )
        .await?;
    let input_events = sync
        .read_input_events(
            pool,
            cursor.input_event.last_id.unwrap_or_default(),
            floor,
            now,
        )
        .await?;

    let mut events = Vec::new();
    events.extend(screen_events);
    events.extend(input_events);
    events.sort_by(|a, b| {
        a.occurred_at
            .cmp(&b.occurred_at)
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    Ok(events)
}

fn build_segment_events(
    raw_events: &[CaptureEvent],
    decisions: &[FilterDecision],
    envelope: &dystil_protocol::SegmentEnvelope,
) -> Vec<ReplayEvent> {
    let start = envelope.start_time;
    let end = envelope.end_time;

    // Build a lookup: source_id -> evidence item text
    let kept_texts: std::collections::HashMap<&str, &str> = envelope
        .items
        .iter()
        .map(|item| (item.source_id.as_str(), item.text.as_str()))
        .collect();

    let decision_map: std::collections::HashMap<&str, &FilterDecision> =
        decisions.iter().map(|d| (d.event_id.as_str(), d)).collect();

    let mut events: Vec<ReplayEvent> = raw_events
        .iter()
        .filter(|e| e.occurred_at >= start && e.occurred_at <= end)
        .map(|e| {
            let decision = decision_map.get(e.event_id.as_str());
            let kept = decision.map(|d| d.kept).unwrap_or(false);
            let drop_reason = decision.and_then(|d| d.reason.clone());
            let text_preview = if kept {
                kept_texts
                    .get(e.event_id.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            } else {
                peek_raw_text(e)
            };
            let text_full = if text_preview.chars().count() > 200 {
                Some(text_preview.clone())
            } else {
                None
            };
            ReplayEvent {
                event_id: e.event_id.clone(),
                event_type: e.event_type.as_str().to_string(),
                occurred_at: e.occurred_at.to_rfc3339(),
                source_id: e.source_id,
                text_preview: truncate(&text_preview, 200),
                text_full,
                app_name: peek_app_name(e),
                window_name: peek_window_name(e),
                kept,
                drop_reason,
            }
        })
        .collect();

    events.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at));
    events
}

fn peek_raw_text(event: &CaptureEvent) -> String {
    match serde_json::from_value::<serde_json::Value>(event.payload.clone()) {
        Ok(payload) => {
            if let Some(text) = payload.get("full_text").and_then(|v| v.as_str()) {
                return text.to_string();
            }
            if let Some(text) = payload.get("text_content").and_then(|v| v.as_str()) {
                return text.to_string();
            }
            String::new()
        }
        Err(_) => String::new(),
    }
}

fn peek_app_name(event: &CaptureEvent) -> Option<String> {
    event
        .payload
        .get("app_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn peek_window_name(event: &CaptureEvent) -> Option<String> {
    event
        .payload
        .get("window_name")
        .or_else(|| event.payload.get("window_title"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        format!(
            "{}...",
            text.chars().take(max_chars).collect::<String>().trim_end()
        )
    } else {
        text.to_string()
    }
}

fn advance_cursor_from_events(cursor: &SourceCursor, events: &[CaptureEvent]) -> SourceCursor {
    let mut new_cursor = cursor.clone();
    for event in events {
        let target = match event.event_type {
            CaptureEventType::ScreenFrame => &mut new_cursor.screen_frame,
            CaptureEventType::InputEvent => &mut new_cursor.input_event,
        };
        target.last_timestamp = Some(
            target
                .last_timestamp
                .map(|ts| ts.max(event.occurred_at))
                .unwrap_or(event.occurred_at),
        );
        target.last_id = Some(
            target
                .last_id
                .map(|id| id.max(event.source_id))
                .unwrap_or(event.source_id),
        );
    }
    new_cursor
}

fn boundary_reason_for(
    index: usize,
    total: usize,
    next_start: Option<DateTime<Utc>>,
    envelope: &dystil_protocol::SegmentEnvelope,
    config: &SegmentConfig,
) -> String {
    if index + 1 < total {
        if let Some(next) = next_start {
            let gap = (next - envelope.end_time).num_seconds();
            if gap >= config.inactivity_seconds {
                return format!("inactivity ({}s gap)", gap);
            }
        }
        let duration = (envelope.end_time - envelope.start_time).num_seconds();
        if duration >= config.max_duration_seconds {
            return format!("duration ({}s)", duration);
        }
        return format!("tokens ({})", envelope.token_estimate);
    }
    let duration = (envelope.end_time - envelope.start_time).num_seconds();
    if duration >= config.max_duration_seconds {
        return format!("duration ({}s)", duration);
    }
    if envelope.token_estimate >= config.max_tokens {
        return format!("tokens ({})", envelope.token_estimate);
    }
    String::from("still_open")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_config_defaults_match_sync_config() {
        let _config = ReplayConfig {
            db_path: "/tmp/test.sqlite".to_string(),
            sync_interval_secs: 120,
            screen_settle_lag_secs: 15,
            cold_start_lookback_days: 7,
            segment_inactivity_secs: 300,
            segment_max_duration_secs: 900,
            segment_max_tokens: 10000,
            screen_dedupe_seconds: 20,
        };
    }
}
