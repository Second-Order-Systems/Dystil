//! Tauri commands for managing the dystil server and capture session.
//!
//! Two independent lifecycles:
//! - **Runtime** (SQLite and shared state): started once, lives until app quits.
//! - **Capture** (visual + accessibility UI): can be toggled without reopening the DB.

use crate::capture_config::DystilCaptureConfig;
use crate::capture_session::CaptureSession;
use crate::config;
use crate::permissions::do_permissions_check;
use crate::server_core::ServerCore;
use crate::store::SettingsStore;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

static PAUSE_TIMER: Lazy<StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    Lazy::new(|| StdMutex::new(None));

fn cancel_pause_timer() {
    if let Some(handle) = PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        handle.abort();
    }
}

pub fn persisted_pause_deadline(settings: &SettingsStore) -> Option<DateTime<Utc>> {
    settings
        .capture_pause_until
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

/// Clear an expired, missing, or malformed timed pause before startup decides
/// whether to create a capture session.
pub fn normalize_pause_for_startup(settings: &mut SettingsStore) -> bool {
    if !settings.capture_paused {
        settings.capture_pause_until = None;
        return false;
    }
    if persisted_pause_deadline(settings).is_none_or(|deadline| deadline <= Utc::now()) {
        settings.capture_paused = false;
        settings.capture_pause_until = None;
        return false;
    }
    true
}

fn schedule_pause_resume(app: tauri::AppHandle, deadline: DateTime<Utc>) {
    cancel_pause_timer();
    let delay = (deadline - Utc::now())
        .to_std()
        .unwrap_or_else(|_| Duration::from_secs(0));
    let handle = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        // Detach this task's handle before resuming so the resume path does not
        // abort the task that is currently running.
        PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner()).take();
        match resume_capture_from_pause_inner(app.clone()).await {
            Ok(()) => {
                crate::notifications::client::send("Capture resumed", "Dystil is recording again.")
            }
            Err(error) => {
                error!(%error, "failed to auto-resume capture after timed pause");
                let retry_at = Utc::now() + chrono::Duration::minutes(1);
                schedule_pause_resume(app, retry_at);
            }
        }
    });
    *PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

pub fn restore_pause_timer(app: tauri::AppHandle, settings: &SettingsStore) {
    if settings.capture_paused {
        if let Some(deadline) = persisted_pause_deadline(settings) {
            schedule_pause_resume(app, deadline);
        }
    }
}

pub async fn pause_capture_until(
    app: tauri::AppHandle,
    deadline: DateTime<Utc>,
) -> Result<(), String> {
    let previous = SettingsStore::get(&app)?.unwrap_or_default();
    let mut updated = previous.clone();
    updated.capture_paused = true;
    updated.capture_pause_until = Some(deadline.to_rfc3339());
    updated.save(&app)?;

    let state = app.state::<RecordingState>();
    if let Err(error) = stop_capture(state, app.clone()).await {
        let _ = previous.save(&app);
        return Err(error);
    }
    cancel_pause_timer();
    schedule_pause_resume(app.clone(), deadline);
    notify_recording_state_changed(&app);
    Ok(())
}

async fn resume_capture_from_pause_inner(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let server_running = state.server.lock().await.is_some();
    if server_running {
        start_capture(state, app.clone()).await?;
    } else {
        spawn_capture(state, app.clone(), None).await?;
    }

    let mut settings = SettingsStore::get(&app)?.unwrap_or_default();
    settings.capture_paused = false;
    settings.capture_pause_until = None;
    if let Err(error) = settings.save(&app) {
        // Do not leave capture running if its persisted pause could not be
        // cleared; otherwise restart would unexpectedly re-apply the pause.
        let state = app.state::<RecordingState>();
        let _ = stop_capture(state, app.clone()).await;
        return Err(error);
    }
    notify_recording_state_changed(&app);
    Ok(())
}

pub async fn resume_capture_from_pause(app: tauri::AppHandle) -> Result<(), String> {
    cancel_pause_timer();
    resume_capture_from_pause_inner(app).await
}

/// Build a `DystilCaptureConfig` from the current settings store.
fn build_config(app: &tauri::AppHandle) -> Result<DystilCaptureConfig, String> {
    let store = SettingsStore::get(app).ok().flatten().unwrap_or_default();
    let (data_dir, _) = config::resolve_data_dir(&store.data_dir);
    Ok(store.to_dystil_capture_config(data_dir))
}

