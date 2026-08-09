//! Local-only harness for exercising Dystil's native capture pipeline.
//!
//! This example deliberately owns its startup wiring so it can debug capture
//! independently of the Tauri application and its service dependencies.

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use dystil_capture::{
    a11y::{tree::TreeWalkerConfig, ExtractionThreadPriority},
    accessibility_provider::DystilAccessibilityProvider,
    capture_loop::{DystilAxCaptureConfig, DystilAxCaptureHandle},
    capture_store::DystilCaptureStore,
    linker::DystilLinkerRuntime,
    start_dystil_ui_recording, CaptureConfig, CaptureContext, CaptureCoordinator, CaptureMode,
    CaptureTrigger, CaptureTriggerMessage, DystilUiRecorderConfig, TriggerBus, VisualProvider,
    TRIGGER_CHANNEL_BUFFER,
};
use sqlx::{Row, SqlitePool};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[cfg(any(target_os = "linux", target_os = "windows"))]
use dystil_capture::non_macos_visual_capture::DystilFullCaptureVisualProvider;
#[cfg(target_os = "macos")]
use dystil_capture::visual_capture::DystilMacosOneShotVisualProvider;

#[derive(Debug)]
struct Args {
    data_dir: PathBuf,
    text_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct Baseline {
    frame_id: i64,
    event_id: i64,
}

fn main() -> Result<()> {
    init_logging();
    let args = parse_args()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")?;
    runtime.block_on(run(args))
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,dystil_capture=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

fn parse_args() -> Result<Args> {
    let mut data_dir = None;
    let mut text_only = false;
    let mut args = env::args_os().skip(1);

    while let Some(argument) = args.next() {
        match argument.to_string_lossy().as_ref() {
            "--text-only" => text_only = true,
            "--data-dir" => {
                if data_dir.is_some() {
                    bail!("--data-dir may only be supplied once\n\n{}", usage());
                }
                let Some(value) = args.next() else {
                    bail!("--data-dir requires a path\n\n{}", usage());
                };
                if value.to_string_lossy().starts_with("--") {
                    bail!("--data-dir requires a path\n\n{}", usage());
                }
                data_dir = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}\n\n{}", usage()),
        }
    }

    Ok(Args {
        data_dir: data_dir.unwrap_or_else(default_run_directory),
        text_only,
    })
}

fn usage() -> &'static str {
    "Usage: cargo run -p dystil-capture --example local_capture --features native -- [--text-only] [--data-dir <path>]\n\n\
     Starts local capture until Ctrl-C. Without --data-dir, the run is stored in a unique directory below the system temp directory."
}

fn default_run_directory() -> PathBuf {
    let name = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        std::process::id()
    );
    env::temp_dir().join("dystil-capture-debug").join(name)
}

async fn run(args: Args) -> Result<()> {
    fs::create_dir_all(&args.data_dir)
        .with_context(|| format!("failed to create {}", args.data_dir.display()))?;
    let data_dir = fs::canonicalize(&args.data_dir)
        .with_context(|| format!("failed to resolve {}", args.data_dir.display()))?;
    let snapshot_root = data_dir.join("data");
    let database_path = data_dir.join("db.sqlite");
    let capture_mode = if args.text_only {
        CaptureMode::TextOnly
    } else {
        CaptureMode::FullCapture
    };

    println!("Local Dystil capture debug harness");
    println!(
        "  mode: {}",
        if args.text_only { "text-only" } else { "full" }
    );
    println!("  database: {}", database_path.display());
    println!("  snapshots: {}", snapshot_root.display());

    let pool = dystil_storage::open_capture_database(&database_path)
        .await
        .with_context(|| format!("failed to open {}", database_path.display()))?;
    let baseline = read_baseline(&pool).await?;

    let trigger_bus = TriggerBus::<CaptureTriggerMessage>::new(TRIGGER_CHANNEL_BUFFER);
    let linker = DystilLinkerRuntime::start(pool.clone());
    let accessibility = Arc::new(DystilAccessibilityProvider::new(TreeWalkerConfig::default()));
    let store = Arc::new(DystilCaptureStore::new(
        pool.clone(),
        &snapshot_root,
        "capture-debug",
        false,
    ));
    let coordinator = Arc::new(CaptureCoordinator::new(
        CaptureConfig { capture_mode },
        accessibility,
        visual_provider(args.text_only),
        store,
    ));
    let capture_handle = DystilAxCaptureHandle::start(
        trigger_bus.subscribe(),
        linker.sender(),
        coordinator.clone(),
        DystilAxCaptureConfig::default(),
    );

    let ui_recorder = match start_dystil_ui_recording(
        pool.clone(),
        ui_recorder_config(),
        trigger_bus.sender(),
        linker.sender(),
    ) {
        Ok(handle) => {
            info!("UI event recording started");
            Some(handle)
        }
        Err(error) => {
            warn!(%error, "UI event recording unavailable; continuing with manual capture diagnostics");
            None
        }
    };

    verify_manual_capture(&pool, &coordinator, args.text_only).await;

    println!("Capture running. Press Ctrl-C to stop.");
    tokio::signal::ctrl_c()
        .await
        .context("failed to wait for Ctrl-C")?;

    if let Some(handle) = ui_recorder {
        handle.stop();
        if tokio::time::timeout(Duration::from_secs(5), handle.join())
            .await
            .is_err()
        {
            warn!("UI recorder did not stop within 5 seconds");
        }
    }
    capture_handle.shutdown().await;
    linker.shutdown().await;
    print_run_summary(&pool, baseline).await?;
    pool.close().await;
    println!("  database retained at: {}", database_path.display());
    println!("  snapshots retained at: {}", snapshot_root.display());
    Ok(())
}

