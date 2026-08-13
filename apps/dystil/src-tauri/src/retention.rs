use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dystil_telemetry::{ErrorKind, Outcome, StorageOperationKind, Telemetry};
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, State};
use tracing::{info, warn};

use crate::recording::RecordingState;
use crate::store::SettingsStore;

const ALLOWED_RETENTION_DAYS: [u32; 6] = [7, 14, 30, 90, 365, 0];
const CLEANUP_BATCH_SIZE: i64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RetentionStorageView {
    pub retention_days: u32,
    pub total_data_size: String,
    pub total_data_bytes: u64,
    pub available_space: String,
    pub available_space_bytes: u64,
    pub fixed_bytes: u64,
    pub daily_history_bytes: u64,
    pub observed_days: u32,
    pub estimate_is_early: bool,
}

fn validate_retention_days(days: u32) -> Result<u32, String> {
    ALLOWED_RETENTION_DAYS
        .contains(&days)
        .then_some(days)
        .ok_or_else(|| {
            "retention must be 1 week, 2 weeks, 1 month, 3 months, 1 year, or forever".to_string()
        })
}

async fn runtime_paths(state: &RecordingState) -> Result<(SqlitePool, PathBuf, PathBuf), String> {
    state
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| {
            (
                server.db.pool.clone(),
                server.data_dir.clone(),
                server.data_path.clone(),
            )
        })
        .ok_or_else(|| "local capture database is not ready".to_string())
}

async fn storage_view(
    app: &AppHandle,
    state: &RecordingState,
    force_refresh: bool,
) -> Result<RetentionStorageView, String> {
    let (_, data_dir, _) = runtime_paths(state).await?;
    let usage = crate::disk_usage::disk_usage(&data_dir, force_refresh)
        .await?
        .ok_or_else(|| "storage usage is not available".to_string())?;
    let settings = SettingsStore::get(app)?.unwrap_or_default();

    // The capture database and media are the footprint that grows with raw
    // history. Everything else (models, runtimes, logs and app files) is a
    // fixed local footprint for projection purposes.
    let capture_start = usage
        .recording_since
        .as_deref()
        .and_then(parse_capture_date);
    let history_bytes = if capture_start.is_some() {
        usage
            .database_size_bytes
            .saturating_add(usage.media_size_bytes)
    } else {
        0
    };
    let fixed_bytes = usage.total_data_bytes.saturating_sub(history_bytes);
    let observed_seconds = capture_start
        .map(|start| Utc::now().signed_duration_since(start).num_seconds().max(0) as f64)
        .unwrap_or(0.0);
    let observed_days_fractional = (observed_seconds / 86_400.0).max(1.0);
    let observed_days = observed_days_fractional.ceil() as u32;
    let daily_history_bytes = if capture_start.is_some() {
        (history_bytes as f64 / observed_days_fractional).round() as u64
    } else {
        0
    };

    Ok(RetentionStorageView {
        retention_days: settings.retention_days,
        total_data_size: usage.total_data_size,
        total_data_bytes: usage.total_data_bytes,
        available_space: usage.available_space,
        available_space_bytes: usage.available_space_bytes,
        fixed_bytes,
        daily_history_bytes,
        observed_days,
        estimate_is_early: observed_days < 7,
    })
}

fn parse_capture_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map(|date| {
                DateTime::<Utc>::from_naive_utc_and_offset(date.and_hms_opt(0, 0, 0).unwrap(), Utc)
            })
        })
        .ok()
}

