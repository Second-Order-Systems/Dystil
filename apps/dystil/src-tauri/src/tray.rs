use crate::commands::{hide_main_window, show_main_window};
use crate::health::{get_recording_info, get_recording_status, RecordingStatus};
use crate::recording::{spawn_capture, start_capture, stop_capture, RecordingState};
use crate::store::{OnboardingStore, SettingsStore};

use crate::window::ShowRewindWindow;
use anyhow::Result;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::Emitter;
use tauri::{
    menu::{
        CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem,
        SubmenuBuilder,
    },
    AppHandle, Manager, Wry,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

use tracing::{debug, error, info};

/// Flag set by the "quit dystil" menu item so that the ExitRequested
/// handler in main.rs knows this is an intentional quit (not just a window close).
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Pre-fetched data for building the tray menu. All store reads, settings
/// deserialization, and permission checks happen OFF the main thread; only
/// the lightweight menu-item construction runs on the main thread.
#[derive(Clone)]
#[allow(dead_code)]
struct TrayMenuData {
    onboarding_completed: bool,
    is_logged_in: bool,
    has_permission_issue: bool,
}

/// Gather all data needed by `create_dynamic_menu` on the current (non-main)
/// thread so the main-thread closure does zero I/O.
fn prefetch_tray_menu_data(app: &AppHandle) -> TrayMenuData {
    let onboarding_completed = OnboardingStore::get(app)
        .ok()
        .flatten()
        .map(|o| o.is_completed)
        .unwrap_or(false);

    let settings = SettingsStore::get(app)
        .unwrap_or_default()
        .unwrap_or_default();
    let is_logged_in = settings
        .user
        .token
        .as_ref()
        .map_or(false, |token| !token.is_empty())
        || settings.user.id.as_ref().map_or(false, |id| !id.is_empty());
    let has_permission_issue = if onboarding_completed {
        #[cfg(target_os = "macos")]
        {
            let perms = crate::permissions::do_permissions_check(false);
            !perms.accessibility.permitted()
                || (!settings.recording.disable_vision && !perms.screen_recording.permitted())
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    } else {
        false
    };

    TrayMenuData {
        onboarding_completed,
        is_logged_in,
        has_permission_issue,
    }
}

/// Global storage for the update menu item so we can recreate the tray
/// without needing to pass the update_item through every call chain.
static UPDATE_MENU_ITEM: Lazy<Mutex<Option<MenuItem<Wry>>>> = Lazy::new(|| Mutex::new(None));

// Track last known state to avoid unnecessary updates
static LAST_MENU_STATE: Lazy<Mutex<MenuState>> = Lazy::new(|| Mutex::new(MenuState::default()));

/// Optimistic recording status override — set on start/stop click for instant UI feedback.
/// Tuple of (status, expiry_instant). Cleared when real status matches or after timeout.
static OPTIMISTIC_STATUS: Lazy<Mutex<Option<(RecordingStatus, std::time::Instant)>>> =
    Lazy::new(|| Mutex::new(None));

fn set_optimistic_status(status: RecordingStatus) {
    let mut opt = OPTIMISTIC_STATUS.lock().unwrap_or_else(|e| e.into_inner());
    *opt = Some((
        status,
        std::time::Instant::now() + std::time::Duration::from_secs(15),
    ));
}

/// Pending "pause for X minutes" timer. Held so a manual resume — or a fresh
/// pause click — can abort the previous one and prevent a stale auto-resume
/// from firing later. The start instant + total duration are kept so the tray
/// tooltip can show a live "resumes in 12m" countdown via the existing 5-sec
/// updater loop. No persistence: app quit / crash drops the timer and
/// recording stays paused, which is the safer default for a privacy bias.
struct PauseTimer {
    handle: JoinHandle<()>,
    started: std::time::Instant,
    total: std::time::Duration,
}

static PAUSE_TIMER: Lazy<Mutex<Option<PauseTimer>>> = Lazy::new(|| Mutex::new(None));

fn cancel_pause_timer() {
    if let Some(t) = PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        t.handle.abort();
    }
}

/// Remaining time until auto-resume, if a pause timer is currently active.
/// Returns None if the timer has already fired or no timer is set.
#[allow(dead_code)]
fn pause_remaining() -> Option<std::time::Duration> {
    let guard = PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|t| {
        let elapsed = t.started.elapsed();
        if elapsed >= t.total {
            None
        } else {
            Some(t.total - elapsed)
        }
    })
}

