// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(deprecated)] // cocoa/objc crate deprecations — will migrate to objc2 later
#![allow(unused_imports)]

use commands::focus_existing_window;
use serde_json::json;
use std::env;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
#[allow(unused_imports)]
use tauri_plugin_shell::process::CommandEvent;
use tracing::{debug, error, info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[cfg(target_os = "macos")]
use tracing_oslog::OsLogger;
use window::ShowRewindWindow;

mod agent_commands;
mod agent_mailbox;
mod agent_worker;
mod ai;
mod ai_presets;
mod ai_runtime;
mod app_config;
mod auth;
mod automation_commands;
mod ask_for_fix_commands;
mod build_capabilities;
mod capture_config;
mod capture_policy;
mod capture_session;
#[cfg(feature = "cloud-sync")]
mod capture_state_reporter;
#[allow(deprecated)]
mod commands;
mod deletion;
mod disk_usage;
mod dystil_paths;
mod hardware;
#[allow(deprecated)]
mod icons;
mod oauth;
mod permissions;
mod ready_to_use_commands;
mod recording;
mod recording_settings;
mod retention;
mod secret_store;
mod secrets;
mod server;
mod server_core;
#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod space_monitor;
mod store;
mod telemetry_exporter;
mod telemetry_resources;
mod tray;
mod updates;
mod window;
mod windows_ca_bundle;
#[cfg(target_os = "windows")]
mod windows_overlay;
#[cfg(target_os = "windows")]
mod windows_webview_env;
#[cfg(feature = "cloud-sync")]
mod work_insights_engine;
mod worth_fixing_commands;
mod worth_fixing_engine;

pub use agent_commands::*;
pub use ai::*;
pub use ai_presets::*;
pub use auth::*;
pub use automation_commands::*;
pub use ask_for_fix_commands::*;
pub use build_capabilities::*;
pub use deletion::*;
pub use server::*;
pub use worth_fixing_commands::*;

pub use ready_to_use_commands::*;
pub use recording::*;
pub use retention::*;
pub use updates::*;

pub use icons::*;
pub use store::get_store;

mod config;
pub use config::get_base_dir;

pub use commands::set_tray_health_icon;
pub use commands::set_tray_unhealth_icon;
pub use commands::write_browser_log;
pub use commands::write_browser_logs;
pub use recording::spawn_capture;
pub use recording::stop_capture;
pub use recording::stop_engine;
pub use server::spawn_server;
// Removed: pub use store::get_profiles_store; // Profile functionality has been removed

pub use permissions::do_permissions_check;
pub use permissions::open_permission_settings;
pub use permissions::request_permission;
use tauri::AppHandle;
#[cfg(target_os = "macos")]
mod dock_menu;
mod health;
mod log_files;
mod native_notification;
mod notifications;
mod safe_icon;
mod specta_bindings;

use base64::Engine;
use health::start_health_check;
use log_files::{get_dystil_data_dir, get_log_files};

#[tauri::command]
#[specta::specta]
fn get_env(name: &str) -> String {
    std::env::var(String::from(name)).unwrap_or(String::from(""))
}

use tokio::time::{sleep, Duration};

#[tauri::command]
#[specta::specta]
async fn get_media_file(file_path: &str) -> Result<serde_json::Value, String> {
    use std::path::Path;

    const MAX_RETRIES: u32 = 3;
    const INITIAL_DELAY_MS: u64 = 100;

    debug!("Reading media file: {}", file_path);

    let path = Path::new(file_path);

    // Retry loop to handle files that may be in the process of being written
    let mut last_error = String::new();
    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay = INITIAL_DELAY_MS * (1 << (attempt - 1)); // exponential backoff
            debug!(
                "Retry attempt {} for {}, waiting {}ms",
                attempt, file_path, delay
            );
            sleep(Duration::from_millis(delay)).await;
        }

        if !path.exists() {
            last_error = format!("File does not exist: {}", file_path);
            if attempt < MAX_RETRIES {
                continue;
            }
            return Err(last_error);
        }

        // Read file contents
        match tokio::fs::read(path).await {
            Ok(contents) => {
                // Check for empty or suspiciously small files (might still be writing)
                if contents.is_empty() {
                    last_error = "File is empty (may still be writing)".to_string();
                    debug!("{}: {}", last_error, file_path);
                    if attempt < MAX_RETRIES {
                        continue;
                    }
                    return Err(last_error);
                }

                debug!(
                    "Successfully read file of size: {} bytes (attempt {})",
                    contents.len(),
                    attempt + 1
                );

                // Convert to base64
                let data = base64::prelude::BASE64_STANDARD.encode(&contents);

                // Determine MIME type
                let mime_type = get_mime_type(file_path);

                return Ok(serde_json::json!({
                    "data": data,
                    "mimeType": mime_type
                }));
            }
            Err(e) => {
                last_error = format!("Failed to read file: {}", e);
                debug!("{} (attempt {})", last_error, attempt + 1);
                if attempt < MAX_RETRIES {
                    continue;
                }
                error!("{}", last_error);
                return Err(last_error);
            }
        }
    }

    Err(last_error)
}

