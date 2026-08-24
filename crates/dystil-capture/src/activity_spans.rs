//! Compact Stage 3 activity records. They intentionally contain no UIA tree
//! text; the linked frame remains the sole state snapshot.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::{CaptureContext, ScreenPoint};

pub async fn ensure_activity_span_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS activity_spans (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT NOT NULL,
            duration_ms INTEGER NOT NULL,
            event_count INTEGER NOT NULL,
            app_sequence_json TEXT NOT NULL DEFAULT '[]',
            final_app_name TEXT,
            final_window_title TEXT,
            final_target_json TEXT,
            scroll_delta_x INTEGER NOT NULL DEFAULT 0,
            scroll_delta_y INTEGER NOT NULL DEFAULT 0,
            final_frame_id INTEGER
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_activity_span(
    pool: &SqlitePool,
    session_id: &str,
    kind: &str,
    duration_ms: u64,
    event_count: u64,
    app_sequence: &[String],
    context: &CaptureContext,
    scroll_delta_x: i64,
    scroll_delta_y: i64,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now();
    let target = context.target.map(target_json);
    let result = sqlx::query(
        "INSERT INTO activity_spans(session_id,kind,started_at,ended_at,duration_ms,event_count,
            app_sequence_json,final_app_name,final_window_title,final_target_json,scroll_delta_x,scroll_delta_y)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
    )
    .bind(session_id)
    .bind(kind)
    .bind((now - chrono::Duration::milliseconds(duration_ms as i64)).to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(duration_ms as i64)
    .bind(event_count as i64)
    .bind(serde_json::to_string(app_sequence).unwrap_or_else(|_| "[]".into()))
    .bind(&context.application)
    .bind(&context.window)
    .bind(target)
    .bind(scroll_delta_x)
    .bind(scroll_delta_y)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn link_activity_span_frame(
    pool: &SqlitePool,
    span_id: i64,
    frame_id: i64,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "debug-capture")]
    let rss_before = crate::debug_capture::process_rss_bytes();
    #[cfg(feature = "debug-capture")]
    let started = std::time::Instant::now();
    let result = sqlx::query("UPDATE activity_spans SET final_frame_id=?1 WHERE id=?2")
        .bind(frame_id)
        .bind(span_id)
        .execute(pool)
        .await;
    #[cfg(feature = "debug-capture")]
    crate::debug_capture::record_capture_phase(
        "ui_event_activity_linking",
        "activity_span_link",
        started,
        None,
        None,
        None,
        None,
        None,
        rss_before,
        crate::debug_capture::process_rss_bytes(),
    );
    result.map(|_| ())
}

fn target_json(point: ScreenPoint) -> String {
    serde_json::json!({"x": point.x, "y": point.y}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[tokio::test]
    async fn span_schema_keeps_compact_activity_separate_from_frame_text() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        ensure_activity_span_schema(&pool).await.unwrap();
        let context = CaptureContext {
            application: Some("msedge.exe".into()),
            target: Some(ScreenPoint { x: 12, y: 34 }),
            ..CaptureContext::default()
        };
        let id = insert_activity_span(
            &pool,
            "session",
            "scroll_burst",
            2_500,
            3,
            &["msedge.exe".into()],
            &context,
            0,
            -360,
        )
        .await
        .unwrap();
        link_activity_span_frame(&pool, id, 42).await.unwrap();

        let row =
            sqlx::query("SELECT event_count, scroll_delta_y, final_frame_id FROM activity_spans")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.get::<i64, _>("event_count"), 3);
        assert_eq!(row.get::<i64, _>("scroll_delta_y"), -360);
        assert_eq!(row.get::<i64, _>("final_frame_id"), 42);
        let columns = sqlx::query("PRAGMA table_info(activity_spans)")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(columns
            .iter()
            .all(|row| row.get::<String, _>("name") != "frame_text"));
    }
}