#[allow(dead_code)]
fn format_remaining_secs(secs: u64) -> String {
    format_remaining(std::time::Duration::from_secs(secs))
}

#[allow(dead_code)]
fn format_remaining(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{}h", h)
        } else {
            format!("{}h {}m", h, m)
        }
    } else if secs >= 60 {
        format!("{}m", (secs + 59) / 60) // round up
    } else {
        format!("{}s", secs.max(1))
    }
}

fn send_notify(title: impl Into<String>, body: impl Into<String>) {
    crate::notifications::client::send(title, body);
}

/// Immediately rebuild the tray menu (called from main thread after optimistic status set).
pub(crate) fn force_tray_rebuild(app: &AppHandle) -> Result<()> {
    let update_item = UPDATE_MENU_ITEM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let state = {
        let mut last = LAST_MENU_STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Reset to force rebuild
        let s = last.clone();
        last.recording_status = None;
        s
    };
    // Build new state with effective (optimistic) status
    let effective = get_effective_recording_status();
    let mut new_state = state;
    new_state.recording_status = Some(effective);

    let data = prefetch_tray_menu_data(app);
    let menu = create_dynamic_menu(app, &new_state, update_item.as_ref(), &data)?;
    if let Some(tray) = app.tray_by_id("dystil_main") {
        install_tray_menu(&tray, menu)?;
        clear_pending_tray_menu();
    }
    // Update last state so the poller doesn't immediately rebuild again
    {
        let mut last = LAST_MENU_STATE.lock().unwrap_or_else(|e| e.into_inner());
        *last = new_state;
    }
    Ok(())
}

fn get_effective_recording_status() -> RecordingStatus {
    let real = get_recording_status();
    let mut opt = OPTIMISTIC_STATUS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((status, expiry)) = opt.as_ref() {
        if std::time::Instant::now() < *expiry {
            // Don't mask a failed start — optimistic "Starting" is only useful
            // while capture is genuinely booting, not after a terminal error or
            // when the work-hours schedule has parked capture (ScheduledPause).
            if *status == RecordingStatus::Starting
                && matches!(
                    real,
                    RecordingStatus::Error
                        | RecordingStatus::Paused
                        | RecordingStatus::ScheduledPause
                        | RecordingStatus::Stopped
                )
            {
                *opt = None;
                drop(opt);
                return real;
            }
            return status.clone();
        }
    }
    drop(opt);
    // Clear expired optimistic status
    let mut opt = OPTIMISTIC_STATUS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((ref s, _)) = *opt {
        // Clear if real status caught up or expired
        if *s == real {
            *opt = None;
        }
    }
    drop(opt);
    real
}

/// Keep the active tray menu alive and defer macOS menu replacement safely.
///
/// muda's macOS backend stores raw `*const MenuChild` pointers as NSMenuItem
/// instance variables. When `tray.set_menu(new_menu)` is called while the old
/// menu is still displayed, the old `MenuChild` items can be freed while their
/// NSMenuItems survive. Clicking an item in that stale menu makes
/// `fire_menu_item_click` dereference freed memory inside an `extern "C"`
/// callback, so catch_unwind cannot keep the process alive.
///
/// We avoid background `set_menu` on macOS. The poller caches the latest menu
/// inputs, then the tray mouse-down handler installs that menu before AppKit
/// opens the native menu.
static ACTIVE_TRAY_MENU: Lazy<Mutex<Option<tauri::menu::Menu<Wry>>>> =
    Lazy::new(|| Mutex::new(None));

