use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityRecord {
    pub source_id: String,
    pub timestamp: String,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub text: String,
}

pub async fn search_activity(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<ActivityRecord>, StorageError> {
    let Some(expression) = fts_expression(query) else {
        return Ok(vec![]);
    };
    let rows = sqlx::query(
        "SELECT d.source_type, d.source_row_id, d.timestamp, d.app_name, d.window_name, d.browser_url, d.text
         FROM activity_search_fts f JOIN activity_search_documents d ON d.id = f.rowid
         WHERE activity_search_fts MATCH ?1
         ORDER BY bm25(activity_search_fts), datetime(d.timestamp) DESC, d.id DESC
         LIMIT ?2",
    )
    .bind(expression)
    .bind(limit.clamp(1, 30) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_activity).collect()
}

pub async fn get_activity_context(
    pool: &SqlitePool,
    source_id: &str,
    before_seconds: u32,
    after_seconds: u32,
    limit: u32,
) -> Result<Vec<ActivityRecord>, StorageError> {
    let Some((source_type, source_row_id)) = parse_source_id(source_id) else {
        return Ok(vec![]);
    };
    let rows = sqlx::query(
        "SELECT d.source_type, d.source_row_id, d.timestamp, d.app_name, d.window_name, d.browser_url, d.text
         FROM activity_search_documents d
         WHERE datetime(d.timestamp) BETWEEN
            datetime((SELECT timestamp FROM activity_search_documents WHERE source_type = ?1 AND source_row_id = ?2), '-' || ?3 || ' seconds')
            AND datetime((SELECT timestamp FROM activity_search_documents WHERE source_type = ?1 AND source_row_id = ?2), '+' || ?4 || ' seconds')
         ORDER BY datetime(d.timestamp), d.id
         LIMIT ?5",
    )
    .bind(source_type)
    .bind(source_row_id)
    .bind(before_seconds.clamp(1, 3600) as i64)
    .bind(after_seconds.clamp(1, 3600) as i64)
    .bind(limit.clamp(1, 50) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_activity).collect()
}

pub(crate) fn parse_source_id(value: &str) -> Option<(&str, i64)> {
    let (source_type, row_id) = value.split_once(':')?;
    matches!(source_type, "frame" | "event")
        .then(|| row_id.parse::<i64>().ok())
        .flatten()
        .map(|row_id| (source_type, row_id))
}

fn row_to_activity(row: &sqlx::sqlite::SqliteRow) -> Result<ActivityRecord, StorageError> {
    let source_type: String = row.try_get("source_type")?;
    let source_row_id: i64 = row.try_get("source_row_id")?;
    Ok(ActivityRecord {
        source_id: format!("{source_type}:{source_row_id}"),
        timestamp: row.try_get("timestamp")?,
        app_name: row.try_get("app_name")?,
        window_name: row.try_get("window_name")?,
        browser_url: row.try_get("browser_url")?,
        text: row.try_get("text")?,
    })
}

fn fts_expression(query: &str) -> Option<String> {
    let terms = query
        .split(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .filter(|value| value.chars().count() >= 2)
        .take(24)
        .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}
