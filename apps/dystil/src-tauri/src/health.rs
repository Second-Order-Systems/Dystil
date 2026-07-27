use anyhow::Result;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::{atomic::Ordering, RwLock};
use std::time::Instant;
use tauri::{Emitter, Manager};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

/// How long after startup to treat connection errors as "starting up" instead of "error".
/// The capture runtime needs time to initialize its database and providers.
const STARTUP_GRACE_PERIOD: Duration = Duration::from_secs(30);

/// Consecutive connection failures (refused/timeout) before showing Stopped.
/// Must be high enough to ride out transient DB pool saturation, which can cause
/// the health endpoint to timeout for 10-20 seconds without the server being down.
const CONSECUTIVE_FAILURES_THRESHOLD: u32 = 30;

/// Consecutive explicit "unhealthy"/"error" responses from a *responding* server
/// before showing Error. Set high (2 min sustained at 1Hz polling) because the
/// /health endpoint is a soft signal that flaps on transient backend issues
/// (DB pool pressure or OCR queue backpressure) while recording
/// itself continues normally. Genuine recording failures surface through the
/// dedicated `permission_monitor` + capture-module events, not through this debounce.
const CONSECUTIVE_UNHEALTHY_THRESHOLD: u32 = 120;

// ─────────────────────────────────────────────────────────────────────────
// Boot phase — tracks where we are inside ServerCore::start.
//
// The HTTP server only binds near the *end* of startup (after DB migration
// and capture initialization). That means /health is unreachable for the entire
// window we care most about (e.g. 13.2s for Mike's 31.5GB DB migration). The
// frontend and the spawn watchdog can't distinguish "server is migrating" from
// "server is dead" via HTTP alone — so they both time out and retry, and the
// retry races the still-running migration on the SQLite lock (see the Mike
// Cloke incident 2026-04-22).
//
// Rather than refactor the HTTP server to bind early and serve /health while
// the DB is offline, we expose boot phase via a process-local atomic and a
// Tauri command. The watchdog polls the atomic; the UI polls the command.
// Both become the source of truth during startup.
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BootPhaseSnapshot {
    /// One of: idle | starting | migrating_database |
    /// starting_pipes | ready | error
    pub phase: String,
    /// Human-readable detail to show the user (may be long-running hint)
    pub message: Option<String>,
    /// Present only when phase == "error"
    pub error: Option<String>,
    /// Unix epoch seconds when the current phase was entered. Lets the UI
    /// show "X minutes" on slow migrations.
    pub since_epoch_secs: u64,
}

impl BootPhaseSnapshot {
    pub fn idle() -> Self {
        Self {
            phase: "idle".to_string(),
            message: None,
            error: None,
            since_epoch_secs: 0,
        }
    }
}

static BOOT_PHASE: Lazy<RwLock<BootPhaseSnapshot>> =
    Lazy::new(|| RwLock::new(BootPhaseSnapshot::idle()));

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
pub fn set_boot_phase(phase: &str, message: Option<&str>) {
    let mut guard = BOOT_PHASE.write().unwrap_or_else(|e| e.into_inner());
    // Don't reset since_epoch if the phase is unchanged (no-op writes)
    if guard.phase != phase {
        guard.since_epoch_secs = now_epoch();
    }
    guard.phase = phase.to_string();
    guard.message = message.map(String::from);
    guard.error = None;
    info!(
        "boot phase → {}{}",
        phase,
        message.map(|m| format!(" ({})", m)).unwrap_or_default()
    );
}

pub fn set_boot_error(err: &str) {
    let mut guard = BOOT_PHASE.write().unwrap_or_else(|e| e.into_inner());
    guard.phase = "error".to_string();
    guard.error = Some(err.to_string());
    guard.since_epoch_secs = now_epoch();
    tracing::error!("boot phase → error: {}", err);
}

pub fn get_boot_phase_snapshot() -> BootPhaseSnapshot {
    BOOT_PHASE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Snapshot of where the boot lifecycle currently is.
///
/// Used as a gate before actions that race process teardown against
/// still-initializing native sessions — see #3622 (onnxruntime SIGSEGV during
/// auto-updater restart while native capture is initializing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootReadiness {
    /// Phase is still pre-ready (`starting`, `migrating_database`,
    /// `starting_pipes`). Process teardown is unsafe.
    Pending,
    /// Phase is `ready`. Safe to restart.
    Ready,
    /// Phase is `error`. Process is in a stuck state; restart won't help and
    /// callers should fail fast rather than waiting.
    Errored,
}

fn read_boot_phase() -> String {
    // Match existing pattern in this file: recover from poisoning rather than
    // silently returning a wrong answer (which would cause wait loops to spin
    // until timeout on a poisoned lock).
    BOOT_PHASE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .phase
        .clone()
}

pub fn boot_readiness() -> BootReadiness {
    match read_boot_phase().as_str() {
        "ready" => BootReadiness::Ready,
        "error" => BootReadiness::Errored,
        _ => BootReadiness::Pending,
    }
}

