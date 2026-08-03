use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{ActivityRecord, StorageError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameObservation {
    pub timestamp: String,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityHealthRaw {
    pub last_frame_at: Option<String>,
    pub last_event_at: Option<String>,
    pub redaction_backlog: u64,
    pub search_documents: u64,
    pub fts_documents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityOverviewRaw {
    pub frames: Vec<FrameObservation>,
    pub event_count: u64,
    pub samples: Vec<ActivityRecord>,
    pub health: ActivityHealthRaw,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureSourceUsageRaw {
    pub app_name: String,
    pub browser_url: Option<String>,
    pub active_seconds: f64,
    pub observations: u64,
}

/// Aggregate every app/browser URL observed in a time range for settings.
///
/// Unlike the retrieval overview, this intentionally has no result limit: a
/// privacy inventory must not hide less-active sources. Active time uses the
/// same rule as the overview (the gap to the next frame, capped at five
/// minutes), but the work and grouping happen in SQLite.
pub async fn get_capture_source_usage(
    pool: &SqlitePool,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<CaptureSourceUsageRaw>, StorageError> {
    let rows = sqlx::query(
        "WITH ordered_frames AS (
             SELECT
                 app_name,
                 browser_url,
                 timestamp,
                 (julianday(LEAD(timestamp) OVER (
                     ORDER BY datetime(timestamp), id
                 )) - julianday(timestamp)) * 86400.0 AS gap_seconds
             FROM frames
             WHERE datetime(timestamp) BETWEEN datetime(?1) AND datetime(?2)
         )
         SELECT
             COALESCE(NULLIF(TRIM(app_name), ''), 'Unknown') AS app_name,
             NULLIF(TRIM(browser_url), '') AS browser_url,
             TOTAL(CASE
                 WHEN gap_seconds > 0.0 AND gap_seconds < 300.0 THEN gap_seconds
                 ELSE 0.0
             END) AS active_seconds,
             COUNT(*) AS observations
         FROM ordered_frames
         GROUP BY
             COALESCE(NULLIF(TRIM(app_name), ''), 'Unknown'),
             NULLIF(TRIM(browser_url), '')",
    )
    .bind(start_time)
    .bind(end_time)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(CaptureSourceUsageRaw {
                app_name: row.try_get("app_name")?,
                browser_url: row.try_get("browser_url")?,
                active_seconds: row.try_get("active_seconds")?,
                observations: row.try_get::<i64, _>("observations")?.max(0) as u64,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

pub async fn get_activity_overview_raw(
    pool: &SqlitePool,
    start_time: &str,
    end_time: &str,
    app_name: Option<&str>,
) -> Result<ActivityOverviewRaw, StorageError> {
    let frames = sqlx::query(
        "SELECT timestamp, app_name, window_name, browser_url
         FROM frames
         WHERE datetime(timestamp) BETWEEN datetime(?1) AND datetime(?2)
           AND (?3 IS NULL OR app_name = ?3)
         ORDER BY datetime(timestamp), id
         LIMIT 250000",
    )
    .bind(start_time)
    .bind(end_time)
    .bind(app_name)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(FrameObservation {
            timestamp: row.try_get("timestamp")?,
            app_name: row.try_get("app_name")?,
            window_name: row.try_get("window_name")?,
            browser_url: row.try_get("browser_url")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let event_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM ui_events
         WHERE datetime(timestamp) BETWEEN datetime(?1) AND datetime(?2)
           AND (?3 IS NULL OR app_name = ?3)",
    )
    .bind(start_time)
    .bind(end_time)
    .bind(app_name)
    .fetch_one(pool)
    .await?
    .max(0) as u64;

    let sample_rows = sqlx::query(
        "SELECT source_type, source_row_id, timestamp, app_name, window_name, browser_url, text
         FROM activity_search_documents
         WHERE datetime(timestamp) BETWEEN datetime(?1) AND datetime(?2)
           AND (?3 IS NULL OR app_name = ?3)
         ORDER BY datetime(timestamp), id
         LIMIT 5000",
    )
    .bind(start_time)
    .bind(end_time)
    .bind(app_name)
    .fetch_all(pool)
    .await?;
    let samples = sample_rows
        .iter()
        .filter_map(|row| {
            let source_type = row.try_get::<String, _>("source_type").ok()?;
            let source_row_id = row.try_get::<i64, _>("source_row_id").ok()?;
            let text = row.try_get::<String, _>("text").ok()?;
            if text.trim().chars().count() < 20 {
                return None;
            }
            Some(ActivityRecord {
                source_id: format!("{source_type}:{source_row_id}"),
                timestamp: row.try_get("timestamp").ok()?,
                app_name: row.try_get("app_name").ok()?,
                window_name: row.try_get("window_name").ok()?,
                browser_url: row.try_get("browser_url").ok()?,
                text,
            })
        })
        .collect();

    let health_row = sqlx::query(
        "SELECT
           (SELECT MAX(timestamp) FROM frames) AS last_frame_at,
           (SELECT MAX(timestamp) FROM ui_events) AS last_event_at,
           (SELECT COUNT(*) FROM dystil_text_redaction_state WHERE status = 'pending') AS redaction_backlog,
           (SELECT COUNT(*) FROM activity_search_documents) AS search_documents,
           (SELECT COUNT(*) FROM activity_search_fts) AS fts_documents",
    )
    .fetch_one(pool)
    .await?;
    let health = ActivityHealthRaw {
        last_frame_at: health_row.try_get("last_frame_at")?,
        last_event_at: health_row.try_get("last_event_at")?,
        redaction_backlog: nonnegative_count(&health_row, "redaction_backlog")?,
        search_documents: nonnegative_count(&health_row, "search_documents")?,
        fts_documents: nonnegative_count(&health_row, "fts_documents")?,
    };
    Ok(ActivityOverviewRaw {
        frames,
        event_count,
        samples,
        health,
    })
}

pub async fn count_activity_in_range(
    pool: &SqlitePool,
    start_time: &str,
    end_time: &str,
) -> Result<u64, StorageError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM activity_search_documents
         WHERE datetime(timestamp) BETWEEN datetime(?1) AND datetime(?2)",
    )
    .bind(start_time)
    .bind(end_time)
    .fetch_one(pool)
    .await?
    .max(0) as u64)
}

fn nonnegative_count(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<u64, sqlx::Error> {
    Ok(row.try_get::<i64, _>(column)?.max(0) as u64)
}