static PENDING_TRAY_MENU: Lazy<Mutex<Option<(MenuState, TrayMenuData)>>> =
    Lazy::new(|| Mutex::new(None));

fn install_tray_menu(tray: &TrayIcon, menu: tauri::menu::Menu<Wry>) -> Result<()> {
    {
        let mut active = ACTIVE_TRAY_MENU.lock().unwrap_or_else(|e| e.into_inner());
        *active = Some(menu.clone());
    }
    tray.set_menu(Some(menu))?;
    Ok(())
}

fn clear_pending_tray_menu() {
    let mut pending = PENDING_TRAY_MENU.lock().unwrap_or_else(|e| e.into_inner());
    *pending = None;
}

#[cfg(target_os = "macos")]
fn queue_pending_tray_menu(state: MenuState, data: TrayMenuData) {
    let mut pending = PENDING_TRAY_MENU.lock().unwrap_or_else(|e| e.into_inner());
    *pending = Some((state, data));
}

#[cfg(target_os = "macos")]
fn apply_pending_tray_menu(app: &AppHandle) -> Result<()> {
    let pending = {
        let mut pending = PENDING_TRAY_MENU.lock().unwrap_or_else(|e| e.into_inner());
        pending.take()
    };

    let Some((state, data)) = pending else {
        return Ok(());
    };

    let update_item = UPDATE_MENU_ITEM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let menu = create_dynamic_menu(app, &state, update_item.as_ref(), &data)?;
    if let Some(tray) = app.tray_by_id("dystil_main") {
        install_tray_menu(&tray, menu)?;
    }
    Ok(())
}

#[derive(Default, PartialEq, Clone)]
struct MenuState {
    recording_status: Option<RecordingStatus>,
    onboarding_completed: bool,
    has_permission_issue: bool,
    /// Device names + active status for change detection
    devices: Vec<(String, bool)>,
    is_logged_in: bool,
}

pub fn setup_tray(app: &AppHandle, update_item: Option<&tauri::menu::MenuItem<Wry>>) -> Result<()> {
    // Store update_item globally so recreate_tray can use it (None for enterprise)
    if let Ok(mut guard) = UPDATE_MENU_ITEM.lock() {
        *guard = update_item.cloned();
    }

    if let Some(main_tray) = app.tray_by_id("dystil_main") {
        // Initial menu setup with empty state
        let data = prefetch_tray_menu_data(app);
        let menu = create_dynamic_menu(app, &MenuState::default(), update_item, &data)?;
        install_tray_menu(&main_tray, menu)?;
        clear_pending_tray_menu();

        // Setup click handlers
        setup_tray_click_handlers(&main_tray)?;

        // Set autosaveName so macOS remembers position after user Cmd+drags it
        set_autosave_name(&main_tray);
    }
    Ok(())
}

/// Destroy and recreate the tray icon to get a fresh rightmost position.
/// On MacBook Pro models with a notch, the tray icon can get pushed behind
/// the notch when there are many status bar items. Recreating it assigns
/// the rightmost (most visible) position.
///
/// IMPORTANT: NSStatusBar operations must happen on the main thread.
/// This function dispatches the work to the main thread automatically.
/// Log the tray icon position for debugging notch visibility issues.
#[allow(dead_code)] // called only on macOS
pub fn log_tray_position(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("dystil_main") {
        match tray.rect() {
            Ok(Some(rect)) => {
                info!(
                    "tray icon position: {:?} size: {:?} (if behind notch, Cmd+drag it right)",
                    rect.position, rect.size
                );
            }
            Ok(None) => {
                info!("tray icon exists but rect is None");
            }
            Err(e) => {
                error!("failed to get tray icon rect: {}", e);
            }
        }
    } else {
        error!("tray icon 'dystil_main' not found");
    }
}

