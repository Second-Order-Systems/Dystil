use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
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
                    content_hash, simhash, ax_capture_diagnostics_json
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
                ax_capture_diagnostics: parse_optional_frame_json(
                    frame_id,
                    "ax_capture_diagnostics_json",
                    row.try_get("ax_capture_diagnostics_json")?,
                ),
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
            "SELECT id, timestamp, session_id, relative_ms, event_type,
                    CAST(x AS INTEGER) AS x,
                    CAST(y AS INTEGER) AS y,
                    CAST(delta_x AS INTEGER) AS delta_x,
                    CAST(delta_y AS INTEGER) AS delta_y,
                    CAST(button AS INTEGER) AS button,
                    CAST(click_count AS INTEGER) AS click_count,
                    CAST(key_code AS INTEGER) AS key_code,
                    CAST(modifiers AS INTEGER) AS modifiers,
                    text_content, app_name, app_pid,
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

fn parse_optional_frame_json(frame_id: i64, column: &str, raw: Option<String>) -> Option<Value> {
    let raw = raw?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str(&raw) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                frame_id,
                column,
                error = %error,
                "dystil-sync: omitting malformed historical frame JSON"
            );
            None
        }
    }
}

pub(crate) async fn clear_frame_structural_json(
    db_path: &std::path::Path,
    frame_ids: &[i64],
) -> Result<u64, SyncError> {
    let frame_ids = frame_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if frame_ids.is_empty() {
        return Ok(0);
    }

    let options = SqliteConnectOptions::new().filename(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let mut tx = pool.begin().await?;
    let mut cleared = 0;
    for frame_id in frame_ids {
        cleared += sqlx::query(
            "UPDATE frames
             SET accessibility_tree_json = NULL,
                 ax_capture_diagnostics_json = NULL
             WHERE id = ?1",
        )
        .bind(frame_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(cleared)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_sync(db_path: std::path::PathBuf) -> DystilSync {
        DystilSync {
            state_db_path: db_path.with_extension("state.sqlite"),
            db_path,
            cloud_base_url: "https://example.invalid".to_string(),
            device_token: "test-device-token".to_string(),
            machine_id: "test-machine".to_string(),
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
        }
    }

    #[tokio::test]
    async fn reads_numeric_input_fields_from_legacy_real_and_text_columns() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("capture.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE ui_events (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                session_id TEXT,
                relative_ms INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                x REAL, y REAL, delta_x REAL, delta_y REAL,
                button TEXT, click_count INTEGER, key_code TEXT, modifiers TEXT,
                text_content TEXT, app_name TEXT, app_pid INTEGER,
                window_title TEXT, browser_url TEXT, frame_id INTEGER,
                element_role TEXT, element_name TEXT, element_value TEXT,
                element_description TEXT, element_automation_id TEXT, element_bounds TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ui_events (
                id, timestamp, session_id, relative_ms, event_type,
                x, y, delta_x, delta_y, button, click_count, key_code, modifiers
             ) VALUES (
                1, '2026-08-05T02:22:00Z', 'session-1', 42, 'click',
                21.0, 457.0, -2.0, 4.0, '1', 2, '53', '8'
             )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let events = test_sync(db_path)
            .read_input_events(
                &pool,
                0,
                DateTime::parse_from_rfc3339("2026-08-05T02:21:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339("2026-08-05T02:23:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        let payload: InputEventPayload = serde_json::from_value(events[0].payload.clone()).unwrap();
        assert_eq!(payload.x, Some(21));
        assert_eq!(payload.y, Some(457));
        assert_eq!(payload.delta_x, Some(-2));
        assert_eq!(payload.delta_y, Some(4));
        assert_eq!(payload.button, Some(1));
        assert_eq!(payload.click_count, Some(2));
        assert_eq!(payload.key_code, Some(53));
        assert_eq!(payload.modifiers, Some(8));
    }

    #[tokio::test]
    async fn clears_only_acknowledged_structural_json_and_keeps_frame_text() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("capture.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE frames (
                id INTEGER PRIMARY KEY,
                frame_text TEXT,
                accessibility_tree_json TEXT,
                ax_capture_diagnostics_json TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for id in [1_i64, 2] {
            sqlx::query(
                "INSERT INTO frames VALUES (?1, 'visible text', '{\"role\":\"window\"}', '{\"source\":\"ax\"}')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;

        assert_eq!(
            clear_frame_structural_json(&db_path, &[1]).await.unwrap(),
            1
        );

        let pool = SqlitePool::connect(&format!("sqlite:{}?mode=ro", db_path.display()))
            .await
            .unwrap();
        let rows = sqlx::query(
            "SELECT id, frame_text, accessibility_tree_json, ax_capture_diagnostics_json
             FROM frames ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows[0].try_get::<String, _>("frame_text").unwrap(),
            "visible text"
        );
        assert!(rows[0]
            .try_get::<Option<String>, _>("accessibility_tree_json")
            .unwrap()
            .is_none());
        assert!(rows[0]
            .try_get::<Option<String>, _>("ax_capture_diagnostics_json")
            .unwrap()
            .is_none());
        assert!(rows[1]
            .try_get::<Option<String>, _>("accessibility_tree_json")
            .unwrap()
            .is_some());
    }
}