fn visual_provider(text_only: bool) -> Option<Arc<dyn VisualProvider>> {
    if text_only {
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        Some(Arc::new(DystilMacosOneShotVisualProvider::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )))
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        Some(Arc::new(DystilFullCaptureVisualProvider::new()))
    }
}

fn ui_recorder_config() -> DystilUiRecorderConfig {
    DystilUiRecorderConfig {
        capture_clicks: true,
        capture_scroll: false,
        capture_clipboard: true,
        capture_clipboard_content: false,
        capture_text: false,
        capture_keystrokes: true,
        record_keyboard_events: false,
        record_clipboard_events: false,
        ignored_windows: Vec::new(),
        included_windows: Vec::new(),
        batch_size: 100,
        batch_timeout_ms: 1_000,
        typing_pause_delay_ms: 1_500,
        prioritize_input_latency: false,
        extraction_thread_priority: ExtractionThreadPriority::default(),
        pause_extraction_on_input_ms: 0,
    }
}

async fn verify_manual_capture(
    pool: &SqlitePool,
    coordinator: &CaptureCoordinator,
    text_only: bool,
) {
    println!("Manual capture:");
    let stored = match coordinator
        .capture(CaptureTrigger::Manual, CaptureContext::default())
        .await
    {
        Ok(stored) => stored,
        Err(error) => {
            println!("  result: failed ({error})");
            return;
        }
    };
    println!("  frame_id: {}", stored.frame_id);

    let row = match sqlx::query("SELECT snapshot_path, frame_text FROM frames WHERE id = ?")
        .bind(stored.frame_id)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            println!("  frame row: missing");
            return;
        }
        Err(error) => {
            println!("  frame row: query failed ({error})");
            return;
        }
    };

    let snapshot_path: String = row.get("snapshot_path");
    let frame_text: Option<String> = row.get("frame_text");
    println!(
        "  accessibility text: {}",
        if frame_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            "stored"
        } else {
            "not available"
        }
    );

    if text_only {
        if snapshot_path.is_empty() {
            println!("  screenshot: correctly disabled");
        } else {
            println!("  screenshot: FAILED (unexpected path: {snapshot_path})");
        }
        return;
    }

    if snapshot_path.is_empty() {
        println!("  screenshot: FAILED (no snapshot path; check screen-recording permission/provider logs)");
        return;
    }
    println!("  screenshot path: {snapshot_path}");
    match image::open(&snapshot_path) {
        Ok(image) => println!(
            "  jpeg decode: passed ({}x{})",
            image.width(),
            image.height()
        ),
        Err(error) => println!("  jpeg decode: FAILED ({error})"),
    }
}

async fn read_baseline(pool: &SqlitePool) -> Result<Baseline> {
    Ok(Baseline {
        frame_id: sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM frames")
            .fetch_one(pool)
            .await
            .context("failed to read frame baseline")?,
        event_id: sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM ui_events")
            .fetch_one(pool)
            .await
            .context("failed to read UI-event baseline")?,
    })
}

async fn print_run_summary(pool: &SqlitePool, baseline: Baseline) -> Result<()> {
    let frames: i64 = count(
        pool,
        "SELECT COUNT(*) FROM frames WHERE id > ?",
        baseline.frame_id,
    )
    .await?;
    let screenshots: i64 = count(
        pool,
        "SELECT COUNT(*) FROM frames WHERE id > ? AND snapshot_path <> ''",
        baseline.frame_id,
    )
    .await?;
    let accessibility: i64 = count(pool, "SELECT COUNT(*) FROM frames WHERE id > ? AND frame_text IS NOT NULL AND TRIM(frame_text) <> ''", baseline.frame_id).await?;
    let events: i64 = count(
        pool,
        "SELECT COUNT(*) FROM ui_events WHERE id > ?",
        baseline.event_id,
    )
    .await?;
    let linked_events: i64 = count(
        pool,
        "SELECT COUNT(*) FROM ui_events WHERE id > ? AND frame_id IS NOT NULL",
        baseline.event_id,
    )
    .await?;

    println!("Capture summary (this run):");
    println!("  frames: {frames}");
    println!("  frames with screenshots: {screenshots}");
    println!("  frames with accessibility text: {accessibility}");
    println!("  UI events: {events}");
    println!("  UI events linked to frames: {linked_events}");

    let trigger_rows = sqlx::query(
        "SELECT COALESCE(capture_trigger, 'unknown') AS trigger, COUNT(*) AS count \
         FROM frames WHERE id > ? GROUP BY capture_trigger ORDER BY trigger",
    )
    .bind(baseline.frame_id)
    .fetch_all(pool)
    .await
    .context("failed to summarize capture triggers")?;
    if trigger_rows.is_empty() {
        println!("  triggers: none");
    } else {
        println!("  triggers:");
        for row in trigger_rows {
            let trigger: String = row.get("trigger");
            let count: i64 = row.get("count");
            println!("    {trigger}: {count}");
        }
    }

    let paths = sqlx::query_scalar::<_, String>(
        "SELECT snapshot_path FROM frames WHERE id > ? AND snapshot_path <> ''",
    )
    .bind(baseline.frame_id)
    .fetch_all(pool)
    .await
    .context("failed to inspect snapshot paths")?;
    let mut missing = 0;
    let mut invalid = 0;
    for path in paths {
        if !Path::new(&path).is_file() {
            missing += 1;
        } else if image::open(&path).is_err() {
            invalid += 1;
        }
    }
    println!("  missing snapshot files: {missing}");
    println!("  invalid snapshot files: {invalid}");
    Ok(())
}

async fn count(pool: &SqlitePool, query: &str, baseline_id: i64) -> Result<i64> {
    sqlx::query_scalar(query)
        .bind(baseline_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to run summary query: {query}"))
}