pub(crate) fn notify_recording_state_changed(app: &tauri::AppHandle) {
    let _ = app.emit("recording-status-changed", ());
    #[cfg(feature = "cloud-sync")]
    crate::capture_state_reporter::schedule();
    let app_clone = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = crate::tray::force_tray_rebuild(&app_clone) {
            error!("tray rebuild failed after recording state change: {}", e);
        }
    });
}

/// Minimum seconds between consecutive stop→spawn cycles.
const RESTART_COOLDOWN_SECS: u64 = 30;
/// Two-phase state: server (long-lived) + capture (togglable).
///
/// **Lock ordering**: `capture` may be locked independently (it's self-contained).
/// When both locks are needed (e.g. `start_capture`), always lock `capture` first,
/// then `server`. Never hold `server` while waiting on `capture`.
pub struct RecordingState {
    /// Long-lived server core (DB, HTTP, pipes). None until first start.
    pub server: Arc<Mutex<Option<ServerCore>>>,
    /// Current capture session. None when recording is stopped/paused.
    /// Self-contained — `CaptureSession::stop()` needs no external references.
    pub capture: Arc<Mutex<Option<CaptureSession>>>,
    /// Lock-free mirror of `capture`. This is the source of truth for UI
    /// surfaces (especially the macOS menu bar), where waiting for the async
    /// capture mutex would block AppKit and `try_lock` can return a stale
    /// fallback while a start/stop transition holds the mutex.
    pub capture_active: Arc<AtomicBool>,
    /// True while a server start is in progress (prevents race between main.rs boot and frontend)
    pub is_starting: Arc<AtomicBool>,
    /// True while a `start_capture` invocation is in flight. The frontend
    /// mounts `<DeeplinkHandler />` in every webview window, and the tray
    /// emits `shortcut-start-recording` app-wide — every listening window
    /// fires `commands.startCapture()` simultaneously. Without this guard,
    /// concurrent calls both pass the is_some() check, both build a
    /// CaptureSession, and the second clobbers the first — dropping the
    /// first runs its shutdown handlers and tears down workers shared with
    /// the second, surfacing as a PoolClosed cascade and lost capture rows.
    pub is_starting_capture: Arc<AtomicBool>,
    /// Epoch seconds of last successful spawn — enforces cooldown between restarts
    pub last_spawn_epoch: Arc<AtomicU64>,
    /// App-scoped cloud-auth token (Clerk JWT). Outlives the Server (which
    /// is recreated on every recording restart) so that writes from the
    /// `set_cloud_token` Tauri command — pushed by the frontend on every
    /// sign-in / sign-out — survive capture toggles. The Server's own
    /// `cloud_token` field is replaced with this same Arc at start, and
    /// `PiExecutor` is constructed with `with_shared_user_token(this)`, so
    /// one update propagates to all three readers (cloud_proxy.rs, the
    /// pi-agent's models.json apiKey, and any future Tauri-side consumer).
    pub cloud_token: Arc<arc_swap::ArcSwap<Option<String>>>,
}

#[tauri::command]
#[specta::specta]
pub async fn is_capture_running(state: State<'_, RecordingState>) -> Result<bool, String> {
    Ok(state.capture_active.load(Ordering::SeqCst))
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CaptureHealth {
    pub status: String,
    pub status_code: u16,
    pub last_frame_timestamp: Option<String>,
    pub last_ui_timestamp: Option<String>,
    pub frame_status: String,
    pub ui_status: String,
    pub message: String,
}

/// Direct in-process health snapshot for the Dystil UI. This replaces the
/// legacy localhost Dystil HTTP/WebSocket health endpoint.
#[tauri::command]
#[specta::specta]
pub fn get_capture_health(state: State<'_, RecordingState>) -> Result<CaptureHealth, String> {
    let running = state.capture_active.load(Ordering::SeqCst);
    let (status, status_code, message) = if running {
        ("healthy", 200, "Capture is running")
    } else {
        ("paused", 200, "Capture is paused")
    };
    Ok(CaptureHealth {
        status: status.to_string(),
        status_code,
        last_frame_timestamp: None,
        last_ui_timestamp: None,
        frame_status: format!("{status:?}").to_lowercase(),
        ui_status: format!("{status:?}").to_lowercase(),
        message: format!("{message} ({status:?})"),
    })
}

// ---------------------------------------------------------------------------
// Device listing (unchanged)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDevice {
    pub id: u32,
    pub stable_id: String,
    pub name: String,
    pub is_default: bool,
    pub width: u32,
    pub height: u32,
}

