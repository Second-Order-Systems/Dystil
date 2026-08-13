use crate::store::SettingsStore;
use serde::Serialize;
use specta::Type;
use std::sync::RwLock;
use tauri::Emitter;

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
use log::{debug, error, info, warn};

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
use std::time::Duration;

#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
use crate::{stop_capture, tray::QUIT_REQUESTED, RecordingState};
#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
use std::sync::atomic::Ordering;
#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
use tauri::Manager;
#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
use tauri_plugin_updater::UpdaterExt;
#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
use tokio::time::interval;

#[cfg(all(target_os = "windows", feature = "windows-store"))]
use crate::{recording::stop_engine, tray::QUIT_REQUESTED, RecordingState};
#[cfg(all(target_os = "windows", feature = "windows-store"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(target_os = "windows", feature = "windows-store"))]
use windows::Foundation::Collections::IVectorView;
#[cfg(all(target_os = "windows", feature = "windows-store"))]
use windows::Services::Store::{StoreContext, StorePackageUpdate, StorePackageUpdateState};
#[cfg(all(target_os = "windows", feature = "windows-store"))]
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

const UPDATER_AVAILABLE: bool = cfg!(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
));

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
const AUTO_UPDATE_GATE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static AVAILABLE_VERSION: RwLock<Option<String>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateSettingsView {
    pub auto_update: bool,
    pub updater_available: bool,
    pub available_version: Option<String>,
}

fn effective_auto_update(app: &tauri::AppHandle) -> Result<bool, String> {
    if cfg!(feature = "enterprise-client") {
        return Ok(true);
    }
    Ok(SettingsStore::get(app)?.unwrap_or_default().auto_update)
}

fn update_settings(app: &tauri::AppHandle) -> Result<AppUpdateSettingsView, String> {
    Ok(AppUpdateSettingsView {
        auto_update: effective_auto_update(app)?,
        updater_available: UPDATER_AVAILABLE,
        available_version: AVAILABLE_VERSION
            .read()
            .map_err(|_| "update state is unavailable".to_string())?
            .clone(),
    })
}

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
fn publish_available_version(app: &tauri::AppHandle, version: Option<String>) {
    if let Ok(mut current) = AVAILABLE_VERSION.write() {
        if *current == version {
            return;
        }
        *current = version;
    }
    if let Ok(view) = update_settings(app) {
        let _ = app.emit("app-update-state-changed", view);
    }
}

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
fn automatic_updates_enabled(app: &tauri::AppHandle) -> bool {
    update_settings(app)
        .map(|settings| settings.updater_available && settings.auto_update)
        .unwrap_or(false)
}

