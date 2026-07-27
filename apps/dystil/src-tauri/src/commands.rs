use crate::{
    native_notification,
    recording::RecordingState,
    store::{OnboardingStore, SettingsStore, SyncConsent},
    window::{RewindWindowId, ShowRewindWindow},
};
use tauri::{Emitter, Manager, State};
use tracing::{debug, error, info, warn};

/// Log a `WebviewWindowBuilder::build()` failure with structured context.
///
/// Why: Sentry events for webview build failures currently say only
/// "failed to create webview: WebView2 error: …". Without knowing which
/// window was being built (pipe-store, login, notifications, etc.) we
/// can't triage.
///
/// Tracing's `sentry` layer (see `main.rs`) maps structured fields to
/// Sentry tags, so `webview_label` and `webview_url` become filterable
/// tags in the Sentry dashboard.
///
/// Call at every `WebviewWindowBuilder::build()` error site instead of
/// a bare `error!(...)`. Return the error unchanged — this function is
/// purely observability.
fn log_webview_build_failure(label: &str, url_hint: &str, err: &(impl std::fmt::Display + ?Sized)) {
    tracing::error!(
        webview_label = label,
        webview_url = url_hint,
        "failed to create webview (label={}, url={}): {}",
        label,
        url_hint,
        err
    );
}

/// Global app handle stored so the native notification action callback can emit events.
#[cfg(target_os = "macos")]
static GLOBAL_APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Callback invoked from Swift when user clicks a notification action.
/// Handles "manage" directly in Rust (opens home window to notifications settings).
/// Other actions are forwarded as Tauri events to JS.
///
/// A Rust panic crossing this Cocoa→Rust trampoline aborts the whole app via
/// `panic_cannot_unwind` (extern "C" can't unwind through ObjC frames). Catch
/// any panic and log it instead — losing one notification click is much better
/// than killing the user's session.
#[cfg(target_os = "macos")]
extern "C" fn native_notif_action_callback(json_ptr: *const std::os::raw::c_char) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        native_notif_action_callback_inner(json_ptr);
    }));
}

#[cfg(target_os = "macos")]
fn native_notif_action_callback_inner(json_ptr: *const std::os::raw::c_char) {
    if json_ptr.is_null() {
        return;
    }
    let json = unsafe { std::ffi::CStr::from_ptr(json_ptr) }
        .to_string_lossy()
        .to_string();
    info!("native notification action: {}", json);

    let Some(app) = GLOBAL_APP_HANDLE.get() else {
        return;
    };

    // Parse once so downstream branches can dispatch on structured fields
    // instead of doing fragile substring matches on the JSON string.
    let parsed: Option<serde_json::Value> = serde_json::from_str(&json).ok();
    let action_type = parsed
        .as_ref()
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str());

    // "manage" — open the Home window to notifications settings. Handled in
    // Rust rather than via JS emit so it works even when no React window is
    // currently mounted.
    if action_type == Some("manage") {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let app_for_show = app_clone.clone();
            let _ = app_clone.run_on_main_thread(move || {
                if let Err(e) = (ShowRewindWindow::Home { page: None }).show(&app_for_show) {
                    error!("failed to show home window for manage: {}", e);
                }
            });
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = app_clone.emit(
                "navigate",
                serde_json::json!({ "url": "/home?section=notifications" }),
            );
        });
        return;
    }

    // Compound meeting action: open the actual call URL, then route the app to
    // the live note. This is intentionally separate from generic link/deeplink
    // handling because meeting-start notifications need both side effects.
    if action_type == Some("meeting_join") {
        let meeting_url = parsed
            .as_ref()
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let deeplink_url = parsed
            .as_ref()
            .and_then(|v| v.get("deeplink_url").or_else(|| v.get("deeplinkUrl")))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let Some(meeting_url) = meeting_url else {
            warn!("meeting_join notification action has no url: {}", json);
            return;
        };

        let app_clone = app.clone();
        std::thread::spawn(move || {
            use tauri_plugin_opener::OpenerExt;
            if let Err(e) = app_clone.opener().open_url(&meeting_url, None::<&str>) {
                error!(
                    "failed to open meeting url '{}' from notification: {}",
                    meeting_url, e
                );
            }

            let Some(deeplink_url) = deeplink_url else {
                return;
            };
            if deeplink_url.starts_with("dystil://") {
                let app_for_show = app_clone.clone();
                let _ = app_clone.run_on_main_thread(move || {
                    if let Err(e) = ShowRewindWindow::Main.show(&app_for_show) {
                        error!("failed to show window for deeplink: {}", e);
                    }
                });
                let _ = app_clone.emit("deep-link-received", deeplink_url);
            }
        });
        return;
    }

    // URL-opening actions. Two distinct semantics, explicit types so senders
    // can't conflate them:
    //   "link"      → external URL, opened in the user's default browser
    //   "deeplink"  → dystil:// in-app route, dispatched to DeeplinkHandler
    //
    // Both are handled in Rust rather than via JS emit so clicks work even
    // when no frontend notification listener is mounted. Previous
    // implementation relied on a webview-side listener and silently did
    // nothing when the app surface wasn't running.
    if action_type == Some("link") || action_type == Some("deeplink") {
        let url = parsed
            .as_ref()
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(url) = url else {
            warn!(
                "{} notification action has no url: {}",
                action_type.unwrap(),
                json
            );
            return;
        };

        // Guard against senders putting a browser URL into "deeplink" or a
        // dystil:// URL into "link". We route on actual scheme, not on
        // the declared type, so a typo doesn't break the click.
        let is_in_app = url.starts_with("dystil://");
        let app_clone = app.clone();
        std::thread::spawn(move || {
            if is_in_app {
                let app_for_show = app_clone.clone();
                let _ = app_clone.run_on_main_thread(move || {
                    if let Err(e) = ShowRewindWindow::Main.show(&app_for_show) {
                        error!("failed to show window for deeplink: {}", e);
                    }
                });
                std::thread::sleep(std::time::Duration::from_millis(150));
                let _ = app_clone.emit("deep-link-received", url);
            } else {
                // External URL — hand off to the opener plugin.
                use tauri_plugin_opener::OpenerExt;
                if let Err(e) = app_clone.opener().open_url(&url, None::<&str>) {
                    error!("failed to open url '{}' from notification: {}", url, e);
                }
            }
        });
        return;
    }

    // Everything else (pipe, api, mute, dismiss, auto_dismiss, legacy string
    // actions) still goes to the JS handler.
    let _ = app.emit("native-notification-action", &json);
}

