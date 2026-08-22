use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, TimeZone, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use tauri::{AppHandle, State};
use tracing::warn;

use crate::recording::RecordingState;
use crate::store::SettingsStore;
use crate::worth_fixing_commands::WorthFixingState;

const BATCH_SIZE: usize = 1_000;
const SOURCE_NAMESPACE: &str = "local-capture";
static DELETION_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletionScope {
    pub kind: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub source_kind: Option<String>,
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletionSource {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletionPreview {
    pub frame_count: u64,
    pub event_count: u64,
    pub captured_duration_seconds: u64,
    pub screenshot_count: u64,
    pub media_bytes: u64,
    pub oldest_at: Option<String>,
    pub newest_at: Option<String>,
    pub cloud_copy_may_remain: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletionResult {
    pub deleted_frames: u64,
    pub deleted_events: u64,
    pub deleted_screenshots: u64,
    pub forgotten_evidence: u64,
    pub withdrawn_findings: u64,
    pub cloud_copy_may_remain: bool,
}

#[derive(Debug, Clone)]
enum ResolvedScope {
    All,
    Range { start: String, end: String },
    App(String),
    Site(String),
}

fn resolve_scope(scope: &DeletionScope) -> Result<ResolvedScope, String> {
    match scope.kind.as_str() {
        "all" => Ok(ResolvedScope::All),
        "today" => {
            let now = Local::now();
            let start = now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|value| Local.from_local_datetime(&value).earliest())
                .ok_or_else(|| "could not resolve the start of today".to_string())?;
            Ok(ResolvedScope::Range {
                start: start.with_timezone(&Utc).to_rfc3339(),
                end: Utc::now().to_rfc3339(),
            })
        }
        "dateRange" => {
            let start = parse_local_day(scope.start_date.as_deref(), false)?;
            let end = parse_local_day(scope.end_date.as_deref(), true)?;
            if start >= end {
                return Err("the end date must not be before the start date".to_string());
            }
            Ok(ResolvedScope::Range { start, end })
        }
        "source" => {
            let name = scope
                .source_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "choose an app or site".to_string())?;
            match scope.source_kind.as_deref() {
                Some("app") if !name.contains("::") => Ok(ResolvedScope::App(name.to_string())),
                Some("site") => normalize_domain(name)
                    .map(ResolvedScope::Site)
                    .ok_or_else(|| "choose a valid site".to_string()),
                _ => Err("choose a valid app or site".to_string()),
            }
        }
        _ => Err("unknown deletion scope".to_string()),
    }
}

fn parse_local_day(value: Option<&str>, end_exclusive: bool) -> Result<String, String> {
    let date = NaiveDate::parse_from_str(
        value.ok_or_else(|| "choose both dates".to_string())?,
        "%Y-%m-%d",
    )
    .map_err(|_| "choose valid dates".to_string())?;
    let date = if end_exclusive {
        date.succ_opt()
            .ok_or_else(|| "end date is too large".to_string())?
    } else {
        date
    };
    let local = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Local.from_local_datetime(&value).earliest())
        .ok_or_else(|| "could not resolve that local date".to_string())?;
    Ok(local.with_timezone(&Utc).to_rfc3339())
}

fn normalize_domain(value: &str) -> Option<String> {
    let value = value.trim().to_lowercase();
    let parsed = url::Url::parse(&value)
        .or_else(|_| url::Url::parse(&format!("https://{value}")))
        .ok()?;
    parsed
        .host_str()
        .map(|host| host.trim_start_matches("www.").to_string())
}

