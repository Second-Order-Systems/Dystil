use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value};
use sqlx::{Row, SqlitePool};

use crate::event::build_event;
use crate::types::{
    CaptureEvent, CaptureEventType, DystilSync, InputEventPayload, ScreenFramePayload,
    SourceCursor, SyncConfig, SyncError,
};
use crate::utils::parse_sqlite_timestamp;

impl DystilSync {
    pub(crate) async fn read_events(
        &self,
        cursor: &SourceCursor,
        config: &SyncConfig,
    ) -> Result<Vec<CaptureEvent>, SyncError> {
        let db_url = format!("sqlite:{}?mode=ro", self.db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        let now = Utc::now();
        let floor = now - Duration::days(config.cold_start_lookback_days as i64);
        let screen_cutoff = now - Duration::seconds(config.screen_settle_lag_secs as i64);

        let screen_events = self
            .read_screen_events(
                &pool,
                cursor.screen_frame.last_id.unwrap_or_default(),
                floor,
                screen_cutoff,
                config,
            )
            .await?;
        let input_events = self
            .read_input_events(
                &pool,
                cursor.input_event.last_id.unwrap_or_default(),
                floor,
                now,
            )
            .await?;
        tracing::info!(
            screen_events = screen_events.len(),
            input_events = input_events.len(),
            "dystil-sync: stream queries completed"
        );

        let mut events = screen_events;
        events.extend(input_events);
        events.sort_by(|a, b| {
            a.occurred_at
                .cmp(&b.occurred_at)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        Ok(events)
    }

    pub(crate) async fn read_screen_events(
        &self,
        pool: &SqlitePool,
        last_id: i64,
        floor: DateTime<Utc>,
        until: DateTime<Utc>,
        _config: &SyncConfig,
    ) -> Result<Vec<CaptureEvent>, SyncError> {
        let rows = sqlx::query(
            "SELECT id, timestamp, app_name, window_name, browser_url, document_path, focused,
                    device_name, capture_trigger, text_source, frame_text,
                    content_hash, simhash
             FROM frames
             WHERE id > ?1
               AND datetime(timestamp) >= datetime(?2)
               AND datetime(timestamp) <= datetime(?3)
             ORDER BY datetime(timestamp) ASC, id ASC",
        )
        .bind(last_id)
        .bind(floor.to_rfc3339())
        .bind(until.to_rfc3339())
        .fetch_all(pool)
        .await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let frame_id: i64 = row.try_get("id")?;
            let occurred_at =
                parse_sqlite_timestamp(row.try_get::<String, _>("timestamp")?.as_str());
            let payload = ScreenFramePayload {
                frame_id,
                app_name: row.try_get("app_name")?,
                window_name: row.try_get("window_name")?,
                browser_url: row.try_get("browser_url")?,
                document_path: row.try_get("document_path")?,
                focused: row.try_get("focused")?,
                device_name: row.try_get("device_name")?,
                capture_trigger: row.try_get("capture_trigger")?,
                text_source: row.try_get("text_source")?,
                frame_text: row.try_get("frame_text")?,
                content_hash: row
                    .try_get::<Option<i64>, _>("content_hash")?
                    .map(|value| value.to_string()),
                simhash: row
                    .try_get::<Option<i64>, _>("simhash")?
                    .map(|value| value.to_string()),
            };
            events.push(build_event(
                CaptureEventType::ScreenFrame,
                occurred_at,
                "frames",
                frame_id,
                &payload,
            )?);
        }
        Ok(events)
    }

    pub(crate) async fn read_input_events(
        &self,
        pool: &SqlitePool,
        last_id: i64,
        floor: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<CaptureEvent>, SyncError> {
        let rows = sqlx::query(
            "SELECT id, timestamp, session_id, relative_ms, event_type, x, y, delta_x, delta_y,
                    button, click_count, key_code, modifiers, text_content, app_name, app_pid,
                    window_title, browser_url, frame_id, element_role, element_name, element_value,
                    element_description, element_automation_id, element_bounds
             FROM ui_events
             WHERE id > ?1
               AND datetime(timestamp) >= datetime(?2)
               AND datetime(timestamp) <= datetime(?3)
             ORDER BY datetime(timestamp) ASC, id ASC",
        )
        .bind(last_id)
        .bind(floor.to_rfc3339())
        .bind(until.to_rfc3339())
        .fetch_all(pool)
        .await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let payload = InputEventPayload {
                ui_event_id: row.try_get("id")?,
                session_id: row.try_get("session_id")?,
                relative_ms: row.try_get::<i64, _>("relative_ms").unwrap_or_default(),
                event_type_detail: row.try_get("event_type")?,
                x: row.try_get("x")?,
                y: row.try_get("y")?,
                delta_x: row.try_get("delta_x")?,
                delta_y: row.try_get("delta_y")?,
                button: row.try_get("button")?,
                click_count: row.try_get("click_count")?,
                key_code: row.try_get("key_code")?,
                modifiers: row.try_get("modifiers")?,
                text_content: row.try_get("text_content")?,
                app_name: row.try_get("app_name")?,
                app_pid: row.try_get("app_pid")?,
                window_title: row.try_get("window_title")?,
                browser_url: row.try_get("browser_url")?,
                frame_id: row.try_get("frame_id")?,
                element: build_element_context(&row)?,
            };
            let occurred_at =
                parse_sqlite_timestamp(row.try_get::<String, _>("timestamp")?.as_str());
            events.push(build_event(
                CaptureEventType::InputEvent,
                occurred_at,
                "ui_events",
                payload.ui_event_id,
                &payload,
            )?);
        }
        Ok(events)
    }
}

pub(crate) fn build_element_context(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Option<Value>, SyncError> {
    let mut map = Map::new();
    for (input, output) in [
        ("element_role", "role"),
        ("element_name", "name"),
        ("element_value", "value"),
        ("element_description", "description"),
        ("element_automation_id", "automation_id"),
    ] {
        if let Some(value) = row.try_get::<Option<String>, _>(input)? {
            map.insert(output.to_string(), Value::String(value));
        }
    }
    if let Some(bounds) = row.try_get::<Option<String>, _>("element_bounds")? {
        if let Ok(value) = serde_json::from_str::<Value>(&bounds) {
            map.insert("bounds".to_string(), value);
        }
    }
    Ok((!map.is_empty()).then_some(Value::Object(map)))
}
