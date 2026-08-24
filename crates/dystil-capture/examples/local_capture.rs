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

#[cfg(feature = "debug-capture")]
use sysinfo::{ProcessExt, System, SystemExt};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use dystil_capture::non_macos_visual_capture::DystilFullCaptureVisualProvider;
#[cfg(target_os = "macos")]
use dystil_capture::visual_capture::DystilMacosOneShotVisualProvider;

#[derive(Debug)]
struct Args {
    data_dir: PathBuf,
    text_only: bool,
    capture_scroll: bool,
    diagnostics: bool,
    policy: String,
    measurement_mode: String,
    duration_seconds: Option<u64>,
    stop_file: Option<PathBuf>,
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
    let mut capture_scroll = false;
    let mut diagnostics = false;
    let mut policy = "baseline".to_string();
    let mut measurement_mode = "baseline".to_string();
    let mut duration_seconds = None;
    let mut stop_file = None;
    let mut args = env::args_os().skip(1);

    while let Some(argument) = args.next() {
        match argument.to_string_lossy().as_ref() {
            "--text-only" => text_only = true,
            "--capture-scroll" => capture_scroll = true,
            "--diagnostics" | "--compare" => diagnostics = true,
            "--policy" => {
                let Some(value) = args.next() else {
                    bail!("--policy requires a value\n\n{}", usage());
                };
                policy = value.to_string_lossy().into_owned();
            }
            "--measurement-mode" => {
                let Some(value) = args.next() else {
                    bail!("--measurement-mode requires a value\n\n{}", usage());
                };
                measurement_mode = value.to_string_lossy().into_owned();
            }
            "--duration-seconds" => {
                let Some(value) = args.next() else {
                    bail!(
                        "--duration-seconds requires a positive integer\n\n{}",
                        usage()
                    );
                };
                let parsed = value
                    .to_string_lossy()
                    .parse::<u64>()
                    .context("--duration-seconds must be a positive integer")?;
                if parsed == 0 {
                    bail!("--duration-seconds must be greater than zero");
                }
                duration_seconds = Some(parsed);
            }
            "--stop-file" => {
                let Some(value) = args.next() else {
                    bail!("--stop-file requires a path\n\n{}", usage());
                };
                stop_file = Some(PathBuf::from(value));
            }
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

    if !matches!(
        policy.as_str(),
        "baseline"
            | "stage1_no_background_trees"
            | "stage2_click_coalesce"
            | "stage3_settled_state"
            | "stage4_visible_relevant"
    ) {
        bail!("unknown policy '{policy}'; expected baseline, stage1_no_background_trees, stage2_click_coalesce, stage3_settled_state, or stage4_visible_relevant");
    }
    if measurement_mode != "baseline" && measurement_mode != "matched_ab" {
        bail!("unknown measurement mode '{measurement_mode}'; expected baseline or matched_ab");
    }
    if duration_seconds.is_some() && stop_file.is_some() {
        bail!("--duration-seconds and --stop-file cannot be used together");
    }
    Ok(Args {
        data_dir: data_dir.unwrap_or_else(default_run_directory),
        text_only,
        capture_scroll,
        diagnostics,
        policy,
        measurement_mode,
        duration_seconds,
        stop_file,
    })
}

fn usage() -> &'static str {
    "Usage: cargo run -p dystil-capture --example local_capture --features native,debug-capture -- [--text-only] [--capture-scroll] [--diagnostics|--compare] [--policy baseline|stage1_no_background_trees|stage2_click_coalesce|stage3_settled_state|stage4_visible_relevant] [--measurement-mode baseline|matched_ab] [--duration-seconds <n>|--stop-file <path>] [--data-dir <path>]\n\n\
     Starts local capture until Ctrl-C. Without --data-dir, the run is stored in a unique directory below the system temp directory. Diagnostics are local-only and require the debug-capture feature."
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
    println!("  policy: {}", args.policy);
    println!("  measurement: {}", args.measurement_mode);
    println!("  database: {}", database_path.display());
    println!("  snapshots: {}", snapshot_root.display());

    let pool = dystil_storage::open_capture_database(&database_path)
        .await
        .with_context(|| format!("failed to open {}", database_path.display()))?;
    let baseline = read_baseline(&pool).await?;

    #[cfg(not(feature = "debug-capture"))]
    if args.diagnostics {
        bail!("--diagnostics requires --features native,debug-capture");
    }

    #[cfg(feature = "debug-capture")]
    let diagnostic_session = if args.diagnostics {
        let run_id = data_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("capture-debug")
            .to_string();
        Some(
            dystil_capture::debug_capture::DebugCaptureSession::start(
                dystil_capture::debug_capture::DebugCaptureConfig {
                    run_dir: data_dir.clone(),
                    run_id,
                    policy: args.policy.clone(),
                    measurement_mode: args.measurement_mode.clone(),
                    baseline_frame_id: baseline.frame_id,
                    baseline_event_id: baseline.event_id,
                    remote_writes: false,
                    uploads: false,
                    database_path: Some(database_path.clone()),
                },
            )
            .map_err(anyhow::Error::msg)?,
        )
    } else {
        None
    };

    #[cfg(feature = "debug-capture")]
    let (process_stop_tx, process_sampler) = if args.diagnostics {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        (
            Some(stop_tx),
            Some(tokio::spawn(sample_process_resources(
                database_path.clone(),
                stop_rx,
            ))),
        )
    } else {
        (None, None)
    };

    let trigger_bus = TriggerBus::<CaptureTriggerMessage>::new(TRIGGER_CHANNEL_BUFFER);
    let linker = DystilLinkerRuntime::start(pool.clone());
    let accessibility = DystilAccessibilityProvider::new(tree_walker_config(&args.policy));
    let accessibility = if args.policy == "stage4_visible_relevant" {
        accessibility.with_visible_relevant_projection()
    } else {
        accessibility
    };
    let accessibility = Arc::new(accessibility);
    let store = Arc::new(DystilCaptureStore::new(
        pool.clone(),
        &snapshot_root,
        "capture-debug",
        false,
    ));
    let coordinator = CaptureCoordinator::new(
        CaptureConfig { capture_mode },
        accessibility,
        visual_provider(args.text_only),
        store,
    );
    let coordinator = if args.policy == "stage4_visible_relevant" {
        coordinator.with_exact_surface_reuse()
    } else {
        coordinator
    };
    let coordinator = Arc::new(coordinator);
    let mut capture_loop_config = DystilAxCaptureConfig::default();
    capture_loop_config.settled_state = matches!(
        args.policy.as_str(),
        "stage3_settled_state" | "stage4_visible_relevant"
    );
    capture_loop_config.app_cadence_guard =
        matches!(args.policy.as_str(), "stage4_visible_relevant");
    if capture_loop_config.settled_state {
        capture_loop_config.activity_span_pool = Some(pool.clone());
        capture_loop_config.activity_span_session_id =
            Some(format!("stage3-{}", uuid::Uuid::new_v4()));
    }
    let capture_handle = DystilAxCaptureHandle::start(
        trigger_bus.subscribe(),
        linker.sender(),
        coordinator.clone(),
        capture_loop_config,
    );

    let ui_recorder = match start_dystil_ui_recording(
        pool.clone(),
        ui_recorder_config(&args.policy, args.capture_scroll),
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

    if let Some(seconds) = args.duration_seconds {
        println!("Capture running for {seconds} seconds.");
        tokio::time::sleep(Duration::from_secs(seconds)).await;
    } else if let Some(stop_file) = args.stop_file {
        println!(
            "Capture running until stop file exists: {}",
            stop_file.display()
        );
        while !stop_file.is_file() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    } else {
        println!("Capture running. Press Ctrl-C to stop.");
        tokio::signal::ctrl_c()
            .await
            .context("failed to wait for Ctrl-C")?;
    }

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
    #[cfg(feature = "debug-capture")]
    {
        if let Some(stop_tx) = process_stop_tx {
            let _ = stop_tx.send(true);
        }
        if let Some(task) = process_sampler {
            let _ = task.await;
        }
    }
    print_run_summary(&pool, baseline).await?;
    pool.close().await;
    println!("  database retained at: {}", database_path.display());
    println!("  snapshots retained at: {}", snapshot_root.display());
    #[cfg(feature = "debug-capture")]
    if diagnostic_session.is_some() {
        println!("  diagnostics retained at: {}", data_dir.display());
        drop(diagnostic_session);
    }
    Ok(())
}

#[cfg(feature = "debug-capture")]
async fn sample_process_resources(
    database_path: PathBuf,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let Some(pid) = sysinfo::get_current_pid().ok() else {
        return;
    };
    let mut system = System::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            _ = tick.tick() => {
                system.refresh_process(pid);
                if let Some(process) = system.process(pid) {
                    dystil_capture::debug_capture::record_process_sample(
                        process.cpu_usage(),
                        process.memory(),
                        sqlite_bytes(&database_path),
                    );
                }
            }
        }
    }
}