#[allow(dead_code)] // called only on macOS
pub fn recreate_tray(app: &AppHandle) {
    let app_for_thread = app.clone();
    // Wrap in catch_unwind: ObjC exceptions during tray operations can panic
    // across the FFI boundary (nounwind → abort). catch_unwind prevents this.
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::window::with_autorelease_pool(|| {
                let app = app_for_thread;
                let update_item = match UPDATE_MENU_ITEM.lock() {
                    Ok(guard) => guard.clone(),
                    Err(_) => {
                        error!("failed to lock UPDATE_MENU_ITEM for tray recreation");
                        return;
                    }
                };

                // Remove the old tray icon (must be on main thread for NSStatusBar)
                debug!("recreate_tray: removing old tray icon");
                let _old = app.remove_tray_by_id("dystil_main");
                // Drop the old tray icon explicitly on main thread
                drop(_old);
                debug!("recreate_tray: old tray removed, building new one");

                // Create a new tray icon — macOS assigns it the rightmost position
                let icon = crate::safe_icon::load_main_tray_icon(&app).ok();

                let mut builder = TrayIconBuilder::<Wry>::with_id("dystil_main")
                    .icon_as_template(false)
                    .show_menu_on_left_click(!cfg!(target_os = "windows"));

                if let Some(ref icon) = icon {
                    if icon.width() > 0 && icon.height() > 0 {
                        builder = builder.icon(icon.clone());
                    } else {
                        error!(
                            "tray icon has zero dimensions ({}x{}), skipping",
                            icon.width(),
                            icon.height()
                        );
                    }
                } else {
                    error!("failed to load tray icon for recreation");
                }

                debug!("recreate_tray: calling builder.build()");
                match builder.build(&app) {
                    Ok(new_tray) => {
                        debug!("recreate_tray: build succeeded, setting menu");
                        // Setup menu
                        let data = prefetch_tray_menu_data(&app);
                        if let Ok(menu) = create_dynamic_menu(
                            &app,
                            &MenuState::default(),
                            update_item.as_ref(),
                            &data,
                        ) {
                            let _ = install_tray_menu(&new_tray, menu);
                            clear_pending_tray_menu();
                        }
                        // NOTE: do NOT re-register click handlers here.
                        // The handler from setup_tray() is keyed by tray ID and persists
                        // across tray icon recreation. Re-registering causes double-firing.

                        info!("tray icon recreated at rightmost position");
                    }
                    Err(e) => {
                        error!("failed to recreate tray icon: {}", e);
                    }
                }
            }); // with_autorelease_pool
        })) {
            // The panic hook already sent the panic message + backtrace to Sentry
            // (as a Fatal-level capture_message). Log here for local diagnostics.
            let panic_msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                format!("{:?}", e)
            };
            error!(
                "panic caught in recreate_tray (ObjC exception?): {}",
                panic_msg
            );
        }
    });
}

/// Set autosaveName on the NSStatusItem so macOS remembers the user's
/// preferred position (after they Cmd+drag it out from behind the notch).
/// Uses Tauri's `with_inner_tray_icon` → `ns_status_item()` for direct access.
/// Set autosaveName on our NSStatusItem so macOS remembers user's Cmd+drag position.
/// Safe: wrapped in catch_unwind to prevent abort crashes.
#[cfg(target_os = "macos")]
fn set_autosave_name(_tray: &TrayIcon<Wry>) {
    // no-op for now: autosaveName through NSStatusBar iteration was crash-prone.
    // The tray icon position is handled by the recreate trick instead.
    // TODO: implement safely once we can reliably identify our NSStatusItem.
}

#[cfg(not(target_os = "macos"))]
fn set_autosave_name(_tray: &TrayIcon<Wry>) {}