/// Read the current boot phase of the server. Used by the onboarding UI to
/// show progress ("updating database", "loading pipes", ...) while the HTTP
/// server is not yet listening — in particular during long DB migrations
/// where /health is unreachable.
#[tauri::command]
#[specta::specta]
pub async fn get_boot_phase() -> crate::health::BootPhaseSnapshot {
    crate::health::get_boot_phase_snapshot()
}

pub async fn get_available_monitors() -> Result<Vec<MonitorDevice>, String> {
    debug!("Getting available monitors");
    let monitors = dystil_capture::screen::monitor::list_monitors().await;

    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let result: Vec<MonitorDevice> = monitors
        .iter()
        .enumerate()
        .map(|(i, m)| MonitorDevice {
            id: m.id(),
            stable_id: m.stable_id(),
            name: if m.name().is_empty() {
                format!("Monitor {}", i + 1)
            } else {
                m.name().to_string()
            },
            is_default: i == 0,
            width: m.width(),
            height: m.height(),
        })
        .collect();

    debug!("Found {} monitors", result.len());
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn get_monitors() -> Result<Vec<MonitorDevice>, String> {
    get_available_monitors().await
}

// ---------------------------------------------------------------------------
// Capture-only commands (fast toggle, server stays alive)
// ---------------------------------------------------------------------------

/// Stop recording without killing the server.
/// Pipes, memories, search, and the HTTP API remain accessible.
#[tauri::command]
#[specta::specta]
pub async fn stop_capture(
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    info!("Stopping capture session (server stays alive)");

    let mut capture_guard = state.capture.lock().await;
    state.capture_active.store(false, Ordering::SeqCst);
    if let Some(session) = capture_guard.take() {
        session.stop().await;
        info!("Capture session stopped");
    } else {
        debug!("No capture session running");
    }
    crate::health::set_recording_status(crate::health::RecordingStatus::Paused);
    notify_recording_state_changed(&app);
    Ok(())
}

/// Start recording. Requires the server to be running.
#[tauri::command]
#[specta::specta]
pub async fn start_capture(
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    info!("Starting capture session");
    crate::health::set_recording_status(crate::health::RecordingStatus::Starting);
    notify_recording_state_changed(&app);

    // Race guard: short-circuit duplicate invocations.
    //
    // `<DeeplinkHandler />` is mounted in every non-overlay webview, and the
    // tray emits `shortcut-start-recording` app-wide — every listening window
    // fires `commands.startCapture()` simultaneously. Without this guard, two
    // concurrent calls both pass the `is_some()` check, both build a
    // CaptureSession (~290ms), and the second clobbers the first. Dropping
    // the first runs its shutdown handlers, which tear down workers shared
    // with the second — surfacing as a PoolClosed cascade and silently lost
    // capture rows.
    if state.is_starting_capture.swap(true, Ordering::SeqCst) {
        info!("Capture start already in progress, skipping duplicate");
        return Ok(());
    }
    struct ResetGuard<'a>(&'a AtomicBool);
    impl Drop for ResetGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _reset = ResetGuard(&state.is_starting_capture);

    // Hold the capture lock from the is_some check through the assign so a
    // concurrent `start_capture_internal` (called from spawn_capture's
    // existing-server path, not gated by is_starting_capture) can't race us.
    let mut capture_guard = state.capture.lock().await;
    if capture_guard.is_some() {
        state.capture_active.store(true, Ordering::SeqCst);
        info!("Capture session already running");
        return Ok(());
    }

    let server_guard = state.server.lock().await;
    let server = server_guard
        .as_ref()
        .ok_or_else(|| "Server not running — cannot start capture".to_string())?;
    let config = build_config(&app)?;
    let session = CaptureSession::start(server, &config, false).await?;
    drop(server_guard);

    *capture_guard = Some(session);
    state.capture_active.store(true, Ordering::SeqCst);
    crate::health::set_recording_status(crate::health::RecordingStatus::Recording);
    notify_recording_state_changed(&app);

    info!("Capture session started");
    Ok(())
}