/// Return the macOS bundle identifier of the running app
/// (e.g. `dystil`, `dystil.beta`, `dystil.dev`). The onboarding stuck-screen
/// surfaces this so users who switched build channels (prod ↔ beta ↔ dev) can
/// see they're looking at a *different* TCC record from the one they may have
/// already granted under a sibling bundle id.
#[tauri::command]
#[specta::specta]
pub fn get_app_identifier(app_handle: tauri::AppHandle) -> String {
    app_handle.config().identifier.clone()
}

/// Get the app-local focus/notification server port.
#[tauri::command]
#[specta::specta]
pub fn get_app_server_config() -> serde_json::Value {
    let port = std::env::var("DYSTIL_FOCUS_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(11435);

    serde_json::json!({ "port": port })
}

/// Read the user's dystil cloud session JWT from `~/.dystil/
/// auth.json`. Returns None when the file is missing, malformed, or the
/// token field is empty.
///
/// The settings store (`store.bin → user.token`) is the canonical
/// runtime cache for this token but is only populated after a fresh
/// in-app sign-in. `auth.json` is the durable on-disk copy written by
/// the pi-agent configuration flow — it survives store resets and dev-
/// mode launches where the in-memory user object hasn't been hydrated
/// yet. Used by the enterprise-policy hook to send the Bearer header
/// even when the in-app user object is still null.
#[tauri::command]
#[specta::specta]
pub fn get_cloud_token() -> Option<String> {
    let path = crate::dystil_paths::data_dir().join("auth.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("token")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Push a fresh cloud-auth token into the running sidecar.
///
/// The frontend invokes this on every sign-in (after `loadUser` writes
/// `settings.user`) and on sign-out (passing `None`). Without it, the
/// `Server.cloud_token` and `PiExecutor.user_token` captured at engine
/// boot would be permanent for the lifetime of the sidecar process —
/// users who signed in AFTER the engine started would stay on the
/// gateway's anonymous tier on every pipe run. Logout + log-in from
/// the webview alone does NOT restart the sidecar, which is why the
/// previous user-facing workaround was "fully quit the app from the tray."
///
/// The pi-agent's `models.json` apiKey shares the same
/// `Arc<ArcSwap<Option<String>>>`, so one write here updates it on the
/// next pipe run.
#[tauri::command]
#[specta::specta]
pub async fn set_cloud_token(
    token: Option<String>,
    state: tauri::State<'_, crate::recording::RecordingState>,
) -> Result<(), String> {
    let normalized = token.filter(|t| !t.is_empty());
    state.cloud_token.store(std::sync::Arc::new(normalized));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn write_browser_log(level: String, message: String) {
    match level.as_str() {
        "error" => error!("[webview] {}", message),
        "warn" => warn!("[webview] {}", message),
        "debug" => debug!("[webview] {}", message),
        _ => info!("[webview] {}", message),
    }
}

#[derive(serde::Deserialize, specta::Type)]
pub struct BrowserLogEntry {
    pub level: String,
    pub message: String,
}

#[tauri::command]
#[specta::specta]
pub fn write_browser_logs(entries: Vec<BrowserLogEntry>) {
    for entry in entries {
        match entry.level.as_str() {
            "error" => error!("[webview] {}", entry.message),
            "warn" => warn!("[webview] {}", entry.message),
            "debug" => debug!("[webview] {}", entry.message),
            _ => info!("[webview] {}", entry.message),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_tray_unhealth_icon(app_handle: tauri::AppHandle) {
    let app = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(main_tray) = app.tray_by_id("dystil_main") {
            match crate::safe_icon::load_main_tray_icon(&app) {
                Ok(icon) => {
                    if let Err(e) = crate::safe_icon::safe_set_icon(&main_tray, icon) {
                        error!("failed to set tray unhealthy icon: {}", e);
                    }
                }
                Err(e) => {
                    error!("failed to load tray unhealthy icon: {}", e);
                }
            }
        }
    });
}

#[tauri::command]
#[specta::specta]
pub fn set_tray_health_icon(app_handle: tauri::AppHandle) {
    let app = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(main_tray) = app.tray_by_id("dystil_main") {
            match crate::safe_icon::load_main_tray_icon(&app) {
                Ok(icon) => {
                    if let Err(e) = crate::safe_icon::safe_set_icon(&main_tray, icon) {
                        error!("failed to set tray healthy icon: {}", e);
                    }
                }
                Err(e) => {
                    error!("failed to load tray healthy icon: {}", e);
                }
            }
        }
    });
}

#[tauri::command]
#[specta::specta]
pub fn show_main_window(app_handle: tauri::AppHandle) {
    info!("show_main_window called");
    let window_to_show = ShowRewindWindow::Main;

    match window_to_show.show(&app_handle) {
        Ok(window) => {
            info!(
                "show_main_window succeeded, window label: {}",
                window.label()
            );
            // Don't call set_focus() on macOS — both overlay and window modes use
            // NSPanel with order_front_regardless() which handles visibility correctly.
            // Calling set_focus() causes macOS space switching.
            #[cfg(not(target_os = "macos"))]
            if let Err(e) = window.set_focus() {
                error!("Failed to set focus on main window: {}", e);
            }

            // Emit window-focused so the timeline refreshes immediately.
            // Without this, opening via tray/shortcut (where the window was
            // already "focused" or never lost focus) wouldn't trigger a re-fetch.
            let _ = app_handle.emit("window-focused", true);
        }
        Err(e) => {
            error!("ShowRewindWindow::Main.show failed: {}", e);
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn hide_main_window(app_handle: tauri::AppHandle) {
    let window_to_close = ShowRewindWindow::Main;

    if let Err(e) = window_to_close.close(&app_handle) {
        error!("failed to close window: {}", e);
    }
}

fn present_existing_window(app_handle: &tauri::AppHandle, label: &str) -> bool {
    match label {
        "main" | "main-window" => {
            let _ = ShowRewindWindow::Main.show(app_handle);
            true
        }
        "home" => {
            let _ = ShowRewindWindow::Home { page: None }.show(app_handle);
            true
        }
        "search" => {
            let _ = ShowRewindWindow::Search { query: None }.show(app_handle);
            true
        }
        "onboarding" => {
            let _ = ShowRewindWindow::Onboarding.show(app_handle);
            true
        }
        "permission-recovery" => {
            let _ = ShowRewindWindow::PermissionRecovery.show(app_handle);
            true
        }
        _ => {
            if let Some(window) = app_handle.get_webview_window(label) {
                let _ = window.unminimize();
                let _ = window.show();
                #[cfg(not(target_os = "macos"))]
                let _ = window.set_focus();
                let _ = app_handle.emit("window-focused", true);
                true
            } else {
                false
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn focus_existing_window(app_handle: tauri::AppHandle) {
    info!("focus_existing_window called");

    let known_labels = [
        "main",
        "main-window",
        "home",
        "search",
        "onboarding",
        "permission-recovery",
        "google-calendar-auth",
    ];

    if let Some(label) = app_handle
        .webview_windows()
        .values()
        .find(|window| window.is_focused().unwrap_or(false))
        .map(|window| window.label().to_string())
    {
        if present_existing_window(&app_handle, &label) {
            return;
        }
    }

    if let Some(label) = known_labels.iter().find(|label| {
        app_handle
            .get_webview_window(label)
            .map(|window| window.is_visible().unwrap_or(false))
            .unwrap_or(false)
    }) {
        if present_existing_window(&app_handle, label) {
            return;
        }
    }

    if let Some(label) = known_labels
        .iter()
        .find(|label| app_handle.get_webview_window(label).is_some())
    {
        if present_existing_window(&app_handle, label) {
            return;
        }
    }

    info!("focus_existing_window: no existing dystil window found");
}

/// Enable click-through mode on the main overlay window (Windows only)
/// When enabled, mouse events pass through to windows below
#[tauri::command]
#[specta::specta]
pub fn enable_overlay_click_through(_app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(window) = _app_handle.get_webview_window("main") {
            crate::windows_overlay::enable_click_through(&window)?;
        }
    }
    Ok(())
}

/// Disable click-through mode on the main overlay window (Windows only)
/// When disabled, the overlay receives mouse events normally
#[tauri::command]
#[specta::specta]
pub fn disable_overlay_click_through(_app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(window) = _app_handle.get_webview_window("main") {
            crate::windows_overlay::disable_click_through(&window)?;
        }
    }
    Ok(())
}

/// Check if click-through is currently enabled (Windows only)
#[tauri::command]
#[specta::specta]
pub fn is_overlay_click_through(_app_handle: tauri::AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        if let Some(window) = _app_handle.get_webview_window("main") {
            return crate::windows_overlay::is_click_through_enabled(&window);
        }
    }
    false
}

#[tauri::command]
#[specta::specta]
pub async fn open_pipe_window(
    app_handle: tauri::AppHandle,
    port: u16,
    title: String,
) -> Result<(), String> {
    // Close existing window if it exists
    if let Some(existing_window) = app_handle.get_webview_window(&title) {
        if let Err(e) = existing_window.destroy() {
            error!("failed to destroy existing window: {}", e);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    let url = format!("http://localhost:{}", port);
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app_handle,
        &title,
        tauri::WebviewUrl::External(url.parse().unwrap()),
    )
    .title(title.clone())
    .inner_size(1200.0, 850.0)
    .min_inner_size(600.0, 400.0)
    .focused(true)
    .fullscreen(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder.hidden_title(true);
    }

    let window = match builder.build().map(crate::window::finalize_webview_window) {
        Ok(window) => window,
        Err(e) => {
            log_webview_build_failure(&title, &url, &e);
            return Err(format!("failed to create window: {}", e));
        }
    };

    // flag to prevent infinite loop
    let is_closing = std::sync::Arc::new(std::sync::Mutex::new(false));
    let is_closing_clone = std::sync::Arc::clone(&is_closing);

    // event listener for the window close event
    let window_clone = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let mut is_closing = is_closing_clone.lock().unwrap_or_else(|e| e.into_inner());
            if *is_closing {
                return;
            }
            *is_closing = true;
            if window_clone.is_fullscreen().unwrap_or(false) {
                let _ = window_clone.destroy();
            } else {
                api.prevent_close();
                let _ = window_clone.close();
            }
        }
    });

    // Only try to manipulate window if creation succeeded
    if let Err(e) = window.set_focus() {
        error!("failed to set window focus: {}", e);
    }
    if let Err(e) = window.show() {
        error!("failed to show window: {}", e);
    }

    #[cfg(target_os = "macos")]
    crate::window::reset_to_regular_and_refresh_tray(&app_handle);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_disk_usage(
    _app_handle: tauri::AppHandle,
    force_refresh: Option<bool>,
    data_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    let dystil_dir_path = match data_dir {
        Some(d) if !d.is_empty() && d != "default" => std::path::PathBuf::from(d),
        _ => crate::dystil_paths::data_dir(),
    };

    match crate::disk_usage::disk_usage(&dystil_dir_path, force_refresh.unwrap_or(false)).await {
        Ok(Some(disk_usage)) => match serde_json::to_value(&disk_usage) {
            Ok(json_value) => Ok(json_value),
            Err(e) => {
                error!("Failed to serialize disk usage: {}", e);
                Err(format!("Failed to serialize disk usage: {}", e))
            }
        },
        Ok(None) => Err("No disk usage data found".to_string()),
        Err(e) => {
            error!("Failed to get disk usage: {}", e);
            Err(format!("Failed to get disk usage: {}", e))
        }
    }
}

/// Open Google Calendar OAuth inside an in-app WebView.
/// Intercepts the dystil:// deep-link redirect so we don't rely on Safari
/// custom-scheme support.
#[allow(dead_code)] // invoked via Tauri IPC, not direct Rust calls
#[tauri::command]
#[specta::specta]
pub async fn open_google_calendar_auth_window(
    app_handle: tauri::AppHandle,
    auth_url: String,
) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let label = "google-calendar-auth";

    // If already open, just focus it
    if let Some(w) = app_handle.get_webview_window(label) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let app_for_nav = app_handle.clone();

    let parsed_url = auth_url.parse().map_err(|e| format!("invalid url: {e}"))?;
    let mut builder =
        WebviewWindowBuilder::new(&app_handle, label, WebviewUrl::External(parsed_url))
            .title("connect google calendar")
            .inner_size(500.0, 700.0)
            .focused(true);

    #[cfg(target_os = "macos")]
    {
        builder = builder.hidden_title(true);
    }

    builder = builder.on_navigation(move |url| {
        if url.scheme() == "dystil" {
            info!("google calendar auth window intercepted deep link: {}", url);
            let _ = app_for_nav.emit("deep-link-received", url.to_string());
            if let Some(w) = app_for_nav.get_webview_window("google-calendar-auth") {
                let _ = w.close();
            }
            false // block navigation to custom scheme
        } else {
            true // allow all https navigations (Google OAuth, etc.)
        }
    });
    builder
        .build()
        .map(crate::window::finalize_webview_window)
        .map_err(|e| {
            log_webview_build_failure(label, &auth_url, &e);
            e.to_string()
        })?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn show_window(
    app_handle: tauri::AppHandle,
    window: ShowRewindWindow,
) -> Result<(), String> {
    // Close Main window when opening other windows, EXCEPT for Search
    let window_id = window.id();
    if !matches!(window_id, RewindWindowId::Main | RewindWindowId::Search) {
        // Hide Main without restoring the previous frontmost app — we're
        // transitioning to another dystil window so focus should stay
        // with us, not bounce to the previous app.
        ShowRewindWindow::Main
            .hide_without_restore(&app_handle)
            .map_err(|e| e.to_string())?;
    }

    // Hide Main timeline when opening Search (search is standalone, timeline shows on result pick)
    if matches!(window_id, RewindWindowId::Search) {
        hide_main_window(app_handle.clone());
    }

    window.show(&app_handle).map_err(|e| e.to_string())?;
    Ok(())
}

/// Like `show_window` but forces macOS app activation first, so the target
/// window actually comes to the foreground when the caller is a
/// `NSNonactivatingPanelMask` panel (notifications, tray, etc.).
///
/// Without this, clicking "Open" in the notification panel on macOS often
/// appears to do nothing: the non-activating panel style prevents the app
/// from becoming active, and overlay/fullscreen main modes rely on an
/// activate-aware `show_panel_visible(activate_app=true)` path that only
/// fires for `overlay_mode == "window"`. The window technically shows but
/// stays behind whatever app the user was in.
///
/// Callers that represent explicit user intent (clicking Open on a
/// notification) should use this variant. Passive show-surface callers
/// should keep using `show_window` to avoid stealing focus unnecessarily.
#[tauri::command]
#[specta::specta]
pub async fn show_window_activated(
    app_handle: tauri::AppHandle,
    window: ShowRewindWindow,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app_handle
            .run_on_main_thread(|| {
                use objc::{msg_send, sel, sel_impl};
                use tauri_nspanel::cocoa::base::id;
                unsafe {
                    let ns_app: id = msg_send![objc::class!(NSApplication), sharedApplication];
                    let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];
                }
            })
            .map_err(|e| format!("failed to activate app: {}", e))?;
    }
    show_window(app_handle, window).await
}

/// Programmatically adjust a window's always-on-top level after creation.
///
/// Tauri's JS `setAlwaysOnTop` can be unreliable for macOS panel-style
/// windows. For permission flows we need Dystil to stay normally
/// always-on-top, but temporarily drop below System Settings while the user is
/// granting permissions. On macOS this directly sets the underlying NSWindow
/// level: floating when enabled, normal when disabled.
#[tauri::command]
#[specta::specta]
pub async fn set_window_always_on_top_native(
    app_handle: tauri::AppHandle,
    label: String,
    always_on_top: bool,
) -> Result<(), String> {
    use tauri::Manager;

    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| format!("window not found: {}", label))?;

    window
        .set_always_on_top(always_on_top)
        .map_err(|e| format!("failed to set always-on-top: {}", e))?;

    #[cfg(target_os = "macos")]
    {
        use crate::window::run_on_main_thread_safe;
        use raw_window_handle::HasWindowHandle;

        let window_clone = window.clone();
        run_on_main_thread_safe(&app_handle, move || {
            if let Ok(handle) = window_clone.window_handle() {
                if let raw_window_handle::RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
                    use objc::{msg_send, sel, sel_impl};
                    let ns_view = appkit_handle.ns_view.as_ptr() as *mut objc::runtime::Object;
                    let ns_window: *mut objc::runtime::Object =
                        unsafe { msg_send![ns_view, window] };
                    if !ns_window.is_null() {
                        // NSNormalWindowLevel = 0. NSFloatingWindowLevel = 3.
                        // Floating keeps recovery/onboarding above normal app
                        // windows; normal lets System Settings sit above it.
                        let level: i64 = if always_on_top { 3 } else { 0 };
                        let _: () = unsafe { msg_send![ns_window, setLevel: level] };
                    }
                }
            }
        });
    }

    Ok(())
}

/// Re-assert the WKWebView as first responder for the current key panel.
/// Called from JS on pointer enter / window focus to ensure trackpad pinch
/// gestures (magnifyWithEvent:) reach the WKWebView for zoom handling.
#[tauri::command]
#[specta::specta]
pub async fn ensure_webview_focus(_app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use crate::window::run_on_main_thread_safe;
        use tauri_nspanel::ManagerExt;

        let app = _app_handle.clone();
        run_on_main_thread_safe(&_app_handle, move || {
            for label in &["main", "main-window"] {
                if let Ok(panel) = app.get_webview_panel(label) {
                    unsafe {
                        crate::window::make_webview_first_responder(&panel);
                    }
                    return;
                }
            }
        });
    }
    Ok(())
}

/// Resize the Search NSPanel. Regular Tauri setSize doesn't work on NSPanels.
#[tauri::command]
#[specta::specta]
pub async fn resize_search_window(
    app_handle: tauri::AppHandle,
    width: f64,
    height: f64,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use crate::window::run_on_main_thread_safe;
        use tauri_nspanel::ManagerExt;

        let app = app_handle.clone();
        run_on_main_thread_safe(&app_handle, move || {
            let label = RewindWindowId::Search.label();
            // Check window still exists before touching the panel
            if app.get_webview_window(&label).is_none() {
                return;
            }
            if let Ok(panel) = app.get_webview_panel(&label) {
                unsafe {
                    use objc::{msg_send, sel, sel_impl};
                    use tauri_nspanel::cocoa::foundation::{NSPoint, NSRect, NSSize};

                    // Get current frame to preserve position (x, y)
                    let frame: NSRect = msg_send![&*panel, frame];
                    // New frame: keep x, adjust y so top edge stays fixed
                    let new_h = height;
                    let new_y = frame.origin.y + frame.size.height - new_h;
                    let new_frame = NSRect::new(
                        NSPoint::new(frame.origin.x, new_y),
                        NSSize::new(width, new_h),
                    );
                    // animate: false (NO) to avoid use-after-free if panel closes mid-animation
                    let _: () =
                        msg_send![&*panel, setFrame: new_frame display: true animate: false];
                }
            } else {
                // Fallback: try as regular window
                if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.set_size(tauri::LogicalSize::new(width, height));
                }
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let label = RewindWindowId::Search.label();
        if let Some(window) = app_handle.get_webview_window(&label) {
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
        }
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn close_window(
    app_handle: tauri::AppHandle,
    window: ShowRewindWindow,
) -> Result<(), String> {
    // Emit window-hidden event so React components can clean up
    let _ = app_handle.emit("window-hidden", ());

    window.close(&app_handle).map_err(|e| e.to_string())?;
    Ok(())
}

// Permission recovery command
#[tauri::command]
#[specta::specta]
/// Hide the Main panel so the next shortcut press reconfigures it for the new mode.
pub fn reset_main_window(app_handle: tauri::AppHandle) {
    info!("reset_main_window: hiding all Main panels for mode switch");

    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        let app_clone = app_handle.clone();
        let _ = app_handle.run_on_main_thread(move || {
            for label in &["main", "main-window"] {
                if let Ok(panel) = app_clone.get_webview_panel(label) {
                    panel.order_out(None);
                }
            }
        });
        crate::window::reset_to_regular_and_refresh_tray(&app_handle);
    }

    #[cfg(not(target_os = "macos"))]
    {
        for label in &["main", "main-window"] {
            if let Some(window) = app_handle.get_webview_window(label) {
                let _ = window.destroy();
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn show_permission_recovery_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    ShowRewindWindow::PermissionRecovery
        .show(&app_handle)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// Onboarding commands
#[tauri::command]
#[specta::specta]
pub async fn get_onboarding_status(
    app_handle: tauri::AppHandle,
) -> Result<OnboardingStore, String> {
    OnboardingStore::get(&app_handle).map(|o| o.unwrap_or_default())
}

/// Read local cloud-sync consent. Defaults are fail-closed for both new and
/// existing installations because `syncConsent` is serde-defaulted.
#[tauri::command]
#[specta::specta]
pub async fn get_sync_consent(app_handle: tauri::AppHandle) -> Result<SyncConsent, String> {
    Ok(SettingsStore::get(&app_handle)?
        .unwrap_or_default()
        .sync_consent)
}

/// Persist explicit local cloud-sync consent. Screenshot uploads are never
/// allowed independently of segment uploads.
#[tauri::command]
#[specta::specta]
pub async fn set_sync_consent(
    app_handle: tauri::AppHandle,
    consent: SyncConsent,
) -> Result<SyncConsent, String> {
    let consent = consent.validate()?;
    let mut settings = SettingsStore::get(&app_handle)?.unwrap_or_default();
    settings.sync_consent = consent;
    settings.save(&app_handle)?;

    #[cfg(feature = "cloud-sync")]
    crate::work_insights_engine::reconcile(app_handle.clone()).await?;

    Ok(consent)
}

#[tauri::command]
#[specta::specta]
pub async fn complete_onboarding(
    app_handle: tauri::AppHandle,
    onboarding_data: serde_json::Value,
) -> Result<(), String> {
    if let Err(error) = crate::auth::enqueue_onboarding_data_sync(onboarding_data).await {
        warn!("failed to queue onboarding data for sync: {}", error);
    } else {
        tauri::async_runtime::spawn(async {
            if let Err(error) = crate::auth::flush_pending_onboarding_data().await {
                warn!("failed to flush onboarding data in background: {}", error);
            }
        });
    }

    // Update the persistent store
    OnboardingStore::update(&app_handle, |onboarding| {
        onboarding.complete();
    })
    .map_err(|e| e.to_string())?;

    // Update the managed state in memory
    if let Some(managed_store) = app_handle.try_state::<OnboardingStore>() {
        // Get the current state and create an updated version
        let mut updated_store = managed_store.inner().clone();
        updated_store.complete();
        // Replace the managed state with the updated version
        app_handle.manage(updated_store);
    }

    let _ = app_handle.emit("onboarding-completed", ());

    Ok(())
}

/// Persist the explicit social-media capture exceptions chosen during
/// onboarding, then refresh an active capture session so the new privacy
/// filters take effect immediately. A paused session stays paused.
#[tauri::command]
#[specta::specta]
pub async fn apply_onboarding_capture_policy(
    app_handle: tauri::AppHandle,
    state: State<'_, RecordingState>,
    selected_social_services: Vec<String>,
) -> Result<(), String> {
    let mut settings = SettingsStore::get(&app_handle)?.unwrap_or_default();
    settings.apply_social_capture_policy(&selected_social_services);
    settings.save(&app_handle)?;

    let was_recording = state
        .capture_active
        .load(std::sync::atomic::Ordering::SeqCst);
    info!(
        selected_social_services = ?selected_social_services,
        was_recording,
        "applied onboarding social capture policy"
    );

    if !was_recording {
        return Ok(());
    }

    crate::recording::stop_capture(state.clone(), app_handle.clone()).await?;
    crate::recording::start_capture(state, app_handle).await
}

/// Persist the user's explicit screenshot-capture choice and refresh an
/// active capture session so the new mode takes effect immediately. A paused
/// session stays paused. Enabling screenshots is rejected unless the platform
/// reports Screen Recording permission as granted (or not required).
#[tauri::command]
#[specta::specta]
pub async fn set_screenshot_capture_enabled(
    app_handle: tauri::AppHandle,
    state: State<'_, RecordingState>,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        let permissions = crate::permissions::do_permissions_check(false);
        if !permissions.screen_recording.permitted() {
            return Err(
                "Screen Recording permission is required before screenshots can be enabled."
                    .to_string(),
            );
        }
    }

    let mut settings = SettingsStore::get(&app_handle)?.unwrap_or_default();
    let previous_disable_vision = settings.recording.disable_vision;
    let next_disable_vision = !enabled;
    if previous_disable_vision == next_disable_vision {
        return Ok(());
    }

    let was_recording = state
        .capture_active
        .load(std::sync::atomic::Ordering::SeqCst);
    settings.recording.disable_vision = next_disable_vision;
    settings.save(&app_handle)?;

    info!(
        enabled,
        was_recording, "updated explicit screenshot capture preference"
    );

    if !was_recording {
        return Ok(());
    }

    if let Err(error) = crate::recording::stop_capture(state.clone(), app_handle.clone()).await {
        settings.recording.disable_vision = previous_disable_vision;
        let _ = settings.save(&app_handle);
        return Err(error);
    }

    if let Err(error) = crate::recording::start_capture(state.clone(), app_handle.clone()).await {
        // Do not strand a previously running session if applying the new mode
        // fails. Restore both the persisted preference and the old session.
        settings.recording.disable_vision = previous_disable_vision;
        let rollback_save_error = settings.save(&app_handle).err();
        let rollback_start_error = crate::recording::start_capture(state, app_handle)
            .await
            .err();
        return Err(format!(
            "Failed to apply screenshot capture preference: {error}. Rollback save: {}. Rollback start: {}",
            rollback_save_error
                .as_deref()
                .unwrap_or("ok"),
            rollback_start_error
                .as_deref()
                .unwrap_or("ok")
        ));
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn reset_onboarding(app_handle: tauri::AppHandle) -> Result<(), String> {
    // Update the persistent store
    OnboardingStore::update(&app_handle, |onboarding| {
        onboarding.reset();
    })?;

    // Update the managed state in memory
    if let Some(managed_store) = app_handle.try_state::<OnboardingStore>() {
        // Get the current state and create an updated version
        let mut updated_store = managed_store.inner().clone();
        updated_store.reset();
        // Replace the managed state with the updated version
        app_handle.manage(updated_store);
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_onboarding_step(app_handle: tauri::AppHandle, step: String) -> Result<(), String> {
    OnboardingStore::update(&app_handle, |onboarding| {
        onboarding.current_step = Some(step);
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn show_onboarding_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    ShowRewindWindow::Onboarding
        .show(&app_handle)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// Keychain / secure storage commands

#[derive(serde::Serialize, specta::Type)]
pub struct KeychainStatus {
    pub state: String,
}

#[tauri::command]
#[specta::specta]
pub async fn get_keychain_status() -> Result<KeychainStatus, String> {
    // Check if encryption is enabled WITHOUT accessing keychain.
    // We only touch keychain when the user explicitly opts in via enable_keychain_encryption().
    // This prevents prompts during onboarding permission checks.
    let is_enabled = crate::secrets::is_encryption_enabled();

    let state = if !is_enabled {
        // Encryption not enabled in settings — definitely disabled
        "disabled"
    } else {
        // Encryption is enabled, but only check keychain key if we actually need it
        // (e.g., when loading secrets). Don't touch keychain just to report status.
        match crate::secrets::get_key() {
            crate::secrets::KeyResult::NotFound => "disabled",
        }
    };

    Ok(KeychainStatus {
        state: state.to_string(),
    })
}

/// Vault encryption is excluded from the Dystil product — always reports disabled.
#[tauri::command]
#[specta::specta]
pub async fn enable_keychain_encryption() -> Result<KeychainStatus, String> {
    Ok(KeychainStatus {
        state: "disabled".to_string(),
    })
}

/// Vault encryption is excluded from the Dystil product — always reports disabled.
#[tauri::command]
#[specta::specta]
pub async fn disable_keychain_encryption() -> Result<KeychainStatus, String> {
    Ok(KeychainStatus {
        state: "disabled".to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_window_size(
    app_handle: tauri::AppHandle,
    window: ShowRewindWindow,
    width: f64,
    height: f64,
) -> Result<(), String> {
    window
        .set_size(&app_handle, width, height)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_search_window(
    app_handle: tauri::AppHandle,
    query: Option<String>,
) -> Result<(), String> {
    ShowRewindWindow::Main
        .close(&app_handle)
        .map_err(|e| e.to_string())?;
    ShowRewindWindow::Search { query }
        .show(&app_handle)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn refresh_tray_menu(app_handle: tauri::AppHandle) -> Result<(), String> {
    let app_handle_clone = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            if let Err(err) = crate::tray::force_tray_rebuild(&app_handle_clone) {
                error!("tray rebuild failed: {}", err);
            }
        })
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
#[allow(unreachable_code, unused_variables)]
pub async fn show_notification_panel(
    app_handle: tauri::AppHandle,
    payload: String,
) -> Result<(), String> {
    use tauri::{Emitter, WebviewWindowBuilder};

    let label = "notification-panel";

    info!("show_notification_panel called");

    // UI notifications are muted in dystil: keep the payload flowing through
    // the backend, but never surface a visible panel.
    return Ok(());

    // On macOS, try the native SwiftUI panel first
    #[cfg(target_os = "macos")]
    {
        // Store app handle for the action callback
        let _ = GLOBAL_APP_HANDLE.set(app_handle.clone());
        native_notification::set_action_callback(native_notif_action_callback);

        if native_notification::is_available() {
            info!("Using native SwiftUI notification panel");
            if native_notification::show(&payload) {
                // Emit event so the main window can save notification history + PostHog analytics
                // (the webview panel page does this in JS, but we bypass it with native)
                let _ = app_handle.emit("native-notification-shown", &payload);
                return Ok(());
            }
            warn!("Native notification panel failed, falling back to webview");
        }
    }

    let window_width = 340.0;
    let window_height = 380.0;

    // Position at top-right of the screen where the cursor is
    let (x, y) = {
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::cocoa::appkit::{NSEvent, NSScreen};
            use tauri_nspanel::cocoa::base::{id, nil};
            use tauri_nspanel::cocoa::foundation::{NSArray, NSPoint, NSRect};
            unsafe {
                let mouse: NSPoint = NSEvent::mouseLocation(nil);
                let screens: id = NSScreen::screens(nil);
                let count: u64 = NSArray::count(screens);
                let mut x = 0.0_f64;
                let mut y = 12.0_f64;
                for i in 0..count {
                    let screen: id = NSArray::objectAtIndex(screens, i);
                    let frame: NSRect = NSScreen::frame(screen);
                    if mouse.x >= frame.origin.x
                        && mouse.x < frame.origin.x + frame.size.width
                        && mouse.y >= frame.origin.y
                        && mouse.y < frame.origin.y + frame.size.height
                    {
                        x = frame.origin.x + frame.size.width - window_width - 16.0;
                        y = 12.0;
                        break;
                    }
                }
                (x, y)
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let monitor = app_handle
                .primary_monitor()
                .map_err(|e| e.to_string())?
                .ok_or("No primary monitor found")?;
            let screen_size = monitor.size();
            let scale_factor = monitor.scale_factor();
            let x = (screen_size.width as f64 / scale_factor) - window_width - 16.0;
            (x, 12.0)
        }
    };

    // Parse autoDismissMs from payload for the server-side safety timeout
    let auto_dismiss_ms: u64 = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|v| v.get("autoDismissMs")?.as_u64())
        .unwrap_or(20000);

    // If window exists, reposition to current screen and show
    if let Some(window) = app_handle.get_webview_window(label) {
        info!("notification-panel window exists, repositioning and showing");
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
        let _ = app_handle.emit_to(label, "notification-panel-update", &payload);

        // On macOS, skip window.show() — it calls makeKeyAndOrderFront which
        // steals focus from the user's current app. Use orderFront: on the
        // NSPanel instead which respects NSNonactivatingPanelMask.
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window.show();
        }

        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            let app_clone = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                if let Ok(panel) = app_clone.get_webview_panel("notification-panel") {
                    use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;
                    use objc::{msg_send, sel, sel_impl};
                    panel.set_level(1001);
                    panel.set_style_mask(128); // NSNonactivatingPanelMask
                    panel.set_hides_on_deactivate(false);
                    panel.set_collection_behaviour(
                        NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle
                            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
                    );
                    // orderFront: (not orderFrontRegardless) respects
                    // NSNonactivatingPanelMask — shows the panel without
                    // stealing focus from the user's current app.
                    let _: () = unsafe { msg_send![&*panel, orderFront: std::ptr::null::<objc::runtime::Object>()] };
                }
            });
        }

        // Server-side safety timeout: force-hide the notification if the JS
        // auto-dismiss timer fails (e.g. webview timer throttled on Windows).
        // Adds 5s buffer so JS normally handles it first.
        let app_safety = app_handle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(auto_dismiss_ms + 5000)).await;
            if let Some(w) = app_safety.get_webview_window("notification-panel") {
                if w.is_visible().unwrap_or(false) {
                    info!("Safety timeout: force-hiding notification panel");
                    let _ = w.hide();
                }
            }
        });

        return Ok(());
    }

    info!("Creating new notification-panel window");
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(
        &app_handle,
        label,
        tauri::WebviewUrl::App("notification-panel".into()),
    )
    .title("")
    .inner_size(window_width, window_height)
    .position(x, y)
    .visible_on_all_workspaces(true)
    .always_on_top(true)
    .decorations(false)
    .skip_taskbar(true)
    .focused(false)
    .transparent(true)
    .visible(false)
    .shadow(false)
    .resizable(false);

    let window = builder
        .build()
        .map(crate::window::finalize_webview_window)
        .map_err(|e| {
            log_webview_build_failure(label, "notification-panel", &e);
            format!("Failed to create notification panel window: {}", e)
        })?;

    info!("notification-panel window created");

    // Convert to NSPanel on macOS for fullscreen support
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::WebviewWindowExt;

        if let Ok(_panel) = window.to_panel() {
            info!("Successfully converted notification-panel to panel");

            // Don't use window.show() — it calls makeKeyAndOrderFront which
            // steals focus. orderFront: in the main thread block handles visibility.

            let window_clone = window.clone();
            let _ = app_handle.run_on_main_thread(move || {
                use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;

                if let Ok(panel) = window_clone.to_panel() {
                    use objc::{msg_send, sel, sel_impl};

                    panel.set_level(1001);
                    panel.set_style_mask(128);
                    panel.set_hides_on_deactivate(false);

                    // Visible in screen capture (NSWindowSharingReadOnly = 1)
                    let _: () = unsafe { msg_send![&*panel, setSharingType: 1_u64] };

                    // Accept mouse events without requiring click-to-activate.
                    // NSNonactivatingPanelMask prevents the panel from becoming key,
                    // which blocks webview hover events. This re-enables mouse tracking.
                    let _: () = unsafe { msg_send![&*panel, setAcceptsMouseMovedEvents: true] };

                    panel.set_collection_behaviour(
                        NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle
                            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
                    );
                    // orderFront: (not orderFrontRegardless) respects
                    // NSNonactivatingPanelMask — shows without stealing focus.
                    let _: () = unsafe { msg_send![&*panel, orderFront: std::ptr::null::<objc::runtime::Object>()] };
                    info!("Notification panel configured for all-Spaces fullscreen support");
                } else {
                    error!("Failed to get notification panel in main thread");
                }
            });
        } else {
            error!("Failed to convert notification-panel to panel");
            let _ = window.show();
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.show();
    }

    // Wait for webview to mount React and register event listeners before emitting
    let app_clone = app_handle.clone();
    let payload_clone = payload.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        info!("Emitting notification-panel-update event");
        let _ = app_clone.emit_to(
            "notification-panel",
            "notification-panel-update",
            &payload_clone,
        );
    });

    // Server-side safety timeout for newly created windows too
    let app_safety = app_handle.clone();
    tokio::spawn(async move {
        // 2s wait for mount + autoDismissMs + 5s buffer
        tokio::time::sleep(std::time::Duration::from_millis(auto_dismiss_ms + 7000)).await;
        if let Some(w) = app_safety.get_webview_window("notification-panel") {
            if w.is_visible().unwrap_or(false) {
                info!("Safety timeout: force-hiding notification panel (new window)");
                let _ = w.hide();
            }
        }
    });

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn hide_notification_panel(app_handle: tauri::AppHandle) -> Result<(), String> {
    // On macOS, try hiding the native panel first
    #[cfg(target_os = "macos")]
    {
        if native_notification::is_available() {
            native_notification::hide();
            // Also hide webview panel if it exists (in case of fallback)
        }
    }

    if let Some(window) = app_handle.get_webview_window("notification-panel") {
        let _ = window.hide();

        // On macOS, window.hide() alone doesn't remove NSPanel from the hit-test
        // hierarchy when NSNonactivatingPanelMask is set. order_out ensures the
        // panel is fully removed so it can't intercept clicks on other apps.
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            let app_clone = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                if let Ok(panel) = app_clone.get_webview_panel("notification-panel") {
                    panel.order_out(None);
                }
            });
        }
    }
    Ok(())
}

/// Copy a frame deeplink (dystil://frame/N) to clipboard. Native API only.
#[tauri::command]
#[specta::specta]
pub async fn copy_deeplink_to_clipboard(frame_id: i64) -> Result<(), String> {
    let link = format!("dystil://frame/{}", frame_id);
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("clipboard error: {}", e))?;
    clipboard
        .set_text(link)
        .map_err(|e| format!("failed to set clipboard: {}", e))?;
    Ok(())
}

/// Copy arbitrary text to the system clipboard (native API, works in Tauri webview).
/// Use this instead of navigator.clipboard.writeText() which fails after async operations.
#[tauri::command]
#[specta::specta]
pub async fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("clipboard error: {}", e))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to set clipboard: {}", e))?;
    Ok(())
}

/// Open a local markdown note in Obsidian (if available), then fallback to OS default app.
#[tauri::command]
#[specta::specta]
pub async fn open_note_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let obsidian_uri = format!("obsidian://open?path={}", urlencoding::encode(&path));
        // Treat successful process launch as success. `open` can return
        // non-zero even when LaunchServices still opens the target app.
        if Command::new("open").arg(&obsidian_uri).spawn().is_ok()
            || Command::new("open").arg(&path).spawn().is_ok()
        {
            Ok(())
        } else {
            Err(format!("failed to open note path: {}", path))
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        let obsidian_uri = format!("obsidian://open?path={}", urlencoding::encode(&path));
        let mut a = Command::new("cmd");
        a.args(["/C", "start", "", &obsidian_uri]);
        a.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut b = Command::new("cmd");
        b.args(["/C", "start", "", &path]);
        b.creation_flags(0x08000000); // CREATE_NO_WINDOW
        if a.spawn().is_ok() || b.spawn().is_ok() {
            Ok(())
        } else {
            Err(format!("failed to open note path: {}", path))
        }
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        use std::process::Command;
        if Command::new("xdg-open").arg(&path).spawn().is_ok() {
            Ok(())
        } else {
            Err(format!("failed to open note path: {}", path))
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn open_windows_shell_target(target: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", &target])
            .creation_flags(0x08000000); // CREATE_NO_WINDOW

        match cmd.status() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!(
                "failed to open Windows shell target {}: {}",
                target, status
            )),
            Err(e) => Err(format!(
                "failed to open Windows shell target {}: {}",
                target, e
            )),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = target;
        Err("Windows shell targets are only supported on Windows".to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_native_theme(app_handle: tauri::AppHandle, theme: String) -> Result<(), String> {
    info!("setting native theme to: {}", theme);
    let tauri_theme = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };

    for window in app_handle.webview_windows().values() {
        let _ = window.set_theme(tauri_theme);
    }

    Ok(())
}

#[derive(serde::Serialize, specta::Type)]
pub struct CacheFile {
    pub path: String,
    pub label: String,
    pub size_bytes: u64,
}

#[tauri::command]
#[specta::specta]
pub async fn list_cache_files() -> Result<Vec<CacheFile>, String> {
    let data_dir = crate::dystil_paths::data_dir();
    let home_dir = dirs::home_dir().ok_or("no home directory")?;
    let mut files = Vec::new();

    // Pi agent node_modules (~/.dystil/pi-agent/)
    let pi_agent = data_dir.join("pi-agent");
    if pi_agent.exists() {
        let size = dir_size(&pi_agent);
        files.push(CacheFile {
            path: pi_agent.to_string_lossy().to_string(),
            label: "AI agent cache (pi-agent)".to_string(),
            size_bytes: size,
        });
    }

    // Pi config (~/.pi/agent/)
    let pi_config = home_dir.join(".pi").join("agent");
    if pi_config.exists() {
        let size = dir_size(&pi_config);
        files.push(CacheFile {
            path: pi_config.to_string_lossy().to_string(),
            label: "AI agent config (.pi/agent)".to_string(),
            size_bytes: size,
        });
    }

    // Stale root-level node_modules (~/.dystil/node_modules/)
    let root_nm = data_dir.join("node_modules");
    if root_nm.exists() {
        let size = dir_size(&root_nm);
        files.push(CacheFile {
            path: root_nm.to_string_lossy().to_string(),
            label: "Legacy node_modules".to_string(),
            size_bytes: size,
        });
    }

    // DB crash recovery/backup files
    for entry in std::fs::read_dir(&data_dir).map_err(|e| e.to_string())? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        // *.corrupt*, *.backup files
        if name.contains(".corrupt") || name.ends_with(".backup") {
            let size = if path.is_dir() {
                dir_size(&path)
            } else {
                path.metadata().map(|m| m.len()).unwrap_or(0)
            };
            files.push(CacheFile {
                path: path.to_string_lossy().to_string(),
                label: format!("DB recovery artifact: {}", name),
                size_bytes: size,
            });
        }

        // db-recovery-* and db-hotfix-* directories
        if path.is_dir() && (name.starts_with("db-recovery-") || name.starts_with("db-hotfix-")) {
            let size = dir_size(&path);
            files.push(CacheFile {
                path: path.to_string_lossy().to_string(),
                label: format!("DB recovery artifact: {}", name),
                size_bytes: size,
            });
        }

        // Old log files (dystil.*.log — legacy CLI format)
        if name.starts_with("dystil.") && name.ends_with(".log") {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(CacheFile {
                path: path.to_string_lossy().to_string(),
                label: format!("Old log: {}", name),
                size_bytes: size,
            });
        }

        // Empty/stale DB files (data.db, dystil.db, store.sqlite)
        if matches!(name.as_str(), "data.db" | "dystil.db" | "store.sqlite") {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            if size == 0 {
                files.push(CacheFile {
                    path: path.to_string_lossy().to_string(),
                    label: format!("Empty DB: {}", name),
                    size_bytes: size,
                });
            }
        }
    }

    // Stale root-level bun artifacts
    for name in &["bun.lock", "bun.lockb", "package.json"] {
        let path = data_dir.join(name);
        if path.exists() {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(CacheFile {
                path: path.to_string_lossy().to_string(),
                label: format!("Stale config: {}", name),
                size_bytes: size,
            });
        }
    }

    Ok(files)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_cache_files(paths: Vec<String>) -> Result<u64, String> {
    let mut freed = 0u64;
    for p in &paths {
        let path = std::path::Path::new(p);
        if !path.exists() {
            continue;
        }
        let size = if path.is_dir() {
            dir_size(path)
        } else {
            path.metadata().map(|m| m.len()).unwrap_or(0)
        };
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match result {
            Ok(_) => {
                info!("cache cleanup: deleted {}", p);
                freed += size;
            }
            Err(e) => warn!("cache cleanup: failed to delete {}: {}", p, e),
        }
    }
    Ok(freed)
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    total += p.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}

#[tauri::command]
#[specta::specta]
pub fn set_autostart(app_handle: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
    let manager = app_handle.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    info!(
        "autostart {}: is_enabled={}",
        if enabled { "enabled" } else { "disabled" },
        manager.is_enabled().unwrap_or(false)
    );
    Ok(())
}