fn create_dynamic_menu(
    app: &AppHandle,
    state: &MenuState,
    _update_item: Option<&tauri::menu::MenuItem<Wry>>,
    _data: &TrayMenuData,
) -> Result<tauri::menu::Menu<Wry>> {
    let capture_running = app
        .try_state::<RecordingState>()
        .map(|recording_state| recording_state.capture_active.load(Ordering::SeqCst))
        .unwrap_or_else(|| {
            let recording_status = state.recording_status.unwrap_or_else(get_recording_status);
            matches!(
                recording_status,
                RecordingStatus::Recording | RecordingStatus::Starting
            )
        });
    let capture_label = if capture_running { "Pause" } else { "Resume" };

    MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("open_app", "Open app").build(app)?)
        .item(&MenuItemBuilder::with_id("toggle_capture", capture_label).build(app)?)
        .build()
        .map_err(Into::into)
}

async fn handle_tray_capture_toggle(app_handle: AppHandle) {
    cancel_pause_timer();

    let state = app_handle.state::<RecordingState>();
    let is_recording = state.capture_active.load(Ordering::SeqCst);

    if is_recording {
        if let Err(e) = stop_capture(state, app_handle.clone()).await {
            error!("tray pause failed: {}", e);
        }
    } else {
        let server_running = state.server.lock().await.is_some();
        let result = if server_running {
            start_capture(state, app_handle.clone()).await
        } else {
            spawn_capture(state, app_handle.clone(), None).await
        };
        if let Err(e) = result {
            error!("tray resume failed: {}", e);
        }
    }
}

fn setup_tray_click_handlers(main_tray: &TrayIcon) -> Result<()> {
    main_tray.on_menu_event(move |app_handle, event| {
        // This runs inside tao::send_event (Obj-C FFI, nounwind). handle_menu_event
        // only clones and schedules work via run_on_main_thread — no heavy work here.
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle_menu_event(app_handle, event);
        })) {
            error!("panic in tray menu event handler: {:?}", e);
        }
    });

    #[cfg(target_os = "macos")]
    {
        main_tray.on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button_state: tauri::tray::MouseButtonState::Down,
                ..
            } = event
            {
                if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let app = tray.app_handle().clone();
                    if let Err(e) = apply_pending_tray_menu(&app) {
                        error!("failed to refresh tray menu before open: {}", e);
                    }
                })) {
                    error!(
                        "panic caught while refreshing tray menu before open: {:?}",
                        e
                    );
                }
            }
        });
    }

    // Windows: left-click opens the app (like macOS dock click), right-click shows menu
    #[cfg(target_os = "windows")]
    {
        main_tray.set_show_menu_on_left_click(false)?;
        main_tray.on_tray_icon_event(|tray, event| {
            // Fix for issue #2495: on_tray_icon_event fires INSIDE the tao Windows event
            // loop dispatcher (synchronously). Calling run_on_main_thread() directly from
            // here causes re-entrancy — tao panics at runner.rs:245 with:
            //   "either event handler is re-entrant (likely), or no event handler is registered"
            // Solution: wrap in catch_unwind for safety, and use async_runtime::spawn to
            // exit the tao callback context before dispatching work to the main thread.
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle().clone();
                    // ⚠️  Do NOT call run_on_main_thread() directly here — that would
                    // re-enter the tao event loop and trigger the panic.
                    // Instead: spawn onto tokio so we exit the tao callback first, then
                    // safely dispatch to the main thread from outside tao's dispatcher.
                    tauri::async_runtime::spawn(async move {
                        let app_inner = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            let _ = ShowRewindWindow::Home { page: None }.show(&app_inner);
                        });
                    });
                }
            })) {
                tracing::error!("panic caught in on_tray_icon_event (Windows): {:?}", e);
            }
        });
    }

    Ok(())
}