// ---------------------------------------------------------------------------
// Full lifecycle commands (backward compat)
// ---------------------------------------------------------------------------

/// Stop capture AND server so the next spawn_capture does a full restart.
/// Called by updates and rollbacks.
/// The tray toggle uses stop_capture / start_capture to keep the server alive.
#[tauri::command]
#[specta::specta]
pub async fn stop_engine(
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    info!("stop_engine: stopping capture and server");

    // Stop capture first
    {
        let mut capture_guard = state.capture.lock().await;
        state.capture_active.store(false, Ordering::SeqCst);
        if let Some(session) = capture_guard.take() {
            session.stop().await;
            info!("Capture stopped");
        } else {
            debug!("No capture session to stop");
        }
    }

    // Shut down the server so the next spawn_capture does a full restart
    // with fresh settings (auth key, port, etc.). Without this, spawn_capture
    // sees the server as healthy and skips the restart entirely.
    {
        let mut server_guard = state.server.lock().await;
        if let Some(server) = server_guard.take() {
            server.shutdown().await;
            info!("Server stopped");
        }
    }

    // Reset flags so the next spawn_capture takes the full-start path
    // rather than the "server already in progress" wait loop.
    state.is_starting.store(false, Ordering::SeqCst);
    state.last_spawn_epoch.store(0, Ordering::SeqCst);
    crate::health::set_recording_status(crate::health::RecordingStatus::Paused);
    notify_recording_state_changed(&app);

    Ok(())
}

