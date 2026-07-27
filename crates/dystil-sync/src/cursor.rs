use chrono::{DateTime, Duration, Utc};

use crate::types::{CaptureEvent, CaptureEventType, SourceCursor, StreamCursor, SyncConfig};

pub(crate) fn resolved_cursor(
    cursor: SourceCursor,
    config: &SyncConfig,
    now: DateTime<Utc>,
) -> SourceCursor {
    let floor = now - Duration::days(config.cold_start_lookback_days as i64);
    SourceCursor {
        screen_frame: with_floor(cursor.screen_frame, floor),
        input_event: with_floor(cursor.input_event, floor),
    }
}

pub(crate) fn with_floor(cursor: StreamCursor, floor: DateTime<Utc>) -> StreamCursor {
    let last_timestamp = cursor
        .last_timestamp
        .map(|ts| ts.max(floor))
        .or(Some(floor));
    StreamCursor {
        last_timestamp,
        last_id: cursor.last_id,
    }
}

pub(crate) fn advance_cursor(cursor: &mut SourceCursor, event: &CaptureEvent) {
    let target = match event.event_type {
        CaptureEventType::ScreenFrame => &mut cursor.screen_frame,
        CaptureEventType::InputEvent => &mut cursor.input_event,
    };
    target.last_timestamp = Some(
        target
            .last_timestamp
            .map(|timestamp| timestamp.max(event.occurred_at))
            .unwrap_or(event.occurred_at),
    );
    target.last_id = Some(
        target
            .last_id
            .map(|source_id| source_id.max(event.source_id))
            .unwrap_or(event.source_id),
    );
}

pub(crate) fn recompute_cursor(
    cursor_before: &SourceCursor,
    events: &[CaptureEvent],
) -> SourceCursor {
    let mut cursor = cursor_before.clone();
    for event in events {
        advance_cursor(&mut cursor, event);
    }
    cursor
}