/// Tray menu handler runs inside tao::send_event (Obj-C FFI, nounwind). We must not
/// do any heavy or panicking work here — defer all window/show/open work to
/// run_on_main_thread so the sync path is minimal and panic-free.
fn handle_menu_event(app_handle: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        "show" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                show_main_window(app.clone());
                let _ = app.emit("tray-show-timeline", ());
            });
        }
        "show_search" => {
            // Show floating Search bar only (hide timeline, it reopens when user picks a result)
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                hide_main_window(app.clone());
                let _ = ShowRewindWindow::Search { query: None }.show(&app);
                let _ = app.emit("tray-show-search", ());
            });
        }
        "toggle_capture" => {
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                handle_tray_capture_toggle(app).await;
            });
        }
        id if id.starts_with("pause_") => {
            let mins: u64 = id
                .strip_prefix("pause_")
                .and_then(|s| s.parse().ok())
                .unwrap_or(15);
            let total = std::time::Duration::from_secs(mins * 60);
            // Cancel any in-flight pause timer before scheduling a new one.
            cancel_pause_timer();
            // Pause now (same path as the manual toggle).
            set_optimistic_status(RecordingStatus::Paused);
            let app_for_stop = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = app_for_stop.emit("shortcut-stop-recording", ());
            });
            // Schedule auto-resume — also fires a notification so the user knows
            // recording is back on without having to open the menu.
            let app_for_resume = app_handle.clone();
            let handle = tauri::async_runtime::spawn(async move {
                tokio::time::sleep(total).await;
                let _ = app_for_resume.emit("shortcut-start-recording", ());
                send_notify("Recording resumed", "dystil is recording again.");
            });
            *PAUSE_TIMER.lock().unwrap_or_else(|e| e.into_inner()) = Some(PauseTimer {
                handle,
                started: std::time::Instant::now(),
                total,
            });
            // Tell the user via a system notification (the tray icon doesn't
            // visually change between recording / paused, so the menubar gives
            // no glance-level signal otherwise).
            let pretty = if mins >= 60 {
                let h = mins / 60;
                if h == 1 {
                    "1 hour".to_string()
                } else {
                    format!("{} hours", h)
                }
            } else {
                format!("{} minutes", mins)
            };
            send_notify(
                "Recording paused",
                format!("dystil will auto-resume in {}.", pretty),
            );
            // Repaint the tray so "Recording" flips to "Paused" immediately.
            let app_for_rebuild = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                if let Err(e) = force_tray_rebuild(&app_for_rebuild) {
                    error!("tray rebuild failed: {}", e);
                }
            });
        }
        "lock_vault" => {
            let _ = app_handle.emit("vault-lock-requested", ());
        }
        "fix_permissions" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::PermissionRecovery.show(&app);
            });
        }
        "check_permissions" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::PermissionRecovery.show(&app);
            });
        }
        "upgrade" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::Home {
                    page: Some("account".to_string()),
                }
                .show(&app);
                let _ = app.emit("tray-upgrade", ());
            });
        }
        "releases" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = app
                    .opener()
                    .open_url("https://dystil.app/changelog", None::<&str>);
            });
        }
        "open_app" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::Home { page: None }.show(&app);
            });
        }
        "settings" => {
            let app = app_handle.clone();
            let page = Some("general".to_string());
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::Home { page }.show(&app);
            });
        }
        "feedback" => {
            let app = app_handle.clone();
            let page = Some("help".to_string());
            let _ = app_handle.run_on_main_thread(move || {
                let _ = ShowRewindWindow::Home { page }.show(&app);
            });
        }
        "book_call" => {
            let app = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = app
                    .opener()
                    .open_url("https://cal.com/team/dystil/chat", None::<&str>);
            });
        }
        "quit" => {
            debug!("Quit requested");

            // Signal that this is an intentional quit so the ExitRequested
            // handler in main.rs won't prevent it.
            QUIT_REQUESTED.store(true, Ordering::SeqCst);

            // Stop recording before exiting
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                info!("Stopping dystil recording before quit...");
                if let Some(recording_state) = app_handle_clone.try_state::<RecordingState>() {
                    // Stop capture first (self-contained)
                    recording_state
                        .capture_active
                        .store(false, Ordering::SeqCst);
                    if let Some(session) = recording_state.capture.lock().await.take() {
                        session.stop().await;
                    }
                    // Then shutdown server
                    if let Some(server) = recording_state.server.lock().await.take() {
                        server.shutdown().await;
                    }
                    info!("Dystil server + recording stopped successfully");
                }
                info!("All tasks stopped, exiting process");
                // Use _exit() instead of exit() to skip C++ atexit/static destructors.
                // Native GPU contexts can register destructors that assert during teardown.
                // We've already done our own cleanup above, so atexit handlers have
                // nothing useful left to do.
                #[cfg(unix)]
                {
                    extern "C" {
                        fn _exit(status: i32) -> !;
                    }
                    unsafe {
                        _exit(0);
                    }
                }
                #[cfg(not(unix))]
                app_handle_clone.exit(0);
            });
        }
        _ => debug!("Unhandled menu event: {:?}", event.id()),
    }
}

