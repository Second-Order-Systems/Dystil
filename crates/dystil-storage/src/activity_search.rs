use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

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

#[derive(Debug, Clone, PartialEq)]
pub struct ActivitySearchRecord {
    pub record: ActivityRecord,
    pub snippet: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActivitySearchQuery {
    pub query: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub source_type: Option<String>,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActivityRangeQuery {
    pub start_time: String,
    pub end_time: String,
    pub source_type: Option<String>,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

/// Compatibility wrapper retained for existing callers while the shared
/// retrieval service becomes the only AI-facing entry point.
pub async fn search_activity(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<ActivityRecord>, StorageError> {
    Ok(search_activity_filtered(
        pool,
        &ActivitySearchQuery {
            query: query.to_string(),
            limit,
            ..Default::default()
        },
    )
    .await?
    .into_iter()
    .map(|row| row.record)
    .collect())
}

pub async fn search_activity_filtered(
    pool: &SqlitePool,
    request: &ActivitySearchQuery,
) -> Result<Vec<ActivitySearchRecord>, StorageError> {
    let Some(expression) = fts_expression(&request.query) else {
        return Ok(vec![]);
    };
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT d.source_type, d.source_row_id, d.timestamp, d.app_name, d.window_name, d.browser_url, d.text, \
         snippet(activity_search_fts, 0, '[', ']', ' … ', 32) AS search_snippet, \
         bm25(activity_search_fts, 1.0, 0.4, 0.6, 0.3) AS search_rank \
         FROM activity_search_fts f JOIN activity_search_documents d ON d.id = f.rowid \
         WHERE activity_search_fts MATCH ",
    );
    query.push_bind(expression);
    push_filters(
        &mut query,
        request.start_time.as_deref(),
        request.end_time.as_deref(),
        request.source_type.as_deref(),
        request.app_name.as_deref(),
        request.window_name.as_deref(),
        request.browser_url.as_deref(),
    );
    query.push(" ORDER BY search_rank, datetime(d.timestamp) DESC, d.id DESC LIMIT ");
    query.push_bind(request.limit.clamp(1, 100) as i64);
    query.push(" OFFSET ");
    query.push_bind(request.offset.min(100_000) as i64);
    let rows = query.build().fetch_all(pool).await?;
    rows.iter()
        .map(|row| {
            Ok(ActivitySearchRecord {
                record: row_to_activity(row)?,
                snippet: row.try_get("search_snippet")?,
                rank: row.try_get("search_rank")?,
            })
        })
        .collect()
}

pub async fn get_activity_source(
    pool: &SqlitePool,
    source_id: &str,
) -> Result<Option<ActivityRecord>, StorageError> {
    let Some((source_type, source_row_id)) = parse_source_id(source_id) else {
        return Ok(None);
    };
    let row = sqlx::query(
        "SELECT source_type, source_row_id, timestamp, app_name, window_name, browser_url, text
         FROM activity_search_documents WHERE source_type = ?1 AND source_row_id = ?2",
    )
    .bind(source_type)
    .bind(source_row_id)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_activity).transpose()
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
    .bind(limit.clamp(1, 100) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_activity).collect()
}

pub async fn get_activity_range(
    pool: &SqlitePool,
    request: &ActivityRangeQuery,
) -> Result<Vec<ActivityRecord>, StorageError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT d.source_type, d.source_row_id, d.timestamp, d.app_name, d.window_name, d.browser_url, d.text \
         FROM activity_search_documents d WHERE 1=1",
    );
    push_filters(
        &mut query,
        Some(&request.start_time),
        Some(&request.end_time),
        request.source_type.as_deref(),
        request.app_name.as_deref(),
        request.window_name.as_deref(),
        request.browser_url.as_deref(),
    );
    query.push(" ORDER BY datetime(d.timestamp), d.id LIMIT ");
    query.push_bind(request.limit.clamp(1, 100) as i64);
    query.push(" OFFSET ");
    query.push_bind(request.offset.min(100_000) as i64);
    let rows = query.build().fetch_all(pool).await?;
    rows.iter().map(row_to_activity).collect()
}

fn push_filters<'args>(
    query: &mut QueryBuilder<'args, Sqlite>,
    start_time: Option<&'args str>,
    end_time: Option<&'args str>,
    source_type: Option<&'args str>,
    app_name: Option<&'args str>,
    window_name: Option<&'args str>,
    browser_url: Option<&'args str>,
) {
    if let Some(value) = start_time {
        query.push(" AND datetime(d.timestamp) >= datetime(");
        query.push_bind(value);
        query.push(")");
    }
    if let Some(value) = end_time {
        query.push(" AND datetime(d.timestamp) <= datetime(");
        query.push_bind(value);
        query.push(")");
    }
    if let Some(value) = source_type {
        query.push(" AND d.source_type = ");
        query.push_bind(value);
    }
    if let Some(value) = app_name {
        query.push(" AND d.app_name = ");
        query.push_bind(value);
    }
    if let Some(value) = window_name {
        query.push(" AND d.window_name LIKE '%' || ");
        query.push_bind(value);
        query.push(" || '%'");
    }
    if let Some(value) = browser_url {
        query.push(" AND d.browser_url LIKE '%' || ");
        query.push_bind(value);
        query.push(" || '%'");
    }
}

pub(crate) fn parse_source_id(value: &str) -> Option<(&str, i64)> {
    let (source_type, row_id) = value.split_once(':')?;
    matches!(source_type, "frame" | "event")
        .then(|| row_id.parse::<i64>().ok())
        .flatten()
        .filter(|row_id| *row_id > 0)
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