#[cfg(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
))]
async fn await_restart_gate(timeout: Duration, label: &str) -> bool {
    let outcome = crate::health::wait_for_boot_ready(timeout).await;
    match outcome {
        crate::health::BootReadiness::Ready => true,
        crate::health::BootReadiness::Errored => {
            warn!("{}: boot phase is 'error' — deferring restart", label);
            false
        }
        crate::health::BootReadiness::Pending => {
            warn!(
                "{}: boot phase still pending after {}s — deferring restart",
                label,
                timeout.as_secs()
            );
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Direct-download updater (all official builds except Windows Store/MSIX).
// ---------------------------------------------------------------------------

#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
async fn install_and_restart(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<(), Box<dyn std::error::Error>> {
    if !await_restart_gate(AUTO_UPDATE_GATE_TIMEOUT, "app-update").await {
        return Err(std::io::Error::other(
            "Dystil is still finishing startup. Try the update again shortly.",
        )
        .into());
    }
    let retry_delays = [
        Duration::from_secs(30),
        Duration::from_secs(120),
        Duration::from_secs(300),
    ];
    let mut attempt = 0;
    loop {
        let result = update.download_and_install(|_, _| {}, || {}).await;
        match result {
            Ok(()) => break,
            Err(error) => {
                let Some(delay) = retry_delays.get(attempt).copied() else {
                    error!(
                        "update download failed after {} attempts: {}",
                        attempt + 1,
                        error
                    );
                    return Err(error.into());
                };
                warn!(
                    "download attempt {} failed: {} — retrying in {}s",
                    attempt + 1,
                    error,
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }

    info!("update downloaded, preparing restart...");
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(
        Duration::from_secs(15),
        stop_capture(app.state::<RecordingState>(), app.clone()),
    )
    .await;
    app.restart();
}

#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
async fn check_for_updates(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    info!("checking for updates...");
    let check_result = app.updater_builder().build()?.check().await;

    match check_result {
        Ok(Some(update)) => {
            info!("update found: v{}", update.version);
            if automatic_updates_enabled(app) {
                publish_available_version(app, None);
                install_and_restart(app, update).await?;
            } else {
                publish_available_version(app, Some(update.version));
            }
            Ok(())
        }
        Ok(None) => {
            debug!("no update available");
            publish_available_version(app, None);
            Ok(())
        }
        Err(error) => {
            warn!("update check failed: {}", error);
            Err(error.into())
        }
    }
}

// ---------------------------------------------------------------------------
// Microsoft Store/MSIX updater. The silent download occurs while Dystil is
// running; only the install phase enters maintenance mode and releases files.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "windows", feature = "windows-store"))]
static STORE_DOWNLOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[cfg(all(target_os = "windows", feature = "windows-store"))]
enum StoreDownloadOutcome {
    NoUpdate,
    SilentUpdatesUnavailable { version: String },
    Downloaded { version: String },
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
fn store_update_version(updates: &IVectorView<StorePackageUpdate>) -> Result<String, String> {
    let update = updates.GetAt(0).map_err(|error| error.to_string())?;
    let version = update
        .Package()
        .and_then(|package| package.Id())
        .and_then(|package_id| package_id.Version())
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "{}.{}.{}.{}",
        version.Major, version.Minor, version.Build, version.Revision
    ))
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
fn with_store_apartment<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() };
    let result = operation();
    if initialized {
        unsafe { RoUninitialize() };
    }
    result
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
fn download_store_update_blocking() -> Result<StoreDownloadOutcome, String> {
    with_store_apartment(|| {
        let context = StoreContext::GetDefault().map_err(|error| error.to_string())?;
        let updates = context
            .GetAppAndOptionalStorePackageUpdatesAsync()
            .and_then(|operation| operation.get())
            .map_err(|error| error.to_string())?;

        if updates.Size().map_err(|error| error.to_string())? == 0 {
            return Ok(StoreDownloadOutcome::NoUpdate);
        }

        let version = store_update_version(&updates)?;
        if !context
            .CanSilentlyDownloadStorePackageUpdates()
            .map_err(|error| error.to_string())?
        {
            return Ok(StoreDownloadOutcome::SilentUpdatesUnavailable { version });
        }

        let result = context
            .TrySilentDownloadStorePackageUpdatesAsync(&updates)
            .and_then(|operation| operation.get())
            .map_err(|error| error.to_string())?;
        let state = result.OverallState().map_err(|error| error.to_string())?;
        if state != StorePackageUpdateState::Completed {
            return Err(format!(
                "silent Store download finished with state {state:?}"
            ));
        }

        Ok(StoreDownloadOutcome::Downloaded { version })
    })
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
fn install_store_update_blocking() -> Result<(), String> {
    with_store_apartment(|| {
        // Re-query on this worker thread instead of moving WinRT interfaces
        // between threads. The Store keeps a successfully downloaded update in
        // its queue, so this is an install-only operation in the normal case.
        let context = StoreContext::GetDefault().map_err(|error| error.to_string())?;
        let updates = context
            .GetAppAndOptionalStorePackageUpdatesAsync()
            .and_then(|operation| operation.get())
            .map_err(|error| error.to_string())?;
        if updates.Size().map_err(|error| error.to_string())? == 0 {
            return Err("Microsoft Store no longer reports an update to install".to_string());
        }
        let result = context
            .TrySilentDownloadAndInstallStorePackageUpdatesAsync(&updates)
            .and_then(|operation| operation.get())
            .map_err(|error| error.to_string())?;
        let state = result.OverallState().map_err(|error| error.to_string())?;
        if state == StorePackageUpdateState::Completed {
            Ok(())
        } else {
            Err(format!(
                "silent Store installation finished with state {state:?}"
            ))
        }
    })
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
async fn download_store_update(app: &tauri::AppHandle) -> Result<bool, String> {
    let outcome = tokio::task::spawn_blocking(download_store_update_blocking)
        .await
        .map_err(|error| format!("Store update worker failed: {error}"))??;

    match outcome {
        StoreDownloadOutcome::NoUpdate => {
            info!("Microsoft Store update check completed: no update available");
            publish_available_version(app, None);
            Ok(false)
        }
        StoreDownloadOutcome::SilentUpdatesUnavailable { version } => {
            // Keep the version visible for diagnostics, but do not show a consent
            // dialog or override the user's Store/metered-network policy.
            warn!(
                "Microsoft Store update v{} is available, but silent download is unavailable",
                version
            );
            publish_available_version(app, Some(version));
            Ok(false)
        }
        StoreDownloadOutcome::Downloaded { version } => {
            info!("Microsoft Store update v{} downloaded", version);
            publish_available_version(app, Some(version));
            Ok(true)
        }
    }
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
async fn install_downloaded_store_update(app: &tauri::AppHandle) -> Result<(), String> {
    if !await_restart_gate(AUTO_UPDATE_GATE_TIMEOUT, "store-update").await {
        return Err("Dystil is still finishing startup; deferring Store installation.".to_string());
    }

    // The update is already downloaded. From this point onwards Dystil must
    // release all package resources before asking the Store deployment service
    // to replace the MSIX files.
    info!("preparing Dystil for Microsoft Store installation");
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
    let shutdown = tokio::time::timeout(
        Duration::from_secs(25),
        stop_engine(app.state::<RecordingState>(), app.clone()),
    )
    .await;
    match shutdown {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            QUIT_REQUESTED.store(false, Ordering::SeqCst);
            return Err(format!("failed to prepare for Store installation: {error}"));
        }
        Err(_) => {
            QUIT_REQUESTED.store(false, Ordering::SeqCst);
            return Err("timed out preparing for Store installation".to_string());
        }
    }

    // Store servicing normally ends this process. If it returns an error after
    // we stopped the runtime, restart the current package to restore capture.
    let install_result = tokio::task::spawn_blocking(install_store_update_blocking)
        .await
        .map_err(|error| format!("Store installation worker failed: {error}"))?;
    if let Err(error) = install_result {
        error!(
            "Microsoft Store installation did not start; restarting current app: {}",
            error
        );
        app.restart();
    }

    info!("Microsoft Store installation completed while Dystil is still running");
    Ok(())
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
async fn check_for_store_updates(app: &tauri::AppHandle, force: bool) -> Result<(), String> {
    if !force && !automatic_updates_enabled(app) {
        debug!("automatic Microsoft Store updates are disabled in Dystil settings");
        return Ok(());
    }
    if STORE_DOWNLOAD_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        debug!("Microsoft Store update check already in progress");
        return Ok(());
    }
    struct ResetUpdateCheck;
    impl Drop for ResetUpdateCheck {
        fn drop(&mut self) {
            STORE_DOWNLOAD_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
    }
    let _reset = ResetUpdateCheck;

    info!("checking Microsoft Store for package updates");
    if !download_store_update(app).await? {
        return Ok(());
    }
    install_downloaded_store_update(app).await
}

#[tauri::command]
#[specta::specta]
pub fn get_app_update_settings(app: tauri::AppHandle) -> Result<AppUpdateSettingsView, String> {
    update_settings(&app)
}

#[tauri::command]
#[specta::specta]
pub fn set_app_auto_update(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<AppUpdateSettingsView, String> {
    let mut settings = SettingsStore::get(&app)?.unwrap_or_default();
    settings.auto_update = if cfg!(feature = "enterprise-client") {
        true
    } else {
        enabled
    };
    settings.save(&app)?;

    #[cfg(all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ))]
    if enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = check_for_updates(&app).await {
                warn!("update check after enabling automatic updates failed: {error}");
            }
        });
    }

    #[cfg(all(target_os = "windows", feature = "windows-store"))]
    if effective_auto_update(&app)? {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = check_for_store_updates(&app, false).await {
                warn!("Store update check after enabling automatic updates failed: {error}");
            }
        });
    }

    let view = update_settings(&app)?;
    let _ = app.emit("app-update-state-changed", view.clone());
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(all(target_os = "windows", feature = "windows-store"))]
    {
        return check_for_store_updates(&app, true).await;
    }

    #[cfg(all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ))]
    {
        let update = app
            .updater_builder()
            .build()
            .map_err(|error| error.to_string())?
            .check()
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Dystil is already up to date.".to_string())?;
        publish_available_version(&app, None);
        return install_and_restart(&app, update)
            .await
            .map_err(|error| error.to_string());
    }

    #[cfg(not(any(
        all(target_os = "windows", feature = "windows-store"),
        all(
            feature = "official-build",
            not(all(target_os = "windows", feature = "windows-store"))
        )
    )))]
    {
        let _ = app;
        Err("Updates are not available in this build of Dystil.".to_string())
    }
}