fn push_filter(builder: &mut QueryBuilder<'_, Sqlite>, scope: &ResolvedScope, app_column: &str) {
    match scope {
        ResolvedScope::All => {
            builder.push("1=1");
        }
        ResolvedScope::Range { start, end } => {
            builder
                .push("datetime(timestamp) >= datetime(")
                .push_bind(start.clone())
                .push(") AND datetime(timestamp) < datetime(")
                .push_bind(end.clone())
                .push(")");
        }
        ResolvedScope::App(name) => {
            builder
                .push("lower(trim(coalesce(")
                .push(app_column)
                .push(",''))) = lower(")
                .push_bind(name.clone())
                .push(")");
        }
        ResolvedScope::Site(domain) => {
            builder
                .push("(lower(trim(coalesce(browser_url,''))) = ")
                .push_bind(domain.clone())
                .push(" OR lower(trim(coalesce(browser_url,''))) = ")
                .push_bind(format!("www.{domain}"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://{domain}/%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://{domain}?%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://{domain}#%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://{domain}"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://www.{domain}/%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://www.{domain}?%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://www.{domain}#%"))
                .push(" OR lower(browser_url) LIKE ")
                .push_bind(format!("%://www.{domain}"))
                .push(")");
        }
    }
}

async fn matching_ids(
    pool: &SqlitePool,
    table: &str,
    app_column: &str,
    scope: &ResolvedScope,
) -> Result<Vec<i64>, String> {
    let mut builder = QueryBuilder::new(format!("SELECT id FROM {table} WHERE "));
    push_filter(&mut builder, scope, app_column);
    builder.push(" ORDER BY id");
    Ok(builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

async fn matching_snapshots(
    pool: &SqlitePool,
    scope: &ResolvedScope,
) -> Result<Vec<PathBuf>, String> {
    let mut builder = QueryBuilder::new(
        "SELECT DISTINCT snapshot_path FROM frames WHERE trim(coalesce(snapshot_path,'')) != '' AND ",
    );
    push_filter(&mut builder, scope, "app_name");
    Ok(builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("snapshot_path").ok())
        .map(PathBuf::from)
        .collect())
}

async fn time_bounds(
    pool: &SqlitePool,
    table: &str,
    app_column: &str,
    scope: &ResolvedScope,
) -> Result<(u64, Option<String>, Option<String>), String> {
    let mut builder = QueryBuilder::new(format!(
        "SELECT COUNT(*) count,MIN(timestamp) oldest,MAX(timestamp) newest FROM {table} WHERE "
    ));
    push_filter(&mut builder, scope, app_column);
    let row = builder
        .build()
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok((
        row.get::<i64, _>("count").max(0) as u64,
        row.try_get("oldest").ok().flatten(),
        row.try_get("newest").ok().flatten(),
    ))
}

async fn captured_duration_seconds(
    pool: &SqlitePool,
    scope: &ResolvedScope,
) -> Result<u64, String> {
    let mut builder =
        QueryBuilder::new("WITH selected(timestamp) AS (SELECT timestamp FROM frames WHERE ");
    push_filter(&mut builder, scope, "app_name");
    builder.push(" UNION ALL SELECT timestamp FROM ui_events WHERE ");
    push_filter(&mut builder, scope, "app_name");
    builder.push(
        "), ordered AS (
           SELECT (julianday(LEAD(timestamp) OVER (ORDER BY datetime(timestamp)))
                 - julianday(timestamp)) * 86400.0 AS gap_seconds
           FROM selected
         )
         SELECT TOTAL(CASE WHEN gap_seconds > 0 AND gap_seconds <= 300
                           THEN gap_seconds ELSE 0 END) AS seconds
         FROM ordered",
    );
    let seconds = builder
        .build()
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?
        .get::<f64, _>("seconds");
    Ok(seconds.max(0.0).round() as u64)
}

fn cloud_copy_may_remain(app: &AppHandle) -> bool {
    SettingsStore::get(app)
        .ok()
        .flatten()
        .unwrap_or_default()
        .sync_consent
        .segments
}

async fn build_preview(
    app: &AppHandle,
    pool: &SqlitePool,
    scope: &ResolvedScope,
) -> Result<DeletionPreview, String> {
    let (frame_count, frame_oldest, frame_newest) =
        time_bounds(pool, "frames", "app_name", scope).await?;
    let (event_count, event_oldest, event_newest) =
        time_bounds(pool, "ui_events", "app_name", scope).await?;
    let captured_duration_seconds = captured_duration_seconds(pool, scope).await?;
    let snapshots = matching_snapshots(pool, scope).await?;
    let media_bytes = snapshots
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum();
    let oldest_at = [frame_oldest, event_oldest].into_iter().flatten().min();
    let newest_at = [frame_newest, event_newest].into_iter().flatten().max();
    Ok(DeletionPreview {
        frame_count,
        event_count,
        captured_duration_seconds,
        screenshot_count: snapshots.len() as u64,
        media_bytes,
        oldest_at,
        newest_at,
        cloud_copy_may_remain: cloud_copy_may_remain(app),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_deletion_sources(
    state: State<'_, RecordingState>,
) -> Result<Vec<DeletionSource>, String> {
    let pool = crate::ai::capture_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT app_name,browser_url FROM frames
         UNION SELECT app_name,browser_url FROM ui_events",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| error.to_string())?;
    let mut apps = BTreeSet::new();
    let mut sites = BTreeSet::new();
    for row in rows {
        if let Ok(Some(app)) = row.try_get::<Option<String>, _>("app_name") {
            let app = app.trim();
            if !app.is_empty() && !app.eq_ignore_ascii_case("unknown") {
                apps.insert(app.to_string());
            }
        }
        if let Ok(Some(url)) = row.try_get::<Option<String>, _>("browser_url") {
            if let Some(domain) = normalize_domain(&url) {
                sites.insert(domain);
            }
        }
    }
    Ok(apps
        .into_iter()
        .map(|name| DeletionSource {
            kind: "app".to_string(),
            name,
        })
        .chain(sites.into_iter().map(|name| DeletionSource {
            kind: "site".to_string(),
            name,
        }))
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn preview_capture_deletion(
    app: AppHandle,
    state: State<'_, RecordingState>,
    scope: DeletionScope,
) -> Result<DeletionPreview, String> {
    let pool = crate::ai::capture_pool(&state).await?;
    build_preview(&app, &pool, &resolve_scope(&scope)?).await
}

async fn delete_ids(
    pool: &SqlitePool,
    table: &str,
    source_table: &str,
    ids: &[i64],
) -> Result<u64, String> {
    let mut deleted = 0;
    for chunk in ids.chunks(BATCH_SIZE) {
        let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
        let mut redaction =
            QueryBuilder::new("DELETE FROM dystil_text_redaction_state WHERE source_table=");
        redaction
            .push_bind(source_table)
            .push(" AND source_row_id IN (");
        let mut separated = redaction.separated(",");
        for id in chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        redaction
            .build()
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;

        let mut rows = QueryBuilder::new(format!("DELETE FROM {table} WHERE id IN ("));
        let mut separated = rows.separated(",");
        for id in chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        deleted += rows
            .build()
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?
            .rows_affected();
        tx.commit().await.map_err(|error| error.to_string())?;
        tokio::task::yield_now().await;
    }
    Ok(deleted)
}

fn delete_media(paths: &[PathBuf], media_dir: &Path) -> Result<u64, String> {
    if paths.is_empty() {
        return Ok(0);
    }
    let mut existing = Vec::new();
    for path in paths {
        match std::fs::canonicalize(path) {
            Ok(candidate) => existing.push((path, candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not verify captured media {}: {error}",
                    path.display()
                ));
            }
        }
    }
    if existing.is_empty() {
        return Ok(0);
    }
    let root = std::fs::canonicalize(media_dir)
        .map_err(|error| format!("could not verify Dystil's media folder: {error}"))?;
    let mut deleted = 0;
    for (path, candidate) in existing {
        if !candidate.starts_with(&root) {
            return Err(format!(
                "refused to delete media outside Dystil's data folder: {}",
                path.display()
            ));
        }
        std::fs::remove_file(candidate).map_err(|error| {
            format!(
                "could not delete captured media {}: {error}",
                path.display()
            )
        })?;
        deleted += 1;
    }
    Ok(deleted)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_capture_data(
    app: AppHandle,
    recording: State<'_, RecordingState>,
    insights: State<'_, WorthFixingState>,
    scope: DeletionScope,
) -> Result<DeletionResult, String> {
    if matches!(
        crate::app_policy::current().capture.local_deletion,
        crate::app_policy::Availability::Disabled
    ) {
        return Err("Local captured-data deletion is unavailable in this edition.".to_string());
    }
    let _guard = DELETION_LOCK
        .try_lock()
        .map_err(|_| "another deletion is already running".to_string())?;
    let scope = resolve_scope(&scope)?;
    let (capture, data_dir, media_dir) = {
        let server = recording.server.lock().await;
        let server = server
            .as_ref()
            .ok_or_else(|| "local capture database is not ready".to_string())?;
        (
            server.db.pool.clone(),
            server.data_dir.clone(),
            server.data_path.clone(),
        )
    };
    let insights_pool = if matches!(
        crate::app_policy::current().local_worth_fixing,
        crate::app_policy::Availability::Enabled
    ) {
        Some(insights.pool(&app).await?.clone())
    } else {
        None
    };
    let was_running = recording
        .capture_active
        .load(std::sync::atomic::Ordering::SeqCst);
    let is_all = matches!(scope, ResolvedScope::All);
    if is_all && was_running {
        crate::recording::stop_capture(recording.clone(), app.clone()).await?;
    }

    let operation = async {
        let frame_ids = matching_ids(&capture, "frames", "app_name", &scope).await?;
        let event_ids = matching_ids(&capture, "ui_events", "app_name", &scope).await?;
        let snapshots = matching_snapshots(&capture, &scope).await?;
        let source_ids = frame_ids
            .iter()
            .map(|id| format!("frame:{id}"))
            .chain(event_ids.iter().map(|id| format!("event:{id}")))
            .collect::<Vec<_>>();

        // Remove media first. If a filesystem permission error interrupts the
        // operation, the database paths remain available so a retry can finish
        // instead of leaving an undiscoverable orphan behind.
        let deleted_screenshots = delete_media(&snapshots, &media_dir)?;

        let (forgotten_evidence, withdrawn_findings) = if let Some(insights_pool) = insights_pool.as_ref() {
            if is_all {
            dystil_insights::delete_all_insights_data(&insights_pool)
                .await
                .map_err(|error| error.to_string())?;
            // Bundle directories are immutable content stores indexed by the
            // insights database. A full Dystil deletion must remove both,
            // while scoped capture deletion deliberately preserves kept work.
            for directory in [
                crate::dystil_paths::data_dir().join("skill-bundles"),
                crate::dystil_paths::data_dir().join("skill-bundle-builds"),
                crate::dystil_paths::data_dir().join("skill-bundle-exports"),
            ] {
                if directory.exists() {
                    std::fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
                }
            }
            (0, 0)
            } else {
            dystil_insights::forget_capture_evidence(
                &insights_pool,
                SOURCE_NAMESPACE,
                &source_ids,
            )
            .await
                .map_err(|error| error.to_string())?
            }
        } else { (0, 0) };

        if let Some(insights_pool) = insights_pool.as_ref() {
            dystil_insights::invalidate_ask_for_fix_retrieval_memos(insights_pool)
                .await
                .map_err(|error| error.to_string())?;
            dystil_insights::invalidate_workflow_reconstructions(insights_pool)
                .await
                .map_err(|error| error.to_string())?;
        }

        if is_all {
            let mut tx = capture.begin().await.map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM local_chat_messages")
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM local_chat_sessions")
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM agent_messages")
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            tx.commit().await.map_err(|error| error.to_string())?;
        }

        let deleted_frames = delete_ids(&capture, "frames", "frames", &frame_ids).await?;
        let deleted_events = delete_ids(&capture, "ui_events", "ui_events", &event_ids).await?;
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&capture)
            .await;
        if let Err(error) = sqlx::query("VACUUM").execute(&capture).await {
            warn!(error = %error, "deletion completed but database compaction was deferred");
        }
        if let Some(insights_pool) = insights_pool.as_ref() {
            let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                .execute(insights_pool)
                .await;
            if let Err(error) = sqlx::query("VACUUM").execute(insights_pool).await {
                warn!(error = %error, "derived history was removed but insights database compaction was deferred");
            }
        }
        if let Err(error) = crate::disk_usage::disk_usage(&data_dir, true).await {
            warn!(error = %error, "deletion completed but the storage-size cache was not refreshed");
        }
        Ok::<_, String>(DeletionResult {
            deleted_frames,
            deleted_events,
            deleted_screenshots,
            forgotten_evidence,
            withdrawn_findings,
            cloud_copy_may_remain: cloud_copy_may_remain(&app),
        })
    }
    .await;

    let resume = if is_all && was_running {
        crate::recording::start_capture(recording.clone(), app.clone()).await
    } else {
        Ok(())
    };
    match (operation, resume) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(format!(
            "Deletion did not finish: {error}. You can safely retry."
        )),
        (Ok(_), Err(error)) => Err(format!(
            "Deletion finished, but capture could not resume: {error}"
        )),
        (Err(delete_error), Err(resume_error)) => Err(format!(
            "Deletion did not finish: {delete_error}. Capture also could not resume: {resume_error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn date_ranges_are_inclusive_calendar_days() {
        let resolved = resolve_scope(&DeletionScope {
            kind: "dateRange".into(),
            start_date: Some("2026-08-01".into()),
            end_date: Some("2026-08-03".into()),
            source_kind: None,
            source_name: None,
        })
        .unwrap();
        let ResolvedScope::Range { start, end } = resolved else {
            panic!("expected range")
        };
        assert!(start.contains("2026-07-31") || start.contains("2026-08-01"));
        assert!(end.contains("2026-08-03") || end.contains("2026-08-04"));
    }

    #[test]
    fn source_scope_rejects_unknown_kinds() {
        assert!(resolve_scope(&DeletionScope {
            kind: "source".into(),
            start_date: None,
            end_date: None,
            source_kind: Some("file".into()),
            source_name: Some("notes".into()),
        })
        .is_err());
    }

    #[tokio::test]
    async fn site_deletion_matches_root_and_www_but_not_subdomains() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        dystil_storage::initialize_capture_schema(&pool)
            .await
            .unwrap();
        for url in [
            "https://example.com/work",
            "https://www.example.com/other",
            "https://private.example.com/work",
        ] {
            sqlx::query(
                "INSERT INTO frames(timestamp,app_name,browser_url,frame_text) VALUES('2026-08-03T09:00:00Z','Browser',?1,'captured text')",
            )
            .bind(url)
            .execute(&pool)
            .await
            .unwrap();
        }
        let scope = ResolvedScope::Site("example.com".into());
        let ids = matching_ids(&pool, "frames", "app_name", &scope)
            .await
            .unwrap();
        assert_eq!(ids.len(), 2);

        assert_eq!(
            delete_ids(&pool, "frames", "frames", &ids).await.unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM frames")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM activity_search_documents")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn captured_duration_ignores_long_idle_gaps() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        dystil_storage::initialize_capture_schema(&pool)
            .await
            .unwrap();
        for timestamp in [
            "2026-08-03T09:00:00Z",
            "2026-08-03T09:01:00Z",
            "2026-08-03T09:20:00Z",
        ] {
            sqlx::query("INSERT INTO frames(timestamp,app_name) VALUES(?1,'Editor')")
                .bind(timestamp)
                .execute(&pool)
                .await
                .unwrap();
        }

        assert_eq!(
            captured_duration_seconds(&pool, &ResolvedScope::All)
                .await
                .unwrap(),
            60
        );
    }
}
