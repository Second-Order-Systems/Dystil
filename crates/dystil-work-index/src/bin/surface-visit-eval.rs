use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use clap::Parser;
use dystil_work_index::{build_surface_visits, FrameObservation, SurfaceVisit, SurfaceVisitConfig};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// Evaluate deterministic surface visits against a capture database.
///
/// This experiment targets Dystil's current frame model. Its reader contains a
/// deliberate fixture-only adapter for historical databases captured before
/// `frames.frame_text` was introduced.
#[derive(Debug, Parser)]
struct Args {
    database: PathBuf,
    /// Emit every span as JSONL instead of a compact aggregate report.
    #[arg(long)]
    jsonl: bool,
    /// Include changed accessibility text in output. Off by default so an
    /// evaluation cannot accidentally print captured content.
    #[arg(long)]
    include_text: bool,
    /// Maximum sample spans included in the aggregate report.
    #[arg(long, default_value_t = 20)]
    samples: usize,
}

#[derive(Serialize)]
struct Report<'a> {
    database: String,
    frame_count: usize,
    span_count: usize,
    total_wall_clock_seconds: i64,
    total_observed_active_seconds: i64,
    source_text_chars: u64,
    indexed_text_chars: u64,
    text_compaction_ratio: f64,
    close_reasons: BTreeMap<String, usize>,
    spans_by_app: BTreeMap<String, usize>,
    sample_spans: Vec<OutputSpan<'a>>,
}

#[derive(Serialize)]
struct OutputSpan<'a> {
    #[serde(flatten)]
    span: &'a SurfaceVisit,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_text_count: Option<usize>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let options = SqliteConnectOptions::new()
        .filename(&args.database)
        .read_only(true)
        .create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let frames = load_frames(&pool).await?;
    let frame_count = frames.len();
    let mut spans = build_surface_visits(frames, &SurfaceVisitConfig::default());
    let changed_text_counts = spans
        .iter()
        .map(|span| (span.id.clone(), span.changed_text.len()))
        .collect::<BTreeMap<_, _>>();
    if !args.include_text {
        for span in &mut spans {
            span.changed_text.clear();
        }
    }

    if args.jsonl {
        for span in spans {
            println!("{}", serde_json::to_string(&span)?);
        }
        return Ok(());
    }

    let mut close_reasons = BTreeMap::new();
    let mut spans_by_app = BTreeMap::new();
    for span in &spans {
        *close_reasons.entry(span.close_reason.clone()).or_default() += 1;
        *spans_by_app.entry(span.app_name.clone()).or_default() += 1;
    }
    let report = Report {
        database: args.database.display().to_string(),
        frame_count,
        span_count: spans.len(),
        total_wall_clock_seconds: spans.iter().map(|span| span.wall_clock_seconds).sum(),
        total_observed_active_seconds: spans.iter().map(|span| span.observed_active_seconds).sum(),
        source_text_chars: spans.iter().map(|span| span.source_text_chars).sum(),
        indexed_text_chars: spans.iter().map(|span| span.indexed_text_chars).sum(),
        text_compaction_ratio: {
            let source = spans.iter().map(|span| span.source_text_chars).sum::<u64>();
            let indexed = spans
                .iter()
                .map(|span| span.indexed_text_chars)
                .sum::<u64>();
            if source == 0 {
                0.0
            } else {
                indexed as f64 / source as f64
            }
        },
        close_reasons,
        spans_by_app,
        sample_spans: spans
            .iter()
            .take(args.samples)
            .map(|span| OutputSpan {
                span,
                changed_text_count: (!args.include_text).then(|| {
                    changed_text_counts
                        .get(&span.id)
                        .copied()
                        .unwrap_or_default()
                }),
            })
            .collect(),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn load_frames(pool: &sqlx::SqlitePool) -> Result<Vec<FrameObservation>, sqlx::Error> {
    let columns = sqlx::query("PRAGMA table_info(frames)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    let has = |name: &str| columns.iter().any(|column| column == name);

    // Current Dystil databases use frame_text. The alternatives are read only
    // for the historical macOS/Windows evaluation fixtures in this repository.
    let text = if has("frame_text") {
        "frame_text"
    } else if has("accessibility_text") {
        "accessibility_text"
    } else if has("full_text") {
        "full_text"
    } else {
        "NULL"
    };
    let expression = |column: &str| {
        if has(column) {
            column.to_owned()
        } else {
            "NULL".to_owned()
        }
    };
    let sql = format!(
        "SELECT id, CAST(timestamp AS TEXT) AS timestamp, \
         {app} AS app_name, {window} AS window_name, {url} AS browser_url, \
         {document} AS document_path, {trigger} AS capture_trigger, {text} AS frame_text, \
         {focused} AS focused FROM frames ORDER BY timestamp, id",
        app = expression("app_name"),
        window = expression("window_name"),
        url = expression("browser_url"),
        document = expression("document_path"),
        trigger = expression("capture_trigger"),
        focused = expression("focused"),
    );

    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut frames = Vec::new();
    for row in rows {
        if row.try_get::<Option<bool>, _>("focused").ok().flatten() == Some(false) {
            continue;
        }
        let timestamp = row.try_get::<String, _>("timestamp")?;
        let Ok(timestamp) = DateTime::parse_from_rfc3339(&timestamp) else {
            continue;
        };
        frames.push(FrameObservation {
            id: row.try_get("id")?,
            timestamp: timestamp.with_timezone(&Utc),
            app_name: row
                .try_get::<Option<String>, _>("app_name")?
                .unwrap_or_default(),
            window_name: row.try_get("window_name")?,
            browser_url: row.try_get("browser_url")?,
            document_path: row.try_get("document_path")?,
            capture_trigger: row.try_get("capture_trigger")?,
            text: row.try_get("frame_text")?,
        });
    }
    Ok(frames)
}