#[allow(dead_code)]
async fn update_menu_if_needed(
    app: &AppHandle,
    update_item: &tauri::menu::MenuItem<Wry>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    let _ = update_item;

    // Pre-fetch all data on the tokio thread (off main thread) so the
    // main-thread closure only does lightweight menu-item construction.
    let data = prefetch_tray_menu_data(app);

    let recording_info = get_recording_info();
    let effective_status = get_effective_recording_status();
    let new_state = MenuState {
        recording_status: Some(effective_status),
        onboarding_completed: data.onboarding_completed,
        has_permission_issue: data.has_permission_issue,
        devices: recording_info
            .devices
            .iter()
            .map(|d| (d.name.clone(), d.active))
            .collect(),
        is_logged_in: data.is_logged_in,
    };

    // Compare with last state (poison-safe: run handler must not panic)
    let should_update = {
        let mut last_state = LAST_MENU_STATE.lock().unwrap_or_else(|e| e.into_inner());
        if *last_state != new_state {
            *last_state = new_state.clone();
            true
        } else {
            false
        }
    };

    // Tooltip refreshes every tick regardless of menu rebuild — countdown
    // ("paused, resumes in 12m") needs to tick down even when no other state
    // has changed. Cheap: just an NSString swap on the existing status item.
    let has_perm_issue = new_state.has_permission_issue;
    let tooltip: String = if has_perm_issue {
        "dystil — ⚠️ permissions needed".to_string()
    } else if effective_status == RecordingStatus::Paused {
        match pause_remaining() {
            Some(d) => format!("dystil — paused, resumes in {}", format_remaining(d)),
            None => "dystil — paused".to_string(),
        }
    } else if effective_status == RecordingStatus::ScheduledPause {
        "dystil — outside work hours (paused by schedule)".to_string()
    } else {
        "dystil".to_string()
    };
    let app_for_tooltip = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = app_for_tooltip.tray_by_id("dystil_main") {
            let _ = tray.set_tooltip(Some(&tooltip));
        }
    });

    if should_update {
        #[cfg(target_os = "macos")]
        {
            queue_pending_tray_menu(new_state, data);
            debug!("tray_menu_update: queued menu refresh for next open");
        }

        #[cfg(not(target_os = "macos"))]
        {
            // IMPORTANT: All NSStatusItem/TrayIcon operations must happen on the main thread.
            // If the TrayIcon is dropped on a tokio thread (e.g., after recreate_tray removed
            // the old one from the manager), NSStatusBar _removeStatusItem fires on the wrong
            // thread and crashes.
            let app_for_thread = app.clone();
            let update_item = update_item.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(tray) = app_for_thread.tray_by_id("dystil_main") {
                        debug!("tray_menu_update: setting menu");
                        if let Ok(menu) = create_dynamic_menu(
                            &app_for_thread,
                            &new_state,
                            Some(&update_item),
                            &data,
                        ) {
                            let _ = install_tray_menu(&tray, menu);
                        }
                    }
                })) {
                    let panic_msg = if let Some(s) = e.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = e.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        format!("{:?}", e)
                    };
                    error!(
                        "panic caught in tray menu update (ObjC exception?): {}",
                        panic_msg
                    );
                }
            });
        }
    }

    Ok(())
}