#[tauri::command]
#[specta::specta]
pub async fn get_retention_storage(
    app: AppHandle,
    state: State<'_, RecordingState>,
    force_refresh: Option<bool>,
) -> Result<RetentionStorageView, String> {
    storage_view(&app, &state, force_refresh.unwrap_or(false)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_retention_days(
    app: AppHandle,
    state: State<'_, RecordingState>,
    retention_days: u32,
) -> Result<RetentionStorageView, String> {
    let retention_days = validate_retention_days(retention_days)?;
    let (pool, _, media_dir) = runtime_paths(&state).await?;
    let mut settings = SettingsStore::get(&app)?.unwrap_or_default();
    settings.retention_days = retention_days;
    settings.save(&app)?;

    if retention_days > 0 {
        // The policy is already persisted. A transient database lock should
        // not make the UI claim the choice was rejected; housekeeping retries
        // daily and the next manual apply/refresh can run another pass.
        let telemetry = {
            state
                .server
                .lock()
                .await
                .as_ref()
                .map(|server| server.telemetry.clone())
        };
        if let Err(error) =
            cleanup_expired_raw_history(&pool, &media_dir, retention_days, telemetry.as_deref())
                .await
        {
            warn!(error = %error, "retention was saved; immediate cleanup will be retried");
        }
    }
    storage_view(&app, &state, true).await
}

pub fn start_housekeeping(
    app: AppHandle,
    pool: SqlitePool,
    media_dir: PathBuf,
    telemetry: std::sync::Arc<Telemetry>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        loop {
            interval.tick().await;
            let days = SettingsStore::get(&app)
                .ok()
                .flatten()
                .unwrap_or_default()
                .retention_days;
            if days == 0 {
                continue;
            }
            match cleanup_expired_raw_history(&pool, &media_dir, days, Some(&telemetry)).await {
                Ok(()) => {
                    telemetry.record_storage_operation(
                        StorageOperationKind::RetentionCleanup,
                        Outcome::Succeeded,
                        None,
                    );
                }
                Err(error) => {
                    telemetry.record_storage_operation(
                        StorageOperationKind::RetentionCleanup,
                        Outcome::Failed,
                        Some(ErrorKind::Database),
                    );
                    warn!(error = %error, "retention cleanup failed");
                }
            }
        }
    });
}