/// Start the server (if not running) and capture.
/// This is the main entry point called by the frontend.
#[tauri::command]
#[specta::specta]
pub async fn spawn_capture(
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
    _override_args: Option<Vec<String>>,
) -> Result<(), String> {
    info!("spawn_capture: starting server + capture");

    // --- Cooldown enforcement ---
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last_spawn = state.last_spawn_epoch.load(Ordering::SeqCst);
    if last_spawn > 0 && now_epoch.saturating_sub(last_spawn) < RESTART_COOLDOWN_SECS {
        let remaining = RESTART_COOLDOWN_SECS - now_epoch.saturating_sub(last_spawn);
        warn!("Restart cooldown active ({remaining}s remaining). Deferring spawn.");
        let last_spawn_epoch = state.last_spawn_epoch.clone();
        let is_starting = state.is_starting.clone();
        let server_arc = state.server.clone();
        let app_handle = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(remaining + 1)).await;
            info!("Cooldown expired, checking if server needs restart");
            {
                let server_guard = server_arc.lock().await;
                if server_guard.is_some() {
                    info!("Deferred spawn: runtime already exists, skipping");
                    return;
                }
            }
            info!("Deferred spawn: server dead, triggering restart");
            is_starting.store(false, Ordering::SeqCst);
            last_spawn_epoch.store(0, Ordering::SeqCst);
            let _ = app_handle.emit("request-server-restart", ());
        });
        return Ok(());
    }

    // --- Race prevention ---
    //
    // If a start is already in progress, wait on it rather than racing. This
    // used to time out after 15s and retry — which was fine for small
    // databases but catastrophic for large ones (Mike Cloke 2026-04-22: 31.5GB
    // db, migration took 13.2s, watchdog fired a retry, both migrations
    // raced on the SQLite lock, both failed, app stuck forever).
    //
    // Now we use boot-phase state as the source of truth:
    //   - "ready" → server is up, we're done
    //   - "error" → initial start failed, safe to take over and retry
    //   - "migrating_database" / "starting_pipes" / "starting"
    //     → another thread is making progress, keep waiting no matter how long
    //
    // A 30-minute safety ceiling prevents a wedged start from hanging the app
    // forever; for context, even a 100GB migration finishes in ~1 minute.
    if state.is_starting.swap(true, Ordering::SeqCst) {
        info!("Server start already in progress, waiting for boot phase...");
        const MAX_WAIT_SECS: u64 = 1800; // 30 minutes
        const POLL_MS: u64 = 500;
        let start_wait = std::time::Instant::now();
        loop {
            let phase = crate::health::get_boot_phase_snapshot();
            match phase.phase.as_str() {
                "ready" => {
                    // Phase says ready — HTTP may be binding right now. Loop
                    // once more without extra wait; it'll resolve on next poll.
                    tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
                    continue;
                }
                "error" => {
                    warn!(
                        "In-flight server start reported error: {}",
                        phase.error.as_deref().unwrap_or("<no detail>")
                    );
                    // Take over: clear is_starting so the full-start path below
                    // can run. Another concurrent caller may beat us; the
                    // swap(true) below detects that.
                    state.is_starting.store(false, Ordering::SeqCst);
                    if state.is_starting.swap(true, Ordering::SeqCst) {
                        // Someone else is already retrying. Bail out cleanly.
                        return Ok(());
                    }
                    break;
                }
                "idle" => {
                    // is_starting was true but phase never updated — the
                    // spawning thread likely died before setting phase. Treat
                    // like error and take over.
                    if start_wait.elapsed() > std::time::Duration::from_secs(30) {
                        warn!("is_starting set but boot phase still idle after 30s — taking over");
                        state.is_starting.store(false, Ordering::SeqCst);
                        if state.is_starting.swap(true, Ordering::SeqCst) {
                            return Ok(());
                        }
                        break;
                    }
                }
                _ => {
                    // starting | migrating_database | starting_pipes
                    // — keep waiting, progress is being made.
                }
            }
            if start_wait.elapsed() > std::time::Duration::from_secs(MAX_WAIT_SECS) {
                warn!(
                    "In-flight server start did not complete after {}s (phase={})",
                    MAX_WAIT_SECS, phase.phase
                );
                state.is_starting.store(false, Ordering::SeqCst);
                return Err(format!(
                    "Server start timed out after {} minutes. Current phase: {}",
                    MAX_WAIT_SECS / 60,
                    phase.phase
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
        }
    }

    // --- Check existing runtime ---
    {
        let server_guard = state.server.lock().await;
        if server_guard.is_some() {
            info!("Runtime already exists; ensuring capture is running");
            drop(server_guard);
            let capture_guard = state.capture.lock().await;
            if capture_guard.is_some() {
                state.capture_active.store(true, Ordering::SeqCst);
                state.is_starting.store(false, Ordering::SeqCst);
                return Ok(());
            }
            drop(capture_guard);
            return start_capture_internal(&state, &app).await;
        }
    }

    // --- Full start: server + capture ---
    // Stop any existing capture first (self-contained, no server lock needed)
    state.capture_active.store(false, Ordering::SeqCst);
    if let Some(session) = state.capture.lock().await.take() {
        session.stop().await;
    }
    // Shutdown existing server if any
    {
        let mut server_guard = state.server.lock().await;
        if let Some(server) = server_guard.take() {
            server.shutdown().await;
        }
    }

    // Permissions check
    let store = SettingsStore::get(&app).ok().flatten().unwrap_or_default();
    let permissions_check = do_permissions_check(false);
    if crate::capture_policy::product_capture_mode(store.recording.disable_vision)
        == dystil_capture::CaptureMode::FullCapture
        && !permissions_check.screen_recording.permitted()
    {
        warn!(
            "Screen recording permission not granted: {:?}. Cannot start server.",
            permissions_check.screen_recording
        );
        state.is_starting.store(false, Ordering::SeqCst);
        state.is_starting_capture.store(false, Ordering::SeqCst);
        // Flip the tray state machine to a terminal Error so the
        // recording status indicator stops showing "Starting…" forever
        // when the user has clicked "click to record" with TCC denied.
        crate::health::set_recording_status(crate::health::RecordingStatus::Error);
        notify_recording_state_changed(&app);
        return Err(
                "Screen recording permission required for FullCapture. Please grant permission or choose TextOnly."
                .to_string(),
        );
    }

    info!("Permissions OK. Starting server + capture.");

    let (data_dir, fell_back) = config::resolve_data_dir(&store.data_dir);
    if fell_back {
        warn!(
            "Custom data dir '{}' unavailable, using default: {}",
            store.data_dir,
            data_dir.display()
        );
    }

    let recording_config = store.to_dystil_capture_config(data_dir);

    let server_arc = state.server.clone();
    let capture_arc = state.capture.clone();
    let capture_active_arc = state.capture_active.clone();

    // Oneshot for result
    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

    // Spawn dedicated thread with its own runtime
    std::thread::Builder::new()
        .name("dystil-capture".to_string())
        .spawn(move || {
            let server_runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(16)
                .thread_name("dystil-worker")
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let msg = format!("Failed to create server runtime: {}", e);
                    crate::health::set_boot_error(&msg);
                    let _ = result_tx.send(Err(msg));
                    return;
                }
            };

            server_runtime.block_on(async move {
                // Phase 1: Start server
                let server = match ServerCore::start(&recording_config).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to start server core: {}", e);
                        let _ = result_tx.send(Err(e));
                        return;
                    }
                };

                // Phase 2: Start capture
                let capture = match CaptureSession::start(&server, &recording_config, true).await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to start capture session: {}", e);
                        // Server started but capture failed — store server anyway
                        // so pipes/search still work
                        {
                            let mut guard = server_arc.lock().await;
                            *guard = Some(server);
                        }
                        let _ = result_tx.send(Err(e));
                        return;
                    }
                };

                info!("Server + capture started successfully on dedicated runtime");
                {
                    let mut guard = server_arc.lock().await;
                    *guard = Some(server);
                }
                {
                    let mut guard = capture_arc.lock().await;
                    *guard = Some(capture);
                }
                capture_active_arc.store(true, Ordering::SeqCst);
                let _ = result_tx.send(Ok(()));

                // Keep runtime alive as long as server exists
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    let guard = server_arc.lock().await;
                    if guard.is_none() {
                        info!("Server removed from state, shutting down server thread");
                        break;
                    }
                }
            });
        })
        .map_err(|e| format!("Failed to spawn server thread: {}", e))?;

    match result_rx.await {
        Ok(Ok(())) => {
            info!("Dystil started successfully");
            let spawn_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            state.last_spawn_epoch.store(spawn_epoch, Ordering::SeqCst);
            crate::health::set_recording_status(crate::health::RecordingStatus::Recording);
            notify_recording_state_changed(&app);
            Ok(())
        }
        Ok(Err(e)) => {
            state.is_starting.store(false, Ordering::SeqCst);
            state.is_starting_capture.store(false, Ordering::SeqCst);
            if e.contains("no monitors matched") {
                crate::health::set_recording_status(crate::health::RecordingStatus::Error);
            }
            notify_recording_state_changed(&app);
            Err(e)
        }
        Err(_) => {
            state.is_starting.store(false, Ordering::SeqCst);
            notify_recording_state_changed(&app);
            Err("Server startup channel dropped unexpectedly".to_string())
        }
    }
}

