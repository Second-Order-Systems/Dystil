//! Async text-redaction worker.
//!
//! `dystil-redact` applies deterministic regex sanitization synchronously at
//! write time. This worker closes the loop: it polls `dystil_text_redaction_state`
//! for `Pending` rows after an ONNX model is available, overwrites source
//! columns in place, and marks the row
//! `Complete` or `DeterministicFallback` on exhausted retries.
//!
//! To upgrade to an ONNX model: add a new arm in `dispatch_surface` that calls
//! the model, falls back to `sanitize_text` on model error.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use sqlx::{Row, SqlitePool};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use dystil_redact::{record_state, sanitize_text, RedactionStatus, TextRedactor};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const BATCH_SIZE: i64 = 20;
const MAX_ATTEMPTS: u32 = 3;
/// Minimum text length (post-regex) to bother running the ONNX pass.
const MIN_LEN_FOR_ONNX: usize = 12;

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

pub struct RedactionWorker {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    model: Arc<RwLock<Option<Arc<dyn TextRedactor>>>>,
}

impl RedactionWorker {
    /// Start the worker. It waits while `model` is `None`; each row gets a
    /// regex pre-pass followed by an ONNX pass once the model is installed.
    pub fn start(pool: SqlitePool, model: Option<Arc<dyn TextRedactor>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let model = Arc::new(RwLock::new(model));
        let join = tokio::spawn(run_worker(pool, model.clone(), stop.clone()));
        Self {
            stop,
            join: Some(join),
            model,
        }
    }

    /// Replace the optional model without stopping capture or the worker.
    /// Records already persisted safely with deterministic redaction remain
    /// valid; later pending batches use the newly available model.
    pub async fn set_model(&self, model: Arc<dyn TextRedactor>) {
        *self.model.write().await = Some(model);
    }

    pub fn model_handle(&self) -> Arc<RwLock<Option<Arc<dyn TextRedactor>>>> {
        self.model.clone()
    }

    pub async fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
        }
    }
}

impl Drop for RedactionWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

async fn run_worker(
    pool: SqlitePool,
    model: Arc<RwLock<Option<Arc<dyn TextRedactor>>>>,
    stop: Arc<AtomicBool>,
) {
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.tick().await; // discard the immediate first tick

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        tick.tick().await;
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let Some(active_model) = model.read().await.clone() else {
            debug!("redaction worker: waiting for opt-in model");
            continue;
        };
        match process_pending_batch(&pool, active_model.as_ref()).await {
            Ok(0) => debug!("redaction worker: nothing pending"),
            Ok(n) => info!(processed = n, "redaction worker: batch complete"),
            Err(e) => warn!("redaction worker: batch error: {}", e),
        }
    }

    info!("redaction worker: stopped");
}

// ---------------------------------------------------------------------------
// Batch processing
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct PendingRow {
    source_table: String,
    source_row_id: i64,
    surface: String,
    attempts: i64,
}