pub async fn cleanup_expired_raw_history(
    pool: &SqlitePool,
    media_dir: &Path,
    retention_days: u32,
    telemetry: Option<&Telemetry>,
) -> Result<(), String> {
    validate_retention_days(retention_days)?;
    if retention_days == 0 {
        return Ok(());
    }

    let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
    let cutoff = cutoff.to_rfc3339();
    let snapshot_rows = sqlx::query(
        "SELECT DISTINCT snapshot_path FROM frames expired
         WHERE expired.timestamp < ?1 AND trim(coalesce(expired.snapshot_path, '')) != ''
           AND NOT EXISTS (
             SELECT 1 FROM frames retained
             WHERE retained.snapshot_path = expired.snapshot_path AND retained.timestamp >= ?1
           )",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let mut deleted_rows = 0u64;
    loop {
        let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
        sqlx::query(
            "DELETE FROM dystil_text_redaction_state
             WHERE source_table = 'frames' AND source_row_id IN
               (SELECT id FROM frames WHERE timestamp < ?1 ORDER BY timestamp, id LIMIT ?2)",
        )
        .bind(&cutoff)
        .bind(CLEANUP_BATCH_SIZE)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        let result = sqlx::query(
            "DELETE FROM frames WHERE id IN
               (SELECT id FROM frames WHERE timestamp < ?1 ORDER BY timestamp, id LIMIT ?2)",
        )
        .bind(&cutoff)
        .bind(CLEANUP_BATCH_SIZE)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        tx.commit().await.map_err(|error| error.to_string())?;
        let count = result.rows_affected();
        deleted_rows += count;
        if count < CLEANUP_BATCH_SIZE as u64 {
            break;
        }
        tokio::task::yield_now().await;
    }

    loop {
        let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
        sqlx::query(
            "DELETE FROM dystil_text_redaction_state
             WHERE source_table = 'ui_events' AND source_row_id IN
               (SELECT id FROM ui_events WHERE timestamp < ?1 ORDER BY timestamp, id LIMIT ?2)",
        )
        .bind(&cutoff)
        .bind(CLEANUP_BATCH_SIZE)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        let result = sqlx::query(
            "DELETE FROM ui_events WHERE id IN
               (SELECT id FROM ui_events WHERE timestamp < ?1 ORDER BY timestamp, id LIMIT ?2)",
        )
        .bind(&cutoff)
        .bind(CLEANUP_BATCH_SIZE)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        tx.commit().await.map_err(|error| error.to_string())?;
        let count = result.rows_affected();
        deleted_rows += count;
        if count < CLEANUP_BATCH_SIZE as u64 {
            break;
        }
        tokio::task::yield_now().await;
    }

    let canonical_media_dir = std::fs::canonicalize(media_dir).ok();
    let mut deleted_files = 0u64;
    for row in snapshot_rows {
        let path: String = row
            .try_get("snapshot_path")
            .map_err(|error| error.to_string())?;
        let path = PathBuf::from(path);
        let safe = canonical_media_dir
            .as_ref()
            .and_then(|root| {
                std::fs::canonicalize(&path)
                    .ok()
                    .map(|candidate| candidate.starts_with(root))
            })
            .unwrap_or(false);
        if !safe {
            warn!(path = %path.display(), "refusing to delete capture media outside Dystil's data directory");
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => deleted_files += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(path = %path.display(), error = %error, "could not delete expired capture media")
            }
        }
    }

    if deleted_rows > 0 {
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(pool)
            .await;
        if let Err(error) = sqlx::query("VACUUM").execute(pool).await {
            if let Some(telemetry) = telemetry {
                telemetry.record_storage_operation(
                    StorageOperationKind::DatabaseCompaction,
                    Outcome::Failed,
                    Some(ErrorKind::Database),
                );
            }
            warn!(error = %error, "expired history was removed but database compaction was deferred");
        } else if let Some(telemetry) = telemetry {
            telemetry.record_storage_operation(
                StorageOperationKind::DatabaseCompaction,
                Outcome::Succeeded,
                None,
            );
        }
    }
    info!(
        retention_days,
        deleted_rows, deleted_files, "retention cleanup completed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn accepts_only_ui_retention_choices() {
        for days in ALLOWED_RETENTION_DAYS {
            assert_eq!(validate_retention_days(days).unwrap(), days);
        }
        assert!(validate_retention_days(8).is_err());
    }

    #[tokio::test]
    async fn deletes_expired_raw_capture_and_keeps_derived_data() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        dystil_storage::initialize_capture_schema(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE findings (id INTEGER PRIMARY KEY, body TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO findings(id, body) VALUES (1, 'keep me')")
            .execute(&pool)
            .await
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let media_dir = dir.path().join("data");
        std::fs::create_dir_all(&media_dir).unwrap();
        let old_image = media_dir.join("old.jpg");
        let new_image = media_dir.join("new.jpg");
        std::fs::write(&old_image, b"old").unwrap();
        std::fs::write(&new_image, b"new").unwrap();
        let old_timestamp = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        let new_timestamp = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO frames(id, timestamp, snapshot_path, frame_text)
             VALUES (1, ?1, ?2, 'expired text'), (2, ?3, ?4, 'current text')",
        )
        .bind(&old_timestamp)
        .bind(old_image.to_string_lossy().as_ref())
        .bind(&new_timestamp)
        .bind(new_image.to_string_lossy().as_ref())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ui_events(id, timestamp, event_type, text_content)
             VALUES (1, ?1, 'input', 'expired event'), (2, ?2, 'input', 'current event')",
        )
        .bind(&old_timestamp)
        .bind(&new_timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dystil_text_redaction_state(source_table, source_row_id, surface, status)
             VALUES ('frames', 1, 'frame_text', 'ready'), ('ui_events', 1, 'text_content', 'ready')",
        )
        .execute(&pool)
        .await
        .unwrap();

        cleanup_expired_raw_history(&pool, &media_dir, 7, None)
            .await
            .unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM frames")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ui_events")
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
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dystil_text_redaction_state")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM findings")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert!(!old_image.exists());
        assert!(new_image.exists());
    }
}
