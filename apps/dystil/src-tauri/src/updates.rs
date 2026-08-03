use crate::store::SettingsStore;
#[cfg(feature = "official-build")]
use crate::{stop_capture, tray::QUIT_REQUESTED, RecordingState};
#[cfg(feature = "official-build")]
use log::{debug, error, info, warn};
use serde::Serialize;
use specta::Type;
#[cfg(feature = "official-build")]
use std::sync::atomic::Ordering;
use std::sync::RwLock;
#[cfg(feature = "official-build")]
use std::time::Duration;
use tauri::Emitter;
#[cfg(feature = "official-build")]
use tauri::Manager;
#[cfg(feature = "official-build")]
use tauri_plugin_updater::UpdaterExt;
#[cfg(feature = "official-build")]
use tokio::time::interval;

#[cfg(feature = "official-build")]
const AUTO_UPDATE_GATE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
static AVAILABLE_VERSION: RwLock<Option<String>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateSettingsView {
    pub auto_update: bool,
    pub updater_available: bool,
    pub available_version: Option<String>,
}

fn update_settings(app: &tauri::AppHandle) -> Result<AppUpdateSettingsView, String> {
    Ok(AppUpdateSettingsView {
        auto_update: SettingsStore::get(app)?.unwrap_or_default().auto_update,
        updater_available: cfg!(feature = "official-build"),
        available_version: AVAILABLE_VERSION
            .read()
            .map_err(|_| "update state is unavailable".to_string())?
            .clone(),
    })
}

#[cfg(feature = "official-build")]
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

#[cfg(feature = "official-build")]
fn automatic_updates_enabled(app: &tauri::AppHandle) -> bool {
    update_settings(app)
        .map(|settings| settings.updater_available && settings.auto_update)
        .unwrap_or(false)
}

#[cfg(feature = "official-build")]
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

#[cfg(feature = "official-build")]
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

#[cfg(feature = "official-build")]
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
        Err(e) => {
            warn!("update check failed: {}", e);
            Err(e.into())
        }
    }
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
    settings.auto_update = enabled;
    settings.save(&app)?;

    #[cfg(feature = "official-build")]
    if enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = check_for_updates(&app).await {
                warn!("update check after enabling automatic updates failed: {error}");
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
    #[cfg(not(feature = "official-build"))]
    {
        let _ = app;
        return Err("Updates are not available in this build of Dystil.".to_string());
    }

    #[cfg(feature = "official-build")]
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
        install_and_restart(&app, update)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(feature = "official-build")]
pub fn start_update_check(app: &tauri::AppHandle, interval_minutes: u64) {
    let app_handle = app.clone();

    tokio::spawn(async move {
        if let Err(e) = check_for_updates(&app_handle).await {
            warn!("boot update check failed: {}", e);
        }
    });

    let app_handle = app.clone();
    tokio::spawn(async move {
        let check_interval = Duration::from_secs(interval_minutes * 60);
        let mut interval = interval(check_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = check_for_updates(&app_handle).await {
                warn!("periodic update check failed: {}", e);
            }
        }
    });
}
