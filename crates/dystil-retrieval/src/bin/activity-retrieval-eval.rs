//! Read-only evaluation adapter for pre-`frame_text` fixture databases.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser;
use dystil_retrieval::{OverviewRequest, RetrievalService, SearchRequest};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

#[derive(Debug, Parser)]
struct Args {
    database: PathBuf,
    #[arg(long, default_value_t = 5)]
    probes: usize,
    /// Keep the compatibility-imported database at this path for MCP/provider integration tests.
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct Report {
    database: String,
    imported_frames: u64,
    imported_events: u64,
    search_documents: u64,
    fts_documents: u64,
    probe_queries: usize,
    successful_probe_queries: usize,
    max_probe_snippet_chars: usize,
    overview_status: String,
    estimated_active_minutes: f64,
    coverage_gap_count: u64,
    app_count: usize,
    window_count: usize,
    transition_count: usize,
    representative_evidence_count: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let source = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&args.database)
                .read_only(true)
                .create_if_missing(false),
        )
        .await?;
    let scratch_dir = args.output.is_none().then(tempfile::tempdir).transpose()?;
    let target_path = args.output.clone().unwrap_or_else(|| {
        scratch_dir
            .as_ref()
            .expect("scratch directory exists without --output")
            .path()
            .join("eval.sqlite")
    });
    if target_path.exists() {
        return Err(format!("output database already exists: {}", target_path.display()).into());
    }
    let target = dystil_storage::open_capture_database(&target_path).await?;
    let (imported_frames, start, end, probe_terms) =
        import_frames(&source, &target, args.probes).await?;
    let imported_events = import_events(&source, &target).await?;
    let retrieval = RetrievalService::new(target.clone());

    let mut successful_probe_queries = 0;
    let mut max_probe_snippet_chars = 0;
    for term in &probe_terms {
        let page = retrieval
            .search(SearchRequest {
                query: term.clone(),
                limit: Some(5),
                max_snippet_chars: Some(300),
                ..Default::default()
            })
            .await?;
        if !page.records.is_empty() {
            successful_probe_queries += 1;
        }
        max_probe_snippet_chars = max_probe_snippet_chars.max(
            page.records
                .iter()
                .map(|record| record.text.chars().count())
                .max()
                .unwrap_or(0),
        );
    }

    let overview = retrieval
        .overview(OverviewRequest {
            start_time: start,
            end_time: end,
            max_snippets: Some(8),
            ..Default::default()
        })
        .await?;
    let (search_documents, fts_documents) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT (SELECT COUNT(*) FROM activity_search_documents),
                (SELECT COUNT(*) FROM activity_search_fts)",
    )
    .fetch_one(&target)
    .await?;
    let report = Report {
        database: args.database.display().to_string(),
        imported_frames,
        imported_events,
        search_documents: search_documents.max(0) as u64,
        fts_documents: fts_documents.max(0) as u64,
        probe_queries: probe_terms.len(),
        successful_probe_queries,
        max_probe_snippet_chars,
        overview_status: serde_json::to_value(&overview.data_status)?
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        estimated_active_minutes: overview.estimated_active_minutes,
        coverage_gap_count: overview.coverage_gap_count,
        app_count: overview.apps.len(),
        window_count: overview.windows.len(),
        transition_count: overview.transitions.len(),
        representative_evidence_count: overview.representative_evidence.len(),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn import_frames(
    source: &sqlx::SqlitePool,
    target: &sqlx::SqlitePool,
    probe_limit: usize,
) -> Result<(u64, String, String, Vec<String>), Box<dyn std::error::Error>> {
    let columns = table_columns(source, "frames").await?;
    let text_column = if columns.contains("frame_text") {
        "frame_text"
    } else if columns.contains("full_text") {
        "full_text"
    } else if columns.contains("accessibility_text") {
        "accessibility_text"
    } else {
        "NULL"
    };
    let column = |name: &str| {
        if columns.contains(name) {
            name.to_string()
        } else {
            "NULL".to_string()
        }
    };
    let query = format!(
        "SELECT CAST(timestamp AS TEXT) timestamp, {app} app_name, {window} window_name,
                {url} browser_url, {focused} focused, {text_column} frame_text
         FROM frames ORDER BY timestamp, id",
        app = column("app_name"),
        window = column("window_name"),
        url = column("browser_url"),
        focused = column("focused"),
    );
    let rows = sqlx::query(&query).fetch_all(source).await?;
    let mut imported = 0u64;
    let mut start = None;
    let mut end = None;
    let mut probes = BTreeSet::new();
    for row in rows {
        if row.try_get::<Option<bool>, _>("focused").ok().flatten() == Some(false) {
            continue;
        }
        let timestamp: String = row.try_get("timestamp")?;
        let text: Option<String> = row.try_get("frame_text")?;
        let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
            continue;
        };
        sqlx::query(
            "INSERT INTO frames(timestamp, app_name, window_name, browser_url, frame_text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&timestamp)
        .bind(row.try_get::<Option<String>, _>("app_name")?)
        .bind(row.try_get::<Option<String>, _>("window_name")?)
        .bind(row.try_get::<Option<String>, _>("browser_url")?)
        .bind(&text)
        .execute(target)
        .await?;
        start.get_or_insert_with(|| timestamp.clone());
        end = Some(timestamp);
        imported += 1;
        if probes.len() < probe_limit {
            for term in text
                .split(|character: char| {
                    !character.is_alphanumeric() && character != '-' && character != '_'
                })
                .filter(|term| term.chars().count() >= 8 && term.chars().count() <= 40)
            {
                probes.insert(term.to_string());
                if probes.len() == probe_limit {
                    break;
                }
            }
        }
    }
    let start = start.ok_or("fixture contained no searchable focused frames")?;
    let end = end.ok_or("fixture contained no searchable focused frames")?;
    Ok((imported, start, end, probes.into_iter().collect()))
}

async fn import_events(
    source: &sqlx::SqlitePool,
    target: &sqlx::SqlitePool,
) -> Result<u64, Box<dyn std::error::Error>> {
    if !table_exists(source, "ui_events").await? {
        return Ok(0);
    }
    let rows = sqlx::query(
        "SELECT CAST(timestamp AS TEXT) timestamp, event_type, text_content, app_name,
                window_title, browser_url, element_name, element_value, element_description
         FROM ui_events ORDER BY timestamp, id",
    )
    .fetch_all(source)
    .await?;
    let mut imported = 0;
    for row in rows {
        sqlx::query(
            "INSERT INTO ui_events(timestamp, event_type, text_content, app_name, window_title,
                    browser_url, element_name, element_value, element_description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(row.try_get::<String, _>("timestamp")?)
        .bind(row.try_get::<String, _>("event_type")?)
        .bind(row.try_get::<Option<String>, _>("text_content")?)
        .bind(row.try_get::<Option<String>, _>("app_name")?)
        .bind(row.try_get::<Option<String>, _>("window_title")?)
        .bind(row.try_get::<Option<String>, _>("browser_url")?)
        .bind(row.try_get::<Option<String>, _>("element_name")?)
        .bind(row.try_get::<Option<String>, _>("element_value")?)
        .bind(row.try_get::<Option<String>, _>("element_description")?)
        .execute(target)
        .await?;
        imported += 1;
    }
    Ok(imported)
}

async fn table_columns(
    pool: &sqlx::SqlitePool,
    table: &str,
) -> Result<BTreeSet<String>, sqlx::Error> {
    Ok(sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get("name").ok())
        .collect())
}

async fn table_exists(pool: &sqlx::SqlitePool, table: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table)
    .fetch_one(pool)
    .await?
        > 0)
}