fn get_mime_type(path: &str) -> String {
    let ext = path.split('.').last().unwrap_or("").to_lowercase();

    match ext.as_str() {
        "mp4" => "video/mp4".to_string(),
        "webm" => "video/webm".to_string(),
        _ => "video/mp4".to_string(),
    }
}

#[tauri::command]
#[specta::specta]
async fn upload_file_to_s3(file_path: &str, signed_url: &str) -> Result<bool, String> {
    debug!("Starting upload for file: {}", file_path);

    // Read file contents - do this outside retry loop to avoid multiple reads
    let file_contents = match tokio::fs::read(file_path).await {
        Ok(contents) => {
            debug!("Successfully read file of size: {} bytes", contents.len());
            contents
        }
        Err(e) => {
            error!("Failed to read file: {}", e);
            return Err(e.to_string());
        }
    };

    let client = reqwest::Client::new();
    let max_retries = 3;
    let mut attempt = 0;
    let mut last_error = String::new();

    while attempt < max_retries {
        attempt += 1;
        debug!("Upload attempt {} of {}", attempt, max_retries);

        match client
            .put(signed_url)
            .body(file_contents.clone())
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    debug!("Successfully uploaded file on attempt {}", attempt);
                    return Ok(true);
                }
                // Surface the response body — S3/Supabase wraps the reason for
                // 400/403 (signed URL expired, content-type mismatch, etc.) in
                // an XML payload that we'd otherwise discard.
                let body = response.text().await.unwrap_or_default();
                let snippet: String = body.chars().take(500).collect();
                last_error = format!("Upload failed with status: {} body: {}", status, snippet);
                error!("{} (attempt {}/{})", last_error, attempt, max_retries);
            }
            Err(e) => {
                last_error = format!("Request failed: {}", e);
                error!("{} (attempt {}/{})", last_error, attempt, max_retries);
            }
        }

        if attempt < max_retries {
            let delay = Duration::from_secs(2u64.pow(attempt as u32 - 1)); // Exponential backoff
            debug!("Waiting {}s before retry...", delay.as_secs());
            sleep(delay).await;
        }
    }

    Err(format!(
        "Upload failed after {} attempts. Last error: {}",
        max_retries, last_error
    ))
}

/// Shared tauri-specta registry body.
macro_rules! define_specta_builder {
    () => {{
        use crate::store::{OnboardingStore, SettingsStore};
        use tauri_specta::Builder;

        Builder::new()
            .commands(tauri_helper::specta_collect_commands!())
            .typ::<SettingsStore>()
            .typ::<OnboardingStore>()
            .typ::<hardware::HardwareCapability>()
            .typ::<oauth::OAuthStatus>()
    }};
}