#[cfg(all(
    feature = "official-build",
    not(all(target_os = "windows", feature = "windows-store"))
))]
pub fn start_update_check(app: &tauri::AppHandle, interval_minutes: u64) {
    let app_handle = app.clone();
    tokio::spawn(async move {
        if let Err(error) = check_for_updates(&app_handle).await {
            warn!("boot update check failed: {error}");
        }
    });

    let app_handle = app.clone();
    tokio::spawn(async move {
        let check_interval = Duration::from_secs(interval_minutes * 60);
        let mut interval = interval(check_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = check_for_updates(&app_handle).await {
                warn!("periodic update check failed: {error}");
            }
        }
    });
}

#[cfg(all(target_os = "windows", feature = "windows-store"))]
pub fn start_update_check(app: &tauri::AppHandle, _interval_minutes: u64) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Application Restart only guarantees automatic restart after a process
        // has been running for at least 60 seconds. Delay the first possible
        // install beyond that point.
        tokio::time::sleep(Duration::from_secs(65)).await;
        if let Err(error) = check_for_store_updates(&app_handle, false).await {
            warn!("initial Microsoft Store update check failed: {error}");
        }

        let mut interval = tokio::time::interval(Duration::from_secs(6 * 60 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = check_for_store_updates(&app_handle, false).await {
                warn!("periodic Microsoft Store update check failed: {error}");
            }
        }
    });
}

#[cfg(not(any(
    all(
        feature = "official-build",
        not(all(target_os = "windows", feature = "windows-store"))
    ),
    all(target_os = "windows", feature = "windows-store")
)))]
pub fn start_update_check(_app: &tauri::AppHandle, _interval_minutes: u64) {}