async fn process_pending_batch(
    pool: &SqlitePool,
    model: &dyn TextRedactor,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query_as::<_, PendingRow>(
        "SELECT source_table, source_row_id, surface, attempts
         FROM dystil_text_redaction_state
         WHERE status = 'pending'
         ORDER BY updated_at ASC
         LIMIT ?",
    )
    .bind(BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    let count = rows.len();
    for row in &rows {
        if let Err(e) = process_row(pool, row, Some(model)).await {
            warn!(
                table = %row.source_table,
                id = row.source_row_id,
                surface = %row.surface,
                "redaction worker: row failed: {}",
                e
            );
        }
    }
    Ok(count)
}

async fn process_row(
    pool: &SqlitePool,
    row: &PendingRow,
    model: Option<&dyn TextRedactor>,
) -> Result<(), String> {
    let attempts = row.attempts as u32 + 1;

    if attempts > MAX_ATTEMPTS {
        set_status(
            pool,
            row,
            RedactionStatus::DeterministicFallback,
            attempts,
            None,
            Some("max attempts exceeded"),
        )
        .await?;
        return Ok(());
    }

    set_status(pool, row, RedactionStatus::Processing, attempts, None, None).await?;

    // Returns Ok(backend_name) on success, Err(message) on failure.
    let result = dispatch_surface(
        pool,
        &row.source_table,
        row.source_row_id,
        &row.surface,
        model,
    )
    .await;

    match result {
        Ok(backend) => {
            set_status(
                pool,
                row,
                RedactionStatus::Complete,
                attempts,
                Some(backend),
                None,
            )
            .await?;
        }
        Err(ref e) => {
            let status = if attempts >= MAX_ATTEMPTS {
                RedactionStatus::DeterministicFallback
            } else {
                // Back to Pending so next poll retries
                RedactionStatus::Pending
            };
            set_status(pool, row, status, attempts, None, Some(e.as_str())).await?;
        }
    }
    Ok(())
}

async fn set_status(
    pool: &SqlitePool,
    row: &PendingRow,
    status: RedactionStatus,
    attempts: u32,
    backend: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    record_state(
        pool,
        &row.source_table,
        row.source_row_id,
        &row.surface,
        status,
        attempts,
        backend.or(Some("deterministic")),
        error,
    )
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Surface dispatch — add new (table, surface) arms here
// Returns the backend name used ("deterministic" | "v45_phase5_pruned")
// ---------------------------------------------------------------------------

async fn dispatch_surface(
    pool: &SqlitePool,
    source_table: &str,
    source_row_id: i64,
    surface: &str,
    model: Option<&dyn TextRedactor>,
) -> Result<&'static str, String> {
    match (source_table, surface) {
        ("frames", "frame_text") => redact_frame(pool, source_row_id, model).await,
        _ => {
            warn!("redaction worker: unknown surface ({source_table}, {surface}), skipping");
            Ok("deterministic")
        }
    }
}

// ---------------------------------------------------------------------------
// Per-table redaction
// ---------------------------------------------------------------------------

/// Apply regex + optional ONNX pass to all text columns in one `frames` row.
/// Returns the backend name actually used.
async fn redact_frame(
    pool: &SqlitePool,
    frame_id: i64,
    model: Option<&dyn TextRedactor>,
) -> Result<&'static str, String> {
    let row = sqlx::query(
        "SELECT frame_text, app_name, window_name, browser_url, document_path, device_name
         FROM frames WHERE id = ?",
    )
    .bind(frame_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let Some(row) = row else {
        return Ok("deterministic"); // frame was deleted, nothing to do
    };

    let frame_text: Option<String> = row.try_get("frame_text").ok().flatten();
    let app_name: Option<String> = row.try_get("app_name").ok().flatten();
    let window_name: Option<String> = row.try_get("window_name").ok().flatten();
    let browser_url: Option<String> = row.try_get("browser_url").ok().flatten();
    let document_path: Option<String> = row.try_get("document_path").ok().flatten();
    let device_name: Option<String> = row.try_get("device_name").ok().flatten();

    // Regex pre-pass (always, free)
    let a_text = frame_text.as_deref().map(sanitize_text);
    let a_app = app_name.as_deref().map(sanitize_text);
    let a_win = window_name.as_deref().map(sanitize_text);
    let a_url = browser_url.as_deref().map(sanitize_text);
    let a_doc = document_path.as_deref().map(sanitize_text);
    let a_dev = device_name.as_deref().map(sanitize_text);

    // ONNX pass on frame_text (richest surface — full window content)
    let (stored_text, backend) = if let Some(m) = model {
        let candidate = a_text.as_deref().unwrap_or("");
        if candidate.len() > MIN_LEN_FOR_ONNX {
            match m.redact(candidate).await {
                Ok(s) => (Some(s), m.name()),
                Err(e) => {
                    warn!("redaction worker: ONNX pass failed for frame {frame_id}: {e}");
                    (a_text.clone(), "deterministic")
                }
            }
        } else {
            (a_text.clone(), "deterministic")
        }
    } else {
        (a_text.clone(), "deterministic")
    };

    sqlx::query(
        "UPDATE frames SET
            frame_text    = ?,
            app_name      = ?,
            window_name   = ?,
            browser_url   = ?,
            document_path = ?,
            device_name   = ?
         WHERE id = ?",
    )
    .bind(stored_text)
    .bind(a_app)
    .bind(a_win)
    .bind(a_url)
    .bind(a_doc)
    .bind(a_dev)
    .bind(frame_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(backend)
}