/// Block until boot reaches a terminal state (`Ready` or `Errored`) or `timeout`
/// elapses, then return the final readiness. Callers decide what to do with
/// `Errored` and timed-out `Pending`.
pub async fn wait_for_boot_ready(timeout: Duration) -> BootReadiness {
    let deadline = Instant::now() + timeout;
    loop {
        match boot_readiness() {
            BootReadiness::Ready => return BootReadiness::Ready,
            BootReadiness::Errored => return BootReadiness::Errored,
            BootReadiness::Pending => {
                if Instant::now() >= deadline {
                    return BootReadiness::Pending;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

// Shared recording status that can be read by the tray menu
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RecordingStatus {
    Starting,
    Recording,
    /// Capture paused but server (HTTP/pipes/search) still alive.
    Paused,
    /// Capture intentionally stopped by the user's work-hours schedule. Kept
    /// distinct from `Paused` so the tray can say "outside work hours" rather
    /// than implying a transient/manual pause the user can just click to resume.
    ScheduledPause,
    Stopped,
    Error,
}

/// Kind of recording device
#[derive(Clone, PartialEq, Debug)]
pub enum DeviceKind {
    Monitor,
}

/// Per-device status info for tray display
#[derive(Clone, PartialEq, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub kind: DeviceKind,
    pub active: bool,
    pub last_seen_secs_ago: u64,
}

/// Full recording info including per-device status
#[derive(Clone, PartialEq, Debug)]
pub struct RecordingInfo {
    pub status: RecordingStatus,
    pub devices: Vec<DeviceInfo>,
}

static RECORDING_INFO: Lazy<RwLock<RecordingInfo>> = Lazy::new(|| {
    RwLock::new(RecordingInfo {
        status: RecordingStatus::Starting,
        devices: Vec::new(),
    })
});

pub fn get_recording_status() -> RecordingStatus {
    RECORDING_INFO
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .status
}

#[allow(dead_code)]
pub fn get_recording_info() -> RecordingInfo {
    RECORDING_INFO
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn set_recording_status(status: RecordingStatus) {
    RECORDING_INFO
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .status = status;
}

fn set_recording_info(status: RecordingStatus, devices: Vec<DeviceInfo>) {
    let mut info = RECORDING_INFO.write().unwrap_or_else(|e| e.into_inner());
    info.status = status;
    info.devices = devices;
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct HealthCheckResponse {
    status: String,
    #[serde(default)]
    status_code: Option<i32>,
    #[serde(rename = "last_frame_timestamp")]
    last_frame_timestamp: Option<String>,
    #[serde(rename = "last_ui_timestamp", default)]
    last_ui_timestamp: Option<String>,
    #[serde(default)]
    frame_status: Option<String>,
    #[serde(default)]
    ui_status: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(rename = "verbose_instructions", default)]
    verbose_instructions: Option<String>,
    #[serde(default)]
    device_status_details: Option<String>,
    /// Monitor names from the server
    #[serde(default)]
    monitors: Option<Vec<String>>,
    /// Vision capture alive but DB writes stopped (pool exhaustion)
    #[serde(default)]
    vision_db_write_stalled: bool,
    /// DRM streaming content detected — capture should be fully stopped
    #[serde(default)]
    drm_content_paused: bool,
    /// Recording intentionally paused by the user's work-hours schedule. The
    /// engine reports this in /health; when true it has stopped capture on
    /// purpose, so the tray must say "outside work hours" instead of letting a
    /// stale start flag render a stuck "Starting…".
    #[serde(default)]
    schedule_paused: bool,
}

/// Decide recording status based on health check result and time since startup.
///
/// During the grace period, connection errors are treated as "starting up"
/// rather than errors, to avoid false-positive unhealthy indicators while
/// the recording server is still loading.
///
/// When transitioning away from Recording, we require `consecutive_failures`
/// to meet or exceed `failure_threshold` to prevent flickering caused by
/// transient timeouts or momentary server busyness.
///
/// "stale" responses (server responding but frame timestamps are old)
/// are treated as Recording — the server IS running, it's just behind on
/// DB writes (e.g. pool saturation). Showing the error icon for this causes
/// false alarms and user panic when data is actually still being captured.
fn decide_status(
    health_result: &Result<HealthCheckResponse>,
    elapsed_since_start: Duration,
    grace_period: Duration,
    ever_connected: bool,
    consecutive_failures: u32,
    failure_threshold: u32,
    consecutive_unhealthy: u32,
    unhealthy_threshold: u32,
    current_status: RecordingStatus,
) -> RecordingStatus {
    match health_result {
        Ok(health) if health.status == "unhealthy" || health.status == "error" => {
            // Server is responding but explicitly reporting a problem.
            // Debounce heavily: 2 min sustained before flipping to Error.
            // /health is a soft signal — DB pool pressure, OCR queue backpressure,
            // and slow database writes all flap "unhealthy" while recording continues.
            // Genuine failures (permission revoked, capture crashed) surface via
            // the permission_monitor + capture-module event paths, not here.
            if consecutive_unhealthy >= unhealthy_threshold {
                RecordingStatus::Error
            } else if current_status == RecordingStatus::Recording {
                RecordingStatus::Recording
            } else {
                current_status
            }
        }
        Ok(_) => {
            // Server is responding (healthy, stale, or degraded — with or without
            // DRM-pause). "stale" means timestamps are old but the server process
            // is alive; this happens during DB pool saturation and resolves on its
            // own. "degraded" is a soft signal that does NOT mean recording stopped
            // — real permission/capture failures are detected by permission_monitor
            // (see line 498-504 below). Don't surface Error in the tray for this.
            RecordingStatus::Recording
        }
        Err(_) => {
            // Connection error — is the server still starting up?
            if !ever_connected && elapsed_since_start < grace_period {
                RecordingStatus::Starting
            } else if current_status == RecordingStatus::Recording
                && consecutive_failures < failure_threshold
            {
                // We were recording and haven't hit enough consecutive failures yet.
                // Hold the Recording status to avoid flickering.
                RecordingStatus::Recording
            } else {
                RecordingStatus::Stopped
            }
        }
    }
}

/// Cap how long the `is_starting*` session flags may pin the tray on
/// "Starting…" while the server is RESPONDING. The flags are AtomicBools
/// cleared across many exit paths in recording.rs, and `capture_running`
/// comes from a `try_lock` that can fail under contention — a leaked flag or
/// permanently contended lock pinned a Windows enterprise machine on
/// "Starting…" for hours while /health showed capture flowing (2026-06-11
/// feedback log, device 40af21d0). A real server-up-but-capture-booting
/// window is seconds; even a 100GB DB migration happens BEFORE the server
/// responds. Past this ceiling we stop trusting the flag and let the
/// health-derived status through. Generous on purpose.
const START_PIN_CEILING: Duration = Duration::from_secs(300);

/// Returns the start-in-progress flag, clamped: once it has been
/// continuously true for longer than `ceiling` (tracked via `since`), it
/// reads as false so a leaked flag can't pin the status forever. Resets the
/// timer whenever the raw flag drops.
fn clamp_start_in_progress(raw: bool, since: &mut Option<Instant>, ceiling: Duration) -> bool {
    if !raw {
        *since = None;
        return false;
    }
    let started = since.get_or_insert_with(Instant::now);
    if started.elapsed() > ceiling {
        return false;
    }
    true
}

fn apply_capture_session_status(
    base_status: RecordingStatus,
    server_responding: bool,
    capture_running: Option<bool>,
    start_in_progress: bool,
    schedule_paused: bool,
) -> RecordingStatus {
    if !server_responding {
        return base_status;
    }

    // The work-hours schedule intentionally parks capture outside the user's
    // window. Honor it BEFORE the start-in-progress / capture-absent branches:
    // when a boot lands outside work hours, capture never comes up (it's held
    // off on purpose) and never errors, so the asserted start flag would
    // otherwise pin the tray on a misleading "Starting…" forever — the exact
    // bug a user with a work-hours schedule hit when booting before their window.
    if schedule_paused {
        return RecordingStatus::ScheduledPause;
    }

    if capture_running == Some(true) {
        return base_status;
    }

    if start_in_progress {
        return RecordingStatus::Starting;
    }

    match capture_running {
        Some(false) => RecordingStatus::Paused,
        _ => base_status,
    }
}

/// Map RecordingStatus to tray icon status string
fn status_to_icon_key(status: RecordingStatus) -> &'static str {
    match status {
        RecordingStatus::Starting => "starting",
        RecordingStatus::Recording => "healthy",
        RecordingStatus::Paused => "starting",
        // Outside work hours is a neutral, intentional state — show the calm
        // "starting"/amber icon, never the red error/unhealthy variant.
        RecordingStatus::ScheduledPause => "starting",
        RecordingStatus::Stopped => "error",
        RecordingStatus::Error => "unhealthy",
    }
}

/// Whether the tray icon should show the "failed" variant
#[cfg(test)]
fn is_unhealthy_icon(icon_key: &str) -> bool {
    icon_key == "unhealthy" || icon_key == "error"
}

/// Parse device info from a health check response for tray display.
fn parse_devices_from_health(health_result: &Result<HealthCheckResponse>) -> Vec<DeviceInfo> {
    let health = match health_result {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    let mut devices = Vec::new();

    // Parse monitors
    if let Some(monitors) = &health.monitors {
        for name in monitors {
            devices.push(DeviceInfo {
                name: name.clone(),
                kind: DeviceKind::Monitor,
                active: health.frame_status.as_deref() == Some("ok"),
                last_seen_secs_ago: 0,
            });
        }
    }

    devices
}

/// Starts a background task that periodically checks the health of the sidecar
/// and updates the tray icon accordingly.
pub async fn start_health_check(app: tauri::AppHandle) -> Result<()> {
    let mut interval = interval(Duration::from_secs(1));
    let mut last_status = String::new();
    let start_time = Instant::now();
    let mut ever_connected = false;
    let mut consecutive_failures: u32 = 0;
    let mut consecutive_unhealthy: u32 = 0;

    // How long the recording-session "start in progress" flags have been
    // continuously true — feeds clamp_start_in_progress so a leaked flag
    // can't pin the tray on "Starting…" forever (see START_PIN_CEILING).
    let mut start_in_progress_since: Option<Instant> = None;
    let mut start_pin_warned = false;

    tokio::spawn(async move {
        loop {
            interval.tick().await;

            let health_result = check_health(&app).await;

            // Track consecutive failures (connection errors) and unhealthy responses separately.
            // Connection errors = server unreachable (crash, restart, port conflict).
            // Unhealthy = server responding but reporting a problem (DB issues, stalls).
            match &health_result {
                Ok(health) if health.status == "unhealthy" || health.status == "error" => {
                    // Only hard "unhealthy"/"error" counts toward the Error transition.
                    // "degraded" is treated as healthy in decide_status (see comments there).
                    ever_connected = true;
                    consecutive_failures = 0;
                    consecutive_unhealthy = consecutive_unhealthy.saturating_add(1);
                }
                Ok(_) => {
                    ever_connected = true;
                    consecutive_failures = 0;
                    consecutive_unhealthy = 0;
                }
                Err(_) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    // Don't reset consecutive_unhealthy on connection error — if the server
                    // was unhealthy and then crashed, we want the counter to persist.
                }
            }

            let current_status = get_recording_status();
            let status = decide_status(
                &health_result,
                start_time.elapsed(),
                STARTUP_GRACE_PERIOD,
                ever_connected,
                consecutive_failures,
                CONSECUTIVE_FAILURES_THRESHOLD,
                consecutive_unhealthy,
                CONSECUTIVE_UNHEALTHY_THRESHOLD,
                current_status,
            );

            let (capture_running, start_in_progress_raw) = if let Some(recording_state) =
                app.try_state::<crate::recording::RecordingState>()
            {
                let start_in_progress = recording_state.is_starting.load(Ordering::SeqCst)
                    || recording_state.is_starting_capture.load(Ordering::SeqCst);
                (
                    Some(recording_state.capture_active.load(Ordering::SeqCst)),
                    start_in_progress,
                )
            } else {
                (None, false)
            };
            // Clamp the flag so a leaked atomic / contended capture lock can't
            // pin the tray on "Starting…" forever while capture is actually
            // flowing (see START_PIN_CEILING).
            let start_in_progress = clamp_start_in_progress(
                start_in_progress_raw,
                &mut start_in_progress_since,
                START_PIN_CEILING,
            );
            if start_in_progress_raw && !start_in_progress {
                if !start_pin_warned {
                    start_pin_warned = true;
                    warn!(
                        "start-in-progress flag stuck for >{}s while server responding — \
                         ignoring it for tray status (capture_running={:?})",
                        START_PIN_CEILING.as_secs(),
                        capture_running
                    );
                }
            } else if !start_in_progress_raw {
                start_pin_warned = false;
            }

            // Engine intentionally pauses capture outside the work-hours
            // schedule and reports it in /health; surface it as ScheduledPause
            // so the tray doesn't show a stuck "Starting…".
            let schedule_paused = matches!(&health_result, Ok(h) if h.schedule_paused);
            let status = apply_capture_session_status(
                status,
                health_result.is_ok(),
                capture_running,
                start_in_progress,
                schedule_paused,
            );

            // Runtime permission state is handled by the Tauri permission flow;
            // this loop only reflects the in-process capture lifecycle.

            // Parse device info from health response, filtered by monitor settings
            let mut devices = parse_devices_from_health(&health_result);

            // Filter monitors to only show actively recording ones
            if let Ok(Some(store)) = crate::store::SettingsStore::get(&app) {
                if !store.recording.use_all_monitors
                    && !store.recording.monitor_ids.is_empty()
                    && store.recording.monitor_ids != vec!["default".to_string()]
                {
                    devices.retain(|d| {
                        if d.kind != DeviceKind::Monitor {
                            return true;
                        }
                        store.recording.monitor_ids.iter().any(|allowed| {
                            // Stable ID format: "Display 3_1920x1080_0,0"
                            // Extract name prefix before last '_' (position coords)
                            let allowed_name = allowed.rsplitn(2, '_').last().unwrap_or(allowed);
                            // Health monitor format: "Display 3 (1920x1080)"
                            // Extract just the display name
                            let health_name = d.name.split(" (").next().unwrap_or(&d.name);
                            let allowed_short =
                                allowed_name.split('_').next().unwrap_or(allowed_name);
                            // Also match numeric monitor IDs from CLI -m flag
                            // e.g. allowed="3" should match health_name="Display 3"
                            let numeric_match = health_name
                                .strip_prefix("Display ")
                                .map_or(false, |id| id == *allowed);
                            health_name == allowed_short || numeric_match
                        })
                    });
                }
            }

            set_recording_info(status, devices);

            let current_status = status_to_icon_key(status);

            // Update icon only when the health state changes.
            if current_status != last_status {
                last_status = current_status.to_string();

                let image = match crate::safe_icon::load_main_tray_icon(&app) {
                    Ok(img) => img,
                    Err(error) => {
                        error!(%error, "failed to load tray icon");
                        continue;
                    }
                };

                // TrayIcon must be accessed and dropped on the main thread
                // (NSStatusBar operations crash if called from a tokio thread)
                let app_clone = app.clone();
                let _ = app.run_on_main_thread(move || {
                    crate::window::with_autorelease_pool(|| {
                        if let Some(main_tray) = app_clone.tray_by_id("dystil_main") {
                            if let Err(e) = crate::safe_icon::safe_set_icon(&main_tray, image) {
                                error!("failed to set tray icon: {}", e);
                            }
                        }
                    });
                });
            }
        }
    });

    Ok(())
}

/// Build a health snapshot from Dystil's in-process capture state.
///
/// The old implementation queried a localhost capture HTTP server. Dystil no
/// longer runs that service, so health must not depend on a dead port.
async fn check_health(app: &tauri::AppHandle) -> Result<HealthCheckResponse> {
    let capture_running = app
        .try_state::<crate::recording::RecordingState>()
        .map(|state| state.capture_active.load(Ordering::SeqCst))
        .unwrap_or(false);
    Ok(HealthCheckResponse {
        status: if capture_running { "healthy" } else { "paused" }.to_string(),
        status_code: Some(200),
        frame_status: Some(if capture_running { "ok" } else { "paused" }.to_string()),
        ui_status: Some(if capture_running { "ok" } else { "paused" }.to_string()),
        message: Some(if capture_running {
            "Dystil capture is running".to_string()
        } else {
            "Dystil capture is paused".to_string()
        }),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_healthy_response() -> Result<HealthCheckResponse> {
        Ok(HealthCheckResponse {
            status: "healthy".to_string(),
            status_code: Some(200),
            last_frame_timestamp: None,
            last_ui_timestamp: None,
            frame_status: None,
            ui_status: None,
            message: None,
            verbose_instructions: None,
            device_status_details: None,
            monitors: None,
            vision_db_write_stalled: false,
            drm_content_paused: false,
            schedule_paused: false,
        })
    }

    fn make_unhealthy_response() -> Result<HealthCheckResponse> {
        Ok(HealthCheckResponse {
            status: "unhealthy".to_string(),
            status_code: Some(500),
            last_frame_timestamp: None,
            last_ui_timestamp: None,
            frame_status: None,
            ui_status: None,
            message: None,
            verbose_instructions: None,
            device_status_details: None,
            monitors: None,
            vision_db_write_stalled: false,
            drm_content_paused: false,
            schedule_paused: false,
        })
    }

    fn make_connection_error() -> Result<HealthCheckResponse> {
        Err(anyhow::anyhow!("connection refused"))
    }

    // Helper: call decide_status with thresholds exceeded (no debouncing active)
    // Used for tests that don't care about debouncing behavior
    fn decide_no_debounce(
        health_result: &Result<HealthCheckResponse>,
        elapsed: Duration,
        grace: Duration,
        ever_connected: bool,
    ) -> RecordingStatus {
        // consecutive_failures >= threshold means debouncing won't hold Recording
        decide_status(
            health_result,
            elapsed,
            grace,
            ever_connected,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Stopped,
        )
    }

    // ==================== decide_status tests ====================

    #[test]
    fn test_healthy_response_always_recording() {
        let result = make_healthy_response();
        let status =
            decide_no_debounce(&result, Duration::from_secs(0), STARTUP_GRACE_PERIOD, false);
        assert_eq!(status, RecordingStatus::Recording);
    }

    #[test]
    fn test_unhealthy_below_threshold_holds_recording() {
        // Unhealthy responses below the threshold should NOT flip to Error
        let result = make_unhealthy_response();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            0,
            CONSECUTIVE_FAILURES_THRESHOLD,
            1, // only 1 unhealthy — below threshold of 10
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Recording,
            "single unhealthy response should NOT flip to Error"
        );
    }

    #[test]
    fn test_unhealthy_at_threshold_transitions_to_error() {
        // Unhealthy responses at threshold should transition to Error
        let result = make_unhealthy_response();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            0,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Error,
            "sustained unhealthy should transition to Error"
        );
    }

    #[test]
    fn test_connection_error_during_grace_period_is_starting() {
        let result = make_connection_error();
        let status =
            decide_no_debounce(&result, Duration::from_secs(0), STARTUP_GRACE_PERIOD, false);
        assert_eq!(status, RecordingStatus::Starting);

        let result = make_connection_error();
        let status = decide_no_debounce(
            &result,
            Duration::from_secs(15),
            STARTUP_GRACE_PERIOD,
            false,
        );
        assert_eq!(status, RecordingStatus::Starting);

        let result = make_connection_error();
        let status = decide_no_debounce(
            &result,
            Duration::from_secs(29),
            STARTUP_GRACE_PERIOD,
            false,
        );
        assert_eq!(status, RecordingStatus::Starting);
    }

    #[test]
    fn test_connection_error_after_grace_period_is_stopped() {
        let result = make_connection_error();
        let status = decide_no_debounce(
            &result,
            Duration::from_secs(31),
            STARTUP_GRACE_PERIOD,
            false,
        );
        assert_eq!(status, RecordingStatus::Stopped);
    }

    #[test]
    fn test_connection_error_after_previous_connection_is_stopped() {
        let result = make_connection_error();
        let status = decide_status(
            &result,
            Duration::from_secs(5),
            STARTUP_GRACE_PERIOD,
            true,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(status, RecordingStatus::Stopped);
    }

    #[test]
    fn test_grace_period_boundary() {
        let grace = Duration::from_secs(30);

        let result = make_connection_error();
        let status = decide_no_debounce(&result, Duration::from_secs(29), grace, false);
        assert_eq!(status, RecordingStatus::Starting);

        let result = make_connection_error();
        let status = decide_no_debounce(&result, Duration::from_secs(30), grace, false);
        assert_eq!(status, RecordingStatus::Stopped);
    }

    // ==================== debouncing / anti-flicker tests ====================

    #[test]
    fn test_single_failure_while_recording_holds_recording() {
        let result = make_connection_error();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            1,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Recording,
            "single failure while recording should NOT flip to Stopped"
        );
    }

    #[test]
    fn test_threshold_failures_while_recording_transitions_to_stopped() {
        let result = make_connection_error();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Stopped,
            "should transition to Stopped after 30s of consecutive failures"
        );
    }

    #[test]
    fn test_debounce_does_not_apply_when_not_recording() {
        let result = make_connection_error();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            1,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Stopped,
        );
        assert_eq!(status, RecordingStatus::Stopped);
    }

    #[test]
    fn test_healthy_response_resets_after_failures() {
        let result = make_healthy_response();
        let status = decide_status(
            &result,
            Duration::from_secs(60),
            STARTUP_GRACE_PERIOD,
            true,
            2,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(status, RecordingStatus::Recording);
    }

    #[test]
    fn test_capture_absent_with_live_server_is_paused() {
        let status = apply_capture_session_status(
            RecordingStatus::Recording,
            true,
            Some(false),
            false,
            false,
        );
        assert_eq!(status, RecordingStatus::Paused);
    }

    #[test]
    fn test_capture_absent_while_starting_stays_starting() {
        let status = apply_capture_session_status(
            RecordingStatus::Recording,
            true,
            Some(false),
            true,
            false,
        );
        assert_eq!(status, RecordingStatus::Starting);
    }

    #[test]
    fn test_capture_status_does_not_mask_connection_error() {
        let status = apply_capture_session_status(
            RecordingStatus::Stopped,
            false,
            Some(false),
            false,
            false,
        );
        assert_eq!(status, RecordingStatus::Stopped);
    }

    #[test]
    fn test_running_capture_keeps_recording_status() {
        let status = apply_capture_session_status(
            RecordingStatus::Recording,
            true,
            Some(true),
            false,
            false,
        );
        assert_eq!(status, RecordingStatus::Recording);
    }

    #[test]
    fn test_running_capture_wins_over_stale_starting_flag() {
        let status =
            apply_capture_session_status(RecordingStatus::Recording, true, Some(true), true, false);
        assert_eq!(status, RecordingStatus::Recording);
    }

    // ── Work-hours schedule pause ───────────────────────────────────────────
    //
    // Repro of a field report: a user with a work-hours schedule booted
    // before their window, the engine started then immediately
    // stopped capture ("outside work-hours schedule — stopping all capture"),
    // and the tray sat on a stuck "Starting…". The inputs below are identical
    // to `test_capture_absent_while_starting_stays_starting` (server up, no
    // capture session, start flag still asserted) — only `schedule_paused` is
    // true. Before the fix this returned Starting; now it must report the
    // honest ScheduledPause so the tray can say "outside work hours".
    #[test]
    fn test_schedule_paused_overrides_stuck_starting() {
        let status =
            apply_capture_session_status(RecordingStatus::Recording, true, Some(false), true, true);
        assert_eq!(status, RecordingStatus::ScheduledPause);
    }

    // A live capture session struct that the engine has schedule-stopped behind
    // our back must NOT keep reading as Recording — that's the "overlay says
    // recording but nothing is captured" footgun. schedule_paused wins.
    #[test]
    fn test_schedule_paused_overrides_recording() {
        let status =
            apply_capture_session_status(RecordingStatus::Recording, true, Some(true), false, true);
        assert_eq!(status, RecordingStatus::ScheduledPause);
    }

    // Within the work-hours window (schedule_paused = false) nothing changes:
    // the stale-start-flag path still yields Starting, exactly as before.
    #[test]
    fn test_within_schedule_leaves_starting_untouched() {
        let status = apply_capture_session_status(
            RecordingStatus::Recording,
            true,
            Some(false),
            true,
            false,
        );
        assert_eq!(status, RecordingStatus::Starting);
    }

    // schedule_paused only comes from a successful /health read, but guard the
    // precedence anyway: a connection error must surface the real Stopped/boot
    // state, never a stale "outside work hours".
    #[test]
    fn test_schedule_paused_ignored_when_server_down() {
        let status =
            apply_capture_session_status(RecordingStatus::Stopped, false, Some(false), false, true);
        assert_eq!(status, RecordingStatus::Stopped);
    }

    // Outside work hours is intentional, not a failure — calm icon, not red.
    #[test]
    fn test_scheduled_pause_shows_healthy_icon() {
        assert!(!is_unhealthy_icon(status_to_icon_key(
            RecordingStatus::ScheduledPause
        )));
    }

    #[test]
    fn test_pool_saturation_scenario() {
        // Simulate DB pool saturation: server responds but with unhealthy status
        // for a few seconds, then recovers. Tray should stay green the whole time.
        let grace = Duration::from_secs(30);

        // tick 1-5: unhealthy responses (below threshold of 10)
        for i in 1..=5 {
            let status = decide_status(
                &make_unhealthy_response(),
                Duration::from_secs(60),
                grace,
                true,
                0,
                CONSECUTIVE_FAILURES_THRESHOLD,
                i,
                CONSECUTIVE_UNHEALTHY_THRESHOLD,
                RecordingStatus::Recording,
            );
            assert_eq!(
                status,
                RecordingStatus::Recording,
                "unhealthy tick {i}: should hold Recording (below threshold)"
            );
        }

        // tick 6: server recovers
        let status = decide_status(
            &make_healthy_response(),
            Duration::from_secs(65),
            grace,
            true,
            0,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(status, RecordingStatus::Recording);
    }

    #[test]
    fn test_flicker_scenario_simulation() {
        // Server under load: intermittent timeouts that never exceed threshold
        let grace = Duration::from_secs(30);
        let threshold = CONSECUTIVE_FAILURES_THRESHOLD;

        // 10 consecutive failures — still below threshold of 30
        let status = decide_status(
            &make_connection_error(),
            Duration::from_secs(70),
            grace,
            true,
            10,
            threshold,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Recording,
            "10s of failures should NOT flip to Stopped (threshold is 30)"
        );

        // Back to healthy
        let status = decide_status(
            &make_healthy_response(),
            Duration::from_secs(71),
            grace,
            true,
            0,
            threshold,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(status, RecordingStatus::Recording);
    }

    #[test]
    fn test_real_crash_still_detected() {
        // Server truly crashes — 30 consecutive seconds of failures
        let grace = Duration::from_secs(30);
        let threshold = CONSECUTIVE_FAILURES_THRESHOLD;

        // At threshold (30 failures = 30s) — transitions to Stopped
        let status = decide_status(
            &make_connection_error(),
            Duration::from_secs(90),
            grace,
            true,
            threshold,
            threshold,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(
            status,
            RecordingStatus::Stopped,
            "should detect real crash after 30s of failures"
        );
    }

    // ==================== icon mapping tests ====================

    #[test]
    fn test_starting_shows_healthy_icon() {
        assert!(!is_unhealthy_icon(status_to_icon_key(
            RecordingStatus::Starting
        )));
    }

    #[test]
    fn test_recording_shows_healthy_icon() {
        assert!(!is_unhealthy_icon(status_to_icon_key(
            RecordingStatus::Recording
        )));
    }

    #[test]
    fn test_stopped_shows_failed_icon() {
        assert!(is_unhealthy_icon(status_to_icon_key(
            RecordingStatus::Stopped
        )));
    }

    #[test]
    fn test_error_shows_failed_icon() {
        assert!(is_unhealthy_icon(status_to_icon_key(
            RecordingStatus::Error
        )));
    }

    // ==================== realistic boot sequence simulation ====================

    #[test]
    fn test_boot_sequence_no_false_positive() {
        let grace = Duration::from_secs(30);

        let status = decide_no_debounce(
            &make_connection_error(),
            Duration::from_secs(0),
            grace,
            false,
        );
        assert_eq!(status, RecordingStatus::Starting);
        assert!(!is_unhealthy_icon(status_to_icon_key(status)));

        let status = decide_no_debounce(
            &make_healthy_response(),
            Duration::from_secs(5),
            grace,
            false,
        );
        assert_eq!(status, RecordingStatus::Recording);
        assert!(!is_unhealthy_icon(status_to_icon_key(status)));
    }

    #[test]
    fn test_server_crash_after_boot_shows_error() {
        let grace = Duration::from_secs(30);

        // Server was healthy, now crashes — after threshold failures (30s)
        let status = decide_status(
            &make_connection_error(),
            Duration::from_secs(60),
            grace,
            true,
            CONSECUTIVE_FAILURES_THRESHOLD,
            CONSECUTIVE_FAILURES_THRESHOLD,
            0,
            CONSECUTIVE_UNHEALTHY_THRESHOLD,
            RecordingStatus::Recording,
        );
        assert_eq!(status, RecordingStatus::Stopped);
        assert!(
            is_unhealthy_icon(status_to_icon_key(status)),
            "should show failed icon after crash"
        );
    }

    #[test]
    fn test_server_never_starts_shows_error_after_grace() {
        let grace = Duration::from_secs(30);

        // Server never starts — after grace period, show the error
        let status = decide_no_debounce(
            &make_connection_error(),
            Duration::from_secs(35),
            grace,
            false,
        );
        assert_eq!(status, RecordingStatus::Stopped);
        assert!(
            is_unhealthy_icon(status_to_icon_key(status)),
            "should show failed icon if server never started"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Boot-readiness gate (#3622)
    //
    // These tests mutate the process-wide BOOT_PHASE singleton. They share a
    // mutex so they run serially even under `cargo test`'s default parallel
    // runner — otherwise one test's `set_boot_phase("ready")` would race
    // another's `set_boot_phase("error")` and flap.
    // ─────────────────────────────────────────────────────────────────────────

    use std::sync::Mutex as StdMutex;
    static BOOT_PHASE_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_boot_phase<F: FnOnce()>(phase: &str, body: F) {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_boot_phase(phase, None);
        body();
        // Reset so other tests see a known-pending baseline.
        set_boot_phase("idle", None);
    }

    #[test]
    fn boot_readiness_ready_when_ready_phase() {
        with_boot_phase("ready", || {
            assert_eq!(boot_readiness(), BootReadiness::Ready);
            assert_eq!(boot_readiness(), BootReadiness::Ready);
        });
    }

    #[test]
    fn boot_readiness_errored_when_error_phase() {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // set_boot_error uses a different code path than set_boot_phase
        set_boot_error("simulated boot failure");
        assert_eq!(boot_readiness(), BootReadiness::Errored);
        assert_ne!(boot_readiness(), BootReadiness::Ready);
        set_boot_phase("idle", None);
    }

    #[test]
    fn boot_readiness_pending_during_intermediate_phases() {
        for phase in [
            "starting",
            "migrating_database",
            "starting",
            "starting_pipes",
        ] {
            with_boot_phase(phase, || {
                assert_eq!(
                    boot_readiness(),
                    BootReadiness::Pending,
                    "phase {phase} should be pending"
                );
                assert_ne!(
                    boot_readiness(),
                    BootReadiness::Ready,
                    "phase {phase} should not be ready"
                );
            });
        }
    }

    #[tokio::test]
    async fn wait_for_boot_ready_returns_immediately_when_ready() {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_boot_phase("ready", None);
        let start = Instant::now();
        let result = wait_for_boot_ready(Duration::from_secs(5)).await;
        assert_eq!(result, BootReadiness::Ready);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "should not poll when already ready (took {:?})",
            start.elapsed()
        );
        set_boot_phase("idle", None);
    }

    #[tokio::test]
    async fn wait_for_boot_ready_fails_fast_on_error_phase() {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_boot_error("simulated startup failure");
        let start = Instant::now();
        let result = wait_for_boot_ready(Duration::from_secs(60)).await;
        assert_eq!(
            result,
            BootReadiness::Errored,
            "must short-circuit on error, not wait out full timeout"
        );
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "error phase must fail fast (took {:?})",
            start.elapsed()
        );
        set_boot_phase("idle", None);
    }

    #[tokio::test]
    async fn wait_for_boot_ready_returns_pending_on_timeout() {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_boot_phase("starting", None);
        // 200 ms is long enough for the polling loop to make at least one
        // pass (poll interval is 500 ms, deadline check fires first), short
        // enough not to slow the suite.
        let start = Instant::now();
        let result = wait_for_boot_ready(Duration::from_millis(200)).await;
        let elapsed = start.elapsed();
        assert_eq!(
            result,
            BootReadiness::Pending,
            "timeout while still pending should return Pending"
        );
        assert!(
            elapsed < Duration::from_millis(800),
            "should not overshoot timeout by much (took {:?})",
            elapsed
        );
        set_boot_phase("idle", None);
    }

    #[tokio::test]
    async fn wait_for_boot_ready_observes_transition_to_ready() {
        let _guard = BOOT_PHASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_boot_phase("starting", None);

        // Flip to ready after 100 ms. The waiter polls every 500 ms, so
        // worst case it observes the transition within ~500 ms of the flip.
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            set_boot_phase("ready", None);
        });

        let result = wait_for_boot_ready(Duration::from_secs(5)).await;
        assert_eq!(result, BootReadiness::Ready);
        set_boot_phase("idle", None);
    }

    #[test]
    fn clamp_start_in_progress_passes_within_ceiling_and_resets() {
        let mut since: Option<Instant> = None;
        // raw=false → false, no timer
        assert!(!clamp_start_in_progress(
            false,
            &mut since,
            Duration::from_secs(60)
        ));
        assert!(since.is_none());
        // raw=true within ceiling → true, timer starts
        assert!(clamp_start_in_progress(
            true,
            &mut since,
            Duration::from_secs(60)
        ));
        assert!(since.is_some());
        // raw drops → false + timer resets (a fresh start later gets a fresh window)
        assert!(!clamp_start_in_progress(
            false,
            &mut since,
            Duration::from_secs(60)
        ));
        assert!(since.is_none());
    }

    #[test]
    fn clamp_start_in_progress_stops_trusting_leaked_flag_past_ceiling() {
        // Timer started in the past; with a ZERO ceiling any elapsed time
        // exceeds it — models the leaked-flag case that pinned the Windows
        // enterprise tray on "Starting…" for hours.
        let mut since = Some(Instant::now() - Duration::from_secs(1));
        assert!(!clamp_start_in_progress(true, &mut since, Duration::ZERO));
        // Timer must NOT reset while raw stays true — the episode is one pin.
        assert!(since.is_some());
    }
}