#[cfg(feature = "debug-capture")]
fn sqlite_bytes(database_path: &Path) -> u64 {
    let mut total = database_path
        .metadata()
        .map(|value| value.len())
        .unwrap_or(0);
    let path = database_path.to_string_lossy();
    for suffix in ["-wal", "-shm"] {
        total = total.saturating_add(
            PathBuf::from(format!("{path}{suffix}"))
                .metadata()
                .map(|value| value.len())
                .unwrap_or(0),
        );
    }
    total
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

fn ui_recorder_config(policy: &str, capture_scroll: bool) -> DystilUiRecorderConfig {
    DystilUiRecorderConfig {
        capture_clicks: true,
        capture_scroll,
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
        merge_click_enrichment: matches!(
            policy,
            "stage2_click_coalesce" | "stage3_settled_state" | "stage4_visible_relevant"
        ),
        settled_state_scheduler: matches!(
            policy,
            "stage3_settled_state" | "stage4_visible_relevant"
        ),
        scroll_stop_delay_ms: if matches!(
            policy,
            "stage3_settled_state" | "stage4_visible_relevant"
        ) {
            // The Stage 3 scheduler owns the full 2.5-second quiet period.
            // Emit the recorder's aggregate promptly so it is not added twice.
            0
        } else {
            300
        },
        capture_background_trees: policy == "baseline",
        precise_click_window_context: policy == "stage4_visible_relevant",
    }
}

fn tree_walker_config(policy: &str) -> TreeWalkerConfig {
    let mut config = TreeWalkerConfig::default();
    if policy == "stage4_visible_relevant" {
        config.prefer_incremental_chromium_walk = true;
    }
    config
}

async fn verify_manual_capture(
    pool: &SqlitePool,
    coordinator: &CaptureCoordinator,
    text_only: bool,
) {
    println!("Manual capture:");
    #[cfg(feature = "debug-capture")]
    let diagnostic_started = std::time::Instant::now();
    #[cfg(feature = "debug-capture")]
    let diagnostic_id = dystil_capture::debug_capture::record_capture_request(
        &CaptureTrigger::Manual,
        &CaptureContext::default(),
        0,
        false,
    );
    let result = coordinator
        .capture(CaptureTrigger::Manual, CaptureContext::default())
        .await;
    #[cfg(feature = "debug-capture")]
    match &result {
        Ok(stored) => dystil_capture::debug_capture::record_capture_result(
            diagnostic_id,
            diagnostic_started,
            Some(stored.frame_id),
            None,
            None,
        ),
        Err(error) => dystil_capture::debug_capture::record_capture_result(
            diagnostic_id,
            diagnostic_started,
            None,
            Some("error"),
            Some(&error.to_string()),
        ),
    }
    let stored = match result {
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