/// Internal helper: start capture on an already-running server.
///
/// Lock-first pattern matches `start_capture` so a concurrent `start_capture`
/// can't build a parallel session and clobber ours.
async fn start_capture_internal(
    state: &RecordingState,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let mut capture_guard = state.capture.lock().await;
    if capture_guard.is_some() {
        state.capture_active.store(true, Ordering::SeqCst);
        // A concurrent start_capture beat us to it.
        state.is_starting.store(false, Ordering::SeqCst);
        info!("Capture already started by concurrent caller");
        return Ok(());
    }

    let server_guard = state.server.lock().await;
    let server = server_guard
        .as_ref()
        .ok_or_else(|| "Server not running".to_string())?;

    let config = build_config(app)?;
    let session = CaptureSession::start(server, &config, false).await?;
    drop(server_guard);

    *capture_guard = Some(session);
    state.capture_active.store(true, Ordering::SeqCst);
    state.is_starting.store(false, Ordering::SeqCst);
    crate::health::set_recording_status(crate::health::RecordingStatus::Recording);
    notify_recording_state_changed(app);

    info!("Capture started on existing server");
    Ok(())
}

#[cfg(test)]
mod pause_tests {
    use super::*;

    #[test]
    fn startup_preserves_active_timed_pause() {
        let mut timed = SettingsStore::default();
        timed.capture_paused = true;
        timed.capture_pause_until = Some((Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
        assert!(normalize_pause_for_startup(&mut timed));
        assert!(timed.capture_paused);
    }

    #[test]
    fn startup_clears_expired_missing_or_malformed_timed_pauses() {
        for deadline in [
            Some((Utc::now() - chrono::Duration::minutes(1)).to_rfc3339()),
            Some("not-a-date".to_string()),
            None,
        ] {
            let mut settings = SettingsStore::default();
            settings.capture_paused = true;
            settings.capture_pause_until = deadline;
            assert!(!normalize_pause_for_startup(&mut settings));
            assert!(!settings.capture_paused);
            assert!(settings.capture_pause_until.is_none());
        }
    }
}