#[tokio::main]
async fn main() {
    const AUTOSTART_ARG: &str = "--from-autostart";

    let _ = fix_path_env::fix();

    #[cfg(target_os = "windows")]
    windows_webview_env::install_user_data_dir();

    // Export the Windows root/CA cert stores to a PEM file and set
    // NODE_EXTRA_CA_CERTS before any bun/node subprocess can spawn. Fixes
    // "unable to verify the first certificate" on corporate networks where
    // antivirus (ESET, Zscaler, etc.) injects a private root CA. No-op on
    // macOS/Linux. Must run before Pi, PortableGit download, and pipe
    // subprocesses are touched.
    windows_ca_bundle::install();

    // Handle --check-arc-automation / --trigger-arc-automation flags early,
    // before any Tauri initialization. Used by the permission system to run
    // this binary via launchctl (detached from Terminal) so that macOS TCC
    // checks the binary's own identity instead of Terminal's.
    let launched_from_autostart = std::env::args().any(|arg| arg == AUTOSTART_ARG);

    #[cfg(target_os = "macos")]
    {
        let early_args: Vec<String> = std::env::args().collect();
        let is_check = early_args.iter().any(|a| a == "--check-arc-automation");
        let is_trigger = early_args.iter().any(|a| a == "--trigger-arc-automation");
        if is_check || is_trigger {
            let result = permissions::ae_check_automation_direct(
                "company.thebrowser.Browser",
                is_trigger, // askUserIfNeeded = true for trigger
            );
            match result {
                0 => print!("granted"),
                -1744 => print!("denied"),
                -1745 => print!("not_asked"),
                _ => print!("error"),
            }
            return;
        }
    }

    // Single-instance check: if sidecar server is already listening, hand off and exit.
    // This covers Linux (where tauri-plugin-single-instance is disabled due to
    // zbus/tokio conflict) and acts as a fallback on macOS/Windows.
    {
        let args: Vec<String> = std::env::args().collect();
        let deep_link_url = args.iter().find(|a| a.starts_with("dystil://")).cloned();

        let focus_port: u16 = server::focus_port();
        if let Ok(resp) = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{}/focus", focus_port))
            .timeout(std::time::Duration::from_secs(2))
            .json(&serde_json::json!({
                "args": args,
                "deep_link_url": deep_link_url,
            }))
            .send()
            .await
        {
            if resp.status().is_success() {
                eprintln!("dystil: another instance is already running — focused existing window, exiting.");
                std::process::exit(0);
            }
        }
    }

    // Install a panic hook that logs to stderr + Sentry BEFORE the default hook runs.
    // This is critical because panics inside `tao::send_event` (called from Obj-C)
    // hit `panic_cannot_unwind` → `abort()`, and the default hook's output may be lost.
    // By logging here we capture the actual panic message for diagnosis.
    //
    // Rotate the crash log on startup (don't truncate). Relaunch after a crash
    // is the common case — truncating loses the message we most need to diagnose.
    // Previous panic moves to last-panic.log.prev; new file starts empty.
    {
        let log_dir = crate::dystil_paths::data_dir();
        let cur = log_dir.join("last-panic.log");
        let prev = log_dir.join("last-panic.log.prev");
        if cur.exists() {
            let _ = std::fs::rename(&cur, &prev);
        }
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Log the actual panic first — before any processing. Once unwinding hits
        // Obj-C (e.g. tao::send_event), we get panic_cannot_unwind and lose the real message.
        eprintln!("PANIC: {}", info);

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();

        // Suppress "tokio context being shutdown" panics from background
        // tasks (redact workers, etc.) — these fire when a task is mid-
        // sqlx/timer poll at the moment the runtime tears down on app
        // quit. ServerCore::shutdown signals workers to exit cleanly, but
        // a residual race is possible if the worker is inside an await
        // that doesn't include the shutdown future. Either way, this is
        // orderly-shutdown noise — not a crash — and logging it to
        // last-panic.log + Sentry makes the app look unstable to users
        // and skews crash-rate dashboards.
        if payload.contains("Tokio 1.x context was found, but it is being shutdown") {
            eprintln!(
                "(suppressed tokio shutdown-time panic on thread '{}' at {})",
                thread_name, location
            );
            return;
        }

        // Force-capture a backtrace before abort() kills us
        let backtrace = std::backtrace::Backtrace::force_capture();

        let crash_msg = format!(
            "PANIC on thread '{}' at {}: {}\n\nBacktrace:\n{}",
            thread_name, location, payload, backtrace
        );

        // Log to stderr (survives even if tracing isn't initialized yet)
        eprintln!("{}", crash_msg);

        // Write to a crash log file — this survives abort() since we fsync
        // Critical for diagnosing panics inside tao's extern "C" callbacks
        // (send_event, did_finish_launching) where panic_cannot_unwind → abort()
        let log_dir = crate::dystil_paths::data_dir();
        let crash_path = log_dir.join("last-panic.log");
        // Append instead of truncate — when panic_cannot_unwind fires after
        // the original panic, both messages are preserved in the file.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_path)
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(f, "[{}] {}", timestamp, crash_msg);
            let _ = f.sync_all(); // fsync before abort() kills us
        }

        // Call the default hook (prints backtrace etc.)
        default_hook(info);
    }));

    // Set permanent OLLAMA_ORIGINS env var on Windows if not present
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        if env::var("OLLAMA_ORIGINS").is_err() {
            match std::process::Command::new("setx")
                .args(&["OLLAMA_ORIGINS", "*"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                Ok(output) => {
                    if !output.status.success() {
                        error!(
                            "failed to set OLLAMA_ORIGINS: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    } else {
                        info!("permanently set OLLAMA_ORIGINS=* for user");
                    }
                }
                Err(e) => {
                    warn!("setx not available, skipping OLLAMA_ORIGINS setup: {}", e);
                }
            }
        }
    }

    // Generate TypeScript bindings in debug mode (also via `cargo test` — see
    // specta_bindings.rs).
    #[cfg(debug_assertions)]
    {
        info!("Generating TypeScript bindings...");

        // tauri-specta command registry — must live in crate root scope for `collect_commands!`.
        fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
            define_specta_builder!()
        }

        let bindings_path = specta_bindings::default_bindings_path();
        if let Err(error) =
            specta_bindings::write_bindings_if_changed_with(&bindings_path, specta_builder())
        {
            eprintln!("Warning: {error}");
        }
    }

    let recording_state = RecordingState {
        server: Arc::new(tokio::sync::Mutex::new(None)),
        capture: Arc::new(tokio::sync::Mutex::new(None)),
        capture_active: Arc::new(AtomicBool::new(false)),
        is_starting: Arc::new(AtomicBool::new(false)),
        is_starting_capture: Arc::new(AtomicBool::new(false)),
        last_spawn_epoch: Arc::new(AtomicU64::new(0)),
        cloud_token: Arc::new(arc_swap::ArcSwap::new(Arc::new(None))),
    };
    #[allow(clippy::single_match)]
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_http::init())
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let _ = window.set_always_on_top(false);
                let _ = window.set_visible_on_all_workspaces(false);

                #[cfg(target_os = "macos")]
                if window.label() == "home" {
                    crate::window::hide_from_dock(window.app_handle());
                }
                // On Windows, let the settings window close normally when user
                // clicks X. For other windows, minimize or hide.
                #[cfg(target_os = "windows")]
                {
                    if window.label() == "home" {
                        // Behavior depends on the user setting `minimizeToTrayOnClose`:
                        //  - false (default, historical behavior): minimize the Home
                        //    window so its icon stays in the Windows taskbar as the
                        //    persistent app entry point.
                        //  - true (opt-in): hide the window AND remove it from the
                        //    taskbar. The process keeps running (see ExitRequested
                        //    below), and launching the app again restores the existing
                        //    instance through the single-instance handler.
                        //
                        // Settings reads are best-effort: if the store can't be read
                        // we fall back to the historical minimize() behavior so the
                        // user never loses access to the window. set_skip_taskbar /
                        // hide failures also fall back to minimize() for the same
                        // reason, so the user is never left with a lost (hidden,
                        // off-taskbar) window.
                        let minimize_to_tray = crate::store::SettingsStore::get(
                            window.app_handle(),
                        )
                        .ok()
                        .flatten()
                        .map(|s| s.minimize_to_tray_on_close)
                        .unwrap_or(false);

                        if minimize_to_tray {
                            #[cfg(target_os = "windows")]
                            if let Err(e) =
                                crate::windows_overlay::hide_window_from_user_surfaces(window)
                            {
                                warn!(
                                    "Failed to hide Home from Windows user surfaces: {e}; minimizing instead"
                                );
                                let _ = window.minimize();
                            }
                        } else {
                            // Minimize instead of closing so the Home window stays in
                            // the taskbar as the persistent app icon.
                            let _ = window.minimize();
                        }
                    } else {
                        // Overlay and other windows: hide (they're skip_taskbar anyway)
                        let _ = window.hide();
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = window.hide();
                }
                api.prevent_close();
            }
            _ => {}
        })
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_permission_flow::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        // single-instance plugin uses zbus::blocking on Linux which panics
        // inside an existing tokio runtime (nested block_on), so skip it on Linux
        ;
    #[cfg(not(target_os = "linux"))]
    let app = app.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        // Defer off event stack: plugin may invoke this from run loop (nounwind).
        let app_for_closure = app.clone();
        let args_clone = args.clone();
        let _ = app.run_on_main_thread(move || {
            // Focus the existing window
            focus_existing_window(app_for_closure.clone());

            // Forward deep-link URL from args
            if let Some(url) = args_clone.iter().find(|a| a.starts_with("dystil://")) {
                let _ = app_for_closure.emit("deep-link-received", url.clone());
            }

            // Forward CLI args
            if !args_clone.is_empty() {
                let _ = app_for_closure.emit("second-instance-args", args_clone.clone());
            }
        });
    }));
    #[cfg(feature = "official-build")]
    let app = app.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(target_os = "macos")]
    let app = app.plugin(tauri_nspanel::init());

    let app = app.manage(recording_state)
        .manage(worth_fixing_commands::WorthFixingState::default())
        .manage(ask_for_fix_commands::AskForFixState::default())
        .invoke_handler(tauri_helper::tauri_collect_commands!())
        .setup(move |app| {
            //deep link register_all
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }
            let app_handle = app.handle();
            #[cfg(feature = "cloud-sync")]
            capture_state_reporter::start(app_handle.clone());
            automation_commands::start_manager(app_handle.clone());
            worth_fixing_engine::start(app_handle.clone());

            // Create macOS app menu with Settings
            #[cfg(target_os = "macos")]
            {
                // Hide overlay when user switches Spaces (e.g. three-finger swipe).
                // This no longer causes feedback loops because we removed
                // activateIgnoringOtherApps + activation policy toggling.
                space_monitor::setup_space_listener(app.handle().clone());

                // Set up pinch-to-zoom: store the app handle so the gesture
                // recognizer callback (in window/gesture.rs) can emit Tauri events.
                crate::window::init_magnify_handler(app.handle().clone());

            }

            // Logging setup
            let base_dir = get_base_dir(app_handle, None)
                .unwrap_or_else(|e| {
                    eprintln!("Failed to get base dir, using fallback: {}", e);
                    crate::dystil_paths::data_dir()
                });

            // Set up rolling file appender
            let log_dir = get_dystil_data_dir(app.handle())
                .unwrap_or_else(|_| crate::dystil_paths::data_dir());
            let file_appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("dystil-app")
                .filename_suffix("log")
                .max_log_files(5)
                .build(log_dir)?;

            // Create a custom layer for file logging
            // xcap probes stale monitor / window IDs every refresh and logs
            // ERROR for IDs that don't exist (e.g. after a display unplug).
            // Benign noise that swamps real errors in user feedback logs.
            const LOG_FILTER: &str = "info,hyper=error,tower_http=error,ort=warn,xcap::platform::impl_window=off,xcap::platform::impl_monitor=off,xcap::platform::utils=off";

            // `RUST_LOG` wins when set (e.g. `RUST_LOG=debug`, or
            // `RUST_LOG=info,dystil_capture=trace` to keep the noise down);
            // otherwise use LOG_FILTER. Built per-layer — EnvFilter isn't Clone.
            let make_filter =
                || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(LOG_FILTER));

            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_ansi(false)
                .with_filter(make_filter());

            // Create a custom layer for console logging
            let console_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .with_filter(make_filter());

            // Initialize the tracing subscriber with file + console layers only.
            let registry = tracing_subscriber::registry()
                .with(file_layer)
                .with(console_layer);

            #[cfg(target_os = "macos")]
            let registry = registry.with(OsLogger::new("dystil", "app"));

            registry.init();

            #[cfg(target_os = "windows")]
            windows_webview_env::log_diagnostics();

            // Windows-specific setup
            if cfg!(windows) {
                let exe_dir = env::current_exe()
                    .expect("Failed to get current executable path")
                    .parent()
                    .expect("Failed to get parent directory of executable")
                    .to_path_buf();
                let tessdata_path = exe_dir.join("tessdata");
                env::set_var("TESSDATA_PREFIX", tessdata_path);
            }

            // Autostart setup
            let autostart_manager = app.autolaunch();


            info!("App version: {}", env!("CARGO_PKG_VERSION"));
            info!("Local data directory: {}", base_dir.display());

            // Store setup and initialization - must be done first
            // Note: StoreBuilder handles file creation internally — pre-creating
            // store.bin here caused TOCTOU race conditions ("File exists" os error 17).
            // Use unwrap_or_default to prevent crashes from corrupted stores
            let mut store = store::init_store(&app.handle()).unwrap_or_else(|e| {
                error!("Failed to init settings store, using defaults: {}", e);
                store::SettingsStore::default()
            });
            let pause_before = (store.capture_paused, store.capture_pause_until.clone());
            let startup_pause_active = recording::normalize_pause_for_startup(&mut store);
            if pause_before != (store.capture_paused, store.capture_pause_until.clone()) {
                if let Err(error) = store.save(&app.handle()) {
                    warn!(%error, "failed to clear expired capture pause at startup");
                }
            }

            app.manage(store.clone());

            // Set Chinese HuggingFace mirror early — before any model downloads
            if store.recording.use_chinese_mirror {
                std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
                info!("Chinese HuggingFace mirror enabled (HF_ENDPOINT set early)");
            }

            // Resolve data directory from user setting (custom dir or ~/.dystil)
            let (data_dir, data_dir_fell_back) = config::resolve_data_dir(&store.data_dir);
            info!("Recording data directory: {}", data_dir.display());
            if data_dir_fell_back {
                let app_handle_fb = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    // Small delay so the frontend window is ready to receive events
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let _ = app_handle_fb.emit("data-dir-fallback", ());
                });
            }

            // Initialize sync state

            // Initialize onboarding store
            let onboarding_store = store::init_onboarding_store(&app.handle()).unwrap_or_else(|e| {
                error!("Failed to init onboarding store, using defaults: {}", e);
                store::OnboardingStore::default()
            });
            app.manage(onboarding_store.clone());

            // Show the main home window for manual launches. OS autostart
            // launches stay in the tray. Permission recovery is handled
            // separately below.
            if launched_from_autostart {
                info!("started from OS autostart; keeping app in tray");
            } else {
                let _ = ShowRewindWindow::Home { page: None }.show(&app.handle());
            }

            // Get app handle once for all initializations
            let app_handle = app.handle().clone();

            // Collaboration stays alive independently of capture. It waits for
            // the local database/auth state when startup is still in progress.
            agent_worker::start(app_handle.clone());

            // Initialize the local focus/notification bridge first.
            let focus_port: u16 = server::focus_port();
            let server_shutdown_tx = spawn_server(app_handle.clone(), focus_port);
            app.manage(server_shutdown_tx);
            // TODO: vault lock app integration disabled — CLI-only for now
            // let vault_is_locked = data_dir.join(".vault_locked").exists()
            //     || (data_dir.join("vault.meta").exists()
            //         && data_dir.join("db.sqlite").exists()
            //         && dystil_vault::crypto::is_encrypted_file(&data_dir.join("db.sqlite")).unwrap_or(false));
            // if vault_is_locked {
            //     info!("Vault is locked — skipping server start, waiting for unlock");
            //     let _ = app_handle.emit("vault-locked-on-startup", ());
            // }

            // Start the SQLite runtime + capture on a dedicated thread with its
            // own tokio runtime so capture work does not compete with Tauri UI.
            {
                let store_clone = store.clone();
                let data_dir_clone = data_dir.clone();
                let app_handle_clone = app_handle.clone();
                let recording_state = app_handle.state::<RecordingState>();
                recording_state.is_starting.store(true, std::sync::atomic::Ordering::SeqCst);
                let server_arc = recording_state.server.clone();
                let capture_arc = recording_state.capture.clone();
                let capture_active_arc = recording_state.capture_active.clone();
                let is_starting_clone = recording_state.is_starting.clone();
                let startup_pause_active = startup_pause_active;

                std::thread::Builder::new()
                    .name("dystil-capture".to_string())
                    .spawn(move || {
                        let server_runtime = tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(16)
                            .thread_name("dystil-worker")
                            .enable_all()
                            .build()
                            .expect("Failed to create server runtime");

                        server_runtime.block_on(async move {
                            let config = store_clone.to_dystil_capture_config(data_dir_clone.clone());

                            // Permissions check
                            let permissions_check = permissions::do_permissions_check(false);
                            let disable_vision = config.disable_vision;

                            // Only block server start when the code-owned mode
                            // requires continuous vision. macOS on-demand mode
                            // boots the server and AX lane without touching SCK;
                            // CaptureSession enables its provider only after the
                            // existing permission has been observed as granted.
                            if !startup_pause_active
                                && !disable_vision
                                && !permissions_check.screen_recording.permitted()
                            {
                                warn!("Screen recording permission not granted: {:?}. FullCapture will not start.", permissions_check.screen_recording);
                                // Flip the recording state to a terminal Error
                                // value so the tray stops showing "Starting…"
                                // forever. Without this the user sees a
                                // perpetual spinner with no signal that
                                // anything is wrong; clearing only `is_starting`
                                // leaves RECORDING_INFO at its default Starting
                                // value and the health poll has no
                                // ever_connected signal to recover from.
                                crate::health::set_recording_status(
                                    crate::health::RecordingStatus::Error,
                                );
                                is_starting_clone.store(false, std::sync::atomic::Ordering::SeqCst);
                                return;
                            }

                            info!("Starting Dystil runtime + capture on dedicated runtime...");

                            let server = match server_core::ServerCore::start(&config)
                            .await
                            {
                                Ok(s) => s,
                                Err(e) => {
                                    error!("Failed to start server core: {}", e);
                                    crate::health::set_boot_error(&e);
                                    crate::health::set_recording_status(
                                        crate::health::RecordingStatus::Error,
                                    );
                                    is_starting_clone.store(false, std::sync::atomic::Ordering::SeqCst);
                                    return;
                                }
                            };

                            // Retention is owned by the local app, independently of
                            // cloud sync. The first pass runs immediately, followed
                            // by one pass per day for the lifetime of this runtime.
                            retention::start_housekeeping(
                                app_handle_clone.clone(),
                                server.db.pool.clone(),
                                server.data_path.clone(),
                                server.telemetry.clone(),
                            );

                            // Phase 2: Start capture unless a persisted privacy
                            // pause is active. The runtime still starts so local
                            // retrieval and settings remain available.
                            let capture = if startup_pause_active {
                                None
                            } else {
                                match capture_session::CaptureSession::start(&server, &config, true).await {
                                    Ok(c) => Some(c),
                                    Err(e) => {
                                        error!("Failed to start capture: {}", e);
                                        server.telemetry.record_app_start(
                                            dystil_telemetry::AppStartReason::CaptureInitialization,
                                            dystil_telemetry::Outcome::Failed,
                                        );
                                        crate::health::set_boot_error(&e);
                                        crate::health::set_recording_status(
                                            crate::health::RecordingStatus::Error,
                                        );
                                        // Store server anyway so pipes/search work
                                        let mut guard = server_arc.lock().await;
                                        *guard = Some(server);
                                        drop(guard);
                                        is_starting_clone.store(false, std::sync::atomic::Ordering::SeqCst);
                                        return;
                                    }
                                }
                            };

                            info!("Dystil runtime started successfully");
                            {
                                let mut guard = server_arc.lock().await;
                                *guard = Some(server);
                            }
                            if let Some(capture) = capture {
                                let mut guard = capture_arc.lock().await;
                                *guard = Some(capture);
                                capture_active_arc.store(true, std::sync::atomic::Ordering::SeqCst);
                                crate::health::set_recording_status(crate::health::RecordingStatus::Recording);
                            } else {
                                capture_active_arc.store(false, std::sync::atomic::Ordering::SeqCst);
                                crate::health::set_recording_status(crate::health::RecordingStatus::Paused);
                                crate::recording::restore_pause_timer(app_handle_clone.clone(), &store_clone);
                            }
                            is_starting_clone.store(false, std::sync::atomic::Ordering::SeqCst);
                            crate::recording::notify_recording_state_changed(&app_handle_clone);

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
                    .expect("Failed to spawn server thread");
            }

            // Community/source builds never poll an update endpoint.
            #[cfg(feature = "official-build")]
            updates::start_update_check(&app_handle, 5);

            // Setup tray
            if let Some(_) = app_handle.tray_by_id("dystil_main") {
                if let Err(e) = tray::setup_tray(&app_handle, None) {
                    error!("Failed to setup tray: {}", e);
                }
            }

            // Log tray icon position for diagnostics.
            // On notched MacBooks with many menu bar icons, the tray can land behind
            // the notch. Users can Cmd+drag it to a visible position.
            #[cfg(target_os = "macos")]
            {
                let app_tray = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    tray::log_tray_position(&app_tray);
                });
            }

            let is_autostart_enabled = store
                .auto_start_enabled;

            if is_autostart_enabled {
                let _ = autostart_manager.enable();
            } else {
                let _ = autostart_manager.disable();
            }

            debug!(
                "registered for autostart? {}",
                autostart_manager.is_enabled().unwrap_or(false)
            );

            // Start health check service (macos only)
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_health_check(app_handle_clone).await {
                    error!("Failed to start health check service: {}", e);
                }
            });

            #[cfg(target_os = "macos")]
            crate::window::reset_to_regular_and_refresh_tray(&app_handle);

            // NOTE: Accessory mode watchdog removed — we no longer toggle activation policy
            // The app stays in Regular mode permanently so dock+tray are always visible.

            #[cfg(feature = "cloud-sync")]
            {
                let app_for_sync = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = work_insights_engine::reconcile(app_for_sync).await {
                        warn!(%error, "failed to reconcile cloud sync at startup");
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // CRITICAL: This closure is called from tao::send_event (Obj-C FFI, nounwind).
        // Unwinding cannot cross that boundary, so catch_unwind never runs — any panic
        // triggers panic_cannot_unwind and abort(). Do not use unwrap/expect/panic! here
        // or in any code this synchronously calls (e.g. ShowRewindWindow::show/close).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match event {
                tauri::RunEvent::Ready { .. } => debug!("Ready event"),
                tauri::RunEvent::ExitRequested { api, .. } => {
                    // When the user clicks "quit dystil" in the tray menu,
                    // QUIT_REQUESTED is set to true — let the exit proceed.
                    // Otherwise, prevent auto-exit so the app stays alive in the
                    // tray when all windows are closed / destroyed.
                    if tray::QUIT_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
                        info!("ExitRequested event — quit was requested, allowing exit");
                    } else {
                        info!("ExitRequested event — preventing (app stays in tray)");
                        api.prevent_exit();
                    }
                }

                tauri::RunEvent::Exit => {
                    info!("App exiting — running cleanup");

                    // Shut down capture and the SQLite runtime before exit.
                    //
                    // Run on a dedicated thread to avoid "Cannot start a runtime from within
                    // a runtime" panic when the Exit event fires from a tokio async context.
                    let app_handle_shutdown = app_handle.app_handle().clone();
                    let _ = std::thread::spawn(move || {
                        tauri::async_runtime::block_on(async move {
                            if let Some(recording_state) =
                                app_handle_shutdown.try_state::<recording::RecordingState>()
                            {
                                // Stop capture first (self-contained), then server
                                recording_state
                                    .capture_active
                                    .store(false, Ordering::SeqCst);
                                if let Some(session) = recording_state.capture.lock().await.take() {
                                    session.stop().await;
                                }
                                if let Some(server) = recording_state.server.lock().await.take() {
                                    server.shutdown().await;
                                }
                            }
                        })
                    })
                    .join();
                }

                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    // Defer off the event stack so run handler stays panic-free.
                    // Open the settings/app window (not the timeline overlay).
                    let app = app_handle.app_handle().clone();
                    let _ = app_handle.app_handle().run_on_main_thread(move || {
                        let _ = ShowRewindWindow::Home { page: None }.show(&app);
                    });
                }
                _ => {}
            }
        })); // end catch_unwind
        if let Err(e) = result {
            error!("panic in run event handler: {:?}", e);
        }
    });
}

#[cfg(test)]
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    define_specta_builder!()
}
